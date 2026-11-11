# Permissions and terms from model authors

Records of what the authors of the models Studio uses have told us, so the reasons behind Studio's licences and
downloads are written down.

## BirdNET (Stefan Kahl, K. Lisa Yang Center for Conservation Bioacoustics, Cornell Lab of Ornithology)

Email of 11 November 2026 to Arun Rajiah.

- Wildlife Network may package BirdNET 2.4 in Studio.
- Because the purpose is research and conservation, no commercial licence is needed, including when consultancies
  use it.
- BirdNET's own licence (CC BY-NC-SA 4.0) stays attached to the model files, and BirdNET and the K. Lisa Yang Center
  for Conservation Bioacoustics are credited in the app and in exports.
- He suggested moving to BirdNET 3.0. Not done yet, because it is still a preview release.

Arun's request of 7 November asked whether we may distribute the converted ONNX files and the spectrogram constants
taken from the model, under the same licence and with attribution, either with the app or as a download. Stefan
answered "Yes, feel free to do that", so offering Studio's ONNX conversion as a download is covered.

Still open (asked on 11 November 2026): whether the model licence to show is CC BY-NC-SA 4.0 (their README) or
CC BY-NC 4.0 (their Zenodo record). Studio shows CC BY-NC-SA 4.0 until then.

## BatDetect2 (Oisin Mac Aodha, University of Edinburgh)

Email of 9 November 2026 to Arun Rajiah.

- Studio's Rust translation of BatDetect2's processing must be licensed CC BY-NC 4.0, not Apache 2.0, because it is
  a direct translation of their code. It now lives in crates/batdetect2 with its own LICENSE and NOTICE.
- Wildlife Network must not host a copy of the BatDetect2 model, including an ONNX conversion, because the authors
  will update it and want one authoritative source. Studio has never hosted one; users make the files from the
  official package.
- Credit BatDetect2 with links to the paper (https://doi.org/10.1101/2022.12.14.520490) and the repository
  (https://github.com/macaodha/batdetect2), in the app and in exports.
