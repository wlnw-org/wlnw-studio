# Preprocessing fixture for Studio's Rust tests (crates/batdetect2/src/lib.rs): a synthetic 0.5 s clip (a chirp plus
# LCG noise, rebuilt identically in Rust) through BatDetect2's own preprocessor, sampled at every 997th value of each
# stage, and its first 1,000 samples through BatDetect2's resampler from common bat detector rates.
# Usage: python -I make_test_fixture.py ../tests/fixtures/batdetect2_preprocess.json (in a venv with batdetect2==2.0.0b3)
import sys, json, math
import numpy as np, torch
from batdetect2.api_v2 import BatDetect2API
pre = BatDetect2API.from_checkpoint().preprocessor
sr, n = 256000, 128000
state = 12345; x = np.zeros(n, dtype=np.float64)
for i in range(n):
    state = (state * 1103515245 + 12345) % (1 << 31)
    t = i / sr
    x[i] = 0.3 * math.sin(2 * math.pi * (60000 * t - 40000 * t * t)) * (1 if (i // 6400) % 2 == 0 else 0.1) + 0.02 * (state / (1 << 31) - 0.5)
wav = torch.tensor(x.astype(np.float32)).unsqueeze(0)
stages = {}
s = pre.spectrogram_builder(wav); s = pre.spectrogram_crop(s); stages["crop"] = s
s = pre.spectrogram_transforms[0](s); stages["pcen"] = s
s = pre.spectrogram_transforms[1](s); stages["msub"] = s
s = pre.spectrogram_resizer(s); stages["resize"] = s
assert torch.allclose(s, pre(wav))
out = {}
for k, v in stages.items():
    a = v.squeeze(0).numpy().astype(np.float64)
    flat = a.ravel()
    out[k] = {"shape": list(a.shape), "sum": float(flat.sum()), "max": float(flat.max()), "every997": [float(v) for v in flat[::997]]}
    print(k, a.shape, flat.sum(), flat.max())
from batdetect2.audio.loader import resample_audio
short = x[:1000].astype(np.float32)
out["resample"] = {}
for rate in (192000, 312500, 384000, 500000):
    y = resample_audio(short, sr=rate, samplerate=sr)
    out["resample"][str(rate)] = {"len": int(len(y)), "values": [float(v) for v in y]}
    print(rate, len(y))
json.dump(out, open(sys.argv[1], "w"))
