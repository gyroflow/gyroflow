#!/usr/bin/env python3

import argparse
import datetime as dt
import json
import pathlib
import re
import sys
import urllib.request
import xml.etree.ElementTree as ET


LENSFUN_API = "https://api.github.com/repos/lensfun/lensfun/contents/data/db?ref=master"
LENSFUN_RAW = "https://raw.githubusercontent.com/lensfun/lensfun/master/data/db/{name}"


def clean(value):
    return re.sub(r"\s+", " ", value or "").strip()


def key(value):
    return clean(value).casefold()


def brand_name(value):
    normalized = key(value).replace(".", "")
    names = {
        "activeon": "ACTIVEON",
        "apple": "Apple",
        "akaso": "AKASO",
        "arri": "ARRI",
        "asus": "Asus",
        "asahi optical co,ltd": "Pentax",
        "betafpv": "BetaFPV",
        "blackmagic": "Blackmagic",
        "canon": "Canon",
        "casio computer co,ltd": "Casio",
        "casio computer co,ltd.": "Casio",
        "cooau": "COOAU",
        "dji": "DJI",
        "eastman kodak company": "Kodak",
        "eken": "EKEN",
        "feiyu-tech": "Feiyu Tech",
        "fimi": "FIMI",
        "fufifilm": "Fujifilm",
        "fujifilm": "Fujifilm",
        "gitup": "GitUp",
        "gopro": "GoPro",
        "google": "Google",
        "huawei": "Huawei",
        "iqoo": "IQOO",
        "iqoo 9": "IQOO",
        "insta360": "Insta360",
        "lamax": "LAMAX",
        "leica": "Leica",
        "leica camera ag": "Leica",
        "lg mobile": "LG",
        "lge": "LG",
        "mi": "Xiaomi",
        "nikon": "Nikon",
        "nikon corporation": "Nikon",
        "olympus": "Olympus",
        "olympus corporation": "Olympus",
        "olympus imaging corp": "Olympus",
        "olympus optical co,ltd": "Olympus",
        "oneplus": "OnePlus",
        "oppo": "OPPO",
        "panasonic": "Panasonic",
        "pentax": "Pentax",
        "pentax corporation": "Pentax",
        "red": "RED",
        "ricoh": "Ricoh",
        "runcam": "RunCam",
        "samsung": "Samsung",
        "samsung techwin": "Samsung",
        "samsung techwin co": "Samsung",
        "sjcam": "SJCam",
        "sony": "Sony",
        "volla": "Volla",
        "vivo": "Vivo",
        "wolfang": "Wolfang",
        "wolfgang": "Wolfang",
        "xiaomi": "Xiaomi",
    }
    return names.get(normalized, clean(value))


def camera_brand_name(brand, model):
    normalized = key(brand).replace(".", "")
    if normalized == "ricoh imaging company, ltd":
        return "Pentax" if re.match(r"^(K-|K[A-Z]|KP$|KF$)", clean(model)) else "Ricoh"
    return brand_name(brand)


def model_name(brand, model):
    model = clean(model).replace("CInema", "Cinema")
    if key(brand) == "blackmagic":
        model = re.sub(r"\b([46])k\b", lambda match: f"{match.group(1)}K", model, flags=re.IGNORECASE)
    return model


def first_float(value):
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def sensor_size(crop_factor):
    if crop_factor is None:
        return None
    if crop_factor <= 1.1:
        return "Full frame"
    if crop_factor <= 1.35:
        return "APS-H"
    if crop_factor <= 1.7:
        return "APS-C"
    if crop_factor <= 2.2:
        return "Micro Four Thirds"
    if crop_factor <= 3.0:
        return "1-inch"
    if crop_factor <= 4.8:
        return "1/1.7-inch"
    if crop_factor <= 6.2:
        return "1/2.3-inch"
    return "Small sensor"


def child_texts(node, tag):
    return [clean(x.text) for x in node.findall(tag) if clean(x.text)]


def preferred_model(node):
    models = [(clean(x.text), x.attrib.get("lang", "")) for x in node.findall("model") if clean(x.text)]
    if not models:
        return "", []

    preferred = next((text for text, lang in models if lang == "en"), models[0][0])
    aliases = []
    seen = {key(preferred)}
    for text, _lang in models:
        if key(text) not in seen:
            aliases.append(text)
            seen.add(key(text))
    return preferred, aliases


