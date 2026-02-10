#!/usr/bin/env python3
"""
Build a camera database JSON from the gyroflow lens_profiles repository,
enriched with LensFun camera/lens metadata (sensor sizes, crop factors, mounts).

Walks all .json lens profile files, extracts camera/lens metadata, normalizes
brand/model/lens names for consistency, merges LensFun data, and writes a
structured database to:
    resources/camera_database.json

Usage:
    python3 build_camera_database.py [--profiles-dir PATH] [--output PATH] [--lensfun-data PATH]
"""

import argparse
import json
import os
import re
import sys
from collections import defaultdict
from pathlib import Path

# ---------------------------------------------------------------------------
# Brand normalization
# ---------------------------------------------------------------------------

# Canonical brand names keyed by their lowercased form.
BRAND_CANONICAL = {
    # All-caps acronyms / initialisms
    "dji": "DJI",
    "sjcam": "SJCAM",
    "red": "RED",
    "zte": "ZTE",
    "lg": "LG",
    "lge": "LG",
    "lg v30": "LG",
    "lg mobile": "LG",
    "hp": "HP",
    "jvc": "JVC",
    "aee": "AEE",
    "xdv": "XDV",
    "izi": "IZI",
    "nwo japan": "NWO JAPAN",

    # Mixed-case brands
    "gopro": "GoPro",
    "goplus campro": "GoPlus CamPro",
    "betafpv": "BetaFPV",
    "iflight": "iFlight",
    "hdzero": "HDZero",
    "tomtom": "TomTom",
    "fujifilm": "Fujifilm",
    "fufifilm": "Fujifilm",
    "blackmagic": "Blackmagic",
    "insta360": "Insta360",
    "insta titan": "Insta Titan",
    "walksnail": "Walksnail",
    "walksnail avatar v2": "Walksnail",
    "walksnail avatar v2 pro": "Walksnail",
    "freefly": "Freefly",
    "raspberry pi": "Raspberry Pi",
    "digital bolex": "Digital Bolex",
    "holystone": "Holy Stone",
    "holy stone": "Holy Stone",
    "feiyu-tech": "Feiyu-Tech",
    "feiyu tech": "Feiyu-Tech",
    "my geko gear": "My GEKO Gear",
    "z cam": "Z CAM",
    "ghoststop": "GhostStop",
    "iconntech": "IconnTechs",
    "iconntechs": "IconnTechs",
    "racecam": "RaceCam",
    "oneplus": "OnePlus",
    "yicam": "YiCam",
    "omnivision": "OmniVision",
    "x-try": "X-TRY",
    "blackvue": "Blackvue",

    # Title-case brands
    "apple": "Apple",
    "sony": "Sony",
    "canon": "Canon",
    "nikon": "Nikon",
    "panasonic": "Panasonic",
    "samsung": "Samsung",
    "xiaomi": "Xiaomi",
    "sigma": "Sigma",
    "olympus": "Olympus",
    "huawei": "Huawei",
    "honor": "Honor",
    "honor 80": "Honor",
    "google": "Google",
    "motorola": "Motorola",
    "vivo": "Vivo",
    "oppo": "Oppo",
    "poco": "POCO",
    "redmi": "Redmi",
    "realme": "Realme",
    "nokia": "Nokia",
    "leica": "Leica",
    "arri": "ARRI",
    "pentax": "Pentax",
    "hasselblad": "Hasselblad",
    "ricoh": "Ricoh",
    "meizu": "Meizu",
    "lenovo": "Lenovo",
    "nothing": "Nothing",
    "fairphone": "Fairphone",
    "asus": "ASUS",
    "sharp": "Sharp",
    "infinix": "Infinix",
    "tecno": "Tecno",
    "nubia": "Nubia",
    "ulefone": "Ulefone",
    "philips": "Philips",
    "garmin": "Garmin",
    "sena": "Sena",
    "zoom": "Zoom",
    "chronos": "Chronos",
    "skydio": "Skydio",
    "ryze": "Ryze",
    "tamron": "Tamron",
    "tokina": "Tokina",
    "zeiss": "Zeiss",
    "carl zeiss": "Zeiss",
    "samyang": "Samyang",
    "rokinon": "Samyang",
    "schneider": "Schneider",
    "schneider-kreuznach": "Schneider",
    "kodak": "Kodak",
    "casio": "Casio",
    "vivitar": "Vivitar",
    "minolta": "Minolta",
    "konica minolta": "Konica Minolta",
    "konica": "Konica",
    "mamiya": "Mamiya",
    "contax": "Contax",
    "cosina": "Cosina",
    "viltrox": "Viltrox",
    "yongnuo": "Yongnuo",
    "meike": "Meike",

    # All-caps action-cam / niche brands
    "akaso": "AKASO",
    "eken": "EKEN",
    "xtu": "XTU",
    "wolfang": "WOLFANG",
    "wolfgang": "WOLFANG",
    "cotuo": "COTUO",
    "axnen": "AXNEN",
    "ddpai": "DDPAI",
    "activeon": "ACTIVEON",
    "sbox": "SBOX",
    "aikucam": "AIKUCAM",
    "necker": "NECKER",
    "fimi": "FIMI",
    "moma": "MOMA",
    "tomate": "TOMATE",

    # Other brands
    "runcam": "RunCam",
    "caddx": "Caddx",
    "foxeer": "Foxeer",
    "hawkeye": "Hawkeye",
    "firefly": "Firefly",
    "mobius": "Mobius",
    "morecam": "Morecam",
    "drift": "Drift",
    "rollei": "Rollei",
    "rolleiflex": "Rollei",
    "rollei (\u7984\u6765)": "Rollei",
    "apeman": "Apeman",
    "crosstour": "Crosstour",
    "sioeye": "Sioeye",
    "supremo": "Supremo",
    "evolio": "Evolio",
    "campark": "CamPark",
    "goxtreme": "Goxtreme",
    "niceboy": "Niceboy",
    "lamax": "Lamax",
    "sargo": "Sargo",
    "sargo11": "Sargo",
    "blaupunkt": "Blaupunkt",
    "blackshark": "Blackshark",
    "thieye": "ThiEYE",
    "eachine": "Eachine",
    "odrvm": "ODRVM",
    "ausek": "Ausek",
    "yolansin": "Yolansin",
    "ezviz": "Ezviz",
    "gitup": "Gitup",
    "matecam": "Matecam",
    "dogcam": "Dogcam",
    "overmax": "Overmax",
    "somikon": "Somikon",
    "simulus": "Simulus",
    "visuo": "Visuo",
    "apexcam": "Apexcam",
    "sencor": "Sencor",
    "surfola": "Surfola",
    "cycliq": "Cycliq",
    "biwond": "Biwond",
    "aolbea": "Aolbea",
    "forcite": "Forcite",
    "happymodel": "Happymodel",
    "vaquita": "Vaquita",
    "cleep": "Cleep",
    "duoke": "Duoke",
    "tracer": "Tracer",
    "cooau": "Cooau",
    "denver": "Denver",
    "orion": "Orion",
    "polaroid": "Polaroid",
    "sooyi": "Sooyi",
    "soocoo": "SooCoo",
    "andoer": "Andoer",
    "visi": "Visi",
    "aksogo": "Aksogo",
    "novatek": "Novatek",
    "cyanchen": "Cyanchen",
    "digma": "Digma",
    "forever": "Forever",
    "maginon": "Maginon",
    "decathlon": "Decathlon",
    "gadnic": "Gadnic",
    "monster": "Monster",
    "blackberry": "BlackBerry",
    "volla": "Volla",
    "general mobile": "General Mobile",
    "kinefinity": "Kinefinity",
    "mobula": "Mobula",
    "arducam": "Arducam",
    "nova7": "Nova7",
    "fujitsu": "Fujitsu",
    "yi technology": "YI Technology",
    "om system": "OM System",
    "om digital solutions": "OM System",
    "phase one": "Phase One",
    "soligor": "Soligor",
    "epson": "Epson",
    "microsoft": "Microsoft",

    # Chinese brand
    "\u7ebd\u66fc": "\u7ebd\u66fc",

    # Erroneous brand values that embed model info
    "sj6 legend": "SJCAM",
    "iqoo z3": "iQOO",
    "iqoo 9": "iQOO",
    "iqoo": "iQOO",
    "mate40pro": "Huawei",
    "mi": "Xiaomi",
    "infinix hot 30": "Infinix",
    "techno spark": "Tecno",
    "h7s": "H7S",
    "kf102": "KF102",
    "jjrc x5": "JJRC X5",
    "china_actioncam": "China ActionCam",
    "fujifilm": "Fujifilm",

    # LensFun-specific maker name variants
    "nikon corporation": "Nikon",
    "olympus corporation": "Olympus",
    "olympus imaging corp.": "Olympus",
    "olympus optical co.,ltd": "Olympus",
    "pentax corporation": "Pentax",
    "asahi optical co.,ltd": "Pentax",
    "samsung techwin": "Samsung",
    "samsung techwin co.": "Samsung",
    "leica camera ag": "Leica",
    "ricoh imaging company": "Ricoh",
    "ricoh imaging company, ltd.": "Ricoh",
    "ricoh imaging company, ltd": "Ricoh",
    "eastman kodak company": "Kodak",
    "casio computer co.,ltd": "Casio",
    "casio computer co.,ltd.": "Casio",
    "konica minolta camera, inc.": "Konica Minolta",
    "minolta co., ltd.": "Minolta",
    "seiko epson corp.": "Epson",
    "aee dv": "AEE",
    "fuji": "Fujifilm",
}

