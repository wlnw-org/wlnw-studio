"""BioCLIP 2 as a second opinion next to SpeciesNet: the measurement behind RESULTS.md. Test only, no Studio change.

Stages (run in order; each caches its output in data/eval/):
  studio   SpeciesNet 4.0.3a as Studio runs it: the wlnw CLI (built from this repo) scans data/eval, sets each camera's
           place and country, runs MegaDetector + SpeciesNet. Reads the best animal box and SpeciesNet's answer per image.
  cands    Per place, the species GBIF has records of within 1 degree (every mammal up to 400, birds up to 800 by number
           of records) plus common domestic animals, and BioCLIP 2 text embeddings for each (open_clip text tower).
  bioclip  (b) BioCLIP 2 zero-shot with open_clip/PyTorch on the MegaDetector crop and on the whole frame.
  onnx     (c) the zoo's ONNX image encoder (FP32 and FP16) with ONNX Runtime + the precomputed text table, by dot product.
  (eval_report.py writes the evaluation sections of RESULTS.md from data/eval/*.json.)

Usage: data/.venv/bin/python -I eval.py <stage>
"""
import csv, json, math, os, pathlib, random, resource, sqlite3, subprocess, sys, time, urllib.parse, urllib.request
from concurrent.futures import ThreadPoolExecutor

HERE = pathlib.Path(__file__).resolve().parent
DATA = HERE / "data"
EVAL = DATA / "eval"
REPO = HERE.parent.parent
WLNW = REPO / "target" / "release" / "wlnw"
PROJECT = EVAL / "eval.wlnw"
UA = {"User-Agent": "WildlifeNetworkStudio-spike/0.1"}
ZOO = {"fp32": DATA / "models" / "bioclip-2" / "1" / "model.onnx", "fp16": DATA / "models" / "bioclip-2-fp16" / "1" / "model_fp16.onnx"}
# Common domestic animals: GBIF's nearby lists rarely hold them, and camera traps often see them.
DOMESTIC = ["Canis familiaris", "Felis catus", "Bos taurus", "Bos indicus", "Bubalus bubalis", "Equus caballus", "Equus asinus",
            "Capra hircus", "Ovis aries", "Sus scrofa", "Gallus gallus", "Homo sapiens"]
os.environ.setdefault("HF_HOME", str(DATA / "hf"))


def tests():
    rows = []
    for name in ("testset_inat.csv", "testset_lila.csv", "testset_ours.csv"):
        rows += list(csv.DictReader(open(HERE / name)))
    return rows


def jload(name, default=None):
    p = EVAL / name
    return json.load(open(p)) if p.exists() else default


def jsave(name, obj):
    json.dump(obj, open(EVAL / name, "w"), indent=0)


# ─── GBIF ──────────────────────────────────────────────────────────────────────────────────────────────────────────

_cache_f = DATA / "gbif_cache.json"
CACHE = json.load(open(_cache_f)) if _cache_f.exists() else {}


def get(url):
    for attempt in range(6):
        try:
            return json.load(urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=60))
        except Exception:
            if attempt == 5: raise
            time.sleep(3 * (attempt + 1))


def species(key):
    k = f"sp:{key}"
    if k not in CACHE: CACHE[k] = get(f"https://api.gbif.org/v1/species/{key}")
    return CACHE[k]


def match(name):
    """GBIF accepted species key and genus for a name (synonyms resolve to the accepted species)."""
    k = "match2:" + name
    if k not in CACHE:
        m = get("https://api.gbif.org/v1/species/match?" + urllib.parse.urlencode({"name": name, "kingdom": "Animalia"}))
        CACHE[k] = {"key": m.get("speciesKey") or m.get("acceptedUsageKey") or m.get("usageKey"), "genus": m.get("genus"), "rank": m.get("rank"),
                    "family": m.get("family")}
    return CACHE[k]


def tax(sp):
    return " ".join(sp.get(r, "") for r in ("kingdom", "phylum", "class", "order", "family", "genus")) + " " + sp["species"].split(" ", 1)[-1]


def cell(lat, lon):
    return f"{round(float(lat) * 2) / 2:.1f},{round(float(lon) * 2) / 2:.1f}"


