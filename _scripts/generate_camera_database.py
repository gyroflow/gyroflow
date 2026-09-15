#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build resources/camera_database.json from Gyroflow lens profiles + LensFun.

This is the first-slice generator for gyroflow/gyroflow#742. It collects
canonical brands, bodies, lenses, mounts and sensor sizes so the UI can offer
selectors instead of free-typed strings.

The same script is intended to live later in gyroflow/lens_profiles and publish
`camera_database.json` next to `profiles.cbor.gz` on each release.

Usage:
    python3 _scripts/generate_camera_database.py
    python3 _scripts/generate_camera_database.py --offline
    python3 _scripts/generate_camera_database.py --out resources/camera_database.json

Network sources (cached under --cache-dir):
    - https://github.com/gyroflow/lens_profiles/releases/latest/download/profiles.cbor.gz
    - https://github.com/lensfun/lensfun/archive/refs/heads/master.tar.gz
"""

from __future__ import annotations

import argparse
import gzip
import io
import json
import os
import re
import sys
import tarfile
import urllib.request
import xml.etree.ElementTree as ET
from collections import OrderedDict
from datetime import datetime, timezone
from typing import Any

PROFILES_URL = "https://github.com/gyroflow/lens_profiles/releases/latest/download/profiles.cbor.gz"
LENSFUN_TARBALL_URL = "https://github.com/lensfun/lensfun/archive/refs/heads/master.tar.gz"

# Canonical brand display name -> aliases (lowercase, compared after collapse).
# Gyroflow profile strings and LensFun <maker> values are mapped onto these.
BRAND_ALIASES: dict[str, list[str]] = {
    "GoPro": ["gopro", "go pro", "gopro inc", "gopro, llc"],
    "Sony": ["sony", "sony corporation", "sony alpha"],
    "DJI": ["dji", "sz dji technology", "sz dji technology co., ltd", "dà-jiāng"],
    "Canon": ["canon", "canon inc", "canon eos", "canon inc."],
    "Nikon": ["nikon", "nikon corporation", "nikon corp"],
    "Blackmagic": [
        "blackmagic",
        "blackmagic design",
        "blackmagicdesign",
        "bmpcc",
        "blackmagic pocket cinema camera",
    ],
    "Insta360": ["insta360", "insta 360", "arashi vision", "arashi vision inc"],
    "Panasonic": ["panasonic", "lumix", "panasonic lumix"],
    "Fujifilm": ["fujifilm", "fuji", "fuji film", "fuji photo film", "fujinon", "fufifilm"],
    "Olympus": ["olympus", "om system", "om digital solutions", "omsystem"],
    "RED": ["red", "red digital cinema", "red camera"],
    "Apple": ["apple", "iphone", "ipad"],
    "Samsung": ["samsung", "samsung electronics"],
    "Hasselblad": ["hasselblad"],
    "Leica": ["leica", "leica camera", "leitz"],
    "Sigma": ["sigma"],
    "Tamron": ["tamron"],
    "Tokina": ["tokina"],
    "Zeiss": ["zeiss", "carl zeiss", "carl zeiss ag", "cz"],
    "Voigtlander": ["voigtlander", "voigtländer", "cosina"],
    "Runcam": ["runcam", "run cam"],
    "Caddx": ["caddx"],
    "Autel": ["autel", "autel robotics"],
    "Hawkeye": ["hawkeye", "hawk eye"],
    "Drift": ["drift", "drift innovation"],
    "Mobius": ["mobius"],
    "GitUp": ["gitup"],
    "SJCAM": ["sjcam"],
    "Akaso": ["akaso"],
    "DJI Osmo": [],  # kept only as alias target via DJI
    "Insta360 Ace": [],
    "Sharp": ["sharp"],
    "Kodak": ["kodak"],
    "Ricoh": ["ricoh", "pentax ricoh"],
    "Pentax": ["pentax", "asahi pentax"],
    "Minolta": ["minolta", "konica minolta", "konica-minolta"],
    "Konica": ["konica"],
    "Casio": ["casio"],
    "YI": ["yi", "xiaoyi", "yi technology"],
    "Xiaomi": ["xiaomi"],
    "Google": ["google"],
    "Microsoft": ["microsoft"],
    "Contour": ["contour"],
    "Garmin": ["garmin", "virb"],
    "Labpano": ["labpano"],
    "Qoocam": ["qoocam", "kandao", "kandao qoocam"],
    "Insta360 ONE": [],
    "Z CAM": ["z cam", "zcam"],
    "Kinefinity": ["kinefinity"],
    "ARRI": ["arri", "arnold & richter"],
    "Phantom": ["phantom", "vision research"],
    "Irix": ["irix"],
    "Samyang": ["samyang", "rokinon", "walimex"],
    "Schneider": ["schneider", "schneider-kreuznach"],
    "Vivitar": ["vivitar"],
    "Soligor": ["soligor"],
    "Contax": ["contax", "kyocera"],
    "Phase One": ["phase one", "phaseone"],
    "Mamiya": ["mamiya"],
    "Bronica": ["bronica"],
    "Pentacon": ["pentacon"],
    "Helios": ["helios"],
    "Zenit": ["zenit"],
    "Lomography": ["lomography", "lomo"],
    "DJI Action": [],
    "Back-Bone": ["back-bone", "backbone", "back bone"],
    "Cineovision": ["cineovision"],
    "Entaniya": ["entaniya"],
    "iZugar": ["izugar"],
    "Lensbaby": ["lensbaby"],
    "Meike": ["meike", "meike lens"],
    "7Artisans": ["7artisans", "7 artisans"],
    "TTArtisan": ["ttartisan", "tt artisan"],
    "Laowa": ["laowa", "venus optics", "venusoptics"],
    "Viltrox": ["viltrox"],
    "Yongnuo": ["yongnuo", "yn"],
    "Sirui": ["sirui"],
    "DZOFilm": ["dzofilm", "dzo film"],
    "Atlas": ["atlas", "atlas lens co"],
    "Cooke": ["cooke"],
    "Angenieux": ["angenieux", "angéineux"],
    "Fujinon": [],  # folded into Fujifilm via alias above
    "Panasonic Lumix": [],
}

# Brands that almost never have interchangeable lenses in Gyroflow footage.
FIXED_LENS_BRANDS = {
    "gopro",
    "insta360",
    "runcam",
    "caddx",
    "hawkeye",
    "drift",
    "mobius",
    "gitup",
    "sjcam",
    "akaso",
    "apple",
    "garmin",
    "labpano",
    "qoocam",
    "yi",
    "contour",
}

# Well-known flange focal distances (mm). Optional catalog metadata.
FLANGE_MM = {
    "sony_e": 18.0,
    "sony_a": 44.5,
    "canon_rf": 20.0,
    "canon_ef": 44.0,
    "canon_ef_m": 18.0,
    "canon_fd": 42.0,
    "nikon_z": 16.0,
    "nikon_f": 46.5,
    "fuji_x": 17.7,
    "fuji_g": 26.7,
    "mft": 19.25,
    "micro_43": 19.25,
    "leica_l": 20.0,
    "leica_m": 27.8,
    "leica_r": 47.0,
    "pl": 52.0,
    "m42": 45.46,
    "pentax_k": 45.46,
    "pentax_q": 9.2,
    "four_thirds": 38.67,
    "nikon_1": 17.0,
    "samsung_nx": 25.5,
    "eos_m": 18.0,
    "rf": 20.0,
    "ef": 44.0,
    "z": 16.0,
}

# Physical sensor classes used for compatibility (same class + same mount).
SENSOR_SIZES = [
    {"id": "medium_format", "name": "Medium format", "width_mm": 44.0, "height_mm": 33.0, "crop_factor": 0.79, "aliases": ["GFX", "645"]},
    {"id": "full_frame", "name": "Full frame", "width_mm": 36.0, "height_mm": 24.0, "crop_factor": 1.0, "aliases": ["FF", "35mm", "Full-frame", "35 mm"]},
    {"id": "aps_h", "name": "APS-H", "width_mm": 27.9, "height_mm": 18.6, "crop_factor": 1.3, "aliases": ["APS-H"]},
    {"id": "s35", "name": "Super 35", "width_mm": 24.89, "height_mm": 18.66, "crop_factor": 1.4, "aliases": ["S35", "Super35"]},
    {"id": "aps_c", "name": "APS-C", "width_mm": 23.6, "height_mm": 15.6, "crop_factor": 1.5, "aliases": ["APS-C", "DX", "APS-C (1.5x)"]},
    {"id": "aps_c_canon", "name": "APS-C (Canon)", "width_mm": 22.3, "height_mm": 14.9, "crop_factor": 1.6, "aliases": ["APS-C 1.6", "Canon APS-C"]},
    {"id": "mft", "name": "Micro Four Thirds", "width_mm": 17.3, "height_mm": 13.0, "crop_factor": 2.0, "aliases": ["MFT", "M43", "Micro 4/3", "4/3"]},
    {"id": "one_inch", "name": "1-inch", "width_mm": 13.2, "height_mm": 8.8, "crop_factor": 2.7, "aliases": ["1\"", "1 inch", "1-type"]},
    {"id": "two_thirds", "name": "2/3-inch", "width_mm": 8.8, "height_mm": 6.6, "crop_factor": 3.9, "aliases": ["2/3\""]},
    {"id": "action_1_1_7", "name": "1/1.7-inch", "width_mm": 7.6, "height_mm": 5.7, "crop_factor": 4.6, "aliases": ["1/1.7\""]},
    {"id": "action_1_2_3", "name": "1/2.3-inch", "width_mm": 6.17, "height_mm": 4.55, "crop_factor": 5.6, "aliases": ["1/2.3\"", "1/2.3"]},
    {"id": "unknown", "name": "Unknown", "width_mm": None, "height_mm": None, "crop_factor": 0.0, "aliases": []},
]


def log(msg: str) -> None:
    print(msg, file=sys.stderr, flush=True)


def collapse_ws(s: str) -> str:
    return re.sub(r"\s+", " ", (s or "").strip())


def brand_lookup_key(s: str) -> str:
    return collapse_ws(s).lower().replace(",", "")


def model_slug(name: str) -> str:
    """Compare camera/lens names after stripping punctuation and roman numerals."""
    s = (name or "").lower().replace("α", "a").replace("µ", "u").replace("μ", "u")
    s = re.sub(r"[^a-z0-9]+", "", s)
    return _finish_model_slug(s)


def camera_match_slug(brand_id: str, name: str) -> str:
    """Stronger matcher for camera bodies so ILCE-7SM3 and a7sIII collapse together."""
    s = model_slug(name)
    if brand_id == "sony":
        for prefix in ("sony", "alpha", "ilce", "ilme"):
            if s.startswith(prefix):
                s = s[len(prefix):]
        # ILCE mark codes: 7SM3 -> 7S3, 7M4 -> 74, 7RM5 -> 7R5
        s = re.sub(r"m(\d)$", r"\1", s)
        if re.match(r"^[67]", s):
            s = "a" + s
    elif brand_id == "canon":
        if s.startswith("canon"):
            s = s[5:]
        if s.startswith("eos"):
            s = s[3:]
    elif brand_id == "gopro":
        s = s.replace("black", "").replace("session", "session")
        s = re.sub(r"^hero", "hero", s)
    return s


def _finish_model_slug(s: str) -> str:
    for rom, dig in (
        ("viii", "8"),
        ("vii", "7"),
        ("iii", "3"),
        ("ii", "2"),
        ("ix", "9"),
        ("iv", "4"),
        ("vi", "6"),
    ):
        if s.endswith(rom):
            s = s[: -len(rom)] + dig
            break
    else:
        if s.endswith("v") and not s.endswith(("ov", "av", "iv")):  # A7R V, not "hero"
            # 'iv' already handled; bare trailing v -> 5 (A7R V)
            if len(s) > 1 and s[-2].isdigit():
                s = s[:-1] + "5"
    return s


def incoming_name_better(current: str, incoming: str) -> bool:
    """Prefer 'Alpha 7S III' / 'HERO11 Black' over 'a7sIII' / 'Hero11 black'."""
    if not incoming or incoming == current:
        return False
    cur_spaces = current.count(" ")
    inc_spaces = incoming.count(" ")
    if inc_spaces > cur_spaces:
        return True
    if inc_spaces == cur_spaces and incoming[0].isupper() and current[0].islower():
        return True
    return False


def infer_mount(brand_id: str, name: str) -> str | None:
    slug = camera_match_slug(brand_id, name)
    if brand_id == "sony":
        if any(x in slug for x in ("rx0", "rx100", "rx10", "rx1", "hx", "wx", "zv1")) and "zve" not in slug:
            return "fixed"
        if slug.startswith("a") or slug.startswith("fx") or slug.startswith("zv") or slug.startswith("nex"):
            return "sony_e"
        return "sony_e"
    if brand_id == "gopro" or brand_id in FIXED_LENS_BRANDS:
        return "fixed"
    if brand_id == "canon":
        if re.search(r"r[0-9]|rp$|r5|r6|r8|r3|r1", slug):
            return "canon_rf"
    if brand_id == "nikon":
        if slug.startswith("z"):
            return "nikon_z"
    if brand_id in {"olympus", "panasonic"} and re.search(r"gh|g9|g7|om3|om1|em1|em5", slug):
        return "mft"
    if brand_id == "fujifilm" and re.search(r"^x|^xt|^xh|^xe|^xs", slug):
        return "fuji_x"
    return None


def id_slug(*parts: str) -> str:
    raw = "_".join(p for p in parts if p)
    s = raw.lower().replace("α", "a")
    s = re.sub(r"[^a-z0-9]+", "_", s)
    return s.strip("_") or "unknown"


def sensor_size_id_for_crop(crop: float | None) -> str | None:
    if crop is None or crop <= 0:
        return None
    if crop < 0.9:
        return "medium_format"
    if crop <= 1.08:
        return "full_frame"
    if crop <= 1.35:
        return "aps_h"
    if crop <= 1.45:
        return "s35"
    if crop <= 1.55:
        return "aps_c"
    if crop <= 1.7:
        return "aps_c_canon"
    if crop <= 2.15:
        return "mft"
    if crop <= 3.0:
        return "one_inch"
    if crop <= 4.2:
        return "two_thirds"
    if crop <= 5.0:
        return "action_1_1_7"
    if crop <= 6.5:
        return "action_1_2_3"
    return "unknown"


def parse_focal_from_name(name: str) -> tuple[float | None, float | None]:
    # 24-70mm or 24–70 mm or 35mm
    m = re.search(r"(\d+(?:\.\d+)?)\s*[-–]\s*(\d+(?:\.\d+)?)\s*mm", name, re.I)
    if m:
        return float(m.group(1)), float(m.group(2))
    m = re.search(r"(\d+(?:\.\d+)?)\s*mm", name, re.I)
    if m:
        v = float(m.group(1))
        return v, v
    return None, None


def parse_aperture_from_name(name: str) -> float | None:
    m = re.search(r"f\s*/?\s*(\d+(?:\.\d+)?)", name, re.I)
    if m:
        return float(m.group(1))
    return None


def download(url: str, dest: str, offline: bool) -> None:
    if os.path.exists(dest) and os.path.getsize(dest) > 0:
        log(f"Using cache: {dest}")
        return
    if offline:
        raise SystemExit(f"Missing cached file (offline): {dest} from {url}")
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    log(f"Downloading {url}")
    req = urllib.request.Request(url, headers={"User-Agent": "gyroflow-camera-database-generator/1.0"})
    with urllib.request.urlopen(req, timeout=120) as resp, open(dest, "wb") as f:
        while True:
            chunk = resp.read(1024 * 256)
            if not chunk:
                break
            f.write(chunk)


def load_cbor2():
    try:
        import cbor2  # type: ignore
        return cbor2
    except ImportError:
        log("Installing cbor2 (needed to read profiles.cbor.gz)…")
        import subprocess
        subprocess.check_call([sys.executable, "-m", "pip", "install", "--quiet", "cbor2"])
        import cbor2  # type: ignore
        return cbor2


class Catalog:
    def __init__(self) -> None:
        self.brands: OrderedDict[str, dict[str, Any]] = OrderedDict()
        self.mounts: OrderedDict[str, dict[str, Any]] = OrderedDict()
        self.cameras: OrderedDict[str, dict[str, Any]] = OrderedDict()
        self.lenses: OrderedDict[str, dict[str, Any]] = OrderedDict()
        self._alias_to_brand: dict[str, str] = {}
        self._build_brand_index()
        self._seed_mounts()

    def _build_brand_index(self) -> None:
        for name, aliases in BRAND_ALIASES.items():
            if not aliases and name in {"DJI Osmo", "Insta360 Ace", "Insta360 ONE", "DJI Action", "Fujinon", "Panasonic Lumix"}:
                continue
            bid = id_slug(name)
            self.brands[bid] = {
                "id": bid,
                "name": name,
                "aliases": sorted({a for a in aliases if brand_lookup_key(a) != brand_lookup_key(name)}),
            }
            self._alias_to_brand[brand_lookup_key(name)] = bid
            for a in aliases:
                self._alias_to_brand[brand_lookup_key(a)] = bid

    def _seed_mounts(self) -> None:
        seeds = [
            ("fixed", "Fixed / built-in", ["built-in", "builtin", "integrated"]),
            ("sony_e", "Sony E", ["e", "e-mount", "nex", "sony e-mount", "fe"]),
            ("sony_a", "Sony A", ["a-mount", "minolta a", "sony alpha a"]),
            ("canon_rf", "Canon RF", ["rf", "rf-s", "canon rf"]),
            ("canon_ef", "Canon EF", ["ef", "ef-s", "canon ef"]),
            ("canon_ef_m", "Canon EF-M", ["ef-m", "efm"]),
            ("canon_fd", "Canon FD", ["fd"]),
            ("nikon_z", "Nikon Z", ["z", "z-mount", "nikon z"]),
            ("nikon_f", "Nikon F", ["f-mount", "nikon f", "f"]),
            ("fuji_x", "Fujifilm X", ["x", "x-mount", "fx", "fuji x"]),
            ("fuji_g", "Fujifilm G", ["g-mount", "fuji g"]),
            ("mft", "Micro Four Thirds", ["m43", "mft", "micro 4/3", "micro four thirds", "4/3"]),
            ("leica_l", "Leica L", ["l-mount", "l", "tl", "sl"]),
            ("leica_m", "Leica M", ["m-mount", "leica m"]),
            ("pl", "ARRI PL", ["pl", "arri pl"]),
            ("m42", "M42", ["m42", "praktica"]),
            ("pentax_k", "Pentax K", ["k", "k-mount", "pk"]),
            ("four_thirds", "Four Thirds", ["four thirds", "4/3 slr"]),
            ("unknown", "Unknown", []),
        ]
        for mid, name, aliases in seeds:
            self.mounts[mid] = {
                "id": mid,
                "name": name,
                "aliases": aliases,
                "compat": [],
                "flange_focal_distance_mm": FLANGE_MM.get(mid),
            }

    def ensure_brand(self, raw: str) -> str:
        raw = collapse_ws(raw)
        if not raw:
            return ""
        key = brand_lookup_key(raw)
        if key in self._alias_to_brand:
            bid = self._alias_to_brand[key]
            if raw != self.brands[bid]["name"] and raw not in self.brands[bid]["aliases"]:
                # Keep original spelling as an alias when it is not just case.
                if brand_lookup_key(raw) != brand_lookup_key(self.brands[bid]["name"]):
                    self.brands[bid]["aliases"].append(raw)
            return bid
        bid = id_slug(raw)
        if bid not in self.brands:
            # Title-case new brands so they look like the rest of the catalog.
            display = raw if raw.isupper() and len(raw) <= 5 else raw
            if not raw.isupper() or len(raw) > 5:
                display = raw[0].upper() + raw[1:] if raw else raw
            self.brands[bid] = {"id": bid, "name": display, "aliases": []}
        self._alias_to_brand[key] = bid
        return bid

    def ensure_mount(self, raw: str) -> str | None:
        raw = collapse_ws(raw)
        if not raw:
            return None
        key = brand_lookup_key(raw)
        for mid, m in self.mounts.items():
            if brand_lookup_key(m["name"]) == key:
                return mid
            if key in {brand_lookup_key(a) for a in m["aliases"]}:
                return mid
        mid = id_slug(raw)
        if mid not in self.mounts:
            self.mounts[mid] = {
                "id": mid,
                "name": raw,
                "aliases": [],
                "compat": [],
                "flange_focal_distance_mm": FLANGE_MM.get(mid),
            }
        return mid

    def find_camera(self, brand_id: str, name: str) -> dict[str, Any] | None:
        slug = camera_match_slug(brand_id, name)
        if not slug:
            return None
        for cam in self.cameras.values():
            if cam["brand_id"] != brand_id:
                continue
            if camera_match_slug(brand_id, cam["name"]) == slug:
                return cam
            if any(camera_match_slug(brand_id, a) == slug for a in cam.get("aliases") or []):
                return cam
        return None

    def upsert_camera(
        self,
        brand_id: str,
        name: str,
        *,
        aliases: list[str] | None = None,
        mount_id: str | None = None,
        crop_factor: float | None = None,
        fixed_lens: bool | None = None,
        source: str,
    ) -> str:
        name = collapse_ws(name)
        aliases = [collapse_ws(a) for a in (aliases or []) if collapse_ws(a) and collapse_ws(a) != name]
        existing = self.find_camera(brand_id, name)
        if existing is None:
            for a in aliases:
                existing = self.find_camera(brand_id, a)
                if existing:
                    break
        if existing:
            if name != existing["name"] and name not in existing["aliases"]:
                existing["aliases"].append(name)
            for a in aliases:
                if a != existing["name"] and a not in existing["aliases"]:
                    existing["aliases"].append(a)
            if mount_id and (not existing.get("mount_id") or existing.get("mount_id") == "fixed"):
                existing["mount_id"] = mount_id
            if crop_factor and not existing.get("crop_factor"):
                existing["crop_factor"] = crop_factor
                existing["sensor_size_id"] = sensor_size_id_for_crop(crop_factor)
            if brand_id in FIXED_LENS_BRANDS:
                existing["mount_id"] = "fixed"
                existing["fixed_lens"] = True
            elif existing.get("mount_id") and existing["mount_id"] != "fixed":
                existing["fixed_lens"] = False
            elif fixed_lens is True:
                existing["fixed_lens"] = True
            # Prefer a spaced / official display name over compact profile typography.
            if incoming_name_better(existing["name"], name):
                if existing["name"] not in existing["aliases"]:
                    existing["aliases"].append(existing["name"])
                existing["name"] = name
            if source not in existing["sources"]:
                existing["sources"].append(source)
            return existing["id"]

        if brand_id in FIXED_LENS_BRANDS:
            mount_id = "fixed"
            fixed_lens = True
        elif not mount_id:
            mount_id = infer_mount(brand_id, name)
        cid = id_slug(brand_id, name)
        n = 2
        while cid in self.cameras:
            cid = id_slug(brand_id, name, str(n))
            n += 1
        if fixed_lens is None:
            fixed_lens = brand_id in FIXED_LENS_BRANDS or mount_id in (None, "fixed")
        self.cameras[cid] = {
            "id": cid,
            "brand_id": brand_id,
            "name": name,
            "aliases": aliases,
            "mount_id": mount_id if mount_id != "fixed" else "fixed",
            "sensor_size_id": sensor_size_id_for_crop(crop_factor),
            "crop_factor": crop_factor,
            "fixed_lens": bool(fixed_lens),
            "sources": [source],
        }
        return cid

    def find_lens(self, brand_id: str, name: str, mount_id: str | None) -> dict[str, Any] | None:
        slug = model_slug(name)
        if not slug:
            return None
        for lens in self.lenses.values():
            if lens["brand_id"] != brand_id:
                continue
            if mount_id and lens.get("mount_id") and lens["mount_id"] != mount_id:
                continue
            if model_slug(lens["name"]) == slug:
                return lens
            if any(model_slug(a) == slug for a in lens.get("aliases") or []):
                return lens
        return None

    def upsert_lens(
        self,
        brand_id: str,
        name: str,
        *,
        aliases: list[str] | None = None,
        mount_id: str | None = None,
        camera_id: str | None = None,
        focal_min: float | None = None,
        focal_max: float | None = None,
        aperture_min: float | None = None,
        source: str,
    ) -> str:
        name = collapse_ws(name)
        if not name:
            return ""
        aliases = [collapse_ws(a) for a in (aliases or []) if collapse_ws(a) and collapse_ws(a) != name]
        existing = self.find_lens(brand_id, name, mount_id)
        if existing is None:
            for a in aliases:
                existing = self.find_lens(brand_id, a, mount_id)
                if existing:
                    break
        if existing:
            if name != existing["name"] and name not in existing["aliases"]:
                existing["aliases"].append(name)
            for a in aliases:
                if a != existing["name"] and a not in existing["aliases"]:
                    existing["aliases"].append(a)
            if camera_id and camera_id not in existing["camera_ids"]:
                existing["camera_ids"].append(camera_id)
            if focal_min and not existing.get("focal_min_mm"):
                existing["focal_min_mm"] = focal_min
            if focal_max and not existing.get("focal_max_mm"):
                existing["focal_max_mm"] = focal_max
            if aperture_min and not existing.get("aperture_min"):
                existing["aperture_min"] = aperture_min
            if source not in existing["sources"]:
                existing["sources"].append(source)
            return existing["id"]

        if focal_min is None or focal_max is None:
            fmin, fmax = parse_focal_from_name(name)
            focal_min = focal_min or fmin
            focal_max = focal_max or fmax
        if aperture_min is None:
            aperture_min = parse_aperture_from_name(name)
        kind = "unknown"
        if focal_min and focal_max:
            kind = "zoom" if abs(focal_max - focal_min) > 0.6 else "prime"

        lid = id_slug(brand_id, name, mount_id or "")
        n = 2
        while lid in self.lenses:
            lid = id_slug(brand_id, name, mount_id or "", str(n))
            n += 1
        self.lenses[lid] = {
            "id": lid,
            "brand_id": brand_id,
            "name": name,
            "aliases": aliases,
            "mount_id": mount_id,
            "camera_ids": [camera_id] if camera_id else [],
            "focal_min_mm": focal_min,
            "focal_max_mm": focal_max,
            "aperture_min": aperture_min,
            "kind": kind,
            "sources": [source],
        }
        return lid


def ingest_gyroflow_profiles(cat: Catalog, cbor_path: str) -> dict[str, Any]:
    cbor2 = load_cbor2()
    log(f"Reading Gyroflow profiles from {cbor_path}")
    with gzip.open(cbor_path, "rb") as f:
        data = cbor2.load(f)
    if not isinstance(data, list):
        raise SystemExit(f"Unexpected profiles.cbor.gz root type: {type(data)}")

    version = None
    count = 0
    skipped = 0
    for item in data:
        if not isinstance(item, (list, tuple)) or len(item) < 2:
            continue
        fname, profile = item[0], item[1]
        if fname == "__version":
            try:
                version = int(profile)
            except (TypeError, ValueError):
                version = profile
            continue
        if isinstance(fname, str) and fname.startswith("__"):
            continue
        if not isinstance(profile, dict):
            skipped += 1
            continue
        brand = collapse_ws(str(profile.get("camera_brand") or ""))
        model = collapse_ws(str(profile.get("camera_model") or ""))
        lens = collapse_ws(str(profile.get("lens_model") or ""))
        if not brand and not model:
            skipped += 1
            continue
        bid = cat.ensure_brand(brand) if brand else cat.ensure_brand("Unknown")
        crop = profile.get("crop_factor")
        try:
            crop_f = float(crop) if crop not in (None, "", 0, 0.0) else None
        except (TypeError, ValueError):
            crop_f = None
        # Action-camera profiles sometimes store crop=1.0 meaning "no extra digital crop",
        # not a full-frame sensor. Ignore that so LensFun / class tables can win.
        if bid in FIXED_LENS_BRANDS and crop_f is not None and crop_f <= 1.05:
            crop_f = None
        cid = cat.upsert_camera(bid, model or "Unknown", crop_factor=crop_f, source="gyroflow")
        if lens:
            cat.upsert_lens(bid, lens, camera_id=cid, source="gyroflow")
        count += 1
    return {"release_asset": "profiles.cbor.gz", "profile_count": count, "skipped": skipped, "bundle_version": version}


def _text_variants(elem: ET.Element, tag: str) -> tuple[str, list[str]]:
    primary = ""
    aliases: list[str] = []
    for child in elem.findall(tag):
        text = collapse_ws(child.text or "")
        if not text:
            continue
        lang = child.get("lang")
        if lang is None and not primary:
            primary = text
        elif lang == "en":
            aliases.append(text)
        else:
            aliases.append(text)
    # Prefer the English display name when present.
    en = [a for a in aliases if a]
    # If we have an English model that looks friendlier than a body code, swap.
    if primary and en:
        display = en[0]
        aliases = [primary] + [a for a in aliases if a != display]
        primary = display
    return primary, [a for a in aliases if a != primary]


def ingest_lensfun(cat: Catalog, xml_dir: str) -> dict[str, Any]:
    files = sorted(
        os.path.join(xml_dir, n)
        for n in os.listdir(xml_dir)
        if n.endswith(".xml")
    )
    log(f"Reading {len(files)} LensFun XML files from {xml_dir}")
    cameras = 0
    lenses = 0
    mounts = 0
    for path in files:
        try:
            tree = ET.parse(path)
        except ET.ParseError as e:
            log(f"Skipping {path}: {e}")
            continue
        root = tree.getroot()
        for mount in root.findall("mount"):
            name = collapse_ws(mount.findtext("name") or "")
            if not name:
                continue
            mid = cat.ensure_mount(name)
            mounts += 1
            if mid:
                for compat in mount.findall("compat"):
                    cname = collapse_ws(compat.text or "")
                    if not cname:
                        continue
                    cid = cat.ensure_mount(cname)
                    if cid and cid not in cat.mounts[mid]["compat"]:
                        cat.mounts[mid]["compat"].append(cid)
        for cam in root.findall("camera"):
            maker, maker_aliases = _text_variants(cam, "maker")
            model, model_aliases = _text_variants(cam, "model")
            if not maker or not model:
                continue
            bid = cat.ensure_brand(maker)
            for a in maker_aliases:
                cat.ensure_brand(a)  # maps onto the same brand via alias when known
            mount_id = cat.ensure_mount(collapse_ws(cam.findtext("mount") or ""))
            try:
                crop = float(cam.findtext("cropfactor") or "")
            except ValueError:
                crop = None
            cat.upsert_camera(
                bid,
                model,
                aliases=model_aliases,
                mount_id=mount_id,
                crop_factor=crop,
                fixed_lens=(mount_id in (None, "fixed")),
                source="lensfun",
            )
            cameras += 1
        for lens in root.findall("lens"):
            maker, maker_aliases = _text_variants(lens, "maker")
            model, model_aliases = _text_variants(lens, "model")
            if not maker or not model:
                continue
            bid = cat.ensure_brand(maker)
            mount_id = cat.ensure_mount(collapse_ws(lens.findtext("mount") or ""))
            focals: list[float] = []
            for dist in lens.findall("./calibration/distortion"):
                try:
                    focals.append(float(dist.get("focal") or ""))
                except ValueError:
                    pass
            fmin = min(focals) if focals else None
            fmax = max(focals) if focals else None
            cat.upsert_lens(
                bid,
                model,
                aliases=model_aliases,
                mount_id=mount_id,
                focal_min=fmin,
                focal_max=fmax,
                source="lensfun",
            )
            lenses += 1
    return {"xml_files": len(files), "cameras": cameras, "lenses": lenses, "mounts": mounts}


def extract_lensfun_db(tarball: str, dest_dir: str) -> str:
    if os.path.isdir(dest_dir) and any(n.endswith(".xml") for n in os.listdir(dest_dir)):
        log(f"Using cached LensFun XML in {dest_dir}")
        return dest_dir
    os.makedirs(dest_dir, exist_ok=True)
    log(f"Extracting LensFun data/db from {tarball}")
    with tarfile.open(tarball, "r:gz") as tar:
        for member in tar.getmembers():
            # lensfun-master/data/db/foo.xml
            parts = member.name.split("/")
            if len(parts) >= 4 and parts[1] == "data" and parts[2] == "db" and parts[-1].endswith(".xml"):
                member.name = parts[-1]
                tar.extract(member, dest_dir)
    return dest_dir


def to_jsonable(cat: Catalog, sources: dict[str, Any]) -> dict[str, Any]:
    def sort_key_name(item: dict[str, Any]) -> str:
        return item.get("name", "").lower()

    used_brands = {c["brand_id"] for c in cat.cameras.values()} | {l["brand_id"] for l in cat.lenses.values()}
    brands = []
    for b in sorted(cat.brands.values(), key=sort_key_name):
        if b["id"] not in used_brands:
            continue
        aliases = sorted({collapse_ws(a) for a in b["aliases"] if collapse_ws(a) and collapse_ws(a) != b["name"]}, key=str.lower)
        brands.append({"id": b["id"], "name": b["name"], "aliases": aliases})

    mounts = []
    for m in sorted(cat.mounts.values(), key=sort_key_name):
        mounts.append({
            "id": m["id"],
            "name": m["name"],
            "aliases": sorted(set(m["aliases"]), key=str.lower),
            "compat": sorted(set(m["compat"])),
            "flange_focal_distance_mm": m.get("flange_focal_distance_mm"),
        })

    cameras = []
    for c in sorted(cat.cameras.values(), key=lambda x: (x["brand_id"], x["name"].lower())):
        cameras.append({
            "id": c["id"],
            "brand_id": c["brand_id"],
            "name": c["name"],
            "aliases": sorted({a for a in c["aliases"] if a and a != c["name"]}, key=str.lower),
            "mount_id": c.get("mount_id"),
            "sensor_size_id": c.get("sensor_size_id"),
            "crop_factor": c.get("crop_factor"),
            "fixed_lens": bool(c.get("fixed_lens")),
            "sources": sorted(set(c.get("sources") or [])),
        })

    lenses = []
    for lens in sorted(cat.lenses.values(), key=lambda x: (x["brand_id"], x["name"].lower())):
        lenses.append({
            "id": lens["id"],
            "brand_id": lens["brand_id"],
            "name": lens["name"],
            "aliases": sorted({a for a in lens["aliases"] if a and a != lens["name"]}, key=str.lower),
            "mount_id": lens.get("mount_id"),
            "camera_ids": sorted(set(lens.get("camera_ids") or [])),
            "focal_min_mm": lens.get("focal_min_mm"),
            "focal_max_mm": lens.get("focal_max_mm"),
            "aperture_min": lens.get("aperture_min"),
            "kind": lens.get("kind") or "unknown",
            "sources": sorted(set(lens.get("sources") or [])),
        })

    return {
        "version": 1,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "sources": sources,
        "brands": brands,
        "mounts": mounts,
        "sensor_sizes": SENSOR_SIZES,
        "cameras": cameras,
        "lenses": lenses,
    }


def main() -> int:
    repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", default=os.path.join(os.environ.get("TMPDIR", "/tmp"), "gyroflow-camera-db"))
    parser.add_argument("--out", default=os.path.join(repo_root, "resources", "camera_database.json"))
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()

    cache = args.cache_dir
    os.makedirs(cache, exist_ok=True)
    profiles_path = os.path.join(cache, "profiles.cbor.gz")
    tarball_path = os.path.join(cache, "lensfun-master.tar.gz")
    xml_dir = os.path.join(cache, "lensfun-db")

    download(PROFILES_URL, profiles_path, args.offline)
    download(LENSFUN_TARBALL_URL, tarball_path, args.offline)
    extract_lensfun_db(tarball_path, xml_dir)

    cat = Catalog()
    # LensFun first so official English body names win; Gyroflow then adds aliases.
    lf = ingest_lensfun(cat, xml_dir)
    gf = ingest_gyroflow_profiles(cat, profiles_path)
    payload = to_jsonable(cat, {"gyroflow_lens_profiles": gf, "lensfun": lf})

    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, ensure_ascii=False)
        f.write("\n")

    log(
        f"Wrote {args.out}: {len(payload['brands'])} brands, "
        f"{len(payload['mounts'])} mounts, {len(payload['sensor_sizes'])} sensor sizes, "
        f"{len(payload['cameras'])} cameras, {len(payload['lenses'])} lenses "
        f"({gf['profile_count']} gyroflow profiles, {lf['cameras']} lensfun cameras, {lf['lenses']} lensfun lenses)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