# Map of erroneous brand values to model overrides.
BRAND_TO_MODEL_OVERRIDE = {
    "sj6 legend": "SJ6 Legend",
    "iqoo z3": "Z3",
    "iqoo 9": "9",
    "mate40pro": "Mate 40 Pro",
    "infinix hot 30": "Hot 30",
    "techno spark": "Spark",
    "lg v30": "V30",
    "honor 80": "80",
}


def _auto_capitalize_brand(raw):
    """Fallback brand capitalization for brands not in the canonical map."""
    s = raw.strip()
    if not s:
        return s
    if len(s) <= 5 and s.isalpha():
        return s.upper()
    return s.title()


def normalize_brand(raw):
    """Return a canonical brand name."""
    if not raw:
        return ""
    key = raw.strip().lower()
    if key in BRAND_CANONICAL:
        return BRAND_CANONICAL[key]
    return _auto_capitalize_brand(raw)


# ---------------------------------------------------------------------------
# Model normalization
# ---------------------------------------------------------------------------

_WS = re.compile(r"\s+")


def _collapse_whitespace(s):
    return _WS.sub(" ", s).strip()


def normalize_model(brand, raw_model, raw_brand=""):
    """Normalize a camera model name."""
    s = _collapse_whitespace(raw_model)

    raw_brand_key = raw_brand.strip().lower()
    if raw_brand_key in BRAND_TO_MODEL_OVERRIDE:
        override = BRAND_TO_MODEL_OVERRIDE[raw_brand_key]
        if not s or s == raw_brand.strip():
            s = override
        elif s:
            pass
        else:
            s = override

    if not s:
        return ""

    # Strip leading brand name if the model accidentally includes it
    brand_lower = brand.lower()
    if s.lower().startswith(brand_lower + " "):
        s = s[len(brand) + 1:].strip()
    elif s.lower().startswith(brand_lower + "_"):
        s = s[len(brand) + 1:].strip()

    # Brand-specific model normalization
    if brand == "GoPro":
        s = _normalize_gopro_model(s)
    elif brand == "RunCam":
        s = _normalize_runcam_model(s)
    elif brand == "Apple":
        s = _normalize_apple_model(s)
    elif brand == "Olympus":
        s = _normalize_olympus_model(s)
    elif brand == "AKASO":
        s = _normalize_akaso_model(s)
    elif brand == "EKEN":
        s = _normalize_eken_model(s)
    elif brand in ("Xiaomi", "Redmi", "POCO"):
        s = _normalize_xiaomi_model(s)
    elif brand == "Samsung":
        s = _normalize_samsung_model(s)

    # Generic: title-case heuristic for single-word models that are all lower
    if s and s == s.lower() and " " not in s:
        s = s.title()

    return s


