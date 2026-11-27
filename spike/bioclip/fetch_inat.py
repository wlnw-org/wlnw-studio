"""Test set a: iNaturalist research-grade photos BioCLIP 2 cannot have seen (observed AND uploaded on or after CUTOFF,
well after the GBIF snapshot of 2024-05-01 that TreeOfLife-200M was built from), licences CC0, CC BY or CC BY-NC only.
Wetland and coastal birds from Tamil Nadu and Kerala, mammals and ground birds from Tamil Nadu, Kerala and Karnataka.
At most PER_SPECIES photos per species, one per observation. The evaluation then kept at most 3 per species
(random, seed 11) to keep run time down; the rest were moved out of the test set. Writes data/eval/inat/<state>/<obs id>.jpg and testset_inat.csv.

Usage: data/.venv/bin/python -I fetch_inat.py
"""
import csv, json, pathlib, time, urllib.parse, urllib.request

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE / "data" / "eval" / "inat"
UA = {"User-Agent": "WildlifeNetworkStudio-spike/0.1"}
CUTOFF = "2025-06-01"
LICENCES = {"cc0": "CC0-1.0", "cc-by": "CC-BY-4.0", "cc-by-nc": "CC-BY-NC-4.0"}
PLACES = {"tamil_nadu": 6842, "kerala": 9627, "karnataka": 7043}
WETLAND = ["Scolopacidae", "Laridae", "Ardeidae", "Threskiornithidae", "Alcedinidae", "Ciconiidae", "Charadriidae"]
FOREST = ["Mammalia", "Phasianidae"]
PER_SPECIES = 6
_last = [0.0]


def get(url):
    wait = 1.1 - (time.time() - _last[0])
    if wait > 0: time.sleep(wait)
    for attempt in range(5):
        try:
            _last[0] = time.time()
            return json.load(urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=60))
        except Exception:
            if attempt == 4: raise
            time.sleep(5 * (attempt + 1))


_TAXA = {}


def taxon_id(name):
    """iNaturalist's id for a family or class name (the observations endpoint has no taxon_name filter)."""
    if name not in _TAXA:
        r = get("https://api.inaturalist.org/v1/taxa?" + urllib.parse.urlencode({"q": name, "is_active": "true", "per_page": 10}))
        _TAXA[name] = next(t["id"] for t in r["results"] if t["name"] == name and t["rank"] in ("family", "class"))
    return _TAXA[name]


def observations(taxon, place, pages=3):
    for page in range(1, pages + 1):
        q = {"taxon_id": taxon_id(taxon), "place_id": place, "quality_grade": "research", "photo_license": ",".join(LICENCES), "photos": "true",
             "d1": CUTOFF, "created_d1": CUTOFF, "per_page": 200, "page": page, "order_by": "votes"}
        r = get("https://api.inaturalist.org/v1/observations?" + urllib.parse.urlencode(q))
        yield from r["results"]
        if page * 200 >= r["total_results"]: return


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    rows, per, done = [], {}, set()
    plan = [(t, s) for t in WETLAND for s in ("tamil_nadu", "kerala")] + [(t, s) for t in FOREST for s in PLACES]
    for taxon, state in plan:
        n0 = len(rows)
        for o in observations(taxon, PLACES[state]):
            tx = o.get("taxon") or {}
            if tx.get("rank") not in ("species", "subspecies") or not tx.get("name"): continue
            species = " ".join(tx["name"].split()[:2])
            if per.get(species, 0) >= PER_SPECIES or o["id"] in done: continue
            if (o.get("observed_on") or "") < CUTOFF or (o.get("created_at") or "")[:10] < CUTOFF: continue
            photo = next((p for p in o.get("photos", []) if p.get("license_code") in LICENCES), None)
            if not photo or not o.get("location"): continue
            lat, lon = (float(v) for v in o["location"].split(","))
            url = photo["url"].replace("/square.", "/large.")
            dest = OUT / state / f"{o['id']}.jpg"
            dest.parent.mkdir(exist_ok=True)
            if not dest.exists():
                try:
                    with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=60) as r: dest.write_bytes(r.read())
                except Exception as e:
                    print("skip", o["id"], e); continue
                time.sleep(0.5)
            per[species] = per.get(species, 0) + 1
            done.add(o["id"])
            iconic = tx.get("iconic_taxon_name", "")
            rows.append({"file": str(dest.relative_to(HERE / "data")), "source": "inaturalist", "licence": LICENCES[photo["license_code"]],
                         "attribution": photo.get("attribution", ""), "url": f"https://www.inaturalist.org/observations/{o['id']}",
                         "place": o.get("place_guess") or state, "state": state, "lat": round(lat, 3), "lon": round(lon, 3),
                         "obscured": o.get("obscured", False), "date": o.get("observed_on"), "uploaded": o["created_at"][:10],
                         "true_species": species, "group": "bird" if iconic == "Aves" else "mammal" if iconic == "Mammalia" else iconic.lower(), "country": "IND"})
        print(f"{taxon} {state}: +{len(rows) - n0} (total {len(rows)}, species {len(per)})")
    with open(HERE / "testset_inat.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
    print(len(rows), "images,", len(per), "species")


if __name__ == "__main__":
    main()
