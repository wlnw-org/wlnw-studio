# Spike: BirdNET in Rust + ONNX Runtime (7 November 2026)

Question: can a Rust program run BirdNET V2.4 on CPU at 100x real time or better, under 2 GB of memory, with the same detections as BirdNET-Analyzer? **Yes.**

## Method
- BirdNET V2.4 FP32 (from birdnet-analyzer 2.x). Direct TFLite to ONNX conversion fails (tf2onnx cannot convert the RFFT2D in the model's two built-in mel spectrogram layers).
- So the model is split: the two spectrogram layers are reimplemented in Rust (min-max normalisation to [-1, 1]; Hann window; frame 2048 hop 278 and frame 1024 hop 280; real FFT; the model casts the complex STFT to float, so only the real part is kept; sparse mel filter banks copied from the model, 96 bands; square then power p; mel axis reversed), and the CNN from `model/concatenate/concat` (1, 96, 511, 2) to the 6522 logits is converted to ONNX (opset 17). Constants are exported to `models/` by `spike/mel.py`.
- Confidence = sigmoid(clip(logit, -20, 20)), as BirdNET-Analyzer with sensitivity 1.0.

## Results (Apple M3, 8 cores, 8 GB, no GPU)
| Check | Result |
|---|---|
| Mel spectrogram vs TFLite intermediate tensor | max abs difference 0.0 (Python), < 0.0007 (Rust, f32) |
| CNN in ONNX vs TFLite logits, 89 chunks | max difference 3e-5 |
| Detections vs BirdNET-Analyzer, 4.5 min real recordings, conf >= 0.1 | 82 of 82 identical (species and time), confidence within 0.006 |
| 24 h of 48 kHz mono WAV (24 one-hour files) | 247 s wall time = **350x real time**; spectrograms 27 s, network 158 s |
| Peak memory | 1.07 GB (each one-hour file is loaded whole; streaming will lower it) |

Derived: 1 TB of 48 kHz 16-bit mono WAV = 1e12 / 96,000 B/s = 2,894 h of audio; at 350x that is about 8.3 h on this laptop. Slower Windows laptops: assumed 2 to 4 times slower, so 17 to 33 h per TB (not measured).

## Gotchas
- ort (Rust) with the `api-22` feature sets an EP selection policy of "max efficiency" by default; on macOS this gave wrong logits (differences up to 14). Build ort without `api-22` (or set the policy explicitly to CPU). Studio must test logits against a reference file in CI.
- The remaining confidence difference (< 0.006) is probably BirdNET-Analyzer's bandpass filter and audio loading; not yet replicated.
- The test audio is 4.5 min of real clips looped to 24 h, read from the OS cache; disk speed was not the limit (345 MB per hour of audio).

## Licence
BirdNET V2.4 model weights are CC BY-NC-SA 4.0. The converted CNN and extracted constants are a derivative and must stay under the same licence; they are not committed here (models/ is git-ignored). Ask the BirdNET team before distributing them with the app.

## Perch 2.0 (added 7 November 2026)
- Licence: Apache 2.0, confirmed on Kaggle's model page (every Perch variant) and the perch-hoplite repository.
- ONNX file: Justin Chu's conversion (via huggingface.co/Calotriton/perch-v2-onnx, SHA-256 bf0c8467...cefa1f). Checked here against Google's official `perch_v2_cpu` TensorFlow model on 53 windows of real recordings: logits within 1.8e-4, embeddings within 1.0e-5. Labels identical to Kaggle's.
- Input 5 s at 32 kHz; per window: remove the mean, scale the peak to 0.25. Probabilities: softmax over 14,795 classes.
- Speed: 65x real time on the M3 (batches of 16), five times slower than BirdNET: about 45 h per TB on this laptop.
- Embeddings (1,536 per window) are stored as half floats next to the project (3 KB per 5 s window, about 6.4 GB per TB of audio) for the Wildlife Network Classifier and "find more like this".

## BatDetect2 (added 8 November 2026)
- Model: BatDetect2's default UK model from batdetect2 2.0.0b3 (checkpoint `batdetect2_uk_same.ckpt`, SHA-256 52e3f329...6adcf, identical in the GitHub repo at commit 1920373 and on PyPI). 17 UK bat species, one detection per echolocation call with a box (start, end, low and high frequency).
- Licence: the whole repository, code and weights, is CC BY-NC 4.0 (LICENSE.md, pyproject, and the FAQ: "currently for non-commercial use"). No ShareAlike, so redistribution is allowed for non-commercial use with credit, but no converted copy is published anywhere, so the pack is not downloadable: users make it with `spike/batdetect2_export.py` (ONNX, opset 17, 7.6 MB; plus `batdetect2.json` and `labels.txt`) and add it from a folder. Many UK bat surveys are commercial (planning consultancy): they may not use this model.
- Split as for BirdNET: only the network (U-Net with self-attention, input 1 x 128 x 500 per 0.5 s clip) is in ONNX. Rust (`crates/engine/src/batdetect.rs`) does the rest exactly as BatDetect2: recordings cut into 0.5 s clips at their own rate (no overlap, a last shorter clip dropped), each clip resampled to 256 kHz on its own with scipy's `resample_poly` (Kaiser beta 5, half length 10 x max(up, down), zero padding; reimplemented), STFT with a periodic Hann window of 512 samples, hop 128, centred with reflection, magnitude of bins 20 to 239 (10 to 120 kHz), BatDetect2's PCEN (input x 2^31, IIR smoother s = 0.0488, gain 0.98, bias 2, power 0.5), spectral mean subtraction, bilinear resize to 128 x 500 (align_corners false). After the network: 9 x 9 max-pool peak picking on the detection map, the 50 strongest peaks per clip at or above 0.01, box from the size map (bottom-left anchor, width / 1000 s, height / 0.0011636 Hz), class = highest of the 17 class maps. Studio stores one detection per call: top species and its score (BatDetect2's `class_prob`), at or above 0.1.
- Time expansion: GUANO `TE` is read; the recording is treated as header rate x TE, and times are stored in the file's own seconds. Checked with Echometer.wav rewritten as 25.6 kHz with `TE: 10`: the same 51 calls, times exactly 10 x.
- Default model: a recorder whose first recording is at 192 kHz or more (after time expansion) gets BatDetect2 when installed, otherwise the bird models as before.

| Check | Result |
|---|---|
| ONNX vs PyTorch network, 16 clips | max difference 1.1e-6 (detection and class maps), 1.4e-4 (size map, values up to tens) |
| Rust resampler vs scipy, 192, 312.5, 384 and 500 kHz | max difference 1.2e-7 |
| Rust spectrogram vs BatDetect2 preprocessor, each stage | relative difference under 6e-6 (unit tests, `tests/fixtures/batdetect2_preprocess.json`) |
| Calls vs `BatDetect2API.process_file` (CPU), 10 recordings shipped in the repo: 500, 384 (2), 192, 312.5 (5) and 256 kHz, 30.6 s | all 3,000 calls at detection >= 0.01 matched (697 at >= 0.1, 283 at >= 0.3): same start and end time (difference 0), same top species on every call, detection and class scores within 7e-5 |
| Speed, Apple M3, 8 cores | 7.4x real time (30 s of audio in 4.05 s); the network alone runs at about 8x in ONNX Runtime, so it sets the pace. Python reference: about 1.4x. One hour of 256 kHz audio takes about 8 minutes |

Gotchas
- Resampling the whole recording at once (Studio's rubato stream) instead of clip by clip changed detection scores by up to 0.22 at clip edges and on upsampled 192 kHz audio, and rubato's start delay shifted times; matching BatDetect2 needs its own per-clip polyphase resampling. BirdNET and Perch still use the rubato stream (unchanged).
- The bundled example audio (example_data, tests/data) is CC BY-NC like the rest of the repository: used for checks only, never committed here.
- Review clips and spectrograms in the app are drawn at 48 kHz, which cannot show a bat call; bat review needs its own clip rate (not done yet).

## Camera traps: SpeciesNet with MegaDetector v6 MIT (added 8 November 2026)
- Licences (read from the sources on 8 November 2026). SpeciesNet: code Apache 2.0 (github.com/google/cameratrapai LICENSE and pyproject), weights Apache 2.0 (Kaggle's model API gives `licenseName: Apache 2.0` for every variant, v4.0.0a to v4.0.3b). SpeciesNet's own ensemble runs MegaDetector v5a, a YOLOv5 model made with Ultralytics' AGPL code, so it is not used. Instead: MegaDetector v6 MIT, `MDV6-mit-yolov9-c`, Pytorch-Wildlife's default MIT detector, trained with MultimediaTechLab's MIT YOLO code; the Pytorch-Wildlife code is MIT and the weights' Zenodo record (15398270) is CC BY 4.0. All three allow redistribution with credit (NOTICE). Pytorch-Wildlife's Apache RT-DETR variants are 322 MB and 1.2 GB, too big for a laptop download.
- Files: SpeciesNet v4.0.3a "always crop" from Kaggle (pinned version 1; classifier, labels, taxonomy and geofence dated 2026-06-09) and `MDV6-mit-yolov9-c.ckpt` (md5 d8626ba22fccbd9c520e0deb19d721f7) with `config_v9s.yaml`. `spike/speciesnet_export.py` writes `models/speciesnet/`: `detector.onnx` (29 MB, network plus YOLOv9's box decoding, sigmoid scores), `speciesnet_v403a.onnx` (224 MB, EfficientNetV2-M, NHWC 480 x 480), labels, taxonomy, geofence and `speciesnet.json`. No converted copy is published, so the pack is added from a folder (licences would allow hosting it).
- Pipeline (crates/engine/src/camtrap.rs), reproducing the Python reference step by step: image read as speciesnet's `load_rgb_image` (JPEG through libjpeg-turbo's decoder via mozjpeg, CMYK with Pillow's conversion, EXIF orientation applied); detector input as Pytorch-Wildlife's PadAndResize (Pillow's Lanczos resample reimplemented in fixed point, fit into 640 x 640, grey 114 borders); YOLOv9 post-processing (every anchor and class above 0.01, NMS per class at IoU 0.5, at most 300); boxes mapped back, clipped to the image, slivers under 2 px dropped; every box at 0.1 or more classified as speciesnet does it (torchvision crop of the int-scaled box, bilinear resize to 480 x 480 without antialiasing, 8-bit round trip, softmax, top 5); then SpeciesNet's ensemble per box: detector-first rules for people and vehicles, blank rules, geofencing by the deployment's country (and US state) with roll-up to family, order, class, then taxonomic roll-up at 0.65 and the "animal" fallback.
- Geofencing needs the country as a three-letter code: set per camera on the Recordings screen (Studio does not guess it from coordinates, as speciesnet does not either).
- Studio keeps one row per classified box (kind animal, human, vehicle, blank or unknown) and reviews boxes like sound detections, with a fitted threshold per species. Images with a person (a human box not rejected in review) are never exported or published; vehicles and blanks only appear in Camtrap DP.

| Check (15 sample images shipped with speciesnet, CDLA-Permissive) | Result |
|---|---|
| ONNX vs PyTorch, same input | detector scores within 2.2e-5, boxes within 0.016 px (of 640); classifier logits within 3.2e-5 |
| Decoded pixels vs Pillow (RGB, CMYK, greyscale, EXIF-rotated JPEGs) | identical (0 of 9.4 M values differ) |
| Detector input vs Pytorch-Wildlife's transform | identical (0 of 1.2 M values) |
| First crop vs speciesnet's preprocessing | 6 of 691,200 values differ by 1 level (float rounding in the bilinear step) |
| Boxes and labels vs the Python pipeline (spike/speciesnet_check.py), country USA and no country | all 36 classified boxes matched, none missing or extra: min IoU 1.0000, detector confidence within 2.3e-6, same final label on 36 of 36 (geofencing turns African elephants into "Mammalia" in the USA, as in Python), ensemble score within 1.2e-5 |
| Speed, Apple M3, 8 cores, CPU (the Mac was busy with other work: load average 6) | 0.74 images per second on the samples (2.4 classified boxes per image). Per image: reading 0.05 s, detector 0.22 s; classifier 0.44 s per box. So about 3.5 images per second when nothing is found, 1.4 with one animal. The Python reference (PyTorch, same machine): detector 0.22 s, classifier 0.36 s per box |

Gotchas
- JPEG decoders differ: the `image` crate's decoder (zune-jpeg) gave pixels up to 4 levels off Pillow's, which moved detector confidences by up to 0.047 and scores by 0.028. libjpeg-turbo's decoder (mozjpeg crate, C, no SIMD) gives Pillow's pixels exactly. The faster decoder is still used for review thumbnails.
- MegaDetector v6's confidences are calibrated differently from v5a's, and SpeciesNet's ensemble thresholds (0.2, 0.5, 0.7 on the detector) were set with v5a. The check above proves Studio matches the Python pipeline with v6; it does not measure how v6 changes SpeciesNet's accuracy (not evaluated: needs a labelled camera-trap set).
- Run it: `python spike/speciesnet_check.py ... --out reference.json` then `cargo run --release -p wlnw-engine --example camtrap_check -- models/speciesnet IMAGES reference.json USA`.
