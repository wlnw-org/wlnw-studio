# Downloads the test videos from Wikimedia Commons into data/videos/<site>/ and converts them to MP4 (Studio reads MP4, MOV, AVI).
import json, pathlib, subprocess, time, urllib.error, urllib.parse, urllib.request
UA = {"User-Agent": "WildlifeNetworkStudio-spike/0.1"}
here = pathlib.Path(__file__).parent
t = json.load(open(here / "testset.json"))
for site, name, _ in t["videos"]:
    d = here / "data" / "videos" / site; d.mkdir(parents=True, exist_ok=True)
    out = d / (pathlib.Path(name).stem.replace(" ", "_") + ".mp4")
    if out.exists(): continue
    q = "https://commons.wikimedia.org/w/api.php?" + urllib.parse.urlencode({"action": "query", "titles": "File:" + name, "prop": "imageinfo", "iiprop": "url", "format": "json"})
    url = list(json.load(urllib.request.urlopen(urllib.request.Request(q, headers=UA)))["query"]["pages"].values())[0]["imageinfo"][0]["url"]
    src = d / "src.webm"
    for attempt in range(6):  # Wikimedia answers 429 to quick downloads: wait and try again
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=UA)) as r, open(src, "wb") as f: f.write(r.read())
            break
        except urllib.error.HTTPError as e:
            if e.code != 429 or attempt == 5: raise
            time.sleep(20 * (attempt + 1))
    time.sleep(5)
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", src, "-c:v", "libx264", "-crf", "20", "-c:a", "aac", out], check=True)
    src.unlink()
    print("ok", out.name)
