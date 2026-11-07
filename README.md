<p align="center"><img src="app/public/logo.svg" width="96" alt="Wildlife Network logo" /></p>

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

## Download

Installers for macOS (Apple Silicon) and Windows: [latest release](https://github.com/wlnw-org/wlnw-studio/releases/latest).
Early test releases. macOS builds are signed with an Apple Developer ID and notarized by Apple.

Windows builds: free code signing provided by SignPath.io, certificate by SignPath Foundation (application in progress:
until it is approved, Windows installers are unsigned and Windows shows a SmartScreen warning). See the
[code signing policy](#code-signing-policy).

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

## Code signing policy

Windows releases are to be signed by SignPath Foundation (free code signing provided by SignPath.io, certificate by
SignPath Foundation); macOS releases are signed with the maintainer's Apple Developer ID.

- Only files built by this repository's GitHub Actions workflow ([.github/workflows/build.yml](.github/workflows/build.yml))
  from the tagged source are signed. Nothing built elsewhere is signed.
- Every release is approved by hand before signing.
- Roles:
  - Committers and reviewers: [members of the wlnw-org organisation](https://github.com/orgs/wlnw-org/people)
  - Approvers: [owners of the wlnw-org organisation](https://github.com/orgs/wlnw-org/people?query=role%3Aowner)
- All team members use multi-factor authentication for GitHub and SignPath.

## Privacy

Wildlife Network Studio will not transfer any information to other networked systems unless specifically requested by
the user. Your recordings are processed on your own computer and are never uploaded. The app contacts other systems only
when you ask it to:
- downloading a model (Perch 2.0 from Hugging Face), when you click Download;
- map tiles (OpenFreeMap) when you open the place picker;
- Wildlife Network (wlnw.org), when you publish a dataset with its upload key; then the detections you chose to
  publish are sent ([wlnw.org privacy policy](https://wlnw.org/privacy)).
Studio has no analytics, no crash reporting and no accounts.

## Licence

Code: Apache-2.0 ([LICENSE](LICENSE)). The models have their own licences; see [NOTICE](NOTICE).
