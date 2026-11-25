"""Feasibility check: BioCLIP 2 as a camera-trap species classifier, run by SPARROW-Engine through a TOML manifest,
limited to the species GBIF has records of near each site. Not a Studio feature.

What it does:
1. Takes the video sightings Studio found in data/test.wlnw (run Studio on data/videos first) and crops each sighting's
   best MegaDetector box from its still, as SpeciesNet sees it.
2. Per site, asks GBIF which mammals (and the commonest birds) were recorded within 1 degree, and embeds a BioCLIP text
   prompt for each ("a photo of Animalia Chordata Mammalia ... Puma concolor") with open_clip (BioCLIP's text tower is not
   in the SPARROW zoo: only the image encoder is).
3. Builds a SPARROW classifier per site: the zoo's bioclip-2 image encoder ONNX with a zero-shot head appended
   (L2 normalise, then the text embeddings as a linear layer, times BioCLIP's logit scale), and a manifest.toml with the
   site's species as labels. Runs it with sparrow_engine.classify on the crops and on the whole stills.
4. Reports top-1 and top-5 accuracy against the known species, next to SpeciesNet's answer on the same sightings.

Usage: data/.venv/bin/python bioclip_check.py            (writes RESULTS.md and data/results.json)
"""
import json, math, os, pathlib, shutil, sqlite3, subprocess, sys, time, urllib.parse, urllib.request

HERE = pathlib.Path(__file__).parent
DATA = HERE / "data"
MODELS = DATA / "models"
UA = {"User-Agent": "WildlifeNetworkStudio-spike/0.1"}
TESTSET = json.load(open(HERE / "testset.json"))
N_TEST = 20


def get(url):
    for attempt in range(5):
        try:
            return json.load(urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=60))
        except Exception:
            if attempt == 4: raise
            time.sleep(3 * (attempt + 1))


def gbif_key(name, cache):
    k = "match:" + name
    if k not in cache:
        m = get("https://api.gbif.org/v1/species/match?" + urllib.parse.urlencode({"name": name, "kingdom": "Animalia"}))
        cache[k] = m.get("speciesKey") or m.get("acceptedUsageKey") or m.get("usageKey")
    return cache[k]


def candidates(site, cache):
    """Species recorded within 1 degree of the site: every mammal GBIF lists (up to 400) and the 150 commonest birds."""
    s = TESTSET["sites"][site]
    k = f"cands:{site}"
    if k not in cache:
        keys = []
        for class_key, limit in ((359, 400), (212, 150)):
            q = {"classKey": class_key, "decimalLatitude": f"{s['lat'] - 1},{s['lat'] + 1}", "decimalLongitude": f"{s['lon'] - 1},{s['lon'] + 1}",
                 "occurrenceStatus": "PRESENT", "facet": "speciesKey", "facetLimit": limit, "limit": 0}
            r = get("https://api.gbif.org/v1/occurrence/search?" + urllib.parse.urlencode(q))
            keys += [int(c["name"]) for c in r["facets"][0]["counts"]] if r.get("facets") else []
        out = []
        for key in keys:
            sk = f"sp:{key}"
            if sk not in cache: cache[sk] = get(f"https://api.gbif.org/v1/species/{key}")
            sp = cache[sk]
            if sp.get("rank") != "SPECIES" or not sp.get("species"): continue
            out.append({"key": key, "name": sp["species"], "tax": " ".join(sp.get(r, "") for r in ("kingdom", "phylum", "class", "order", "family", "genus")) + " " + sp["species"].split(" ", 1)[-1]})
        cache[k] = out
    return cache[k]


def sightings():
    """Animal sightings from the Studio run, with the video, site, kept still, best box and SpeciesNet's answer."""
    db = sqlite3.connect(DATA / "test.wlnw")
    rows = db.execute("""SELECT i.id, v.path, i.path, b.x, b.y, b.w, b.h, COALESCE(s.scientific, ''), b.score, i.start_s, i.end_s
        FROM images i JOIN videos v ON v.id = i.video_id JOIN image_boxes b ON b.image_id = i.id LEFT JOIN species s ON s.id = b.species
        WHERE b.kind = 'animal' OR b.category = 'animal' ORDER BY v.path, i.start_s""").fetchall()
    truth = {pathlib.Path(n).stem.replace(" ", "_"): (site, sp) for site, n, sp in TESTSET["videos"]}
    out = []
    for r in rows:
        stem = pathlib.Path(r[1]).stem
        site, sp = truth[stem]
        out.append({"image": r[0], "video": pathlib.Path(r[1]).name, "still": r[2], "box": r[3:7], "speciesnet": r[7], "speciesnet_score": r[8],
                    "start_s": r[9], "end_s": r[10], "site": site, "truth": sp})
    # One per video first, then the rest, so the 20 cover every video.
    seen, first, rest = set(), [], []
    for s in out: (rest if s["video"] in seen else first).append(s); seen.add(s["video"])
    return (first + rest)[:N_TEST]


