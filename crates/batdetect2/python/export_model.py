# Makes Studio's BatDetect2 pack: the default UK network (batdetect2==2.0.0b3, checkpoint batdetect2_uk_same.ckpt) in
# ONNX, plus the constants (batdetect2.json) and labels (labels.txt). The output is a derivative of CC BY-NC 4.0 weights:
# non-commercial use only, credit the authors. Wildlife Network does not host the result: each user makes it from the
# official package (pip install batdetect2==2.0.0b3 onnx). See ../NOTICE and docs/SPIKE.md.
# Usage: python -I export_model.py <out dir> [audio files to check the ONNX network against PyTorch]
import sys, json, os
import numpy as np, torch
from batdetect2.api_v2 import BatDetect2API
from batdetect2.preprocess.spectrogram import _compute_smoothing_constant

out = sys.argv[1]; os.makedirs(out, exist_ok=True)
api = BatDetect2API.from_checkpoint()
det = api.model.detector.eval()

class Net(torch.nn.Module):
    def __init__(self, d): super().__init__(); self.d = d
    def forward(self, spec):
        o = self.d(spec)
        return o.detection_probs, o.size_preds, o.class_probs

net = Net(det).eval()
x = torch.zeros(2, 1, 128, 500)
path = os.path.join(out, "batdetect2_uk.onnx")
torch.onnx.export(net, (x,), path, input_names=["spec"], output_names=["detection", "size", "classes"],
                  dynamic_axes={"spec": {0: "batch"}, "detection": {0: "batch"}, "size": {0: "batch"}, "classes": {0: "batch"}},
                  opset_version=17, dynamo=False)
import onnx; m = onnx.load(path); onnx.save(m, path)  # single file, no external data

pre = api.preprocessor
cfg = {
    "samplerate": pre.input_samplerate, "n_fft": 512, "hop": 128,
    "low_bin": pre.spectrogram_crop.low_index, "high_bin": pre.spectrogram_crop.high_index,
    "pcen_smooth": float(_compute_smoothing_constant(pre.input_samplerate, 0.4)), "pcen_gain": 0.98, "pcen_bias": 2.0, "pcen_power": 0.5, "pcen_eps": 1e-6,
    "height": 128, "resize_factor": 0.5, "min_freq": pre.min_freq, "max_freq": pre.max_freq,
    "output_samplerate": pre.output_samplerate, "clip_s": 0.5, "nms_kernel": 9, "detection_threshold": 0.01, "top_k_per_sec": 100,
    "time_scale": 1000.0, "frequency_scale": 0.0011636363636363637,
}
json.dump(cfg, open(os.path.join(out, "batdetect2.json"), "w"), indent=1)
names = {"myomys": "Whiskered Bat", "myoalc": "Alcathoe Bat", "cneser": "Serotine", "pipnat": "Nathusius's Pipistrelle",
         "barbar": "Western Barbastelle", "myonat": "Natterer's Bat", "myodau": "Daubenton's Bat", "myobra": "Brandt's Bat",
         "pippip": "Common Pipistrelle", "myobec": "Bechstein's Bat", "pippyg": "Soprano Pipistrelle", "rhihip": "Lesser Horseshoe Bat",
         "nyclei": "Leisler's Bat", "rhifer": "Greater Horseshoe Bat", "pleaur": "Brown Long-eared Bat", "nycnoc": "Noctule",
         "pleaus": "Grey Long-eared Bat"}
with open(os.path.join(out, "labels.txt"), "w") as f:
    for c in api.targets.class_names:
        sci = api.targets.decode_class(c)[0].value
        f.write(f"{sci}_{names[c]}\n")
print(cfg)

# ONNX vs torch on real clips
if len(sys.argv) > 2:
    import onnxruntime as ort
    from batdetect2.inference.clips import get_clips_from_files
    sess = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
    clips = get_clips_from_files(sys.argv[2:], duration=0.5)
    worst = [0, 0, 0]
    for c in clips:
        wav = api.audio_loader.load_clip(c)
        spec = pre(torch.tensor(wav).unsqueeze(0)).unsqueeze(0)
        with torch.no_grad(): ref = [t.numpy() for t in net(spec)]
        got = sess.run(None, {"spec": spec.numpy()})
        for i in range(3): worst[i] = max(worst[i], float(np.abs(ref[i] - got[i]).max()))
    print("clips", len(clips), "max abs diff detection/size/classes", worst)