def _normalize_gopro_model(s):
    s = re.sub(r"(?i)\bhero\b", "HERO", s)
    s = re.sub(r"HERO\s*2014", "HERO (2014)", s)
    for color in ["Black", "Silver", "White"]:
        s = re.sub(r"(?i)\b" + color + r"\b", color, s)
    s = re.sub(r"(?i)\bmini\b", "Mini", s)
    s = re.sub(r"(?i)\bsession\b", "Session", s)
    s = re.sub(r"^Session\s*4$", "HERO4 Session", s)
    s = re.sub(r"^Session4$", "HERO4 Session", s)
    s = re.sub(r"^Session$", "HERO Session", s)
    s = re.sub(r"(?i)\bmax\b", "Max", s)
    return s


def _normalize_runcam_model(s):
    s = re.sub(r"(?i)^runcam\s+", "", s).strip()
    for word in ["Split", "Thumb", "Phoenix", "Wasp", "Hybrid", "Nano", "Link"]:
        s = re.sub(r"(?i)\b" + word + r"\b", word, s)
    s = re.sub(r"Split[- ]HD", "Split HD", s, flags=re.IGNORECASE)
    s = re.sub(r"(?i)\bthumb\s*pro\b", "Thumb Pro", s)
    s = re.sub(r"(?i)\bthumb\s*pro?\s*w\b", "Thumb Pro W", s)
    s = re.sub(r"^ThumbPW$", "Thumb Pro W", s)
    s = re.sub(r"(?i)\b4k\b", "4K", s)
    s = re.sub(r"^Split(\d)", r"Split \1", s)
    s = re.sub(r"(?i)\blite\b", "Lite", s)
    s = re.sub(r"(?i)\bhdzero\b", "HDZero", s)
    s = re.sub(r"^Split4K$", "Split 4K", s, flags=re.IGNORECASE)
    for color in ["Black", "Orange"]:
        s = re.sub(r"(?i)\b" + color + r"\b", color, s)
    return s


