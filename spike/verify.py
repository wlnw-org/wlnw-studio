import tensorflow as tf, numpy as np, soundfile as sf, onnxruntime as ort, time
C = ".venv/lib/python3.11/site-packages/birdnet_analyzer/checkpoints/V2.4/"
it = tf.lite.Interpreter(model_path=C + "BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite"); it.allocate_tensors()
K = np.load("melconsts.npz")
sess = ort.InferenceSession("../models/birdnet_v24_cnn.onnx", providers=["CPUExecutionProvider"])
iname = sess.get_inputs()[0].name
print("onnx input", sess.get_inputs()[0].shape)
def mel(s, n, hop, win, M, p):
    s = s - s.min(); s = s / (s.max() + 1e-6); s = (s - 0.5) * 2
    nf = 1 + (len(s) - n) // hop
    fr = np.stack([s[i*hop:i*hop+n] for i in range(nf)]) * win
    m = np.fft.rfft(fr, axis=1).real.astype(np.float32) @ M.T
    return ((m * m) ** p)[:, ::-1].T
x, _ = sf.read("audio/clips.wav", dtype="float32")
chunks = [x[i:i+144000] for i in range(0, len(x) - 48000, 144000)]
chunks = [np.pad(c, (0, 144000 - len(c))) for c in chunks]
worst = 0; agree = 0; tot = 0
for c in chunks:
    it.set_tensor(it.get_input_details()[0]["index"], c[None]); it.invoke(); r = it.get_tensor(it.get_output_details()[0]["index"])[0]
    spec = np.stack([mel(c, 2048, 278, K["w1"], K["M1"], float(K["model_MEL_SPEC1_truediv_1"])), mel(c, 1024, 280, K["w2"], K["M2"], float(K["model_MEL_SPEC2_truediv_1"]))], -1)[None].astype(np.float32)
    o = sess.run(None, {iname: spec})[0][0]
    worst = max(worst, float(np.abs(o - r).max()))
    sr_, so = 1/(1+np.exp(-r)), 1/(1+np.exp(-o))
    a, b = set(np.where(sr_ >= 0.1)[0]), set(np.where(so >= 0.1)[0]); agree += len(a & b); tot += len(a | b)
print("chunks", len(chunks), "max logit diff", worst, "detections agree", agree, "of", tot)