def extra_camera_aliases(brand, model):
    aliases = []
    if key(brand) == "sony":
        match = re.fullmatch(r"Alpha\s+(\d+)([A-Z]*)?(?:\s+([0-9IVX]+[A-Z]?))?", clean(model))
        if match:
            number, series, generation = match.groups()
            aliases.append(f"a{number}{series or ''}{generation or ''}")
    return aliases


def legacy_brand_aliases(brand, model):
    aliases = {
        ("Feiyu Tech", "Pocket3"): ["Feiyu-Tech"],
        ("Fujifilm", "XT200"): ["Fufifilm"],
        ("IQOO", "26MM"): ["IQOO 9"],
        ("LG", "V40"): ["LGE"],
        ("Wolfang", "GA420"): ["WOLFGANG"],
        ("Xiaomi", "12SU"): ["Mi"],
        ("Xiaomi", "K30PRO1X"): ["MI"],
    }
    return aliases.get((brand, model), [])


def merge_values(target, values):
    existing = {key(x) for x in target}
    for value in values:
        if value and key(value) not in existing:
            target.append(value)
            existing.add(key(value))


def merge_camera(cameras, camera):
    original_brand = clean(camera["brand"])
    camera["brand"] = camera_brand_name(camera["brand"], camera["model"])
    camera["model"] = model_name(camera["brand"], camera["model"])
    if not camera["brand"] or not camera["model"]:
        return
    if original_brand and key(original_brand) != key(camera["brand"]):
        camera.setdefault("brand_aliases", []).append(original_brand)

    camera_key = (key(camera["brand"]), key(camera["model"]))
    if camera_key not in cameras:
        camera_key = next((
            existing_key
            for existing_key, existing in cameras.items()
            if existing_key[0] == key(camera["brand"]) and
            any(key(alias) == key(camera["model"]) for alias in existing["aliases"])
        ), camera_key)
    existing = cameras.setdefault(camera_key, {
        "brand": camera["brand"],
        "model": camera["model"],
        "brand_aliases": [],
        "aliases": [],
        "mounts": [],
        "compatible_mounts": [],
        "sensor_size": None,
        "crop_factor": None,
        "source": [],
    })
    if key(existing["model"]) != key(camera["model"]):
        merge_values(existing["aliases"], [camera["model"]])
    merge_values(existing["brand_aliases"], legacy_brand_aliases(existing["brand"], existing["model"]))
    merge_values(existing["brand_aliases"], camera.get("brand_aliases", []))
    merge_values(existing["aliases"], camera.get("aliases", []))
    merge_values(existing["mounts"], camera.get("mounts", []))
    merge_values(existing["compatible_mounts"], camera.get("compatible_mounts", []))
    merge_values(existing["source"], camera.get("source", []))
    if existing["crop_factor"] is None:
        existing["crop_factor"] = camera.get("crop_factor")
    if existing["sensor_size"] is None:
        existing["sensor_size"] = camera.get("sensor_size") or sensor_size(existing["crop_factor"])


def merge_lens(lenses, lens):
    lens["brand"] = brand_name(lens["brand"])
    if not lens["model"]:
        return

    lens_key = (key(lens["brand"]), key(lens["model"]), tuple(sorted(key(x) for x in lens.get("mounts", []))))
    existing = lenses.setdefault(lens_key, {
        "brand": lens["brand"],
        "model": lens["model"],
        "mounts": [],
        "crop_factor": None,
        "source": [],
    })
    merge_values(existing["mounts"], lens.get("mounts", []))
    merge_values(existing["source"], lens.get("source", []))
    if existing["crop_factor"] is None:
        existing["crop_factor"] = lens.get("crop_factor")


