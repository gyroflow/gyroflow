#!/usr/bin/env python3
"""
Build resources/camera_database/camera_database.json from two upstream sources:

  1. gyroflow's lens_profiles repo (https://github.com/gyroflow/lens_profiles)
     - we read the per-camera JSON files in the repo and pull out
       camera_brand / camera_model / crop_factor

  2. LensFun's camera database (https://github.com/lensfun/lensfun)
     - we read the per-mount XML files under data/db/ and pull out
       maker / model / mount / cropfactor

Both inputs are cloned into a sibling _build/ directory the first time
the script runs. Pass --refresh to force a fresh clone.

Output schema (one entry per unique brand|model, case-insensitive):
{
  "version": 1,
  "sources": ["gyroflow_lens_profiles", "lensfun"],
  "cameras": [
    {
      "brand": "Sony",
      "model": "ILCE-7M4",
      "mount": "Sony E",                # empty if unknown
      "crop_factor": 1.0,               # null if unknown
      "sensor_size_mm": [35.6, 23.8],   # null if unknown; from crop factor (3:2)
      "sources": ["gyroflow", "lensfun"]
    },
    ...
  ]
}

Usage:
    python3 resources/camera_database/build.py [--refresh]
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

DATA_DIR = Path(__file__).parent
OUT_FILE = DATA_DIR / "camera_database.json"
BUILD_DIR = DATA_DIR / "_build"
GYROFLOW_REPO = "https://github.com/gyroflow/lens_profiles.git"
LENSFUN_REPO = "https://github.com/lensfun/lensfun.git"

# --- Brand normalization aliases ---
BRAND_ALIASES = {
    "GOPRO": "GoPro",
    "DJI": "DJI",
    "RED": "RED",
    "ARRI": "ARRI",
    "Z CAM": "Z CAM",
    "ZCAM": "Z CAM",
    "BLACKMAGIC": "Blackmagic",
    "BLACKMAGICDESIGN": "Blackmagic",
    "BLACKMAGIC DESIGN": "Blackmagic",
    "SJCAM": "SJCAM",
    "SONY": "Sony",
    "CANON": "Canon",
    "NIKON": "Nikon",
    "NIKON CORPORATION": "Nikon",
    "PANASONIC": "Panasonic",
    "FUJIFILM": "Fujifilm",
    "FUJI": "Fujifilm",
    "FUFIFILM": "Fujifilm",
    "OLYMPUS": "Olympus",
    "OLYMPUS CORPORATION": "Olympus",
    "OLYMPUS IMAGING CORP.": "Olympus",
    "OLYMPUS OPTICAL CO.,LTD": "Olympus",
    "OM SYSTEM": "OM System",
    "OM-SYSTEM": "OM System",
    "OMSYSTEM": "OM System",
    "OM DIGITAL SOLUTIONS": "OM System",
    "SIGMA": "Sigma",
    "TAMRON": "Tamron",
    "TOKINA": "Tokina",
    "SAMYANG": "Samyang",
    "ZEISS": "Zeiss",
    "LEICA": "Leica",
    "LEICA CAMERA AG": "Leica",
    "LEICA CAMERA AG.": "Leica",
    "PENTAX": "Pentax",
    "PENTAX CORPORATION": "Pentax",
    "KODAK": "Kodak",
    "EASTMAN KODAK COMPANY": "Kodak",
    "RICOH": "Ricoh",
    "RICOH IMAGING COMPANY, LTD.": "Ricoh",
    "SAMSUNG": "Samsung",
    "SAMSUNG TECHWIN": "Samsung",
    "SAMSUNG TECHWIN CO.": "Samsung",
    "RUNCAM": "RunCam",
    "INSTA360": "Insta360",
    "KINEFINITY": "Kinefinity",
    "FREEFLY": "Freefly",
    "FOXEER": "Foxeer",
    "CADDX": "Caddx",
    "WALKSNAIL": "Walksnail",
    "HAWKEYE": "Hawkeye",
    "MOBIUS": "Mobius",
    "MORECAM": "Morecam",
    "THIEYE": "ThiEYE",
    "AKASO": "AKASO",
    "EKEN": "Eken",
    "XTU": "XTU",
    "XIAOMI": "Xiaomi",
    "POCO": "Xiaomi",  # POCO is Xiaomi sub-brand, but keep for now
    "REDMI": "Xiaomi",
    "APEMAN": "apeman",
    "KONICA-MINOLTA": "Konica Minolta",
    "KONICA MINOLTA": "Konica Minolta",
    "KONICA MINOLTA CAMERA, INC.": "Konica Minolta",
    "MINOLTA CO., LTD.": "Minolta",
    "HASSELBLAD": "Hasselblad",
    "CASIO": "Casio",
    "CASIO COMPUTER CO.,LTD": "Casio",
    "CASIO COMPUTER CO.,LTD.": "Casio",
    "SCHNEIDER-KREUZNACH": "Schneider Kreuznach",
    "VIVITAR": "Vivitar",
    "SOLIGOR": "Soligor",
    "CONTAX": "Contax",
    "PHASE ONE": "Phase One",
    "MAMIYA": "Mamiya",
    "ASAHI OPTICAL CO.,LTD": "Pentax",
    "ROLLEI": "Rollei",
    "ROLLEIFLEX": "Rollei",
    "APPLE": "Apple",
    "GOOGLE": "Google",
    "HUAWEI": "Huawei",
    "HONOR": "Honor",
    "ONEPLUS": "OnePlus",
    "OPPO": "Oppo",
    "VIVO": "Vivo",
    "REALME": "Realme",
    "MOTOROLA": "Motorola",
    "NOKIA": "Nokia",
    "MICROSOFT": "Microsoft",
    "LG": "LG",
    "LGE": "LG",
    "LG MOBILE": "LG",
    "LG V30": "LG",
    "MEIZU": "Meizu",
    "ZTE": "ZTE",
    "NUBIA": "Nubia",
    "TECNO": "Tecno",
    "INFINIX": "Infinix",
    "UMIDIGI": "Umidigi",
    "ULEFONE": "Ulefone",
    "ASUS": "ASUS",
    "BLACKBERRY": "BlackBerry",
    "FAIRPHONE": "Fairphone",
    "JVC": "JVC",
    "PHILIPS": "Philips",
    "GARMIN": "Garmin",
    "RASPBERRY PI": "Raspberry Pi",
    "SHARP": "Sharp",
    "MI": "Xiaomi",
    "GENERIC": "Generic",
    "ITALIA INDEPENDENT": "Italia Independent",
    "DIGITAL BOLEX": "Digital Bolex",
    "FEIYU TECH": "Feiyu Tech",
    "FEIYU-TECH": "Feiyu Tech",
    "GITUP": "GitUp",
    "AEE DV": "AEE",
    "BETAFPV": "BetaFPV",
    "WALKSNAIL AVATAR V2": "Walksnail",
    "WALKSNAIL AVATAR V2 PRO": "Walksnail",
    "WOLFGANG": "Wolfang",
    "WOLFANG": "Wolfang",
    "HOLYSTONE": "Holy Stone",
    "NICEBOY": "Niceboy",
    "LAMAX": "Lamax",
    "GOXTREME": "Goxtreme",
    "GOPLUS CAMPRO": "GoPlus CamPro",
    "MY GEKO GEAR": "My GEKO Gear",
    "YI TECHNOLOGY": "YI Technology",
    "BLAUPUNKT": "Blaupunkt",
    "COOAU": "Cooau",
    "DDPAI": "DDPAI",
    "EVOLIO": "Evolio",
    "EZVIZ": "Ezviz",
    "GADNIC": "Gadnic",
    "MAGINON": "Maginon",
    "ROLLEI (禄来)": "Rollei",
    "RYZE": "Ryze",
    "SARGO": "Sargo",
    "SENA": "Sena",
    "SENCOR": "Sencor",
    "SIMULUS": "Simulus",
    "SOOYI": "Sooyi",
    "SURFOLA": "Surfola",
    "TRACER": "Tracer",
    "VAQUITA": "Vaquita",
    "VISUO": "Visuo",
    "ACTIVEON": "ActiveON",
    "AIKUCAM": "Aikucam",
    "ANDOER": "Andoer",
    "AOLBEA": "Aolbea",
    "APEXCAM": "Apexcam",
    "ARDUCAM": "Arducam",
    "AUSEK": "Ausek",
    "AXNEN": "Axnen",
    "AKSOGO": "Aksogo",
    "BIWOND": "Biwond",
    "BLACKVUE": "BlackVue",
    "BLACKSHARK": "Black Shark",
    "CAMPARK": "CamPark",
    "CHRONOS": "Chronos",
    "COTUO": "Cotuo",
    "CROSSTOUR": "Crosstour",
    "CYCLIQ": "Cycliq",
    "DECATHLON": "Decathlon",
    "DENVER": "Denver",
    "DIGMA": "Digma",
    "DOGCAM": "DogCam",
    "DRIFT": "Drift",
    "EACHINE": "Eachine",
    "FIMI": "FIMI",
    "FIREFLY": "Firefly",
    "FORCITE": "Forcite",
    "FOREVER": "Forever",
    "GENERAL MOBILE": "General Mobile",
    "GHOSTSTOP": "GhostStop",
    "HAPPYMODEL": "HappyModel",
    "HDZERO": "HDZero",
    "HP": "HP",
    "ICONNTECHS": "IconnTechs",
    "IZI": "IZI",
    "INSTA TITAN": "Insta Titan",
    "KMZ": "KMZ",
    "MATECAM": "Matecam",
    "MOBULA": "Mobula",
    "MOMA": "MOMA",
    "MONSTER": "Monster",
    "NWO JAPAN": "NWO Japan",
    "NECKER": "Necker",
    "NICEBOY": "Niceboy",
    "NOTHING": "Nothing",
    "NOVATEK": "Novatek",
    "ODRVM": "ODRVM",
    "OMNIVISION": "OmniVision",
    "ORION": "Orion",
    "OVERMAX": "Overmax",
    "POLAROID": "Polaroid",
    "RACECAM": "RaceCam",
    "SBOX": "SBOX",
    "SEIKO EPSON CORP.": "Seiko Epson",
    "SIOEYE": "Sioeye",
    "SKYDIO": "Skydio",
    "SOMIKON": "Somikon",
    "SOOCOO": "SOOCOO",
    "TOMTOM": "TomTom",
    "TOMATE": "Tomate",
    "ULEFONE": "Ulefone",
    "VOLLA": "Volla",
    "WOLFANG": "Wolfang",
    "X-TRY": "X-TRY",
    "XDV": "XDV",
    "YICAM": "YiCam",
    "YOLANSIN": "Yolansin",
    "ZOOM": "Zoom",
    "FUJITSU": "Fujitsu",
    "ULEFONE": "Ulefone",
}

# Strings that look like model names rather than brand names,
# rejected to avoid noise. These are typically user-typed mistakes
# in the existing lens profile collection.
BAD_BRANDS = {
    "iqoo z3", "iqoo 9", "iqoo", "honor 80", "nova7", "h7s",
    "jjrc x5", "kf102", "mate40pro", "sargo11", "sj6 legend",
    "techno spark", "cyanchen", "duoke", "cleep",
    "infinix hot 30", "insta titan", "supremo", "visi",
    "china_actioncam", "iflight", "redmi",
}
BAD_BRAND_PATTERNS = re.compile(r"^[a-z]+\d+$", re.IGNORECASE)  # like "sargo11"


def normalize_brand(s: str) -> str:
    s = (s or "").strip()
    if not s:
        return ""
    upper = s.upper()
    if upper in BRAND_ALIASES:
        return BRAND_ALIASES[upper]
    if s.lower() in BAD_BRANDS:
        return ""
    return s


def normalize_model(s: str) -> str:
    s = (s or "").strip()
    s = re.sub(r"\s+", " ", s)
    return s


# Approximate sensor sizes by crop factor — used when only crop is known.
# Diagonal of 35mm full frame = 43.27mm. Standard 3:2 ratio assumed.
def sensor_from_crop(crop_factor: float) -> list | None:
    if not crop_factor or crop_factor <= 0:
        return None
    diag_ff = 43.266615305567875  # sqrt(36^2 + 24^2)
    diag = diag_ff / crop_factor
    # Most stills cameras 3:2; many video cameras 16:9. We can't know the ratio
    # for sure from crop factor alone, so we record diagonal-based 3:2 as a hint.
    h = diag / ((3.0 / 2.0) ** 2 + 1) ** 0.5
    w = h * 3.0 / 2.0
    return [round(w, 2), round(h, 2)]


def parse_gyroflow_db(repo_root: Path) -> tuple[list[dict], int]:
    cameras = {}
    count = 0
    for jf in repo_root.rglob("*.json"):
        if jf.name.startswith("_"):
            continue
        try:
            payload = json.loads(jf.read_text(encoding="utf-8", errors="replace"))
        except json.JSONDecodeError:
            continue
        # Some files contain compatible_settings array of profiles
        candidates = [payload]
        cs = payload.get("compatible_settings")
        if isinstance(cs, list):
            candidates.extend(c for c in cs if isinstance(c, dict))
        for p in candidates:
            brand = normalize_brand(p.get("camera_brand", ""))
            model = normalize_model(p.get("camera_model", ""))
            if not brand or not model:
                continue
            count += 1
            crop = p.get("crop_factor")
            try:
                crop = float(crop) if crop is not None else None
                if crop and crop <= 0:
                    crop = None
            except (TypeError, ValueError):
                crop = None
            key = (brand.lower(), model.lower())
            cur = cameras.setdefault(
                key,
                {
                    "brand": brand,
                    "model": model,
                    "mount": "",
                    "crop_factor": None,
                    "sensor_size_mm": None,
                    "sources": ["gyroflow"],
                },
            )
            if crop and not cur["crop_factor"]:
                cur["crop_factor"] = crop
    print(f"  scanned {count} profiles in gyroflow", file=sys.stderr)
    return list(cameras.values()), count


def parse_lensfun(data_dir: Path) -> list[dict]:
    cameras = {}
    for xml_file in sorted(data_dir.glob("*.xml")):
        if xml_file.stat().st_size < 50:
            continue
        try:
            tree = ET.parse(xml_file)
        except ET.ParseError as e:
            print(f"warn: parse error {xml_file.name}: {e}", file=sys.stderr)
            continue
        root = tree.getroot()
        for cam in root.findall("camera"):
            maker_el = cam.find("maker")
            model_el = cam.find("model")
            mount_el = cam.find("mount")
            crop_el = cam.find("cropfactor")
            if maker_el is None or model_el is None:
                continue
            brand = normalize_brand(maker_el.text or "")
            model = normalize_model(model_el.text or "")
            if not brand or not model:
                continue
            mount = (mount_el.text or "").strip() if mount_el is not None else ""
            try:
                crop = float(crop_el.text) if (crop_el is not None and crop_el.text) else None
            except ValueError:
                crop = None
            key = (brand.lower(), model.lower())
            cur = cameras.setdefault(
                key,
                {
                    "brand": brand,
                    "model": model,
                    "mount": mount,
                    "crop_factor": crop,
                    "sensor_size_mm": None,
                    "sources": ["lensfun"],
                },
            )
            if mount and not cur["mount"]:
                cur["mount"] = mount
            if crop and not cur["crop_factor"]:
                cur["crop_factor"] = crop
    return list(cameras.values())


def merge(gyro: list[dict], lensfun: list[dict]) -> list[dict]:
    merged = {}
    for c in gyro:
        key = (c["brand"].lower(), c["model"].lower())
        merged[key] = dict(c)
    for c in lensfun:
        key = (c["brand"].lower(), c["model"].lower())
        if key in merged:
            cur = merged[key]
            if not cur["mount"] and c["mount"]:
                cur["mount"] = c["mount"]
            if not cur["crop_factor"] and c["crop_factor"]:
                cur["crop_factor"] = c["crop_factor"]
            srcs = set(cur.get("sources", [])) | set(c.get("sources", []))
            cur["sources"] = sorted(srcs)
        else:
            merged[key] = dict(c)
    out = list(merged.values())
    for c in out:
        if c["sensor_size_mm"] is None and c["crop_factor"]:
            c["sensor_size_mm"] = sensor_from_crop(c["crop_factor"])
    out.sort(key=lambda x: (x["brand"].lower(), x["model"].lower()))
    return out


def shallow_clone(url: str, dest: Path, refresh: bool) -> None:
    if dest.exists() and refresh:
        shutil.rmtree(dest)
    if dest.exists():
        return
    subprocess.run(
        ["git", "clone", "--depth", "1", url, str(dest)],
        check=True,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--refresh",
        action="store_true",
        help="re-clone the upstream repos before building",
    )
    args = parser.parse_args()

    BUILD_DIR.mkdir(exist_ok=True)
    gyroflow_root = BUILD_DIR / "lens_profiles"
    lensfun_root = BUILD_DIR / "lensfun"

    print(f"cloning {GYROFLOW_REPO}", file=sys.stderr)
    shallow_clone(GYROFLOW_REPO, gyroflow_root, args.refresh)
    print(f"cloning {LENSFUN_REPO}", file=sys.stderr)
    shallow_clone(LENSFUN_REPO, lensfun_root, args.refresh)

    print("scanning gyroflow profiles tree", file=sys.stderr)
    gyro_cams, gyro_count = parse_gyroflow_db(gyroflow_root)
    print(f"  gyroflow: {len(gyro_cams)} unique cameras from {gyro_count} profiles", file=sys.stderr)

    print("scanning lensfun xml", file=sys.stderr)
    lensfun_cams = parse_lensfun(lensfun_root / "data" / "db")
    print(f"  lensfun: {len(lensfun_cams)} unique cameras", file=sys.stderr)

    merged = merge(gyro_cams, lensfun_cams)
    print(f"  merged total: {len(merged)} unique cameras", file=sys.stderr)

    out = {
        "version": 1,
        "sources": [
            "gyroflow_lens_profiles",
            "lensfun",
        ],
        "cameras": merged,
    }
    OUT_FILE.write_text(json.dumps(out, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OUT_FILE} ({OUT_FILE.stat().st_size} bytes)", file=sys.stderr)

    # Stats
    from_gyro = sum(1 for c in merged if "gyroflow" in c["sources"])
    from_lf = sum(1 for c in merged if "lensfun" in c["sources"])
    both = sum(1 for c in merged if len(c["sources"]) == 2)
    with_mount = sum(1 for c in merged if c["mount"])
    with_crop = sum(1 for c in merged if c["crop_factor"])
    brands = sorted({c["brand"] for c in merged})
    print(f"stats: gyro={from_gyro} lensfun={from_lf} both={both} mount={with_mount} crop={with_crop}", file=sys.stderr)
    print(f"brands ({len(brands)}): {', '.join(brands)}", file=sys.stderr)


if __name__ == "__main__":
    main()
