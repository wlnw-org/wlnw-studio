import tensorflow as tf, numpy as np, soundfile as sf, json
C = ".venv/lib/python3.11/site-packages/birdnet_analyzer/checkpoints/V2.4/"
it = tf.lite.Interpreter(model_path=C + "BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite", experimental_preserve_all_tensors=True)
it.allocate_tensors()
x, sr = sf.read("audio/clips.wav", dtype="float32"); x = x[48000*9:48000*9+144000][None, :]
it.set_tensor(it.get_input_details()[0]["index"], x); it.invoke()
T = {t["name"]: t["index"] for t in it.get_tensor_details()}
g = lambda n: it.get_tensor(T[n])
ref = g("model/concatenate/concat")[0]  # (96, 511, 2)
consts = {k: float(np.array(g(k)).reshape(-1)[0]) for k in ["model/MEL_SPEC1/add/y", "model/MEL_SPEC1/Sub_1/y", "model/MEL_SPEC1/Mul/y", "model/MEL_SPEC1/truediv_1", "model/MEL_SPEC2/truediv_1"]}
print(consts)
def mel(sig, n, hop, win, M, p):
    s = sig - sig.min(); s = s / (s.max() + consts["model/MEL_SPEC1/add/y"]); s = (s - consts["model/MEL_SPEC1/Sub_1/y"]) * consts["model/MEL_SPEC1/Mul/y"]
    nf = 1 + (len(s) - n) // hop
    fr = np.stack([s[i*hop:i*hop+n] for i in range(nf)]) * win
    X = np.fft.rfft(fr.astype(np.float64), axis=1)
    spec = X.real.astype(np.float32)          # tf.cast(complex) keeps the real part
    m = spec @ M.T                             # (nf, 96)
    m = (m * m) ** p
    return m[:, ::-1].T                        # reverse mel axis, (96, nf)
w1 = g("model/MEL_SPEC1/stft/hann_window/sub_2"); w2 = g("model/MEL_SPEC2/stft/hann_window/sub_2")
M1 = g("model/MEL_SPEC1/Tensordot/MatMul"); M2 = g("model/MEL_SPEC2/Tensordot/MatMul")
print("M", M1.shape, M2.shape)
# MEL_SPEC2 normalises its own input too? check intermediates
for name in ["model/MEL_SPEC2/Sub", "model/MEL_SPEC2/Mul"]:
    print(name, name in T)
a = mel(x[0], 2048, 278, w1, M1, consts["model/MEL_SPEC1/truediv_1"])
b = mel(x[0], 1024, 280, w2, M2, consts["model/MEL_SPEC2/truediv_1"])
for k, (mine, r) in enumerate([(a, ref[..., 0]), (b, ref[..., 1])]):
    print(k, mine.shape, "maxabs", float(np.abs(mine - r).max()), "refmax", float(np.abs(r).max()))
np.savez("melconsts.npz", w1=w1, w2=w2, M1=M1, M2=M2, **{k.replace("/", "_"): v for k, v in consts.items()})