def nearby(c):
    k = f"near:{c}"
    if k not in CACHE:
        lat, lon = map(float, c.split(","))
        keys = []
        for class_key, limit in ((359, 400), (212, 800)):
            q = {"classKey": class_key, "decimalLatitude": f"{lat - 1},{lat + 1}", "decimalLongitude": f"{lon - 1},{lon + 1}",
                 "occurrenceStatus": "PRESENT", "facet": "speciesKey", "facetLimit": limit, "limit": 0}
            r = get("https://api.gbif.org/v1/occurrence/search?" + urllib.parse.urlencode(q))
            keys += [int(x["name"]) for x in r["facets"][0]["counts"]] if r.get("facets") else []
        CACHE[k] = keys
    return CACHE[k]


def save_cache():
    json.dump(CACHE, open(_cache_f, "w"))


# ─── stage: studio ─────────────────────────────────────────────────────────────────────────────────────────────────

def stage_studio():
    rows = tests()
    if not WLNW.exists():
        subprocess.run(["cargo", "build", "--release", "-q", "--bin", "wlnw"], cwd=REPO, check=True,
                       env={**os.environ, "PATH": f"{pathlib.Path.home()}/.rustup/toolchains/stable-aarch64-apple-darwin/bin:" + os.environ["PATH"]})
    run = lambda *a: subprocess.run([str(WLNW), *map(str, a)], check=True, capture_output=True, text=True).stdout
    if not PROJECT.exists():
        for folder in sorted({str((DATA / r["file"]).parent) for r in rows}): run("scan", PROJECT, folder)
    db = sqlite3.connect(PROJECT)
    by_folder = {}
    for r in rows: by_folder.setdefault(str((DATA / r["file"]).parent), r)
    for dep_id, folder in db.execute("SELECT id, folder FROM deployments").fetchall():
        r = by_folder[folder]
        run("place", PROJECT, dep_id, r["lat"], r["lon"], 0)
        admin = {"ENA24": "VA"}.get(r["state"])
        run("country", PROJECT, dep_id, r["country"], *([admin] if admin else []))
    db.close()
    run("use", PROJECT, "speciesnet-4.0.3a")
    t = time.time()
    subprocess.run([str(WLNW), "run", str(PROJECT), str(REPO / "models")], check=True)
    secs = time.time() - t
    db = sqlite3.connect(PROJECT)
    out = {}
    for r in rows:
        path = str(DATA / r["file"])
        img = db.execute("SELECT id FROM images WHERE path = ?", [path]).fetchone()
        if not img: out[r["file"]] = {"missing": True}; continue
        boxes = db.execute("""SELECT b.kind, b.category, b.conf, b.x, b.y, b.w, b.h, COALESCE(s.scientific, ''), COALESCE(b.label, ''), b.score, b.top
            FROM image_boxes b LEFT JOIN species s ON s.id = b.species WHERE b.image_id = ? ORDER BY b.conf DESC""", [img[0]]).fetchall()
        animals = [b for b in boxes if b[0] == "animal"]
        best = animals[0] if animals else None
        out[r["file"]] = {"boxes": len(boxes), "animals": len(animals), "kinds": sorted({b[0] for b in boxes}),
                          "box": list(best[3:7]) if best else None, "md_conf": best[2] if best else None,
                          "sn_name": best[7] if best else "", "sn_label": best[8] if best else "", "sn_score": best[9] if best else None,
                          "sn_top": json.loads(best[10]) if best and best[10] else []}
    jsave("studio.json", {"seconds": secs, "images": len(rows), "results": out})
    print(f"SpeciesNet on {len(rows)} images in {secs:.0f} s")


# ─── stage: cands ──────────────────────────────────────────────────────────────────────────────────────────────────

_CLIP = None


def clip():
    global _CLIP
    if _CLIP is None:
        import open_clip
        model, _, pre = open_clip.create_model_and_transforms("hf-hub:imageomics/bioclip-2")
        model.eval()
        _CLIP = (model, pre, open_clip.get_tokenizer("hf-hub:imageomics/bioclip-2"))
    return _CLIP


