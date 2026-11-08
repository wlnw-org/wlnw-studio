"""Reference results for Studio's camera-trap pipeline, from the Python implementations, and a check of the ONNX files.

Pipeline (what Studio reproduces in crates/engine/src/camtrap.rs):
  image  -> speciesnet.utils.load_rgb_image (RGB, EXIF orientation applied)
  detect -> MegaDetector v6 MIT exactly as Pytorch-Wildlife runs it (PadAndResize: PIL Lanczos to fit 640, grey 114 padding,
            to_tensor; YOLOv9 network; sigmoid; PostProcess / bbox_nms with torchvision batched_nms at IoU 0.5, max 300),
            boxes mapped back to the image, clipped to it, under 2 px dropped, and written as SpeciesNet detections (normalised x, y, w, h), at
            confidence >= 0.01 (SpeciesNet's own detector threshold), highest first
  classify -> speciesnet SpeciesNetClassifier.preprocess / batch_predict on every detection >= 0.1 (SpeciesNet's default)
  ensemble -> speciesnet SpeciesNetEnsemble.combine with the country given (geofencing on)

  python spike/speciesnet_check.py --pack models/speciesnet --speciesnet DIR --mdv6 CKPT --mdv6-config YAML \
      --cameratraps PATH --images IMG_DIR --country USA --out reference.json
"""
import argparse
import json
import sys
import time
from pathlib import Path

import numpy as np
import torch


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pack", type=Path, required=True)
    ap.add_argument("--speciesnet", type=Path, required=True)
    ap.add_argument("--mdv6", type=Path, required=True)
    ap.add_argument("--mdv6-config", type=Path, required=True)
    ap.add_argument("--cameratraps", type=Path, required=True)
    ap.add_argument("--images", type=Path, required=True)
    ap.add_argument("--country", default=None)
    ap.add_argument("--admin1", default=None)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--tensors", type=Path, default=None, help="also write preprocessing tensors of the first image for Rust tests")
    a = ap.parse_args()
    torch.set_num_threads(8)

    sys.path.insert(0, str(Path(__file__).parent))
    from speciesnet_export import load_detector
    from speciesnet.classifier import SpeciesNetClassifier
    from speciesnet.ensemble import SpeciesNetEnsemble
    from speciesnet.utils import BBox, load_rgb_image, select_detections_for_classification, build_classifier_result
    from speciesnet.constants import Detection
    import onnxruntime as ort

    cfg, model, converter = load_detector(a.mdv6, a.mdv6_config, a.cameratraps)
    from yolo import AugmentationComposer, PostProcess
    cfg.task.nms.min_confidence = 0.01
    post = PostProcess(converter, cfg.task.nms)
    transform = AugmentationComposer([], cfg.image_size, cfg.image_size[0])
    clf = SpeciesNetClassifier(str(a.speciesnet), device="cpu")
    ens = SpeciesNetEnsemble(str(a.speciesnet), geofence=True)
    so = ort.SessionOptions()
    det_onnx = ort.InferenceSession(str(a.pack / "detector.onnx"), so, providers=["CPUExecutionProvider"])
    clf_onnx = ort.InferenceSession(str(a.pack / "speciesnet_v403a.onnx"), so, providers=["CPUExecutionProvider"])
    names = {0: Detection.ANIMAL.value, 1: Detection.HUMAN.value, 2: Detection.VEHICLE.value}

    out = {"images": [], "onnx": {"detector_max_diff": 0.0, "box_max_diff": 0.0, "classifier_max_diff": 0.0}}
    files = sorted(p for p in a.images.iterdir() if p.suffix.lower() in (".jpg", ".jpeg", ".png") and "truncated" not in p.name)
    t_det = t_clf = 0.0
    for k, f in enumerate(files):
        img = load_rgb_image(str(f))
        if img is None:
            continue
        x, _, rev = transform(img)
        x = x[None]
        t0 = time.time()
        with torch.no_grad():
            raw = model(x)
            cls, _, box = converter(raw["Main"])[:3]
            res = post(raw, rev[None])[0]
        t_det += time.time() - t0
        o_scores, o_boxes = det_onnx.run(None, {"image": x.numpy()})
        out["onnx"]["detector_max_diff"] = max(out["onnx"]["detector_max_diff"], float(np.abs(o_scores - cls.sigmoid().numpy()).max()))
        out["onnx"]["box_max_diff"] = max(out["onnx"]["box_max_diff"], float(np.abs(o_boxes - box.numpy()).max()))
        W, H = img.width, img.height
        dets = []
        for c, x1, y1, x2, y2, conf in res.tolist():
            x1, x2 = min(max(x1, 0.0), W), min(max(x2, 0.0), W)
            y1, y2 = min(max(y1, 0.0), H), min(max(y2, 0.0), H)
            if x2 - x1 < 2 or y2 - y1 < 2:  # slivers (under 2 px) cannot be cropped
                continue
            dets.append({"category": str(int(c) + 1), "label": names[int(c)], "conf": conf,
                         "bbox": [x1 / W, y1 / H, (x2 - x1) / W, (y2 - y1) / H]})
        dets.sort(key=lambda d: d["conf"], reverse=True)
        idx = select_detections_for_classification(dets, 0.1, None)
        crops = clf.preprocess(img, bboxes=[BBox(*dets[i]["bbox"]) for i in idx])
        t0 = time.time()
        pred = clf.batch_predict([str(f)], [crops], num_classes=5)[0]
        t_clf += time.time() - t0
        if crops:
            batch = np.stack([c.arr / 255.0 for c in crops]).astype(np.float32)
            logits = clf_onnx.run(None, {"crop": batch})[0]
            with torch.no_grad():
                ref = torch.cat([clf.model(torch.from_numpy(batch[i:i + 8])) for i in range(0, len(batch), 8)]).numpy()
            out["onnx"]["classifier_max_diff"] = max(out["onnx"]["classifier_max_diff"], float(np.abs(logits - ref).max()))
        classified = build_classifier_result(pred, dets, idx, False)
        geo = {"country": a.country, "admin1_region": a.admin1}
        final = ens.combine([str(f)], {str(f): classified}, {str(f): {"filepath": str(f), "detections": dets}}, {str(f): geo},
                            max_classifications_per_detection=5)[0]
        out["images"].append({"file": f.name, "width": W, "height": H, "detections": final.get("detections", [])})
        if k == 0 and a.tensors is not None:
            np.save(a.tensors / "det_input.npy", x.numpy())
            if crops:
                np.save(a.tensors / "crop0.npy", crops[0].arr)
    out["seconds"] = {"detector": t_det, "classifier": t_clf, "images": len(out["images"])}
    a.out.write_text(json.dumps(out, indent=1))
    print(json.dumps(out["onnx"]), json.dumps(out["seconds"]))
    for im in out["images"]:
        top = [(d["label"], round(d["conf"], 3), (d.get("processed_classification") or {}).get("class", "").split(";")[-1],
                round((d.get("processed_classification") or {}).get("score", 0), 3)) for d in im["detections"] if d["conf"] >= 0.1]
        print(im["file"], top)


if __name__ == "__main__":
    main()