def github_json(url):
    request = urllib.request.Request(url, headers={"User-Agent": "gyroflow-camera-database"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.loads(response.read().decode("utf-8"))


def github_text(url):
    request = urllib.request.Request(url, headers={"User-Agent": "gyroflow-camera-database"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read().decode("utf-8")


def lensfun_xml_files(lensfun_db):
    if lensfun_db:
        for path in sorted(pathlib.Path(lensfun_db).glob("*.xml")):
            yield path
        return

    files = github_json(LENSFUN_API)
    for item in files:
        name = item.get("name", "")
        if name.endswith(".xml") and not name.endswith((".dtd", ".xsd")):
            yield name


def parse_lensfun_file(item, cameras, lenses):
    if isinstance(item, pathlib.Path):
        text = item.read_text(encoding="utf-8")
    else:
        text = github_text(LENSFUN_RAW.format(name=item))

    root = ET.fromstring(text)
    mount_compat = {}

    for mount in root.findall("mount"):
        names = child_texts(mount, "name")
        if not names:
            continue
        mount_compat[key(names[0])] = child_texts(mount, "compat")

    for node in root.findall("camera"):
        brand = clean((node.findtext("maker") or ""))
        model, aliases = preferred_model(node)
        merge_values(aliases, extra_camera_aliases(brand, model))
        mounts = child_texts(node, "mount")
        compatible_mounts = []
        for mount in mounts:
            merge_values(compatible_mounts, mount_compat.get(key(mount), []))

        merge_camera(cameras, {
            "brand": brand,
            "model": model,
            "aliases": aliases,
            "mounts": mounts,
            "compatible_mounts": compatible_mounts,
            "sensor_size": sensor_size(first_float(node.findtext("cropfactor"))),
            "crop_factor": first_float(node.findtext("cropfactor")),
            "source": ["lensfun"],
        })

    for node in root.findall("lens"):
        brand = clean((node.findtext("maker") or ""))
        model, _aliases = preferred_model(node)
        merge_lens(lenses, {
            "brand": brand,
            "model": model,
            "mounts": child_texts(node, "mount"),
            "crop_factor": first_float(node.findtext("cropfactor")),
            "source": ["lensfun"],
        })


def parse_lensfun(lensfun_db, cameras, lenses):
    for item in lensfun_xml_files(lensfun_db):
        parse_lensfun_file(item, cameras, lenses)


def parse_profiles(path, cameras, lenses):
    if not path or not path.exists():
        return

    for file in sorted(path.rglob("*.json")):
        try:
            profile = json.loads(file.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue

        brand = clean(profile.get("camera_brand", ""))
        model = clean(profile.get("camera_model", ""))
        lens_model = clean(profile.get("lens_model", ""))
        if not brand or not model:
            continue

        merge_camera(cameras, {
            "brand": brand,
            "model": model,
            "aliases": [],
            "mounts": [],
            "compatible_mounts": [],
            "sensor_size": sensor_size(first_float(profile.get("crop_factor"))),
            "crop_factor": first_float(profile.get("crop_factor")),
            "source": ["gyroflow"],
        })
        if lens_model:
            merge_lens(lenses, {
                "brand": "",
                "model": lens_model,
                "mounts": [],
                "crop_factor": first_float(profile.get("crop_factor")),
                "source": ["gyroflow"],
            })


def finalize(cameras, lenses):
    camera_list = []
    for camera in cameras.values():
        camera_list.append(camera)

    camera_list.sort(key=lambda x: (key(x["brand"]), key(x["model"])))
    lens_list = sorted(lenses.values(), key=lambda x: (key(x["brand"]), key(x["model"])))
    return camera_list, lens_list


def main():
    repo_root = pathlib.Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument("--lens-profiles", type=pathlib.Path, default=repo_root.parent / "lens_profiles")
    parser.add_argument("--lensfun-db", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, default=pathlib.Path(__file__).with_name("camera_database.json"))
    args = parser.parse_args()

    cameras = {}
    lenses = {}

    parse_lensfun(args.lensfun_db, cameras, lenses)
    parse_profiles(args.lens_profiles, cameras, lenses)
    camera_list, lens_list = finalize(cameras, lenses)

    payload = {
        "version": 1,
        "updated_at": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d"),
        "cameras": camera_list,
        "lenses": lens_list,
    }
    args.output.write_text(json.dumps(payload, indent=2, ensure_ascii=True, sort_keys=True) + "\n", encoding="utf-8")
    print(f"Wrote {args.output} ({len(camera_list)} cameras, {len(lens_list)} lenses)")


if __name__ == "__main__":
    sys.exit(main())