def stage_cands():
    import numpy as np, torch
    rows = tests()
    cells = sorted({cell(r["lat"], r["lon"]) for r in rows})
    print(len(cells), "places")
    with ThreadPoolExecutor(6) as ex: list(ex.map(nearby, cells))
    save_cache()
    keys = sorted({k for c in cells for k in nearby(c)})
    print(len(keys), "species keys to look up")
    with ThreadPoolExecutor(8) as ex: list(ex.map(species, keys))
    save_cache()
    dom = [match(n)["key"] for n in DOMESTIC]
    with ThreadPoolExecutor(8) as ex: list(ex.map(species, [k for k in dom if k]))
    names = {}
    for k in set(keys) | {k for k in dom if k}:
        sp = species(k)
        if sp.get("rank") == "SPECIES" and sp.get("species") and sp.get("kingdom") == "Animalia":
            names[k] = {"key": k, "name": sp["species"], "genus": sp.get("genus"), "family": sp.get("family"), "class": sp.get("class"), "tax": tax(sp)}
    per_cell = {c: sorted({k for k in nearby(c) if k in names} | {k for k in dom if k in names}) for c in cells}
    order = sorted(names)
    model, _, tok = clip()
    feats = []
    with torch.no_grad():
        for i in range(0, len(order), 64):
            t = model.encode_text(tok([f"a photo of {names[k]['tax']}." for k in order[i:i + 64]]))
            feats.append((t / t.norm(dim=-1, keepdim=True)).numpy())
    W = np.concatenate(feats).astype(np.float32)
    np.save(EVAL / "text_fp32.npy", W)
    jsave("cands.json", {"order": order, "names": {str(k): v for k, v in names.items()}, "cells": {c: v for c, v in per_cell.items()},
                         "logit_scale": float(model.logit_scale.exp())})
    print(len(order), "species embedded;", {c: len(v) for c, v in per_cell.items()})


# ─── shared: crops ─────────────────────────────────────────────────────────────────────────────────────────────────

def crop_image(r, studio, pad=0.1):
    from PIL import Image
    im = Image.open(DATA / r["file"]).convert("RGB")
    s = studio.get(r["file"], {})
    if not s.get("box"): return None, im
    W, H = im.size
    x, y, w, h = s["box"]
    x0, y0 = max(0, (x - w * pad) * W), max(0, (y - h * pad) * H)
    x1, y1 = min(W, (x + w * (1 + pad)) * W), min(H, (y + h * (1 + pad)) * H)
    return im.crop((int(x0), int(y0), int(x1), int(y1))), im


def topk(sims, scale, idx, order, k=5):
    import numpy as np
    logits = sims[idx] * scale
    p = np.exp(logits - logits.max()); p /= p.sum()
    best = np.argsort(-p)[:k]
    return [[int(order[idx[i]]), float(p[i])] for i in best]


# ─── stage: bioclip (b) ────────────────────────────────────────────────────────────────────────────────────────────

