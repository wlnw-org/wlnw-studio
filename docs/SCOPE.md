# Studio: scope for a desktop app to process, review and publish wildlife recordings

Status: draft scope, 2026-11-07. Working name "Studio". Nothing here is built yet.

Claim labels used below: **[obs]** observed, with a source; **[der]** derived, with the arithmetic shown; **[asm]** assumed, to be tested.

## 1. Summary

Studio is a free, open-source desktop app for Windows and macOS. A field researcher points it at a drive full of recordings (terabytes from Song Meters or AudioMoths), and it runs BirdNET, Perch and BatDetect2 over everything, locally and offline, surviving crashes and restarts. It then makes review fast: it picks the few clips per species that decide whether a detection is real, learns a per-species threshold from those verdicts, and propagates a verdict to similar-sounding clips. Finished results export as a Darwin Core Archive for GBIF, as WDX, or as Raven, Audacity and Kaleidoscope tables, or go straight to a Wildlife Network dataset with credit. Camera-trap images (SpeciesNet) follow in a later phase.

The promise: **plug in the drive, walk away, and come back to a reviewed, citable species list for every site.**

## 2. Who uses it and what they need done

The intended users run long-term recorder programmes on small budgets: national parks, research institutes (some running GBIF nodes), field stations with a few Song Meters or AudioMoths rotated by students, university groups using Kaleidoscope, farm studies, and national biodiversity information systems. Their common situation: terabytes of audio in the lab, never processed.

Jobs to be done:

1. **Process the backlog.** Many cards, many folders, several devices, mixed sample rates. No scripting.
2. **Know where and when each file was recorded.** Site, coordinates and UTC time, from device metadata or a short form, not by hand per file.
3. **Trust the species list.** BirdNET misses a large share of local species in Africa and Asia (section 3), so a local expert must confirm, reject and correct, and add species the model does not know.
4. **Share the result.** A GBIF-ready download for their node or IPT, or a direct push to Wildlife Network, with their names and institution credited.

Realities the design must respect:

- **Low bandwidth.** Installers and model downloads may run over slow or metered links. Everything after install must work offline. Model packs must be copyable on a USB stick.
- **Non-programmers.** Students and rangers operate it. No terminal, no Python, no config files.
- **Shared lab computers.** One mid-range Windows laptop or desktop, often shared, often without a GPU, sometimes without admin rights. Jobs must survive someone closing the lid or a power cut.
- **Rotating staff.** A season's work outlives the student who started it, so projects must be self-contained folders that another person can open and continue.

## 3. What exists and where it falls short

| Tool | What it does well | Where it falls short | Price |
|---|---|---|---|
| BirdNET-Analyzer GUI (v2.4.0) | Free, 6,000+ species [obs, 5], location and week species filter, Raven/Audacity/Kaleidoscope/CSV output, segments, a review tab with logistic threshold fit, custom classifier training, embeddings search with Perch v2 as an option [obs, 1, 2, 3] | Install and launch trouble on macOS; GUI crashes or stalls on long files; only "skip existing results", no true resume; review is a separate step on extracted segment folders; no notion of sites, deployments or projects; no Darwin Core export | Free; model CC BY-NC-SA 4.0 [obs, 4] |
| Chirpity | Desktop app (Windows, macOS), fast BirdNET runs, spectrogram review, tagging, eBird export [obs, 10, 11] | Built around Nocmig and single-observer use; no deployment metadata, no multi-reviewer agreement, no GBIF export [asm from feature lists] | Free |
| Kaleidoscope Pro (Wildlife Acoustics) | Polished batch processing, clustering of similar sounds, classifier building, strong bat workflow [obs, 12] | Paid annual subscription; Lite only views results; closed source | US$399 per year [obs, 13] |
| Raven Pro / Lite (Cornell) | The standard for manual spectrogram annotation and selection tables | Manual; no batch model inference; licences expire yearly | Pro US$50 to 200 per year, free in some countries; Lite free [obs, 14] |
| Arbimon (RFCx) | Free cloud platform, pattern matching, used at scale | Needs upload of all audio: impractical for terabytes over slow links [der from bandwidth reality] | Free [obs, 15] |
| Whombat | Open-source annotation for building ML datasets, pip or bundled executables [obs, 16] | Annotation tool, not a batch processing and publishing pipeline | Free |
| AviaNZ | Open-source, long field recordings, species filters, batch, human review [obs, 17] | Small model set; dated UI [asm] | Free |
| OpenSoundscape | Python library: CNN training, RIBBIT, localisation [obs, 18] | Code only | Free (MIT) |
| Perch 2.0 + Hoplite (Google) | Strong embeddings for birds and other taxa; agile modelling: vector search plus active learning gives a new classifier in under an hour [obs, 6, 7] | Notebook workflow; official model needed a GPU at release, CPU via community ONNX export [obs, 8] | Free (Apache 2.0) [obs, 8] |
| BatDetect2 | Bat call detection and classification, efficient (ran 10 h of audio per day on a Raspberry Pi 4) [obs, 19] | Python CLI; model CC BY-NC 4.0 [obs, 20] | Free |