def _normalize_apple_model(s):
    s = re.sub(r"(?i)\biphone\b", "iPhone", s)
    for word in ["Pro", "Max", "Plus", "Mini"]:
        s = re.sub(r"(?i)\b" + word + r"\b", word, s)
    return s


def _normalize_olympus_model(s):
    s = re.sub(r"(?i)\bOMD\b", "OM-D", s)
    s = re.sub(r"(?i)\bom-d\b", "OM-D", s)
    s = re.sub(r"(?i)\bem\b", "E-M", s)
    s = re.sub(r"(?i)\bmark\b", "Mark", s)
    s = re.sub(r"(?i)\bMk\.?\b", "Mark", s)
    for num in ["II", "III", "IIIS", "IV", "V"]:
        s = re.sub(r"(?i)\b" + num + r"\b", num, s)
    return s


def _normalize_akaso_model(s):
    s = re.sub(r"(?i)\bbrave\b", "Brave", s)
    s = re.sub(r"(?i)EK7000\s*[Pp]ro", "EK7000 Pro", s)
    s = re.sub(r"(?i)\belite\b", "Elite", s)
    s = re.sub(r"\bLe\b", "LE", s)
    return s


def _normalize_eken_model(s):
    s = s.upper()
    return s


def _normalize_xiaomi_model(s):
    s = re.sub(r"(?i)\byi\b", "Yi", s)
    for word in ["Pro", "Ultra", "Note", "Lite", "Plus", "Max"]:
        s = re.sub(r"(?i)\b" + word + r"\b", word, s)
    return s


def _normalize_samsung_model(s):
    s = re.sub(r"(?i)\bgalaxy\b", "Galaxy", s)
    for word in ["Ultra", "Plus", "FE", "Lite", "Note", "Flip", "Fold"]:
        s = re.sub(r"(?i)\b" + word + r"\b", word, s)
    return s


# ---------------------------------------------------------------------------
# Lens normalization
# ---------------------------------------------------------------------------

