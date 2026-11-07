import numpy as np, onnxruntime as ort, tensorflow as tf
C=".venv/lib/python3.11/site-packages/birdnet_analyzer/checkpoints/V2.4/"
it=tf.lite.Interpreter(model_path=C+"BirdNET_GLOBAL_6K_V2.4_MData_Model_V2_FP16.tflite"); it.allocate_tensors()
x=np.array([[51.5,-0.12,47]],dtype=np.float32)
it.set_tensor(it.get_input_details()[0]["index"],x); it.invoke(); r=it.get_tensor(it.get_output_details()[0]["index"])[0]
s=ort.InferenceSession("../models/birdnet_v24_meta.onnx",providers=["CPUExecutionProvider"]); o=s.run(None,{s.get_inputs()[0].name:x})[0][0]
print(it.get_input_details()[0]["shape"], r.shape, "maxdiff", np.abs(r-o).max(), "species>=0.03", int((r>=0.03).sum()))
