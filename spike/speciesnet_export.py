"""Makes Studio's camera-trap pack (models/speciesnet/) from the published weights.

Detector: MegaDetector v6 MIT (YOLOv9 trained with the MIT-licensed MultimediaTechLab YOLO code), from Pytorch-Wildlife's
Zenodo record 15398270 (CC BY 4.0). Classifier: Google's SpeciesNet v4.0.3a "always crop" (Apache 2.0, Kaggle). Only the
networks go to ONNX; Studio's Rust code does the image resizing, box decoding, NMS, cropping and the SpeciesNet ensemble.

Needs: speciesnet (pip), a checkout of github.com/microsoft/CameraTraps (for the MIT YOLO model code), torch, onnx.

  python spike/speciesnet_export.py --speciesnet DIR_WITH_info.json --mdv6 MDV6-mit-yolov9-c.ckpt --mdv6-config config_v9s.yaml \
      --cameratraps PATH/TO/CameraTraps --out models/speciesnet

Downloads (pinned):
  https://www.kaggle.com/api/v1/models/google/speciesnet/pyTorch/v4.0.3a/1/download  (tar.gz)
  https://zenodo.org/records/15398270/files/MDV6-mit-yolov9-c.ckpt   (md5 d8626ba22fccbd9c520e0deb19d721f7)
  https://zenodo.org/records/15178680/files/config_v9s.yaml          (md5 707474e2d544215d1c87fb398cd1fd2e)
  MDV6-mit-yolov9-c is Pytorch-Wildlife's default MIT detector (29 MB as ONNX). The larger MDV6-mit-yolov9-e.ckpt
  (md5 9b67c5d920a1c4008b82f420abaccc22) goes with config_v9c.yaml and also works (102 MB, several times slower).
"""
import argparse
import json
import shutil
import sys
from pathlib import Path

import torch

IMAGE_SIZE = 640


def load_detector(ckpt: Path, config: Path, cameratraps: Path):
    sys.path.insert(0, str(cameratraps / "PytorchWildlife/models/detection/yolo_mit"))
    import yaml
    from omegaconf import OmegaConf
    from yolo import create_model, create_converter  # noqa: E402

    cfg = OmegaConf.create(yaml.safe_load(config.read_text()))
    cfg.image_size = [IMAGE_SIZE, IMAGE_SIZE]
    model = create_model(cfg.model, weight_path=str(ckpt), class_num=3).eval()
    converter = create_converter(cfg.model.name, model, cfg.model.anchor, cfg.image_size, "cpu")
    return cfg, model, converter


class Detector(torch.nn.Module):
    """Network plus YOLOv9's box decoding: scores (sigmoid of the class logits) and boxes (x1, y1, x2, y2 in the 640 x 640
    letterboxed image) for every anchor. Thresholds and NMS are left to Studio."""

    def __init__(self, model, converter):
        super().__init__()
        self.model = model
        self.converter = converter

    def forward(self, x):
        cls, _, box = self.converter(self.model(x)["Main"])[:3]
        return cls.sigmoid(), box


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--speciesnet", type=Path, required=True)
    ap.add_argument("--mdv6", type=Path, required=True)
    ap.add_argument("--mdv6-config", type=Path, required=True)
    ap.add_argument("--cameratraps", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)

    cfg, model, converter = load_detector(a.mdv6, a.mdv6_config, a.cameratraps)
    det = Detector(model, converter).eval()
    x = torch.zeros(1, 3, IMAGE_SIZE, IMAGE_SIZE)
    torch.onnx.export(det, (x,), str(a.out / "detector.onnx"), opset_version=17, dynamo=False,
                      input_names=["image"], output_names=["scores", "boxes"],
                      dynamic_axes={"image": {0: "n"}, "scores": {0: "n"}, "boxes": {0: "n"}})

    info = json.loads((a.speciesnet / "info.json").read_text())
    clf = torch.load(a.speciesnet / info["classifier"], map_location="cpu", weights_only=False).eval()
    x = torch.zeros(1, 480, 480, 3)
    torch.onnx.export(clf, (x,), str(a.out / "speciesnet_v403a.onnx"), opset_version=17, dynamo=False,
                      input_names=["crop"], output_names=["logits"], dynamic_axes={"crop": {0: "n"}, "logits": {0: "n"}})

    shutil.copy(a.speciesnet / info["classifier_labels"], a.out / "labels.txt")
    shutil.copy(a.speciesnet / info["taxonomy"], a.out / "taxonomy.txt")
    shutil.copy(a.speciesnet / info["geofence"], a.out / "geofence.json")
    (a.out / "speciesnet.json").write_text(json.dumps({
        "speciesnet_version": info["version"], "detector": a.mdv6.stem, "detector_image_size": IMAGE_SIZE,
        "nms_iou": float(cfg.task.nms.min_iou), "max_boxes": int(cfg.task.nms.max_bbox),
        "classes": ["animal", "human", "vehicle"], "crop_size": 480,
    }, indent=1))
    print("wrote", sorted(p.name for p in a.out.iterdir()))


if __name__ == "__main__":
    main()