def crop(s, pad=0.1):
    from PIL import Image
    im = Image.open(s["still"]).convert("RGB")
    W, H = im.size
    x, y, w, h = s["box"]
    x0, y0 = max(0, (x - w * pad) * W), max(0, (y - h * pad) * H)
    x1, y1 = min(W, (x + w * (1 + pad)) * W), min(H, (y + h * (1 + pad)) * H)
    out = DATA / "crops" / f"{s['image']}.jpg"
    out.parent.mkdir(exist_ok=True)
    im.crop((int(x0), int(y0), int(x1), int(y1))).save(out, quality=95)
    return out


_CLIP = None


def clip():
    global _CLIP
    if _CLIP is None:
        import open_clip
        model, _, pre = open_clip.create_model_and_transforms("hf-hub:imageomics/bioclip-2")
        model.eval()
        _CLIP = (model, pre, open_clip.get_tokenizer("hf-hub:imageomics/bioclip-2"))
    return _CLIP


def text_embeddings(cands):
    import torch
    model, _, tok = clip()
    with torch.no_grad():
        feats = []
        for i in range(0, len(cands), 64):
            t = model.encode_text(tok([f"a photo of {c['tax']}." for c in cands[i:i + 64]]))
            feats.append(t / t.norm(dim=-1, keepdim=True))
        return torch.cat(feats).numpy(), float(model.logit_scale.exp())


def openclip_image_embedding(path):
    import torch
    from PIL import Image
    model, pre, _ = clip()
    with torch.no_grad():
        f = model.encode_image(pre(Image.open(path).convert("RGB")).unsqueeze(0))
        return (f / f.norm(dim=-1, keepdim=True))[0].numpy()


def build_classifier(site, cands, W, scale):
    """The zoo encoder with a zero-shot head, as a SPARROW classifier: <models>/bioclip-2-zs-<site>/."""
    import numpy as np, onnx
    from onnx import helper, numpy_helper
    mid = f"bioclip-2-zs-{site.replace('_', '-')}"
    d = MODELS / mid
    if d.exists(): shutil.rmtree(d)
    (d / "1").mkdir(parents=True)
    m = onnx.load(str(MODELS / "bioclip-2" / "1" / "model.onnx"))
    g = m.graph
    feat = g.output[0].name
    g.initializer.extend([numpy_helper.from_array(W.T.astype(np.float32), "zs_text"), numpy_helper.from_array(np.array(scale, dtype=np.float32), "zs_scale")])
    g.node.extend([helper.make_node("LpNormalization", [feat], ["zs_norm"], axis=1, p=2),
                   helper.make_node("MatMul", ["zs_norm", "zs_text"], ["zs_cos"]),
                   helper.make_node("Mul", ["zs_cos", "zs_scale"], ["logits"])])
    del g.output[:]
    g.output.extend([helper.make_tensor_value_info("logits", onnx.TensorProto.FLOAT, ["batch", len(cands)])])
    onnx.save(m, str(d / "1" / "model.onnx"))
    (d / "labels.txt").write_text("".join(f"{c['name']},{i}\n" for i, c in enumerate(cands)))
    base = (MODELS / "bioclip-2" / "manifest.toml").read_text()
    pre = base[base.index("[preprocessing]"):base.index("[postprocessing]")]
    (d / "manifest.toml").write_text(f"""[model]
id = "{mid}"
format = "onnx"
file = "1/model.onnx"
version = "v2-zero-shot"
description = "BioCLIP 2 image encoder with a zero-shot head: text embeddings of the {len(cands)} species GBIF has records of within 1 degree of {TESTSET['sites'][site]['place']}. Feasibility check only."
domain = "camera_trap"
task = "classifier"
family = ["BioCLIP"]
status = "experimental"
license = "MIT"
commercial_use = true
geo_scope = "foundational"
geo_regions = []
reference = "Gu et al. (2025) BioCLIP 2. https://arxiv.org/abs/2505.23883"

{pre}[postprocessing]
method = "softmax"

[labels]
file = "labels.txt"
format = "name_index_csv"
""")
    return mid


def classify(mid, paths):
    """Runs in its own process so each site's classifier is found by a fresh engine."""
    out = subprocess.run([sys.executable, __file__, "classify", mid, *map(str, paths)], capture_output=True, text=True)
    if out.returncode != 0: raise RuntimeError(out.stderr[-3000:])
    return json.loads(out.stdout.strip().splitlines()[-1])


def classify_main(mid, paths):
    import sparrow_engine as spe
    spe.init(device="cpu", model_dir=str(MODELS))
    t = time.time()
    res = spe.classify(paths, model=mid, top_k=5)
    secs = time.time() - t
    by_path = {r.path: [(c.label, c.confidence) for c in r.classifications] for r in res}
    print(json.dumps({"preds": [by_path[p] for p in paths], "seconds": secs}))