def normalize_lens(raw_lens):
    """Normalize a lens model name."""
    s = _collapse_whitespace(raw_lens)
    if not s:
        return ""

    s = re.sub(r"(?i)\bf/?(\d)", r"f/\1", s)
    s = re.sub(r"(?i)\basph\.?\b", "ASPH", s)
    s = re.sub(r"(?i)\bed\b", "ED", s)
    s = re.sub(r"(?i)\bois\b", "OIS", s)
    s = re.sub(r"(?i)\bvr\b", "VR", s)
    s = re.sub(r"(?i)\bis\b", "IS", s)
    s = re.sub(r"(?i)\busm\b", "USM", s)
    s = re.sub(r"(?i)\bstm\b", "STM", s)
    s = re.sub(r"(?i)\boss\b", "OSS", s)
    s = re.sub(r"(?i)\bwr\b", "WR", s)
    s = re.sub(r"(?i)\bgm\b", "GM", s)
    s = re.sub(r"(?i)\bdg\b", "DG", s)
    s = re.sub(r"(?i)\bdn\b", "DN", s)
    s = re.sub(r"(?i)\bdc\b", "DC", s)
    s = re.sub(r"(?i)\bhsm\b", "HSM", s)
    s = re.sub(r"(?i)\bart\b", "Art", s)
    s = re.sub(r"(?i)\bcontemporary\b", "Contemporary", s)
    s = re.sub(r"(?i)\bmm\b", "mm", s)

    for word in ["Wide", "Linear", "Narrow", "Superview", "Max", "Hyperview"]:
        s = re.sub(r"(?i)\b" + word + r"\b", word, s)

    return s


# ---------------------------------------------------------------------------
# LensFun data loading
# ---------------------------------------------------------------------------

def load_lensfun_data(lensfun_path):
    """Load the pre-parsed LensFun JSON and build lookup indexes."""
    if not lensfun_path or not os.path.isfile(lensfun_path):
        return None

    with open(lensfun_path, "r", encoding="utf-8") as f:
        data = json.load(f)

    # Build camera lookup: (brand_lower, model_lower) -> camera info
    camera_lookup = {}
    for brand in data.get("brands", []):
        brand_name = brand["name"]
        # Also normalize via our own BRAND_CANONICAL
        canonical_brand = normalize_brand(brand_name)
        for cam in brand.get("cameras", []):
            # Index by both the display name and model_id
            for model_key in [cam.get("name", ""), cam.get("model_id", "")]:
                if model_key:
                    key = (canonical_brand.lower(), model_key.lower())
                    if key not in camera_lookup:
                        camera_lookup[key] = {
                            "crop_factor": cam.get("crop_factor"),
                            "sensor_width_mm": cam.get("sensor_width_mm"),
                            "sensor_height_mm": cam.get("sensor_height_mm"),
                            "mount": cam.get("mount"),
                        }

    # Build lens lookup: (brand_lower, model_lower) -> lens info
    lens_lookup = {}
    for brand in data.get("brands", []):
        brand_name = brand["name"]
        canonical_brand = normalize_brand(brand_name)
        for lens in brand.get("lenses", []):
            for model_key in [lens.get("name", ""), lens.get("model_id", "")]:
                if model_key:
                    key = (canonical_brand.lower(), model_key.lower())
                    if key not in lens_lookup:
                        lens_lookup[key] = {
                            "mount": lens.get("mount"),
                            "crop_factor": lens.get("crop_factor"),
                            "focal_length_min": lens.get("focal_length_min"),
                            "focal_length_max": lens.get("focal_length_max"),
                        }

    # Also build a brand-level camera list for injecting new models
    brand_cameras = defaultdict(list)
    for brand in data.get("brands", []):
        canonical_brand = normalize_brand(brand["name"])
        for cam in brand.get("cameras", []):
            brand_cameras[canonical_brand].append(cam)

    # Brand-level lens list for injecting new lenses
    brand_lenses = defaultdict(list)
    for brand in data.get("brands", []):
        canonical_brand = normalize_brand(brand["name"])
        for lens in brand.get("lenses", []):
            brand_lenses[canonical_brand].append(lens)

    # Mount info
    mounts = {m["name"]: m for m in data.get("mounts", [])}

    return {
        "camera_lookup": camera_lookup,
        "lens_lookup": lens_lookup,
        "brand_cameras": dict(brand_cameras),
        "brand_lenses": dict(brand_lenses),
        "mounts": mounts,
    }


