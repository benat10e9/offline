"""Assemble a map package source folder for `kb build`.

    python3 factory/build_maps.py --region texas --title "Texas" \
        --bbox -106.65,25.84,-93.51,36.50 --pmtiles texas.pmtiles \
        --places places.geojsonseq --assets basemaps-assets --out src/maps-texas

Inputs are produced in the `maps` workflow: tiles cut from a Protomaps OpenStreetMap build
with go-pmtiles, places extracted from a Geofabrik OSM extract with osmium.
Output layout (inside the package): maps/<region>.pmtiles, maps/map.json, maps/gazetteer.jsonl,
maps/assets/fonts/<stack>/<range>.pbf, maps/assets/sprites/v4/{light,dark}[@2x].{json,png}.
"""
import argparse
import json
import os
import shutil

FONTSTACKS = ["Noto Sans Regular", "Noto Sans Medium", "Noto Sans Italic"]
SPRITES = ["light", "light@2x", "dark", "dark@2x"]

# What people search for in an emergency, and how much a match counts when ranking.
KINDS = {
    ("place", "city"): ("city", 10), ("place", "town"): ("town", 8), ("place", "village"): ("village", 6),
    ("place", "hamlet"): ("hamlet", 3), ("amenity", "hospital"): ("hospital", 9), ("amenity", "clinic"): ("clinic", 6),
    ("amenity", "pharmacy"): ("pharmacy", 6), ("amenity", "fire_station"): ("fire station", 7),
    ("amenity", "police"): ("police", 7), ("amenity", "shelter"): ("shelter", 5),
    ("social_facility", "shelter"): ("emergency shelter", 8), ("amenity", "drinking_water"): ("drinking water", 4),
    ("amenity", "fuel"): ("fuel", 4), ("shop", "supermarket"): ("grocery", 4),
}


def centroid(geom):
    pts = []

    def walk(c):
        if isinstance(c[0], (int, float)):
            pts.append(c)
        else:
            for x in c:
                walk(x)
    walk(geom["coordinates"])
    if not pts:
        return None
    return sum(p[1] for p in pts) / len(pts), sum(p[0] for p in pts) / len(pts)


def kind_of(tags):
    for (k, v), out in KINDS.items():
        if tags.get(k) == v:
            return out
    return None


def gazetteer(src, dest, bbox):
    w, s, e, n = bbox
    count, seen = 0, set()
    with open(src) as f, open(dest, "w") as out:
        for line in f:
            line = line.strip().lstrip("\x1e")
            if not line:
                continue
            feat = json.loads(line)
            tags = feat.get("properties", {})
            name = tags.get("name")
            kind = kind_of(tags)
            if not name or not kind or not feat.get("geometry"):
                continue
            c = centroid(feat["geometry"])
            if not c:
                continue
            lat, lon = c
            if not (s <= lat <= n and w <= lon <= e):
                continue
            key = (name.lower(), kind[0], round(lat, 3), round(lon, 3))
            if key in seen:
                continue
            seen.add(key)
            context = ", ".join(x for x in (tags.get("addr:city"), tags.get("addr:county")) if x)
            out.write(json.dumps({"name": name, "kind": kind[0], "rank": kind[1], "lat": round(lat, 6),
                                  "lon": round(lon, 6), "context": context}, ensure_ascii=False) + "\n")
            count += 1
    return count


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--region", required=True)
    ap.add_argument("--title", required=True)
    ap.add_argument("--bbox", required=True, help="west,south,east,north")
    ap.add_argument("--pmtiles", required=True)
    ap.add_argument("--places", required=True)
    ap.add_argument("--assets", required=True, help="checkout of protomaps/basemaps-assets")
    ap.add_argument("--out", required=True)
    ap.add_argument("--maxzoom", type=int, default=14)
    a = ap.parse_args()
    bbox = [float(x) for x in a.bbox.split(",")]
    maps = os.path.join(a.out, "maps")
    os.makedirs(os.path.join(maps, "assets", "sprites", "v4"), exist_ok=True)

    tiles_name = f"{a.region}.pmtiles"
    shutil.move(a.pmtiles, os.path.join(maps, tiles_name))
    for stack in FONTSTACKS:
        shutil.copytree(os.path.join(a.assets, "fonts", stack), os.path.join(maps, "assets", "fonts", stack), dirs_exist_ok=True)
    shutil.copy(os.path.join(a.assets, "fonts", "OFL.txt"), os.path.join(maps, "assets", "fonts", "OFL.txt"))
    for sp in SPRITES:
        for ext in ("json", "png"):
            shutil.copy(os.path.join(a.assets, "sprites", "v4", f"{sp}.{ext}"), os.path.join(maps, "assets", "sprites", "v4", f"{sp}.{ext}"))
    places = gazetteer(a.places, os.path.join(maps, "gazetteer.jsonl"), bbox)

    w, s, e, n = bbox
    json.dump({
        "region": a.region, "title": a.title, "tiles": tiles_name, "flavor": "protomaps-v4",
        "bounds": bbox, "center": [(w + e) / 2, (s + n) / 2], "minzoom": 0, "maxzoom": a.maxzoom,
        "attribution": "© OpenStreetMap contributors", "places": places,
    }, open(os.path.join(maps, "map.json"), "w"), indent=1)

    json.dump({"id": f"maps-{a.region}", "version": "0.1.0-dev", "title": f"Offline map: {a.title}",
               "editions": ["consumer", "police", "military", "ems"], "depends_on": []},
              open(os.path.join(a.out, "package.json"), "w"), indent=1)
    json.dump([
        {"id": "osm", "title": "OpenStreetMap data", "publisher": "OpenStreetMap contributors", "license": "ODbL-1.0",
         "authority": "community", "applies_to": [], "jurisdictions": [], "url": "https://www.openstreetmap.org/copyright"},
        {"id": "protomaps-fonts", "title": "Noto Sans glyphs (basemaps-assets)", "publisher": "Google / Protomaps",
         "license": "OFL-1.1", "authority": "reference", "applies_to": [], "jurisdictions": [],
         "url": "https://github.com/protomaps/basemaps-assets"},
        {"id": "protomaps-sprites", "title": "Map icons (basemaps-assets, derived from tangrams/icons)", "publisher": "Protomaps",
         "license": "MIT", "authority": "reference", "applies_to": [], "jurisdictions": [],
         "url": "https://github.com/protomaps/basemaps-assets"},
    ], open(os.path.join(a.out, "sources.json"), "w"), indent=1)
    print(json.dumps({"places": places, "tiles_bytes": os.path.getsize(os.path.join(maps, tiles_name))}))


if __name__ == "__main__":
    main()
