import tensorflow as tf, numpy as np, soundfile as sf
C = ".venv/lib/python3.11/site-packages/birdnet_analyzer/checkpoints/V2.4/"
it = tf.lite.Interpreter(model_path=C + "BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite", experimental_preserve_all_tensors=True)
it.allocate_tensors()
x, sr = sf.read("audio/clips.wav", dtype="float32"); x = x[:144000][None, :]
it.set_tensor(it.get_input_details()[0]["index"], x); it.invoke()
ops = it._get_ops_details(); td = {t["index"]: t["name"] for t in it.get_tensor_details()}
def sh(i):
    try: return tuple(it.get_tensor(i).shape)
    except Exception: return None
mel = [o for o in ops if any("MEL_SPEC" in td.get(i, "") for i in o["outputs"])]
last = max(o["index"] for o in mel)
for o in ops:
    if o["index"] > last + 6: break
    if o["index"] >= 40 and o["op_name"] not in ("SHAPE","GATHER","REDUCE_PROD","PACK","STRIDED_SLICE","FLOOR_DIV","RANGE","CONCATENATION") or o["index"] > last:
        print(o["index"], o["op_name"], [(td.get(i), sh(i)) for i in o["inputs"]][:3], "->", [(td.get(i), sh(i)) for i in o["outputs"]])