def _find_lensfun_camera(lensfun, brand, model):
    """Try to find a LensFun camera match using fuzzy matching."""
    if not lensfun:
        return None

    lookup = lensfun["camera_lookup"]
    brand_l = brand.lower()
    model_l = model.lower()

    # Exact match
    key = (brand_l, model_l)
    if key in lookup:
        return lookup[key]

    # Try with brand prefix (LensFun often stores "Canon EOS 5D" as model)
    key_with_brand = (brand_l, f"{brand_l} {model_l}")
    if key_with_brand in lookup:
        return lookup[key_with_brand]

    # Try stripping common prefixes from gyroflow model names
    # e.g., gyroflow has "A7 III" but LensFun has "Alpha 7 III" or "ILCE-7M3"
    if brand_l == "sony":
        # Try Alpha prefix
        for prefix_map in [
            (r"^a(\d)", r"alpha \1"),
            (r"^a(\d)", r"ilce-\1"),
            (r"^zv-", "zv-"),
            (r"^fx", "ilme-fx"),
        ]:
            alt_model = re.sub(prefix_map[0], prefix_map[1], model_l, flags=re.IGNORECASE)
            if alt_model != model_l:
                key2 = (brand_l, alt_model)
                if key2 in lookup:
                    return lookup[key2]

    # Try partial match: if the model contains the LensFun model or vice versa
    for (bl, ml), info in lookup.items():
        if bl == brand_l:
            if model_l in ml or ml in model_l:
                return info

    return None


def _find_lensfun_lens(lensfun, lens_name):
    """Try to find a LensFun lens match."""
    if not lensfun:
        return None
    # Skip very short or generic lens names to avoid false matches
    if len(lens_name) < 5:
        return None

    lookup = lensfun["lens_lookup"]
    lens_l = lens_name.lower()

    # Exact match
    for (brand_l, model_l), info in lookup.items():
        if lens_l == model_l:
            return info

    # Substring match only for longer names (>= 10 chars) to avoid false positives
    if len(lens_name) >= 10:
        for (brand_l, model_l), info in lookup.items():
            if len(model_l) >= 10 and (lens_l in model_l or model_l in lens_l):
                return info

    return None


# ---------------------------------------------------------------------------
# Main extraction and database building
# ---------------------------------------------------------------------------

