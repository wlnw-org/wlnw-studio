import numpy as np, soundfile as sf, onnxruntime as ort, tensorflow as tf, scipy.signal as ss, sys
P=sys.argv[1]
x,sr=sf.read("audio/clips.wav",dtype="float32")
x=ss.resample_poly(x,2,3).astype(np.float32)  # 48k -> 32k
W=160000
wins=[x[i:i+W] for i in range(0,len(x)-W+1,W)]
def prep(w): w=w-w.mean(); m=np.abs(w).max(); return (w*(0.25/m) if m>0 else w).astype(np.float32)
batch=np.stack([prep(w) for w in wins])
m=tf.saved_model.load(P); f=m.signatures["serving_default"]
print("sig", f.structured_input_signature, list(f.structured_outputs.keys()))
r=f(inputs=tf.constant(batch)); tl=r["label"].numpy()
s=ort.InferenceSession("../models/perch_v2/perch_v2.onnx",providers=["CPUExecutionProvider"])
ol=s.run(["label","embedding"],{"inputs":batch})
print("windows",len(wins),"max logit diff",np.abs(tl-ol[0]).max(),"emb diff",np.abs(r["embedding"].numpy()-ol[1]).max())
L=[l.strip() for l in open("../models/perch_v2/labels.csv").read().splitlines()[1:]]
def sm(z): e=np.exp(z-z.max(1,keepdims=True)); return e/e.sum(1,keepdims=True)
p=sm(ol[0]); q=1/(1+np.exp(-ol[0]))
for i in range(min(6,len(wins))): print(i, L[p[i].argmax()], round(float(p[i].max()),3), "sigmoid", round(float(q[i].max()),3))