Top pain points with sources:

1. **Performance on African and Asian species is weak.** A global assessment across 67 sites found BirdNET performed noticeably worse in Africa and Asia, likely from training data coverage, and recall ranged 0.24 to 0.52 across continents [obs, 5, 9]. The authors stress that expert validation is essential [obs, 5]. Field users in southern Africa report that BirdNET finds roughly two thirds to three quarters of their species [obs, conversation].
2. **Installation and launch.** The packaged macOS app did not open on macOS 12.7 (issue #451); an M1 user saw no window or feedback for a minute or more [obs, 21, 22]. The v2.3.0 notes list a known macOS bug where the file dialog cannot select files [obs, 3].
3. **Long files and big batches.** The GUI closed without error on a 4 GB file that the CLI handled (issue #910) [obs, 23]; a user hit "out of memory" beyond about 40 minutes of a 3-hour file [obs, 24]; a user waited over three hours with no output and was told to use the command line [obs, 25].
4. **No resume.** The only option is `--skip_existing_results`, which skips whole finished files [obs, 26]. A crash mid-file restarts that file, and there is no job state.
5. **Thresholds are per species, not global.** Optimal thresholds vary widely by species and goal [obs, 9]; the recommended global values (0.1 to 0.3 for detection, about 0.5 for community work) are compromises [obs, 5].
6. **Time and place are manual.** BirdNET takes one lat, lon and week per run [obs, 26]. Song Meter and AudioMoth file names carry no time zone; it has to come from the device settings or GUANO/comment metadata [obs, 27, 28].
7. **Speed varies 5x between front ends on the same CPU.** On a 6-core Core i5, BirdNET-Analyzer ran a 3 h 6 min file at 20x real time, Chirpity's BirdNET at 76x to 106x [obs, 10]. The gap is software, not hardware.

## 4. What makes Studio far better

Each item is testable. **MVP** items ship in version 1.

1. **Drag a whole drive in and walk away (MVP).** Studio indexes every audio file (WAV, FLAC; MP3 and others later), groups them into deployments by folder and device serial, and queues work. Test: point it at a 500 GB card dump with 30 folders and no manual step is needed before processing starts.
2. **Resumable, crash-safe processing (MVP).** Work is split into chunks (for example 10 minutes of audio); each finished chunk commits to SQLite in one transaction. Killing the process or pulling the power loses at most the chunks in flight. Test: kill the app 20 times during a 100-hour run; final results equal an uninterrupted run, row for row.
3. **Automatic time and place (MVP).** Read GUANO and AudioMoth comment metadata, Song Meter and AudioMoth file-name timestamps, and device serials. Ask once per deployment for site, coordinates and the recorder's time zone, with a map pick and a "same as last deployment" button. Store UTC. Test: 95% of files from a mixed Song Meter Micro 2 and AudioMoth set get correct UTC times with at most one question per deployment.
4. **Per-site, per-week species filter (MVP).** BirdNET's location and week list is applied per deployment and per file date, not once per run. Users can add local species lists (CSV) and the GBIF species list for a country or cell, so species BirdNET's range model omits are not silently dropped.
5. **Multiple models in one run (BirdNET in MVP; Perch and BatDetect2 in phase 2; SpeciesNet in phase 3).** One pass decodes audio once and feeds every enabled model. Detections from different models stay separate and are compared in review.
6. **Review in minutes, not weeks (MVP core, smart parts phase 2).**
   - Species-first review queue: for each species, Studio samples clips across the confidence range (stratified, for example 5 per 0.1 band) instead of top scores only, which is what a threshold fit needs.
   - Spectrogram plus audio, playing instantly; keyboard: Y yes, N no, U unsure, C change species, Space play, arrows move.
   - Running estimate per species: "after 24 verdicts, precision at 0.5 is 0.92 (0.81 to 0.97)".
   - Test: a reviewer confirms or rejects 100 species at 30 clips each in under 4 hours [der: 3,000 clips x about 4.5 s per clip (3 s listen plus decision) = 13,500 s = 3.75 h].
7. **Per-species thresholds calibrated from reviews (MVP).** Logistic fit of verdict against score per species (the Wood and Kahl method that BirdNET's own review tab uses [obs, 1]), with a target precision chosen by the user (for example 0.95). Exports apply these thresholds and record them in the metadata.
8. **Verify by similarity (phase 2).** Store embeddings (BirdNET or Perch). After a verdict, show the 20 nearest unreviewed clips of the same predicted species for one-key bulk confirm. Also "find more like this" for species the model never predicted, which is the Perch agile modelling idea [obs, 6] in a GUI.
9. **Inter-reviewer agreement (phase 2).** A project folder can be reviewed by two people (same machine, or merge a copy). Studio reports agreement (Cohen's kappa) per species and lists the disagreements to settle.
10. **Local model improvement (phase 2).** Train a small classifier head on embeddings from reviewed clips, including new species. Runs on CPU in minutes [asm, a linear head on a few thousand embeddings]. Saved as a custom model in the project.
11. **One-click exports (MVP: Darwin Core Archive, WDX, Raven, Audacity, CSV; phase 2: Kaleidoscope).** Darwin Core Archive with occurrence core, event extension per deployment, `basisOfRecord = MachineObservation`, `identificationVerificationStatus` from reviews, and an EML file with people, institution, licence and methods (model versions, thresholds). Mapping follows the WDX Darwin Core mapping (`wildlife-detection-exchange/MAPPING-DARWIN-CORE.md`). Test: archive validates in the GBIF data validator without errors.
12. **One-click publish to Wildlife Network (MVP).** Paste a dataset upload key once (stored in the OS keychain), choose "reviewed only" or "above calibrated thresholds", then upload in chunks of 2,000 rows, run `check`, show the server's check results, and `submit`. Resends are safe because the server ignores duplicates on (dataset, site, scientific name, time) [obs, `drizzle/0031_datasets.sql`, unique index `dataset_records_dedupe`]. Sends WDX events with the v0.4 `dataset` block for credit.
13. **Fully offline (MVP).** No network call except update checks, opt-in model downloads, and explicit publishing. Model packs install from a file.
14. **Installers that just work (MVP).** Signed and notarised macOS app (Apple Silicon and Intel), signed Windows installer that runs without admin rights. No Python, no terminal step. Test: a fresh Windows 10 laptop and a fresh macOS machine go from download to first result in under 10 minutes with no warnings beyond the first-launch prompt.

Not claimed: better raw accuracy than BirdNET. Studio's gain is in what happens around the model.

## 5. Main screens and flows

```
+--------------------------------------------------------------------------------+
| Studio   Project: Field station 2025 dry season    [Processing 41%]  [Publish]  |
+------+-------------------------------------------------------------------------+
| Home | Deployments                              Map                             |
| Data | SM-0412  Waterhole N   1,820 h  done      [ . dots per deployment . ]     |
| Run  | SM-0413  Ridge         1,640 h  62%                                       |
| Rev. | AM-0007  Camp         ?? needs time zone [Fix]                           |
| Exp. |                                                                         |
+------+-------------------------------------------------------------------------+
```

1. **New project / import.** Drag a drive or folder in. Studio scans (fast: file headers only) and proposes deployments. Each card shows device, date range, hours, sample rate, and what is missing (place, time zone). Fixing one card can apply to its siblings.
2. **Run.** Choose models (BirdNET default), species filter (automatic per site and week, plus optional lists), minimum score to keep (default 0.1). Start. A progress view shows hours done, speed in x real time, estimated finish, and errors per file (corrupt files are listed, never fatal). The app can be closed; the job resumes on next launch.
3. **Overview.** Per deployment and species: detections, days present, first and last date, review state (not started, sampled, calibrated). Charts: detections per hour of day and per week.
4. **Review.**

```
+-------------------------------+------------------------------------------------+
| Species (sorted by need)      |  Cape Turtle Dove  Streptopelia capicola        |
|  Cape Turtle Dove   12/30     |  [ spectrogram, 3 s clip with 1 s context ]     |
|  Grey Go-away-bird   0/30     |  score 0.42  SM-0413  2025-07-14 06:12 UTC      |
|  Unknown / added     3        |  Y yes   N no   U unsure   C change species     |
|                               |  Similar unreviewed: 18  [confirm all similar]  |
| Threshold for 95% precision:  |  Precision at 0.5: 0.92 (0.81 to 0.97)          |
|   0.37 (from 12 verdicts)     |                                                |
+-------------------------------+------------------------------------------------+
```

5. **Export and publish.** Pick format and filter (reviewed only, calibrated thresholds, or raw). Darwin Core and Wildlife Network ask for people, institution, licence (CC0 or CC BY 4.0 for GBIF) and citation once per project. Publishing shows each step and the server's check report.

## 6. Architecture and stack

**Recommendation: a new app built on Tauri 2 (Rust core, React and TypeScript UI), with all models running through ONNX Runtime, results in SQLite, and no Python at run time.**

- **Shell: Tauri 2.** Small installers matter for low-bandwidth users. Long jobs run in a Rust worker, so a UI glitch cannot kill processing. Uses WebView2 on Windows (preinstalled on Windows 10 and 11 [asm]) and WebKit on macOS.
- **Model runtime: ONNX Runtime** (the `ort` crate). CPU by default, DirectML on Windows GPUs, CoreML on Apple Silicon, CUDA as an optional download. BirdNET has community ONNX conversions [obs, 29, 30]; Perch 2.0 runs on CPU via ONNX [obs, 8]; BatDetect2 and SpeciesNet are PyTorch models and are expected to export [asm]. Chirpity shows a non-Python BirdNET runtime can be 4x to 5x faster than BirdNET-Analyzer on the same CPU [obs, 10].
- **Audio:** Rust decoding (`symphonia` for WAV, FLAC, MP3), resampling to each model's rate, decode once per chunk and fan out to models. Spectrograms computed in Rust and sent to the UI as images or tiles; playback via Web Audio from short decoded clips.
- **Storage:** one SQLite file per project (WAL mode) for files, deployments, chunks, detections, verdicts, thresholds and job state. Embeddings in a separate file of fixed-size float16 vectors indexed by detection, with brute-force or small-index similarity search [der: 1 TB of 48 kHz audio is about 2,900 h; BirdNET embeddings every 3 s = 1,200 per hour x 2,900 h = 3.5 M vectors x 1,024 dims x 2 bytes = 7.1 GB, which fits on disk and scans in seconds per species subset].
- **Packaging and updates:** Tauri bundler, GitHub Releases, built-in updater with signed update manifests. Model packs as separate versioned downloads (or files on a USB stick), each with its licence.
- **Signing:** Apple Developer Program (US$99 per year [asm, list price]) for notarisation; Windows via Azure Artifact Signing (formerly Trusted Signing), US$9.99 per month [obs, 31], available to individuals in the USA, Canada and Europe [obs, 31].
- **Wildlife Network:** HTTPS calls to `POST /api/v1/datasets/<id>` with the upload key (`Authorization: Bearer wnds_...`), actions `records` or `events`, then `check` and `submit`. Rows map to WDX via the same rules as wdx-agent. Key stored in macOS Keychain or Windows Credential Manager, never in the project folder.

**Rejected alternatives:**

- **Electron with a bundled Python sidecar (PyInstaller, TensorFlow, PyTorch).** The direct route to BirdNET-Analyzer's own install problems, with installers in the gigabyte range [asm] and slow first launch. Rejected.
- **Local web UI served by a Python app (the SpeciesNet Studio and Whombat model).** Easy reuse of Python models, but still ships Python, needs a browser and a port, and long jobs share the server process. Kept as the shape for a multi-user lab server, not for this app.
- **Electron with onnxruntime-node (the Chirpity route).** Viable and proven. Second choice; pick it if the Tauri spike shows WebView audio or rendering problems on Windows.
- **Python plugins.** Allowed later as an optional "external model" adapter (run a command, read its CSV), never required.

**Base project: new app, not a fork of SpeciesNet Studio.** SpeciesNet Studio is a Docker plus FastAPI plus React web app (about 2,000 lines of backend Python [obs, `wc -l`], SQLite via SQLModel) for camera-trap images. Studio's core problems (audio decoding, long resumable jobs, offline installers) are not there. Reuse instead: its React review patterns (virtualised gallery, keyboard review, filter sidebar, URL-synced filters), its adapter idea, and its export code as a reference. SpeciesNet Studio stays as the server option; its image review moves into Studio in phase 3. From wdx-agent, reuse the parsers for BirdNET, Kaleidoscope and CSV tables (so existing results can be imported and reviewed) and the WDX mapping rules.

## 7. Performance targets

Data volumes:

- AudioMoth at 48 kHz, 16-bit mono: 345.6 MB per hour [obs, 32]; 1 TB holds about 2,900 h [der: 1,000,000 MB / 345.6 = 2,894 h].
- Song Meter Micro 2 at 24 kHz, 16-bit mono: 172.8 MB per hour [der: 24,000 x 2 bytes x 3,600 s]; 1 TB holds about 5,800 h [der: 1,000,000 / 172.8 = 5,787 h]. Compute scales with hours, so a TB of 24 kHz audio takes twice as long as a TB of 48 kHz audio.
- Example field station: 10 Song Meter Micros at 24 kHz. If each records 12 h a day [asm], that is 10 x 12 x 172.8 MB = 20.7 GB per day [der], about 7.6 TB per year [der: x 365].

Speeds observed elsewhere (BirdNET, CPU):

- BirdNET-Analyzer, Core i5 6-core: 20x real time [obs, 10].
- Chirpity BirdNET, Core i5 6-core: 106x; M2 MacBook Pro CPU: 148x; M1 Pro, 300 one-hour files: about 128x [obs, 10].

Studio targets (BirdNET only, CPU, 4 to 8 cores):

- **At least 100x real time**, matching Chirpity [obs, 10]. One TB of 48 kHz audio: 2,894 h / 100 = 29 h [der]. One TB of 24 kHz Micro audio: 5,787 / 100 = 58 h [der]. BirdNET-Analyzer at 20x would need 145 h and 289 h [der].
- **Disk is not the bottleneck:** reading 1 TB from a USB hard disk at 100 MB/s takes 10,000 s = 2.8 h [der, 100 MB/s asm], far under the compute time.
- **GPU (DirectML, CUDA, CoreML): 300x or more** [asm, to benchmark; Chirpity's native model reached 388x on a GTX 1660 Super [obs, 10] but its BirdNET GPU path was slower than CPU, so GPU is not guaranteed to help].
- **Perch 2.0 and BatDetect2:** no published desktop CPU throughput found. Perch 2.0 is an EfficientNet-B3 (about 12 M parameter embedding model) [obs, 8]; assume 3x to 10x slower than BirdNET per audio hour [asm], so Perch on all audio is a GPU or "overnight per week of data" option, and the default is BirdNET on everything plus Perch embeddings only for review sampling.
- **UI:** spectrogram of a clip renders in under 100 ms; review queue moves to the next clip in under 50 ms; project with 5 M detections opens in under 3 s [asm, SQLite with indexes].
- **Memory:** under 2 GB during processing regardless of file length (streamed in chunks), fixing the 4 GB file and 40-minute crashes [obs, 23, 24].

## 8. Phases and effort

Estimates are for one developer with an AI coding assistant, in working weeks [asm].

**Spike (1 to 2 weeks).** BirdNET to ONNX in Rust, CPU, on 100 h of real Song Meter audio. Compare detections with BirdNET-Analyzer and measure x real time. Tauri window with a spectrogram and playback. Go or no-go on the stack.

**MVP, version 1.0 (10 to 14 weeks after the spike).**
- Import and deployment detection, GUANO, AudioMoth and Song Meter metadata, time zone handling.
- BirdNET (latest model) with per-deployment, per-week species filter and custom species lists.
- Chunked, resumable job engine; error list per file.
- Overview tables and simple charts.
- Review: species queue, stratified sampling, keyboard, change species, per-species threshold fit.
- Import of existing BirdNET, Kaleidoscope and Raven results for review.
- Exports: Darwin Core Archive with EML, WDX, Raven, Audacity, CSV.
- Publish to Wildlife Network datasets.
- Signed installers for Windows and macOS, updater, offline model packs. English UI with strings ready for translation.

**Phase 2 (8 to 12 weeks).** Perch 2.0 and BatDetect2; embeddings, similarity propagation and "find more like this"; trainable classifier heads for local and missing species; two-reviewer agreement and merge; Kaleidoscope export; GPU acceleration; French and Portuguese UI (DRC and Rwanda users) [asm on languages].

**Phase 3 (8 to 10 weeks).** Camera-trap images: SpeciesNet and MegaDetector via ONNX, reusing SpeciesNet Studio's review patterns; Darwin Core and Wildlife Insights style exports for images; Linux build.

**Out of scope:** cloud processing or hosting audio; multi-user server mode (SpeciesNet Studio covers that); real-time station monitoring (BirdNET-Pi, BirdNET-Go, wdx-agent cover that); training new foundation models; sound localisation; mobile apps; uploading raw audio anywhere.

## 9. Risks and open questions

1. **BirdNET model licence.** The model is CC BY-NC-SA 4.0; research and education count as non-commercial [obs, 4]. Studio may redistribute it free with attribution, but: a converted ONNX copy is an adaptation and must keep CC BY-NC-SA; Studio's own code licence (Apache 2.0 proposed, matching SpeciesNet Studio and wdx-agent) does not cover the model, so models ship as separately licensed packs; consultancies using Studio commercially cannot use BirdNET. Action: ask the BirdNET team for their view on an ONNX conversion and bundling before the MVP. BatDetect2 is CC BY-NC 4.0 [obs, 20]; Perch 2.0 is Apache 2.0 [obs, 8]; SpeciesNet is Apache 2.0 [obs, SpeciesNet Studio README].
2. **ONNX fidelity.** Converted BirdNET may score differently from the TFLite original. Mitigation: published comparison on a test set; tolerance target of score difference under 0.01 on 99% of windows [asm].
3. **Signing cost and access.** Apple about US$99 per year [asm], Windows Artifact Signing US$9.99 per month and only for individuals in the USA, Canada and Europe [obs, 31]. Developers outside those regions need another route (an OV certificate from a certificate authority, or a free programme for open-source projects). Unsigned Windows installers trigger SmartScreen warnings, which non-programmers read as a virus. Decide before the MVP.
4. **GPU support.** DirectML and CoreML coverage for these models is unproven [asm]. CPU must meet targets alone.
5. **Data privacy and sensitive species.** Recordings can contain human speech and sensitive species locations. Studio keeps audio local; exports offer coordinate rounding and a sensitive species list (as Wildlife Network already uses); publishing never sends audio.
6. **Time zones.** Wrong zone shifts dawn chorus by hours and corrupts phenology. Mitigation: required per deployment, sanity check (most bird activity near dawn).
7. **Maintenance load.** BirdNET releases often (v2.1.1 to v2.4.0 in recent months [obs, 3]). Model packs must be swappable without an app release.
8. **Open questions.** Which Darwin Core extensions do GBIF nodes expect for machine observations? Do national biodiversity systems want direct export to their own systems? Is Kaleidoscope result import more valuable than Kaleidoscope export for Kaleidoscope users?

**Cheapest test that could kill or confirm the plan (spike week 1):** export BirdNET to ONNX, run it from a 200-line Rust program on one 24 h Song Meter Micro recording on a mid-range Windows laptop without a GPU. Pass if it runs at 100x real time or faster, uses under 2 GB of memory, and its detections match BirdNET-Analyzer within the tolerance above. If it fails, fall back to Electron with onnxruntime-node (Chirpity's proven route) before writing any UI.

## Sources

1. BirdNET-Analyzer GUI docs: https://birdnet-team.github.io/BirdNET-Analyzer/stable/usage/gui.html
2. BirdNET-Analyzer embeddings guide: https://birdnet-team.github.io/BirdNET-Analyzer/best-practices/embeddings.html
3. BirdNET-Analyzer releases (v2.1.1 to v2.4.0, known issues): https://github.com/birdnet-team/BirdNET-Analyzer/releases
4. BirdNET-Analyzer README (install, MIT code, CC BY-NC-SA 4.0 models): https://github.com/birdnet-team/BirdNET-Analyzer
5. BOU blog, Pérez-Granados, "Optimizing BirdNET": https://bou.org.uk/blog-perez-granados-optimizing-birdnet-lessons-from-a-global-initiative/
6. Google DeepMind, Perch 2.0 and agile modelling: https://deepmind.google/discover/blog/how-ai-is-helping-advance-the-science-of-bioacoustics-to-save-endangered-species/
7. Perch Hoplite: https://pypi.org/project/perch-hoplite/0.1.3/
8. Perch 2.0 paper and model: https://arxiv.org/html/2508.04665v1 ; https://huggingface.co/Laborator/kaggle-hackathon-birdclef-2026 (CPU via ONNX)
9. Funosas et al. 2026, global assessment of BirdNET, Ecological Indicators: https://hal.inrae.fr/hal-05490415v1
10. Chirpity performance comparisons: https://github.com/Mattk70/Chirpity-Electron/discussions/23
11. Chirpity: https://github.com/Mattk70/Chirpity-Electron ; https://blog.nhbs.com/?p=30345
12. Kaleidoscope Pro and cluster analysis: https://www.wildlifeacoustics.com/products/kaleidoscope/cluster-analysis
13. Kaleidoscope Pro subscription FAQ: https://wildlifeacoustics.com/resources/faqs/do-i-need-a-kaleidoscope-pro-subscription-to-run-species-analysis
14. Cornell Raven store: https://store.birds.cornell.edu/collections/raven-sound-software/pro
15. Arbimon: https://rfcx.org/ecoacoustics
16. Whombat: https://github.com/vogelbam/whombat
17. AviaNZ: https://avianz.net/
18. OpenSoundscape: https://github.com/kitzeslab/opensoundscape
19. UCLouvain thesis on bat detection (BatDetect2 on Raspberry Pi 4): https://thesis.dial.uclouvain.be/entities/masterthesis/e6cb39a5-4f1c-4685-9d77-b1f31450f6aa
20. BatDetect2 licence via acoupi_batdetect2: https://pypi.org/project/acoupi-batdetect2/
21. BirdNET-Analyzer issue #451 (macOS app does not open): https://github.com/kahst/BirdNET-Analyzer/issues/451
22. r/BirdNET_Analyzer, "does not open" (M1): https://www.reddit.com/r/BirdNET_Analyzer/comments/1jyynv2/
23. BirdNET-Analyzer issue #910 (4 GB file): https://github.com/birdnet-team/BirdNET-Analyzer/issues/910
24. r/BirdNET_Analyzer, out of memory on 3 h file: https://www.reddit.com/r/BirdNET_Analyzer/comments/1jpza4x/
25. r/BirdNET_Analyzer, slow GUI: https://www.reddit.com/r/BirdNET_Analyzer/comments/16yurps/
26. BirdNET-Analyzer CLI docs: https://birdnet-team.github.io/BirdNET-Analyzer/stable/usage/cli.html
27. aru_metadata_parser (Song Meter file names, time zones): https://github.com/kitzeslab/aru_metadata_parser
28. BTO advice for recorders (AudioMoth file names, local time): https://www.bto.org/data/tools-products/acoustic-pipeline/support-hub/advice-commonly-used
29. birdnet-onnx crate: https://docs.rs/crate/birdnet-onnx/latest
30. BirdNET ONNX conversion: https://huggingface.co/justinchuby/BirdNET-onnx
31. Azure Artifact Signing pricing: https://azure.microsoft.com/en-us/products/artifact-signing ; https://devclass.com/2026/01/14/code-signing-windows-apps-may-be-easier-and-more-secure-with-new-azure-artifact-service/
32. Audio storage per hour (16-bit 48 kHz = 345.6 MB/h): https://mukurtu-australia-nsw.libraries.wsu.edu/system/files/atoms/file/EstimatingStorageRequiredForAudio.pdf
33. BirdNET library benchmarking (x real time definition, GPU batch sizes): https://birdnet-team.github.io/birdnet/benchmarking.html
