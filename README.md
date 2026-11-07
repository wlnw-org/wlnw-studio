# Wildlife Network Studio

A free desktop app for wildlife researchers with more recordings than time. Point it at a drive of recordings; it finds
the species on your own computer, helps you check them in minutes, and exports them for GBIF or publishes them to
[Wildlife Network](https://wlnw.org).

- **Process locally.** WAV, FLAC and MP3 from AudioMoth, Song Meter and other recorders; time, place and recorder read
  from the files. Resumes after a crash or a closed lid. Nothing is uploaded.
- **Models.** Perch 2.0 (about 15,000 species: birds, frogs, insects, mammals; downloaded on first use) and BirdNET 2.4
  (if you have its files). Perch is the default in Africa and South Asia, where BirdNET knows fewer species.
- **Review fast.** Clips from every confidence level per species, spectrogram and audio, keyboard shortcuts. Each answer
  refits that species' threshold so what you keep is right at least 90% of the time.
- **Wildlife Network Classifier.** Learns your local species from your review answers and from clips you name, including
  species no model knows. Find more like a clip, train, and share the classifier with others.
- **Export.** Darwin Core Archive for GBIF, WDX, Raven and Audacity tables, CSV, or publish to a Wildlife Network
  dataset with credit to its authors.

Status: early test releases. macOS (Apple Silicon) builds are signed and notarized; Windows builds are not yet code
signed.

## Build

Rust (stable), Node 22 and pnpm 10.

```bash
cargo test --release -p wlnw-engine
```
```bash
cd app && pnpm install && pnpm tauri dev
```

The engine also works from the command line: `cargo run --release -p wlnw-engine --bin wlnw` lists its commands.
Design notes: [docs/SCOPE.md](docs/SCOPE.md); model conversion and accuracy checks: [docs/SPIKE.md](docs/SPIKE.md).

## Licence

Code: Apache-2.0 ([LICENSE](LICENSE)). The models have their own licences; see [NOTICE](NOTICE).
