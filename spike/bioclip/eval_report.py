"""Writes the evaluation sections of RESULTS.md (between the licences section and the earlier feasibility check) from
data/eval/{studio,cands,bioclip,onnx,extra}.json and the testset CSVs. Usage: data/.venv/bin/python -I eval_report.py"""
import csv, json, math, pathlib, re

HERE = pathlib.Path(__file__).resolve().parent
EVAL = HERE / "data" / "eval"
J = lambda n: json.load(open(EVAL / n))
CACHE = json.load(open(HERE / "data" / "gbif_cache.json"))


def wilson(k, n, z=1.96):
    if n == 0: return (0.0, 0.0, 0.0)
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return p, max(0, c - h), min(1, c + h)


def fmt(k, n):
    if n == 0: return "n/a"
    p, lo, hi = wilson(k, n)
    return f"{k}/{n} = {100 * p:.0f}% ({100 * lo:.0f} to {100 * hi:.0f})"


def main():
    rows = []
    for name in ("testset_inat.csv", "testset_lila.csv", "testset_ours.csv"): rows += list(csv.DictReader(open(HERE / name)))
    studio, C, B, O, X = J("studio.json"), J("cands.json"), J("bioclip.json"), J("onnx.json"), J("extra.json")
    names = {int(k): v for k, v in C["names"].items()}
    truth = lambda r: CACHE["match2:" + r["true_species"]]
    sres, bres = studio["results"], B["results"]

    def bio_pred(r, which="crop", res=None):
        res = res or bres
        x = res.get(r["file"], {})
        return x.get(which) or None

    def renamed(pn, pfam, r):
        """Same species under another genus name: iNaturalist follows newer bird taxonomy than the GBIF backbone
        (Ixobrychus sinensis = Botaurus sinensis, Charadrius dubius = Thinornis dubius, Bubulcus coromandus =
        Ardea coromanda). Counted as the same species when the family and the species epithet agree."""
        return bool(pn) and pfam is not None and pfam == truth(r).get("family") and pn.split()[-1] == r["true_species"].split()[-1] and len(pn.split()) == 2

    def right(pred_key, r, level):
        t = truth(r)
        if pred_key is None: return False
        p = names.get(pred_key, {})
        if pred_key == t["key"] or renamed(p.get("name"), p.get("family"), r): return True
        return level == "genus" and p.get("genus") == t["genus"]

    def sn_right(r, level):
        s = sres.get(r["file"], {})
        if not s.get("sn_name"): return False
        m = CACHE.get("match2:" + s["sn_name"]) or {}
        t = truth(r)
        if m.get("rank") == "SPECIES" and (m.get("key") == t["key"] or renamed(s["sn_name"], m.get("family"), r)): return True
        if level == "species": return False
        return m.get("genus") is not None and m.get("genus") == t["genus"] and m.get("rank") in ("SPECIES", "GENUS", "SUBSPECIES")

    def sn_case(r):
        s = sres.get(r["file"], {})
        if not s.get("animals"): return "no animal box (blank)"
        m = CACHE.get("match2:" + s["sn_name"]) if s.get("sn_name") else None
        if not s.get("sn_name") or not m or m.get("rank") not in ("SPECIES", "SUBSPECIES"): return "stopped above species"
        return "right" if sn_right(r, "species") else "wrong species"

    groups = [("All", lambda r: True), ("Birds", lambda r: r["group"] == "bird"), ("Mammals", lambda r: r["group"] == "mammal"),
              ("iNaturalist", lambda r: r["source"] == "inaturalist"), ("LILA camera traps", lambda r: r["source"] == "lila"),
              ("Our footage", lambda r: r["source"] == "ours")]
    L = []
    w = L.append
    n_src = {s: sum(1 for r in rows if r["source"] == s) for s in ("inaturalist", "lila", "ours")}
    sp_src = {s: len({r["true_species"] for r in rows if r["source"] == s}) for s in ("inaturalist", "lila", "ours")}
    inat = [r for r in rows if r["source"] == "inaturalist"]
    w("## 2. Test set\n")
    w("| Source | Images | Species | Where | Notes |\n|---|---|---|---|---|")
    w(f"| iNaturalist (testset_inat.csv) | {n_src['inaturalist']} | {sp_src['inaturalist']} | Tamil Nadu and Kerala wetlands and coasts (waders, terns, herons, egrets, ibises, kingfishers, storks, plovers); Tamil Nadu, Kerala and Karnataka (mammals, junglefowl, peafowl, spurfowl) | Research grade, observed and uploaded on or after 2025-06-01, one photo per observation, at most 3 per species (up to 6 fetched, 3 kept at random). Licences: " + ", ".join(f"{l} {sum(1 for r in inat if r['licence'] == l)}" for l in sorted({r['licence'] for r in inat})) + " |")
    w(f"| LILA BC (testset_lila.csv) | {n_src['lila']} | {sp_src['lila']} | ENA24 (eastern USA), Orinoquia (Colombia), Desert Lion (Namibia) | From BioCLIP 2's held-out benchmark IDLE-OO Camera Traps; one image per sequence where known. Site coordinates are approximate (the label files have none). SpeciesNet was trained on LILA sets, so it may have seen these. |")
    w(f"| Our footage (testset_ours.csv) | {n_src['ours']} | {sp_src['ours']} | a privately shared wetland video, Tamil Nadu coast (14 frames, one every 4 s, all the same common redshank); Camera survey demo, northern Italy (13 images) | Hand labels. The redshank's genus is certain and species likely; the 14 frames are one bird, so they count as one sighting in spirit. The demo images' source is not recorded and they date from 2016, so BioCLIP 2 may have seen them. |")
    w("")
    w("Every image keeps its source, licence, URL, place, date and true species in its CSV. Images stay in data/ (not committed).\n")

    md_found = sum(1 for r in rows if sres.get(r["file"], {}).get("animals"))
    covered = sum(1 for r in rows if truth(r)["key"] in set(C["cells"][f"{round(float(r['lat']) * 2) / 2:.1f},{round(float(r['lon']) * 2) / 2:.1f}"]))
    w("## 3. How\n")
    w(f"- **MegaDetector and SpeciesNet** as Studio 0.1.25 runs them: the `wlnw` command line built from this repository scanned the images, each folder got its place and country (`wlnw country`, ENA24 also state VA), and `wlnw run` ran MDV6 + SpeciesNet 4.0.3a with its geofence. The best animal box per image is used. MegaDetector found an animal in {fmt(md_found, len(rows))} images.")
    w(f"- **Candidate species** per place: every mammal and up to 800 birds GBIF has records of within 1 degree (places rounded to 0.5 degree), plus 12 common domestic animals and people. {len(C['order'])} species in all. The true species was in its place's list for {fmt(covered, len(rows))} images.")
    w("- **(b) BioCLIP 2** with open_clip and PyTorch (imageomics/bioclip-2), text prompt `a photo of <kingdom ... genus species>.`, softmax over the place's candidates with BioCLIP's logit scale, on the MegaDetector crop (box plus 10% margin) and on the whole frame.")
    w("- **(c) Zoo ONNX encoder + embedding table**: the SPARROW zoo's `bioclip-2` (FP32) and `bioclip-2-fp16` image encoders run once per image with ONNX Runtime on the CPU (as Studio runs its models), preprocessing as the zoo manifest says, then a dot product with the precomputed text embeddings (FP32 table, or FP16 table for the FP16 run) and the same softmax.")
    pk = lambda r: (bio_pred(r, "crop") or bio_pred(r, "whole") or [[None]])[0][0]
    n_ren = sum(1 for r in rows if pk(r) and pk(r) != truth(r)["key"] and renamed(names[pk(r)]["name"], names[pk(r)]["family"], r))
    RENAMED = f"This changed {n_ren} of BioCLIP's top 1 answers from wrong to right."
    w(f"- Intervals are 95% Wilson intervals. Genus level counts a prediction right when its genus matches the true genus (GBIF backbone). iNaturalist uses newer bird names than the GBIF backbone BioCLIP's candidates come from (Botaurus sinensis is Ixobrychus sinensis there, Thinornis dubius is Charadrius dubius, Ardea coromanda is Bubulcus coromandus), so a prediction with the same family and species epithet as the truth counts as the same species, for SpeciesNet and BioCLIP alike. {RENAMED}\n")

    w("## 4. Results\n")
    w("### 4a. Accuracy\n")
    w("BioCLIP (b) on the crop, for images where MegaDetector found an animal; whole frame for all images. SpeciesNet is Studio's answer (top 1 only: Studio stores its other suggestions by common name).\n")
    w("| Set | n (crop) | BioCLIP crop top 1 species | top 5 species | top 1 genus | top 5 genus | BioCLIP whole frame top 1 species | SpeciesNet species | SpeciesNet genus |")
    w("|---|---|---|---|---|---|---|---|---|")
    for gname, f in groups:
        g = [r for r in rows if f(r)]
        gc = [r for r in g if bio_pred(r, "crop")]
        t1 = sum(1 for r in gc if right(bio_pred(r)[0][0], r, "species"))
        t5 = sum(1 for r in gc if any(right(k, r, "species") for k, _ in bio_pred(r)))
        g1 = sum(1 for r in gc if right(bio_pred(r)[0][0], r, "genus"))
        g5 = sum(1 for r in gc if any(right(k, r, "genus") for k, _ in bio_pred(r)))
        wh = sum(1 for r in g if bio_pred(r, "whole") and right(bio_pred(r, "whole")[0][0], r, "species"))
        sn = sum(1 for r in g if sn_right(r, "species"))
        sg = sum(1 for r in g if sn_right(r, "genus"))
        w(f"| {gname} | {len(gc)} | {fmt(t1, len(gc))} | {fmt(t5, len(gc))} | {fmt(g1, len(gc))} | {fmt(g5, len(gc))} | {fmt(wh, len(g))} | {fmt(sn, len(g))} | {fmt(sg, len(g))} |")
    w("")
    w("The same on the crop with route (c), the zoo ONNX encoder and the embedding table:\n")
    w("| Set | n | (c) FP32 top 1 species | (c) FP32 top 5 species | (c) FP16 top 1 species | (c) FP16 top 5 species |\n|---|---|---|---|---|---|")
    for gname, f in groups[:3]:
        gc = [r for r in rows if f(r) and bio_pred(r, "crop")]
        cells = []
        for v in ("fp32", "fp16"):
            res = O[v]["results"]
            t1 = sum(1 for r in gc if right(res[r["file"]]["crop"][0][0], r, "species"))
            t5 = sum(1 for r in gc if any(right(k, r, "species") for k, _ in res[r["file"]]["crop"]))
            cells += [fmt(t1, len(gc)), fmt(t5, len(gc))]
        w(f"| {gname} | {len(gc)} | " + " | ".join(cells) + " |")
    w("")

    # Second opinion
    w("### 4b. As a second opinion\n")
    w("BioCLIP's answer (crop when there is a box, else whole frame) in the cases where SpeciesNet does not give the right species.\n")
    pick = lambda r: bio_pred(r, "crop") or bio_pred(r, "whole")
    w("| SpeciesNet said | Images | BioCLIP top 1 right (species) | BioCLIP top 1 right (genus) |\n|---|---|---|---|")
    cases = {}
    for r in rows: cases.setdefault(sn_case(r), []).append(r)
    for c in ("stopped above species", "no animal box (blank)", "wrong species", "right"):
        g = cases.get(c, [])
        w(f"| {c} | {len(g)} | {fmt(sum(1 for r in g if right(pick(r)[0][0], r, 'species')), len(g))} | {fmt(sum(1 for r in g if right(pick(r)[0][0], r, 'genus')), len(g))} |")
    rescue = [r for c in ("stopped above species", "no animal box (blank)", "wrong species") for r in cases.get(c, [])]
    k = sum(1 for r in rescue if right(pick(r)[0][0], r, "species"))
    w(f"\nIn all, where SpeciesNet did not name the right species, BioCLIP's top 1 was right for {fmt(k, len(rescue))}.\n")

    w("Confidently wrong: BioCLIP's top 1 is wrong although its score is at or above the threshold (all images, crop or whole frame as above).\n")
    w("| Threshold | Shown (coverage) | Right when shown (precision) | Confidently wrong (share of all images) | Of those, SpeciesNet also wrong or above species | Second-opinion cases only: shown, right when shown |\n|---|---|---|---|---|---|")
    scored = [(pick(r)[0][1], right(pick(r)[0][0], r, "species"), sn_case(r) != "right") for r in rows if pick(r)]
    best_t = None
    for t in (0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9):
        shown = [s for s in scored if s[0] >= t]
        ok = sum(1 for s in shown if s[1])
        bad = [s for s in shown if not s[1]]
        if best_t is None and shown and ok / len(shown) >= 0.9: best_t = t
        rs = [s for s in shown if s[2]]
        w(f"| {t:.1f} | {fmt(len(shown), len(scored))} | {fmt(ok, len(shown))} | {fmt(len(bad), len(scored))} | {sum(1 for s in bad if s[2])} | {len(rs)}, {fmt(sum(1 for s in rs if s[1]), len(rs))} |")
    w("")
    w("### 4c. Threshold\n")
    if best_t is not None:
        shown = [s for s in scored if s[0] >= best_t]
        resc = [r for r in rescue if pick(r)[0][1] >= best_t]
        w(f"Lowest threshold whose precision reaches 90% on this set: **{best_t:.1f}**. At {best_t:.1f} BioCLIP's answer would be shown for {fmt(len(shown), len(scored))} of images, and in the second-opinion cases it would be shown for {len(resc)} of {len(rescue)} and right for {sum(1 for r in resc if right(pick(r)[0][0], r, 'species'))} of those. The upper end of the interval matters: check this again on a larger set before fixing it.\n")
        s8 = [x for x in scored if x[0] >= 0.8]
        r8 = [x for x in s8 if x[2]]
        w(f"**Suggested: 0.8.** Mark BioCLIP's answer as likely in review at 0.8 or more: right {fmt(sum(1 for x in s8 if x[1]), len(s8))} of the time overall and {fmt(sum(1 for x in r8 if x[1]), len(r8))} in the second-opinion cases, against 0.9, which reaches 90% but gives about a sixth fewer right second opinions. Below 0.8, show it only as a list of possibilities.\n")
    else:
        w("No threshold on the grid reached 90% precision on this set.\n")

    w("### 4d. Speed and size (this Mac, Apple silicon, CPU only)\n")
    oa = {v: sum(1 for r in rows if (O[v]['results'].get(r['file'], {}).get('crop') or [[None]])[0][0] == (bio_pred(r) or [[None]])[0][0] and bio_pred(r)) for v in ("fp32", "fp16")}
    nb = sum(1 for r in rows if bio_pred(r))
    w("| Route | Seconds per crop (median) | Download | Model file | Peak memory | Same top 1 as (b) |\n|---|---|---|---|---|---|")
    w(f"| (b) open_clip + PyTorch | {B['seconds_per_crop']:.2f} | about 1.6 GB (Hugging Face, image and text towers) | | not measured | (reference) |")
    for v in ("fp32", "fp16"):
        w(f"| (c) zoo ONNX {v.upper()} + table | {O[v]['seconds_per_crop']:.2f} | {X['zip_mb'][v]:.0f} MB zip | {X['onnx_mb'][v]:.0f} MB | {O[v]['peak_rss_mb']:.0f} MB | {fmt(oa[v], nb)} |")
    w(f"\nSpeciesNet in Studio for comparison: {studio['seconds'] / max(1, studio['images']):.2f} s per image including MegaDetector and image loading.")
    w(f"Embedding cosine between (b) and (c) on the same crop: median {X['cos']['fp32']:.4f} (FP32), {X['cos']['fp16']:.4f} (FP16).")
    ind = X["india"]
    n_ind = sum(ind.values())
    w(f"\nSpecies embedding table for India: GBIF lists {ind['mammals']:,} mammals, {ind['birds']:,} birds, {ind['reptiles']:,} reptiles and {ind['amphibians']:,} amphibians for India, {n_ind:,} species. At 768 numbers each that is {n_ind * 768 * 2 / 1e6:.1f} MB in FP16 ({n_ind * 768 * 4 / 1e6:.1f} MB in FP32), plus names. The whole table for this test ({len(C['order'])} species, 4 continents) is {len(C['order']) * 768 * 2 / 1e6:.1f} MB in FP16.\n")
    L.append(open(HERE / "recommendation.md").read().rstrip() + "\n")
    text = "\n".join(L) + "\n"
    res = open(HERE / "RESULTS.md").read()
    head = res[:res.index("## 2. Test set")] if "## 2. Test set" in res else res[:res.index("# Earlier feasibility check")]
    tail = res[res.index("# Earlier feasibility check"):]
    open(HERE / "RESULTS.md", "w").write(head + text + "\n" + tail)
    print(text)


if __name__ == "__main__":
    main()
