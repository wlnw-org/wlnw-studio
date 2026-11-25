# Lists candidate camera-trap videos on Wikimedia Commons with licence, author, length and description (for choosing the test set).
import json, sys, urllib.parse, urllib.request
UA = {"User-Agent": "WildlifeNetworkStudio-spike/0.1"}
titles = sys.argv[1:]
q = "https://commons.wikimedia.org/w/api.php?" + urllib.parse.urlencode({"action": "query", "titles": "|".join(titles), "prop": "imageinfo", "iiprop": "url|size|extmetadata", "format": "json"})
pages = json.load(urllib.request.urlopen(urllib.request.Request(q, headers=UA)))["query"]["pages"].values()
for p in pages:
    i = p["imageinfo"][0]; m = i["extmetadata"]
    strip = lambda s: __import__("re").sub("<[^>]+>", "", s or "")[:160]
    print(json.dumps({"title": p["title"], "url": i["url"], "mb": round(i["size"] / 1e6, 1), "dur": i.get("duration"), "licence": m.get("LicenseShortName", {}).get("value"),
        "artist": strip(m.get("Artist", {}).get("value")), "desc": strip(m.get("ImageDescription", {}).get("value"))}, ensure_ascii=False))