def stage_bioclip():
    import numpy as np, torch
    rows, studio, C = tests(), jload("studio.json")["results"], jload("cands.json")
    W, order, scale = np.load(EVAL / "text_fp32.npy"), C["order"], C["logit_scale"]
    pos = {k: i for i, k in enumerate(order)}
    model, pre, _ = clip()
    out, secs = {}, []
    for r in rows:
        idx = np.array([pos[k] for k in C["cells"][cell(r["lat"], r["lon"])]])
        crop, whole = crop_image(r, studio)
        res = {}
        for name, im in (("crop", crop), ("whole", whole)):
            if im is None: continue
            t = time.time()
            with torch.no_grad():
                f = model.encode_image(pre(im).unsqueeze(0))
                f = (f / f.norm(dim=-1, keepdim=True))[0].numpy()
            if name == "crop": secs.append(time.time() - t)
            res[name] = topk(W @ f, scale, idx, order)
            res[name + "_emb"] = f.tolist() if name == "crop" else None
        out[r["file"]] = {k: v for k, v in res.items() if not k.endswith("_emb")}
        out[r["file"]]["crop_emb"] = res.get("crop_emb")
    jsave("bioclip.json", {"seconds_per_crop": sorted(secs)[len(secs) // 2] if secs else None, "results": out})
    print("open_clip median s/crop", sorted(secs)[len(secs) // 2])


# ─── stage: onnx (c) ───────────────────────────────────────────────────────────────────────────────────────────────

def preprocess(im):
    """The zoo manifest: shorter side to 224 (bicubic), centre crop 224, [0,1] RGB, NCHW; CLIP normalisation is in the graph."""
    import numpy as np
    from PIL import Image
    w, h = im.size
    s = 224 / min(w, h)
    im = im.resize((max(224, round(w * s)), max(224, round(h * s))), Image.BICUBIC)
    w, h = im.size
    l, t = (w - 224) // 2, (h - 224) // 2
    a = np.asarray(im.crop((l, t, l + 224, t + 224)), dtype=np.float32) / 255.0
    return a.transpose(2, 0, 1)[None]


def onnx_worker(variant):
    """Runs in its own process so its memory can be measured on its own."""
    import numpy as np, onnxruntime as ort
    rows, studio, C = tests(), jload("studio.json")["results"], jload("cands.json")
    order, scale = C["order"], C["logit_scale"]
    W = np.load(EVAL / "text_fp32.npy")
    if variant == "fp16": W = W.astype(np.float16)
    pos = {k: i for i, k in enumerate(order)}
    rss0 = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    t = time.time()
    so = ort.SessionOptions()
    sess = ort.InferenceSession(str(ZOO[variant]), so, providers=["CPUExecutionProvider"])
    load_s = time.time() - t
    inp = sess.get_inputs()[0]
    out, secs = {}, []
    for r in rows:
        idx = np.array([pos[k] for k in C["cells"][cell(r["lat"], r["lon"])]])
        crop, whole = crop_image(r, studio)
        res = {}
        for name, im in (("crop", crop), ("whole", whole)):
            if im is None: continue
            x = preprocess(im)
            if "float16" in inp.type: x = x.astype(np.float16)
            t = time.time()
            f = sess.run(None, {inp.name: x})[0][0].astype(np.float32 if variant == "fp32" else np.float16)
            f = f / np.linalg.norm(f.astype(np.float32)).astype(f.dtype)
            sims = (W @ f).astype(np.float32)
            if name == "crop": secs.append(time.time() - t)
            res[name] = topk(sims, scale, idx, order)
            if name == "crop": res["crop_emb"] = f.astype(np.float32).tolist()
        out[r["file"]] = res
    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss  # bytes on macOS
    print(json.dumps({"variant": variant, "seconds_per_crop": sorted(secs)[len(secs) // 2], "load_s": load_s,
                      "peak_rss_mb": rss / 1e6, "rss_before_mb": rss0 / 1e6, "input_type": inp.type, "results": out}))


def stage_onnx():
    res = {}
    for v in ("fp32", "fp16"):
        p = subprocess.run([sys.executable, "-I", __file__, "onnx-worker", v], capture_output=True, text=True)
        if p.returncode != 0: raise RuntimeError(p.stderr[-3000:])
        res[v] = json.loads(p.stdout.strip().splitlines()[-1])
        print(v, {k: res[v][k] for k in ("seconds_per_crop", "load_s", "peak_rss_mb", "input_type")})
    jsave("onnx.json", res)


# ─── stage: india table size ───────────────────────────────────────────────────────────────────────────────────────

def india_species():
    """How many species of the classes camera traps see GBIF lists for India (facet counts, all records)."""
    out = {}
    for name, key in (("mammals", 359), ("birds", 212), ("reptiles", 11592253), ("amphibians", 131)):
        q = {"country": "IN", "classKey": key, "occurrenceStatus": "PRESENT", "facet": "speciesKey", "facetLimit": 10000, "limit": 0}
        r = get("https://api.gbif.org/v1/occurrence/search?" + urllib.parse.urlencode(q))
        out[name] = len(r["facets"][0]["counts"]) if r.get("facets") else 0
    return out


def stage_matches():
    """GBIF backbone match for every true species and every SpeciesNet answer, so the report can compare keys and genera."""
    names = {r["true_species"] for r in tests()} | {v["sn_name"] for v in jload("studio.json")["results"].values() if v.get("sn_name")}
    with ThreadPoolExecutor(8) as ex: list(ex.map(match, sorted(names)))
    save_cache()
    print(len(names), "names matched")


def stage_extra():
    """Sizes, (b) versus (c) embedding agreement, and the India species count, for the report."""
    import numpy as np
    B, O = jload("bioclip.json")["results"], jload("onnx.json")
    cos = {}
    for v in ("fp32", "fp16"):
        c = [float(np.dot(B[f]["crop_emb"], O[v]["results"][f]["crop_emb"]) / np.linalg.norm(O[v]["results"][f]["crop_emb"]))
             for f in B if B[f].get("crop_emb") and O[v]["results"].get(f, {}).get("crop_emb")]
        cos[v] = sorted(c)[len(c) // 2]
    zips = {"fp32": DATA / "models" / "bioclip-2.zip", "fp16": DATA / "models" / "dl16" / "bioclip-2-fp16.zip"}
    jsave("extra.json", {"cos": cos, "zip_mb": {v: p.stat().st_size / 1e6 for v, p in zips.items()},
                         "onnx_mb": {v: p.stat().st_size / 1e6 for v, p in ZOO.items()}, "india": india_species()})
    print(jload("extra.json"))


if __name__ == "__main__":
    st = sys.argv[1]
    if st == "onnx-worker": onnx_worker(sys.argv[2])
    else: {"studio": stage_studio, "cands": stage_cands, "bioclip": stage_bioclip, "onnx": stage_onnx, "extra": stage_extra, "matches": stage_matches}[st]()