def main():
    cache_f = DATA / "gbif_cache.json"
    cache = json.load(open(cache_f)) if cache_f.exists() else {}
    tests = sightings()
    print(f"{len(tests)} test sightings from {len({t['video'] for t in tests})} videos")
    for t in tests:
        t["truth_key"] = gbif_key(t["truth"], cache)
        t["speciesnet_key"] = gbif_key(t["speciesnet"], cache) if " " in t["speciesnet"] else None
        t["crop"] = str(crop(t))
    # The zoo encoder and open_clip should give the same embedding for the same picture.
    import numpy as np, sparrow_engine as spe
    spe.init(device="cpu", model_dir=str(MODELS))
    a = spe.embed(tests[0]["crop"], model="bioclip-2")
    b = openclip_image_embedding(tests[0]["crop"])
    agree = float(np.dot(a / np.linalg.norm(a), b))
    print(f"SPARROW bioclip-2 vs open_clip image embedding, cosine {agree:.4f}")
    results = {"agree_cosine": agree, "sites": {}, "tests": tests}
    for site in sorted({t["site"] for t in tests}):
        cands = candidates(site, cache)
        json.dump(cache, open(cache_f, "w"))
        keys = {c["key"] for c in cands}
        W, scale = text_embeddings(cands)
        mid = build_classifier(site, cands, W, scale)
        mine = [t for t in tests if t["site"] == site]
        crops = classify(mid, [t["crop"] for t in mine])
        stills = classify(mid, [t["still"] for t in mine])
        name_key = {c["name"]: c["key"] for c in cands}
        for t, pc, ps in zip(mine, crops["preds"], stills["preds"]):
            t["bioclip_crop"] = [(n, round(p, 4), name_key.get(n)) for n, p in pc]
            t["bioclip_still"] = [(n, round(p, 4), name_key.get(n)) for n, p in ps]
            t["truth_in_candidates"] = t["truth_key"] in keys
        results["sites"][site] = {"candidates": len(cands), "mammals_and_birds": len(cands), "seconds_per_crop": crops["seconds"] / max(1, len(mine)), "model": mid}
        shutil.rmtree(MODELS / mid)  # 1.2 GB each
        print(site, len(cands), "candidates")
    json.dump(results, open(DATA / "results.json", "w"), indent=1)
    report(results)


def report(res):
    tests = res["tests"]
    def acc(f): return sum(1 for t in tests if f(t)), len(tests)
    top1c = acc(lambda t: t["bioclip_crop"][0][2] == t["truth_key"])
    top5c = acc(lambda t: any(p[2] == t["truth_key"] for p in t["bioclip_crop"]))
    top1s = acc(lambda t: t["bioclip_still"][0][2] == t["truth_key"])
    sn = acc(lambda t: t["speciesnet_key"] == t["truth_key"])
    inc = acc(lambda t: t["truth_in_candidates"])
    pct = lambda a: f"{a[0]} of {a[1]} ({100 * a[0] / a[1]:.0f}%)"
    lines = ["# BioCLIP 2 through SPARROW-Engine: feasibility check", "",
             f"Run {time.strftime('%Y-%m-%d')} with sparrow-engine {__import__('sparrow_engine').__version__}, the zoo's bioclip-2 encoder (Sparrow Model Zoo 0.33.0, Zenodo record 23992557) and BioCLIP 2's text tower from imageomics/bioclip-2 (open_clip).", "",
             "## Result", "",
             f"| On {len(tests)} video sightings | Top 1 | Top 5 |", "|---|---|---|",
             f"| BioCLIP 2 zero-shot, MegaDetector crop | {pct(top1c)} | {pct(top5c)} |",
             f"| BioCLIP 2 zero-shot, whole frame | {pct(top1s)} | |",
             f"| SpeciesNet 4.0.3a (Studio today, species level) | {pct(sn)} | |", "",
             f"True species among GBIF's nearby list: {pct(inc)}. SPARROW's encoder and open_clip agree on the same picture (cosine {res['agree_cosine']:.4f}).", "",
             "| Site | Species offered (GBIF, within 1 degree) | Seconds per crop (CPU) |", "|---|---|---|"]
    for s, v in res["sites"].items(): lines.append(f"| {TESTSET['sites'][s]['place']} | {v['candidates']} | {v['seconds_per_crop']:.2f} |")
    lines += ["", "## Each sighting", "", "| Video | Truth | SpeciesNet | BioCLIP crop, top 1 | BioCLIP whole frame |", "|---|---|---|---|---|"]
    for t in tests:
        mark = lambda ok: "yes" if ok else "no"
        lines.append(f"| {t['video']} {t['start_s']:.0f} to {t['end_s']:.0f} s | {t['truth']} | {t['speciesnet'] or '(none)'} ({mark(t['speciesnet_key'] == t['truth_key'])}) | {t['bioclip_crop'][0][0]} {t['bioclip_crop'][0][1]:.2f} ({mark(t['bioclip_crop'][0][2] == t['truth_key'])}) | {t['bioclip_still'][0][0]} ({mark(t['bioclip_still'][0][2] == t['truth_key'])}) |")
    (HERE / "RESULTS.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines[:12]))


if __name__ == "__main__":
    os.environ.setdefault("HF_HOME", str(DATA / "hf"))
    if len(sys.argv) > 2 and sys.argv[1] == "classify": classify_main(sys.argv[2], sys.argv[3:])
    else: main()