def extract_profile_data(filepath):
    """Extract relevant fields from a single lens profile JSON file."""
    try:
        with open(filepath, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (json.JSONDecodeError, UnicodeDecodeError, OSError) as exc:
        print(f"  WARNING: Could not read {filepath}: {exc}", file=sys.stderr)
        return None

    camera_brand = data.get("camera_brand", "") or ""
    camera_model = data.get("camera_model", "") or ""
    lens_model = data.get("lens_model", "") or ""
    camera_setting = data.get("camera_setting", "") or ""
    crop_factor = data.get("crop_factor")
    focal_length = data.get("focal_length")

    return {
        "raw_brand": camera_brand.strip(),
        "camera_brand": camera_brand.strip(),
        "camera_model": camera_model.strip(),
        "lens_model": lens_model.strip(),
        "camera_setting": camera_setting.strip() if isinstance(camera_setting, str) else str(camera_setting),
        "crop_factor": crop_factor,
        "focal_length": focal_length,
    }


def build_database(profiles_dir, lensfun_data_path=None):
    """Walk all .json files, merge with LensFun, and build the camera database."""
    profiles_path = Path(profiles_dir)

    # Load LensFun data if available
    lensfun = load_lensfun_data(lensfun_data_path)
    if lensfun:
        lf_cams = sum(len(v) for v in lensfun["brand_cameras"].values())
        lf_lenses = sum(len(v) for v in lensfun["brand_lenses"].values())
        print(f"Loaded LensFun data: {lf_cams} cameras, {lf_lenses} lenses, {len(lensfun['mounts'])} mounts")
    else:
        print("No LensFun data loaded (running without enrichment)")

    raw_profiles = []
    skipped = 0

    for json_file in sorted(profiles_path.rglob("*.json")):
        profile = extract_profile_data(str(json_file))
        if profile is None:
            skipped += 1
            continue
        raw_profiles.append(profile)

    print(f"Loaded {len(raw_profiles)} profiles ({skipped} skipped)")

    # Normalize and aggregate
    brand_models = defaultdict(lambda: defaultdict(int))
    lens_brands = defaultdict(set)

    for p in raw_profiles:
        brand = normalize_brand(p["camera_brand"])
        model = normalize_model(brand, p["camera_model"], p["raw_brand"])
        lens = normalize_lens(p["lens_model"])

        if not brand:
            brand = "(Unknown)"
        if not model:
            model = "(Unknown)"

        brand_models[brand][model] += 1

        if lens:
            lens_brands[lens].add(brand)

    # Enrich models with LensFun data and track which LensFun entries were matched
    model_metadata = {}  # (brand, model) -> metadata dict
    matched_lensfun_brands = set()

    if lensfun:
        for brand_name in brand_models:
            for model_name in brand_models[brand_name]:
                lf_cam = _find_lensfun_camera(lensfun, brand_name, model_name)
                if lf_cam:
                    model_metadata[(brand_name, model_name)] = lf_cam
                    matched_lensfun_brands.add(brand_name)

    # Inject new brands/models from LensFun that are not in gyroflow profiles
    lensfun_new_cameras = 0
    lensfun_new_lenses = 0

    if lensfun:
        for lf_brand, lf_cams in lensfun["brand_cameras"].items():
            canonical_brand = normalize_brand(lf_brand)
            for cam in lf_cams:
                display_name = cam.get("name", cam.get("model_id", ""))
                model_id = cam.get("model_id", display_name)
                # Try both display name and model_id as possible model keys
                existing = False
                for candidate in [display_name, model_id]:
                    norm_model = normalize_model(canonical_brand, candidate)
                    if norm_model in brand_models.get(canonical_brand, {}):
                        existing = True
                        break

                if not existing and display_name:
                    # Add as a new entry with 0 profiles
                    norm_model = normalize_model(canonical_brand, display_name)
                    if not norm_model:
                        norm_model = display_name
                    brand_models[canonical_brand][norm_model] += 0  # 0 gyroflow profiles
                    # Store metadata
                    meta = {}
                    if cam.get("crop_factor"):
                        meta["crop_factor"] = cam["crop_factor"]
                    if cam.get("sensor_width_mm"):
                        meta["sensor_width_mm"] = cam["sensor_width_mm"]
                    if cam.get("sensor_height_mm"):
                        meta["sensor_height_mm"] = cam["sensor_height_mm"]
                    if cam.get("mount"):
                        meta["mount"] = cam["mount"]
                    if meta:
                        model_metadata[(canonical_brand, norm_model)] = meta
                    lensfun_new_cameras += 1

    # Enrich lens info
    lens_metadata = {}  # lens_name -> metadata dict
    if lensfun:
        for lens_name in lens_brands:
            lf_lens = _find_lensfun_lens(lensfun, lens_name)
            if lf_lens:
                lens_metadata[lens_name] = lf_lens

        # Inject new lenses from LensFun
        for lf_brand, lf_lenses in lensfun["brand_lenses"].items():
            canonical_brand = normalize_brand(lf_brand)
            for lens in lf_lenses:
                lens_name = lens.get("name", lens.get("model_id", ""))
                if lens_name:
                    norm_lens = normalize_lens(lens_name)
                    if norm_lens and norm_lens not in lens_brands:
                        lens_brands[norm_lens].add(canonical_brand)
                        meta = {}
                        if lens.get("mount"):
                            meta["mount"] = lens["mount"]
                        if lens.get("focal_length_min") is not None:
                            meta["focal_length_min"] = lens["focal_length_min"]
                        if lens.get("focal_length_max") is not None:
                            meta["focal_length_max"] = lens["focal_length_max"]
                        if lens.get("crop_factor"):
                            meta["crop_factor"] = lens["crop_factor"]
                        if meta:
                            lens_metadata[norm_lens] = meta
                        lensfun_new_lenses += 1

    # Build output structure
    brands_list = []
    for brand_name in sorted(brand_models.keys()):
        models = brand_models[brand_name]
        models_list = []
        for model_name in sorted(models.keys()):
            entry = {
                "name": model_name,
                "lens_profiles_count": models[model_name],
            }
            # Add LensFun metadata if available
            meta = model_metadata.get((brand_name, model_name))
            if meta:
                if meta.get("crop_factor") is not None:
                    entry["crop_factor"] = meta["crop_factor"]
                if meta.get("sensor_width_mm") is not None:
                    entry["sensor_width_mm"] = meta["sensor_width_mm"]
                if meta.get("sensor_height_mm") is not None:
                    entry["sensor_height_mm"] = meta["sensor_height_mm"]
                if meta.get("mount"):
                    entry["mount"] = meta["mount"]
            models_list.append(entry)
        brands_list.append({
            "name": brand_name,
            "models": models_list,
        })

    lenses_list = []
    for lens_name in sorted(lens_brands.keys()):
        entry = {
            "name": lens_name,
            "brands": sorted(lens_brands[lens_name]),
        }
        # Add LensFun metadata if available
        meta = lens_metadata.get(lens_name)
        if meta:
            if meta.get("mount"):
                entry["mount"] = meta["mount"]
            if meta.get("focal_length_min") is not None:
                entry["focal_length_min"] = meta["focal_length_min"]
            if meta.get("focal_length_max") is not None:
                entry["focal_length_max"] = meta["focal_length_max"]
            if meta.get("crop_factor") is not None:
                entry["crop_factor"] = meta["crop_factor"]
        lenses_list.append(entry)

    # Add mounts to the database
    mounts_list = None
    if lensfun and lensfun.get("mounts"):
        mounts_list = []
        for name in sorted(lensfun["mounts"].keys()):
            m = lensfun["mounts"][name]
            mounts_list.append({
                "name": m["name"],
                "compatible_mounts": m.get("compatible_mounts", []),
            })

    database = {
        "version": 2,
        "brands": brands_list,
        "lenses": lenses_list,
    }
    if mounts_list:
        database["mounts"] = mounts_list

    # Print statistics
    total_brands = len(brands_list)
    total_models = sum(len(b["models"]) for b in brands_list)
    total_lenses = len(lenses_list)
    total_profiles = sum(
        m["lens_profiles_count"]
        for b in brands_list
        for m in b["models"]
    )

    models_with_crop = sum(
        1 for b in brands_list
        for m in b["models"]
        if m.get("crop_factor") is not None
    )
    models_with_mount = sum(
        1 for b in brands_list
        for m in b["models"]
        if m.get("mount") is not None
    )
    models_with_sensor = sum(
        1 for b in brands_list
        for m in b["models"]
        if m.get("sensor_width_mm") is not None
    )
    lenses_with_focal = sum(
        1 for l in lenses_list
        if l.get("focal_length_min") is not None
    )

    print(f"\n=== Camera Database Statistics ===")
    print(f"Total profiles processed: {total_profiles}")
    print(f"Total unique brands (after normalization): {total_brands}")
    print(f"Total unique models: {total_models}")
    print(f"Total unique lenses: {total_lenses}")
    if lensfun:
        print(f"\n--- LensFun Enrichment ---")
        print(f"New cameras from LensFun: {lensfun_new_cameras}")
        print(f"New lenses from LensFun:  {lensfun_new_lenses}")
        print(f"Models with crop_factor:  {models_with_crop}/{total_models}")
        print(f"Models with mount info:   {models_with_mount}/{total_models}")
        print(f"Models with sensor size:  {models_with_sensor}/{total_models}")
        print(f"Lenses with focal info:   {lenses_with_focal}/{total_lenses}")
        if mounts_list:
            print(f"Mount definitions:        {len(mounts_list)}")
    print()

    brand_totals = [
        (b["name"], sum(m["lens_profiles_count"] for m in b["models"]), len(b["models"]))
        for b in brands_list
    ]
    brand_totals.sort(key=lambda x: -x[2])
    print("Top 15 brands by model count:")
    for name, profiles, models in brand_totals[:15]:
        print(f"  {name}: {models} models ({profiles} profiles)")
    print()

    return database


def main():
    parser = argparse.ArgumentParser(description="Build gyroflow camera database")
    parser.add_argument(
        "--profiles-dir",
        default="/Users/omermac/Desktop/moneymaking2/workspace/gyroflow_lens_profiles/",
        help="Path to the lens_profiles repository root",
    )
    parser.add_argument(
        "--output",
        default="/Users/omermac/Desktop/moneymaking2/workspace/gyroflow_742/resources/camera_database.json",
        help="Output path for the camera database JSON",
    )
    parser.add_argument(
        "--lensfun-data",
        default="/tmp/lensfun_data.json",
        help="Path to pre-parsed LensFun JSON (from parse_lensfun.py)",
    )
    args = parser.parse_args()

    if not os.path.isdir(args.profiles_dir):
        print(f"ERROR: Profiles directory not found: {args.profiles_dir}", file=sys.stderr)
        sys.exit(1)

    database = build_database(args.profiles_dir, args.lensfun_data)

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    with open(output_path, "w", encoding="utf-8") as f:
        json.dump(database, f, indent=2, ensure_ascii=False)

    print(f"Database written to: {output_path}")
    print(f"File size: {output_path.stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
