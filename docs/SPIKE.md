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
