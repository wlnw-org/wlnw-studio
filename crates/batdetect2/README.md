# BatDetect2 component (CC BY-NC 4.0)

This folder is the part of Wildlife Network Studio that reproduces [BatDetect2](https://github.com/macaodha/batdetect2)'s
processing. It is licensed **CC BY-NC 4.0** (non-commercial use only), like BatDetect2 itself, and not Apache 2.0 like
the rest of Studio. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

The BatDetect2 model is not included and Wildlife Network does not host a copy. To use bats in Studio, make the model
files from the official BatDetect2 package:

```bash
python3 -m venv bd2 && . bd2/bin/activate && pip install batdetect2==2.0.0b3 onnx
python -I python/export_model.py batdetect2
```

Then, in Studio, open Models and choose "Add model files from a folder" with the `batdetect2` folder.

Please cite: Mac Aodha et al. (2022), Towards a General Approach for Bat Echolocation Detection and Classification,
bioRxiv, https://doi.org/10.1101/2022.12.14.520490
