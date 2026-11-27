"""Test set b: LILA BC camera-trap images from BioCLIP 2's own held-out benchmark (IDLE-OO Camera Traps, 5 LILA sets
checked against TreeOfLife-200M by MD5 and metadata). Takes ENA24 (USA), Orinoquia (Colombia) and Desert Lion (Namibia),
a few images per species (one per sequence where sequences are known), no reptiles. Writes data/eval/lila/<set>/... and
testset_lila.csv. The label CSVs come from huggingface.co/datasets/imageomics/IDLE-OO-Camera-Traps.

Site coordinates are not in the label files: SITES gives an approximate centre per set (an assumption, used only for
the GBIF species list and labelled as such in RESULTS.md).

Usage: data/.venv/bin/python -I fetch_lila.py
"""
import csv, io, pathlib, random, time, urllib.request

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE / "data" / "eval" / "lila"
UA = {"User-Agent": "WildlifeNetworkStudio-spike/0.1"}
SITES = {  # set: (per species, country, approximate lat, lon, place)
    "ENA24": (4, "USA", 38.9, -78.2, "Smithsonian Conservation Biology Institute area, Virginia (approximate)"),
    "orinoquia": (3, "COL", 4.5, -71.5, "Orinoquia, Colombia (approximate)"),
    "desert-lion": (2, "NAM", -19.5, 13.8, "Kunene, Namibia (approximate)"),
}


def main():
    rows = []
    random.seed(7)
    for name, (per, country, lat, lon, place) in SITES.items():
        url = f"https://huggingface.co/datasets/imageomics/IDLE-OO-Camera-Traps/resolve/main/{name}-balanced.csv"
        data = list(csv.DictReader(io.StringIO(urllib.request.urlopen(urllib.request.Request(url, headers=UA)).read().decode())))
        by_sp = {}
        for r in data:
            if r["cls"] not in ("mammalia", "aves"): continue
            by_sp.setdefault(r["scientific_name"], []).append(r)
        for sp, rs in sorted(by_sp.items()):
            random.shuffle(rs)
            seen_seq, picked = set(), []
            for r in rs:
                seq = r["sequence_id"]
                if not seq.endswith("unknown") and seq in seen_seq: continue
                seen_seq.add(seq); picked.append(r)
                if len(picked) == per: break
            for r in picked:
                src = r["url_gcp"]
                ext = pathlib.Path(src).suffix.lower() or ".jpg"
                dest = OUT / name / f"{__import__("hashlib").md5(r["image_id"].encode()).hexdigest()[:12]}{ext}"
                dest.parent.mkdir(parents=True, exist_ok=True)
                if not dest.exists():
                    try:
                        with urllib.request.urlopen(urllib.request.Request(urllib.parse.quote(src, safe=":/"), headers=UA), timeout=60) as f: dest.write_bytes(f.read())
                    except Exception as e:
                        print("skip", src, e); continue
                    time.sleep(0.3)
                rows.append({"file": str(dest.relative_to(HERE / "data")), "source": "lila", "licence": "CDLA-Permissive-1.0", "attribution": f"LILA BC {name}",
                             "url": src, "place": place, "state": name, "lat": lat, "lon": lon, "obscured": "approximate", "date": "", "uploaded": "",
                             "true_species": sp.capitalize(), "group": "bird" if r["cls"] == "aves" else "mammal", "country": country})
        print(name, len(rows))
    with open(HERE / "testset_lila.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
    print(len(rows), "images")


if __name__ == "__main__":
    import urllib.parse
    main()
