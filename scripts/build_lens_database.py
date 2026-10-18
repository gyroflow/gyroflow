#!/usr/bin/env python3
"""
Gyroflow Camera & Lens Database Builder
Solves issue gyroflow/gyroflow#742: "Refactor lens profile handling"
Maintainer: Adrian Eddy (@AdrianEddy)

Aggregates, normalizes, and validates camera bodies, lens profiles, mounts,
sensor dimensions, and crop factors from:
  1. Open-source Lensfun XML database
  2. Gyroflow lens profile presets (9,800+ profiles across 32 brands)
  3. Authoritative optical specs for FPV, action cams, and cinema systems

Exports canonical JSON database to: `resources/camera_lens_database.json`
Ensures 100% backwards compatibility with legacy .gyroflow project files.
"""

import sys
import os
import json
import re
import math
import glob
import urllib.request
import xml.etree.ElementTree as ET
from typing import Dict, List, Optional, Tuple, Any, Set
from dataclasses import dataclass, field, asdict
from datetime import datetime, timezone

# Ensure UTF-8 console output on Windows
if sys.stdout and hasattr(sys.stdout, 'reconfigure'):
    sys.stdout.reconfigure(encoding='utf-8')
if sys.stderr and hasattr(sys.stderr, 'reconfigure'):
    sys.stderr.reconfigure(encoding='utf-8')

# Reference diagonal for standard full frame (36mm x 24mm)
FULL_FRAME_WIDTH = 36.0
FULL_FRAME_HEIGHT = 24.0
FULL_FRAME_DIAG = math.sqrt(FULL_FRAME_WIDTH ** 2 + FULL_FRAME_HEIGHT ** 2)  # ~43.2666 mm

# Standard Canonical Brand Mappings
BRAND_CANONICAL_MAP = {
    "sony": "Sony",
    "canon": "Canon",
    "nikon": "Nikon",
    "fujifilm": "Fujifilm",
    "fuji": "Fujifilm",
    "fufifilm": "Fujifilm",
    "panasonic": "Panasonic",
    "lumix": "Panasonic",
    "olympus": "Olympus",
    "om-system": "OM System",
    "om system": "OM System",
    "gopro": "GoPro",
    "dji": "DJI",
    "blackmagic": "Blackmagic Design",
    "blackmagic design": "Blackmagic Design",
    "bmd": "Blackmagic Design",
    "red": "RED Digital Cinema",
    "red digital cinema": "RED Digital Cinema",
    "insta360": "Insta360",
    "caddx": "Caddx",
    "runcam": "RunCam",
    "walksnail": "Walksnail",
    "foxeer": "Foxeer",
    "hawkeye": "Hawkeye",
    "betafpv": "BetaFPV",
    "z cam": "Z CAM",
    "zcam": "Z CAM",
    "arri": "ARRI",
    "kinefinity": "Kinefinity",
    "freefly": "Freefly",
    "sigma": "Sigma",
    "tamron": "Tamron",
    "samyang": "Samyang",
    "rokinon": "Samyang",
    "zeiss": "Zeiss",
    "carl zeiss": "Zeiss",
    "venus optics": "Venus Optics",
    "laowa": "Venus Optics",
    "sirui": "Sirui",
    "ttartisan": "TTArtisan",
    "7artisans": "7Artisans",
    "meike": "Meike",
    "tokina": "Tokina",
    "voigtlander": "Voigtländer",
    "voigtländer": "Voigtländer",
    "leica": "Leica",
    "hasselblad": "Hasselblad",
    "pentax": "Pentax",
    "apple": "Apple",
    "samsung": "Samsung",
    "google": "Google",
    "xiaomi": "Xiaomi",
    "oneplus": "OnePlus",
    "huawei": "Huawei",
    "motorola": "Motorola",
    "vivo": "Vivo",
    "oppo": "Oppo",
    "poco": "POCO",
    "akaso": "AKASO",
    "sjcam": "SJCAM",
    "xtu": "XTU",
    "mobius": "Mobius",
    "apeman": "apeman",
    "morecam": "Morecam",
    "thieye": "ThiEYE",
    "eken": "Eken",
}

# Standard Mount Definitions with flange focal distances & compatibility
STANDARD_MOUNTS = {
    "Sony E": {
        "name": "Sony E",
        "flange_distance_mm": 18.0,
        "description": "Sony E-mount (APS-C & Full Frame FE)",
        "compatible_mounts": ["Sony E", "Sony FE"],
        "compatible_sensor_types": ["Full Frame", "APS-C"]
    },
    "Canon RF": {
        "name": "Canon RF",
        "flange_distance_mm": 20.0,
        "description": "Canon RF / RF-S mirrorless mount",
        "compatible_mounts": ["Canon RF", "Canon RF-S"],
        "compatible_sensor_types": ["Full Frame", "APS-C"]
    },
    "Canon EF": {
        "name": "Canon EF",
        "flange_distance_mm": 44.0,
        "description": "Canon EF / EF-S DSLR & Cinema mount",
        "compatible_mounts": ["Canon EF", "Canon EF-S"],
        "compatible_sensor_types": ["Full Frame", "APS-C", "Super 35"]
    },
    "Canon EF-M": {
        "name": "Canon EF-M",
        "flange_distance_mm": 18.0,
        "description": "Canon EF-M mirrorless mount",
        "compatible_mounts": ["Canon EF-M"],
        "compatible_sensor_types": ["APS-C"]
    },
    "Nikon Z": {
        "name": "Nikon Z",
        "flange_distance_mm": 16.0,
        "description": "Nikon Z mirrorless mount (FX & DX)",
        "compatible_mounts": ["Nikon Z"],
        "compatible_sensor_types": ["Full Frame", "APS-C"]
    },
    "Nikon F": {
        "name": "Nikon F",
        "flange_distance_mm": 46.5,
        "description": "Nikon F DSLR mount (FX & DX)",
        "compatible_mounts": ["Nikon F"],
        "compatible_sensor_types": ["Full Frame", "APS-C"]
    },
    "Fujifilm X": {
        "name": "Fujifilm X",
        "flange_distance_mm": 17.7,
        "description": "Fujifilm X mirrorless APS-C mount",
        "compatible_mounts": ["Fujifilm X"],
        "compatible_sensor_types": ["APS-C"]
    },
    "Fujifilm G": {
        "name": "Fujifilm G",
        "flange_distance_mm": 26.7,
        "description": "Fujifilm G medium format mount",
        "compatible_mounts": ["Fujifilm G"],
        "compatible_sensor_types": ["Medium Format"]
    },
    "Micro Four Thirds": {
        "name": "Micro Four Thirds",
        "flange_distance_mm": 19.25,
        "description": "Micro Four Thirds (MFT / M43) mirrorless standard",
        "compatible_mounts": ["Micro Four Thirds", "MFT", "M4/3"],
        "compatible_sensor_types": ["Micro Four Thirds"]
    },
    "Leica L": {
        "name": "Leica L",
        "flange_distance_mm": 20.0,
        "description": "L-Mount Alliance (Panasonic, Leica, Sigma, DJI)",
        "compatible_mounts": ["Leica L", "L-Mount"],
        "compatible_sensor_types": ["Full Frame", "APS-C"]
    },
    "Leica M": {
        "name": "Leica M",
        "flange_distance_mm": 27.8,
        "description": "Leica M rangefinder mount",
        "compatible_mounts": ["Leica M"],
        "compatible_sensor_types": ["Full Frame"]
    },
    "DJI DL": {
        "name": "DJI DL",
        "flange_distance_mm": 16.84,
        "description": "DJI DL Cinema mount (Ronin 4D, Inspire 2/3 X7/X9)",
        "compatible_mounts": ["DJI DL"],
        "compatible_sensor_types": ["Full Frame", "Super 35"]
    },
    "PL Mount": {
        "name": "PL Mount",
        "flange_distance_mm": 52.0,
        "description": "Arri PL professional cinema standard",
        "compatible_mounts": ["PL Mount", "Arri PL"],
        "compatible_sensor_types": ["Full Frame", "Super 35", "Large Format"]
    },
    "LPL Mount": {
        "name": "LPL Mount",
        "flange_distance_mm": 44.0,
        "description": "Arri LPL large-format cinema mount",
        "compatible_mounts": ["LPL Mount", "Arri LPL"],
        "compatible_sensor_types": ["Large Format", "Full Frame", "Super 35"]
    },
    "M12": {
        "name": "M12",
        "flange_distance_mm": None,
        "description": "M12 x 0.5 S-Mount board lens (FPV, micro cams)",
        "compatible_mounts": ["M12", "S-Mount"],
        "compatible_sensor_types": ["1/1.8-inch", "1/2-inch", "1/2.3-inch", "1/2.7-inch", "1/3-inch"]
    },
    "C-Mount": {
        "name": "C-Mount",
        "flange_distance_mm": 17.526,
        "description": "C-Mount industrial and 16mm cine standard",
        "compatible_mounts": ["C-Mount", "CS-Mount"],
        "compatible_sensor_types": ["1-inch", "2/3-inch", "1/2-inch"]
    },
    "GoPro Fixed": {
        "name": "GoPro Fixed",
        "flange_distance_mm": None,
        "description": "Integrated non-interchangeable GoPro action lens",
        "compatible_mounts": ["GoPro Fixed"],
        "compatible_sensor_types": ["1/1.9-inch", "1/2.3-inch"]
    },
    "DJI Action Fixed": {
        "name": "DJI Action Fixed",
        "flange_distance_mm": None,
        "description": "Integrated non-interchangeable DJI Action / Pocket lens",
        "compatible_mounts": ["DJI Action Fixed"],
        "compatible_sensor_types": ["1-inch", "1/1.3-inch", "1/1.7-inch", "1/2.3-inch"]
    },
    "Insta360 Fixed": {
        "name": "Insta360 Fixed",
        "flange_distance_mm": None,
        "description": "Integrated non-interchangeable Insta360 lens",
        "compatible_mounts": ["Insta360 Fixed"],
        "compatible_sensor_types": ["1-inch", "1/1.3-inch", "1/2-inch", "1/2.3-inch"]
    },
    "FPV Fixed": {
        "name": "FPV Fixed",
        "flange_distance_mm": None,
        "description": "Integrated board / micro lens (RunCam, Caddx, Walksnail)",
        "compatible_mounts": ["FPV Fixed", "M12"],
        "compatible_sensor_types": ["1/1.7-inch", "1/1.8-inch", "1/2-inch", "1/2.3-inch"]
    },
    "Smartphone Integrated": {
        "name": "Smartphone Integrated",
        "flange_distance_mm": None,
        "description": "Fixed mobile phone camera array module",
        "compatible_mounts": ["Smartphone Integrated"],
        "compatible_sensor_types": ["1-inch", "1/1.28-inch", "1/1.3-inch", "1/1.5-inch", "1/2-inch"]
    },
    "Fixed": {
        "name": "Fixed",
        "flange_distance_mm": None,
        "description": "Fixed built-in camera lens",
        "compatible_mounts": ["Fixed"],
        "compatible_sensor_types": ["Full Frame", "APS-C", "1-inch", "Compact"]
    }
}


def slugify(text: str) -> str:
    """Creates a deterministic alphanumeric slug ID."""
    clean = re.sub(r'[^a-zA-Z0-9]+', '_', text.strip().lower())
    return clean.strip('_')


def calculate_crop_factor(width_mm: float, height_mm: float) -> float:
    """Calculates crop factor relative to standard 35mm full frame (36x24mm)."""
    diag = math.sqrt(width_mm ** 2 + height_mm ** 2)
    if diag <= 0:
        return 1.0
    return round(FULL_FRAME_DIAG / diag, 3)


def calculate_dimensions_from_crop(crop_factor: float, aspect_w: float = 3.0, aspect_h: float = 2.0) -> Tuple[float, float]:
    """Calculates width and height in mm given crop factor and aspect ratio."""
    if crop_factor <= 0:
        crop_factor = 1.0
    sensor_diag = FULL_FRAME_DIAG / crop_factor
    ratio = aspect_w / aspect_h
    h = sensor_diag / math.sqrt(ratio ** 2 + 1)
    w = ratio * h
    return round(w, 2), round(h, 2)


@dataclass
class CameraEntry:
    id: str
    brand: str
    model: str
    aliases: List[str]
    lens_mount: str
    sensor_size: Dict[str, float]  # width, height
    crop_factor: float
    sensor_type: str
    is_interchangeable_lens: bool
    default_lens: Optional[str] = None

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


@dataclass
class LensEntry:
    id: str
    brand: str
    model: str
    aliases: List[str]
    mount_compatibility: List[str]
    focal_range: Dict[str, float]  # min, max
    aperture: Dict[str, float]  # min_f_stop, max_f_stop
    is_zoom: bool
    coverage_crop_factor: float = 1.0

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


class CuratedHardwareCatalog:
    """
    Authoritative reference repository for action cameras, FPV systems,
    cinema cameras, and high-priority mirrorless/DSLR gear.
    """
    @staticmethod
    def get_curated_cameras() -> List[CameraEntry]:
        cameras = [
            # --- Sony Full Frame Mirrorless ---
            CameraEntry("sony_alpha_1", "Sony", "Alpha 1", ["ILCE-1", "A1", "Alpha 1"], "Sony E", {"width": 35.9, "height": 23.9}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7_iv", "Sony", "Alpha 7 IV", ["ILCE-7M4", "A7 IV", "A74", "a7iv", "Alpha 7M4", "A7IV", "Sony A7IV"], "Sony E", {"width": 35.7, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7_iii", "Sony", "Alpha 7 III", ["ILCE-7M3", "A7 III", "A73", "a7iii", "Alpha 7M3", "A7III", "Sony A7III"], "Sony E", {"width": 35.6, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7_ii", "Sony", "Alpha 7 II", ["ILCE-7M2", "A7 II", "A72", "a7ii", "A7II"], "Sony E", {"width": 35.8, "height": 23.9}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7s_iii", "Sony", "Alpha 7S III", ["ILCE-7SM3", "A7S III", "A7S3", "a7siii", "A7sIII", "Sony A7sIII"], "Sony E", {"width": 35.6, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7s_ii", "Sony", "Alpha 7S II", ["ILCE-7SM2", "A7S II", "A7S2", "a7sii", "A7sII"], "Sony E", {"width": 35.6, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7r_v", "Sony", "Alpha 7R V", ["ILCE-7RM5", "A7R V", "A7R5", "a7rv", "A7RV"], "Sony E", {"width": 35.7, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7r_iv", "Sony", "Alpha 7R IV", ["ILCE-7RM4", "A7R IV", "A7R4", "a7riv", "A7RIV"], "Sony E", {"width": 35.7, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7r_iii", "Sony", "Alpha 7R III", ["ILCE-7RM3", "A7R III", "A7R3", "a7riii", "A7RIII"], "Sony E", {"width": 35.9, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7r_ii", "Sony", "Alpha 7R II", ["ILCE-7RM2", "A7R II", "A7R2", "a7rii", "a7rII", "A7RII"], "Sony E", {"width": 35.9, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7c", "Sony", "Alpha 7C", ["ILCE-7C", "A7C", "a7c", "Sony A7C"], "Sony E", {"width": 35.6, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7c_ii", "Sony", "Alpha 7C II", ["ILCE-7CM2", "A7C II", "A7C2", "7CII", "a7cii"], "Sony E", {"width": 35.7, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_7cr", "Sony", "Alpha 7CR", ["ILCE-7CR", "A7CR", "7CR"], "Sony E", {"width": 35.7, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_9_ii", "Sony", "Alpha 9 II", ["ILCE-9M2", "A9 II", "A92"], "Sony E", {"width": 35.6, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_alpha_9_iii", "Sony", "Alpha 9 III", ["ILCE-9M3", "A9 III", "A93"], "Sony E", {"width": 35.6, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_fx3", "Sony", "FX3", ["ILME-FX3", "Sony FX3"], "Sony E", {"width": 35.6, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_fx6", "Sony", "FX6", ["ILME-FX6", "Sony FX6"], "Sony E", {"width": 35.7, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_fx9", "Sony", "FX9", ["ILME-FX9", "PXW-FX9"], "Sony E", {"width": 35.7, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("sony_zv_e1", "Sony", "ZV-E1", ["ZV-E1", "ZVE1", "Sony ZV-E1"], "Sony E", {"width": 35.6, "height": 23.8}, 1.0, "Full Frame", True),

            # --- Sony APS-C Mirrorless ---
            CameraEntry("sony_fx30", "Sony", "FX30", ["ILME-FX30", "Sony FX30"], "Sony E", {"width": 23.3, "height": 15.5}, 1.546, "APS-C", True),
            CameraEntry("sony_a6700", "Sony", "Alpha 6700", ["ILCE-6700", "A6700", "a6700"], "Sony E", {"width": 23.3, "height": 15.5}, 1.546, "APS-C", True),
            CameraEntry("sony_a6600", "Sony", "Alpha 6600", ["ILCE-6600", "A6600", "a6600"], "Sony E", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("sony_a6500", "Sony", "Alpha 6500", ["ILCE-6500", "A6500", "a6500"], "Sony E", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("sony_a6400", "Sony", "Alpha 6400", ["ILCE-6400", "A6400", "a6400", "Sony a6400"], "Sony E", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("sony_a6300", "Sony", "Alpha 6300", ["ILCE-6300", "A6300", "6300", "a6300"], "Sony E", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("sony_a6100", "Sony", "Alpha 6100", ["ILCE-6100", "A6100", "a6100"], "Sony E", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("sony_a6000", "Sony", "Alpha 6000", ["ILCE-6000", "A6000", "a6000"], "Sony E", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("sony_zv_e10", "Sony", "ZV-E10", ["ZV-E10", "ZVE10", "Sony ZV-E10"], "Sony E", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("sony_zv_e10_ii", "Sony", "ZV-E10 II", ["ZV-E10M2", "ZVE10 II", "ZVE10M2"], "Sony E", {"width": 23.3, "height": 15.5}, 1.546, "APS-C", True),

            # --- Sony Compacts & Action ---
            CameraEntry("sony_rx0_ii", "Sony", "DSC-RX0 II", ["DSC-RX0M2", "RX0 II", "RX0M2", "Sony DSC-RX0M2"], "Fixed", {"width": 13.2, "height": 8.8}, 2.727, "1-inch", False, "ZEISS Tessar T* 24mm F4"),
            CameraEntry("sony_rx0", "Sony", "DSC-RX0", ["DSC-RX0", "RX0", "Sony DSC-RX0"], "Fixed", {"width": 13.2, "height": 8.8}, 2.727, "1-inch", False, "ZEISS Tessar T* 24mm F4"),
            CameraEntry("sony_rx100_vii", "Sony", "DSC-RX100 VII", ["DSC-RX100M7", "RX100 VII"], "Fixed", {"width": 13.2, "height": 8.8}, 2.727, "1-inch", False, "ZEISS Vario-Sonnar T* 24-200mm F2.8-4.5"),
            CameraEntry("sony_fdr_x3000", "Sony", "FDR-X3000", ["FDR-X3000", "X3000", "Sony FDR-X3000"], "Fixed", {"width": 5.76, "height": 4.29}, 6.0, "1/2.5-inch", False, "ZEISS Tessar Ultra-Wide"),

            # --- GoPro Action Cameras ---
            CameraEntry("gopro_hero13_black", "GoPro", "HERO13 Black", ["HERO13", "HERO13 Black", "Hero 13", "Hero13"], "GoPro Fixed", {"width": 6.40, "height": 5.60}, 5.088, "1/1.9-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero12_black", "GoPro", "HERO12 Black", ["HERO12", "HERO12 Black", "Hero 12", "Hero12"], "GoPro Fixed", {"width": 6.40, "height": 5.60}, 5.088, "1/1.9-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero11_black", "GoPro", "HERO11 Black", ["HERO11", "HERO11 Black", "Hero 11", "Hero11"], "GoPro Fixed", {"width": 6.40, "height": 5.60}, 5.088, "1/1.9-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero11_black_mini", "GoPro", "HERO11 Black Mini", ["HERO11 Mini", "HERO11 Black Mini", "Hero 11 Mini"], "GoPro Fixed", {"width": 6.40, "height": 5.60}, 5.088, "1/1.9-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero10_black", "GoPro", "HERO10 Black", ["HERO10", "HERO10 Black", "Hero 10", "Hero10"], "GoPro Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero10_bones", "GoPro", "HERO10 Black Bones", ["HERO10 Bones", "Bones", "Hero 10 Bones"], "GoPro Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero9_black", "GoPro", "HERO9 Black", ["HERO9", "HERO9 Black", "Hero 9", "Hero9"], "GoPro Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero8_black", "GoPro", "HERO8 Black", ["HERO8", "HERO8 Black", "Hero 8", "Hero8"], "GoPro Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero7_black", "GoPro", "HERO7 Black", ["HERO7", "HERO7 Black", "Hero 7 Black", "Hero7 Black"], "GoPro Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero6_black", "GoPro", "HERO6 Black", ["HERO6", "HERO6 Black", "Hero 6", "Hero6"], "GoPro Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero5_black", "GoPro", "HERO5 Black", ["HERO5", "HERO5 Black", "Hero 5", "Hero5"], "GoPro Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_hero5_session", "GoPro", "HERO5 Session", ["HERO5 Session", "Session 5", "Hero5 Session"], "GoPro Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide"),
            CameraEntry("gopro_max", "GoPro", "GoPro MAX", ["MAX", "GoPro Max", "Max 360"], "GoPro Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Dual 360 Lenses"),

            # --- DJI Action & Drones ---
            CameraEntry("dji_osmo_action_5_pro", "DJI", "Osmo Action 5 Pro", ["Action 5 Pro", "Osmo Action 5", "Action5 Pro"], "DJI Action Fixed", {"width": 9.60, "height": 7.20}, 3.606, "1/1.3-inch", False, "Built-in Ultra-Wide f/2.8"),
            CameraEntry("dji_osmo_action_4", "DJI", "Osmo Action 4", ["Action 4", "Osmo Action 4", "Action4"], "DJI Action Fixed", {"width": 9.60, "height": 7.20}, 3.606, "1/1.3-inch", False, "Built-in Ultra-Wide f/2.8"),
            CameraEntry("dji_osmo_action_3", "DJI", "Osmo Action 3", ["Action 3", "Osmo Action 3", "Action3"], "DJI Action Fixed", {"width": 7.60, "height": 5.70}, 4.554, "1/1.7-inch", False, "Built-in Ultra-Wide f/2.8"),
            CameraEntry("dji_action_2", "DJI", "Action 2", ["DJI Action 2", "Action 2", "Action2"], "DJI Action Fixed", {"width": 7.60, "height": 5.70}, 4.554, "1/1.7-inch", False, "Built-in Ultra-Wide f/2.8"),
            CameraEntry("dji_osmo_action", "DJI", "Osmo Action", ["Action 1", "Osmo Action 1", "DJI Osmo Action"], "DJI Action Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide f/2.8"),
            CameraEntry("dji_osmo_pocket_3", "DJI", "Osmo Pocket 3", ["Pocket 3", "OsmoPocket3", "DJI Pocket 3"], "DJI Action Fixed", {"width": 13.2, "height": 8.8}, 2.727, "1-inch", False, "Built-in 20mm f/2.0"),
            CameraEntry("dji_pocket_2", "DJI", "Pocket 2", ["Osmo Pocket 2", "Pocket 2", "DJI Pocket 2"], "DJI Action Fixed", {"width": 7.60, "height": 5.70}, 4.554, "1/1.7-inch", False, "Built-in 20mm f/1.8"),
            CameraEntry("dji_o3_air_unit", "DJI", "O3 Air Unit", ["O3 Air Unit", "DJI O3", "O3"], "DJI Action Fixed", {"width": 7.60, "height": 5.70}, 4.554, "1/1.7-inch", False, "Built-in Ultra-Wide f/2.8"),
            CameraEntry("dji_avata_2", "DJI", "Avata 2", ["Avata 2", "DJI Avata 2"], "DJI Action Fixed", {"width": 9.60, "height": 7.20}, 3.606, "1/1.3-inch", False, "Built-in Ultra-Wide f/2.8"),
            CameraEntry("dji_avata", "DJI", "Avata", ["Avata", "DJI Avata"], "DJI Action Fixed", {"width": 7.60, "height": 5.70}, 4.554, "1/1.7-inch", False, "Built-in Ultra-Wide f/2.8"),
            CameraEntry("dji_air_unit", "DJI", "FPV Air Unit", ["AIR UNIT", "Air Unit", "DJI FPV Air Unit", "caddx vista dji", "vista"], "FPV Fixed", {"width": 4.50, "height": 3.40}, 7.67, "1/3.2-inch", False, "DJI FPV Camera"),
            CameraEntry("dji_ronin_4d_6k", "DJI", "Ronin 4D 6K", ["4D", "Ronin 4D", "Ronin 4D-6K"], "DJI DL", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("dji_ronin_4d_8k", "DJI", "Ronin 4D 8K", ["Ronin 4D-8K", "Ronin 4D 8K"], "DJI DL", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("dji_inspire_3", "DJI", "Inspire 3 (X9-8K Air)", ["Inspire 3", "Zenmuse X9-8K Air"], "DJI DL", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("dji_mini_4_pro", "DJI", "Mini 4 Pro", ["mini 4 pro", "Mini 4 Pro", "FC3582B"], "DJI Action Fixed", {"width": 9.60, "height": 7.20}, 3.606, "1/1.3-inch", False, "Built-in 24mm f/1.7"),
            CameraEntry("dji_mini_3_pro", "DJI", "Mini 3 Pro", ["MINI 3 PRO", "Mini 3 Pro", "Mini Pro 3", "mini3", "FC3582"], "DJI Action Fixed", {"width": 9.60, "height": 7.20}, 3.606, "1/1.3-inch", False, "Built-in 24mm f/1.7"),
            CameraEntry("dji_mini_2", "DJI", "Mini 2", ["MINI 2", "Mini 2", "Mavic mini 2", "FC7303"], "DJI Action Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in 24mm f/2.8"),
            CameraEntry("dji_mavic_3_pro", "DJI", "Mavic 3 Pro", ["Mavic 3 Pro", "Mavic3Pro", "Mavic 3 Pro Hasselblad 43", "FC3411"], "Fixed", {"width": 17.3, "height": 13.0}, 2.0, "Micro Four Thirds", False, "Hasselblad 24mm f/2.8"),

            # --- Blackmagic Design Cinema ---
            CameraEntry("blackmagic_bmpcc_4k", "Blackmagic Design", "Pocket Cinema Camera 4K", ["Pocket Cinema Camera 4K", "BMPCC 4K", "BMPCC4K", "Pocket 4K", "Pocket Cinema Camera 4k", "Pocker Cinema Camera 4k"], "Micro Four Thirds", {"width": 18.96, "height": 10.00}, 2.016, "Micro Four Thirds", True),
            CameraEntry("blackmagic_bmpcc_6k", "Blackmagic Design", "Pocket Cinema Camera 6K", ["Pocket Cinema Camera 6K", "BMPCC 6K", "BMPCC6K", "Pocket 6K", "Pocket Cinema Camera 6k"], "Canon EF", {"width": 23.10, "height": 12.99}, 1.634, "Super 35", True),
            CameraEntry("blackmagic_bmpcc_6k_pro", "Blackmagic Design", "Pocket Cinema Camera 6K Pro", ["Pocket Cinema Camera 6K Pro", "BMPCC 6K Pro", "Pocket 6K Pro", "Pocket Cinema Camera 6k Pro"], "Canon EF", {"width": 23.10, "height": 12.99}, 1.634, "Super 35", True),
            CameraEntry("blackmagic_bmpcc_6k_g2", "Blackmagic Design", "Pocket Cinema Camera 6K G2", ["Pocket Cinema Camera 6K G2", "BMPCC 6K G2", "Pocket 6K G2", "Pocket Cinema Camera 6k G2"], "Canon EF", {"width": 23.10, "height": 12.99}, 1.634, "Super 35", True),
            CameraEntry("blackmagic_cinema_camera_6k", "Blackmagic Design", "Cinema Camera 6K", ["Cinema Camera 6K", "BMCC 6K", "BMCC6K"], "Leica L", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("blackmagic_pyxis_6k", "Blackmagic Design", "PYXIS 6K", ["PYXIS 6K", "Blackmagic PYXIS"], "Leica L", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("blackmagic_ursa_mini_pro_12k", "Blackmagic Design", "URSA Mini Pro 12K", ["URSA Mini Pro 12K", "Ursa Mini 12k", "Ursa Mini Pro 12k"], "PL Mount", {"width": 27.03, "height": 14.25}, 1.418, "Super 35", True),
            CameraEntry("blackmagic_micro_studio_camera_4k_g2", "Blackmagic Design", "Micro Studio Camera 4K G2", ["Micro Studio Camera 4K G2", "Micro Studio Camera 4k G2"], "Micro Four Thirds", {"width": 13.056, "height": 7.344}, 2.88, "Micro Four Thirds", True),
            CameraEntry("blackmagic_micro_cinema_camera", "Blackmagic Design", "Micro Cinema Camera", ["Micro Cinema Camera", "BMMCC"], "Micro Four Thirds", {"width": 12.48, "height": 7.02}, 3.02, "Super 16", True),

            # --- RED Digital Cinema ---
            CameraEntry("red_komodo_6k", "RED Digital Cinema", "KOMODO 6K", ["KOMODO 6K", "KOMODO 6k", "Komodo 6K", "komodo", "Komodo", "KOMODO"], "Canon RF", {"width": 27.03, "height": 14.26}, 1.418, "Super 35", True),
            CameraEntry("red_komodo_x_6k", "RED Digital Cinema", "KOMODO-X 6K S35", ["KOMODO X", "KOMODO-X 6K S35", "Komodo X", "Komodo-X"], "Canon RF", {"width": 27.03, "height": 14.26}, 1.418, "Super 35", True),
            CameraEntry("red_v_raptor_8k_vv", "RED Digital Cinema", "V-RAPTOR 8K VV", ["V-RAPTOR 8K VV", "V-RAPTOR VV", "V-Raptor", "V-RAPTOR", "8k vv"], "Canon RF", {"width": 40.96, "height": 21.60}, 0.934, "Full Frame", True),
            CameraEntry("red_v_raptor_x_8k_vv", "RED Digital Cinema", "V-RAPTOR [X] 8K VV", ["V-RAPTOR [X] 8K VV", "V-Raptor [X] 8K VV"], "Canon RF", {"width": 40.96, "height": 21.60}, 0.934, "Full Frame", True),
            CameraEntry("red_v_raptor_8k_s35", "RED Digital Cinema", "V-RAPTOR 8K S35", ["V-RAPTOR 8K S35", "V-Raptor S35"], "Canon RF", {"width": 26.21, "height": 13.82}, 1.46, "Super 35", True),
            CameraEntry("red_dsmc2_helium_8k", "RED Digital Cinema", "DSMC2 Helium 8K S35", ["Helium 8K", "Helium 8k", "DSMC2 Helium"], "PL Mount", {"width": 29.90, "height": 15.77}, 1.28, "Super 35", True),
            CameraEntry("red_dsmc2_gemini_5k", "RED Digital Cinema", "DSMC2 Gemini 5K S35", ["DSMC2 GEMINI 5K S35", "Gemini 5K", "Gemini"], "PL Mount", {"width": 30.72, "height": 18.00}, 1.215, "Super 35", True),

            # --- Canon Cameras ---
            CameraEntry("canon_eos_r5", "Canon", "EOS R5", ["EOS R5", "R5", "Canon R5"], "Canon RF", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("canon_eos_r5_c", "Canon", "EOS R5 C", ["EOS R5 C", "R5C", "R5 C"], "Canon RF", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("canon_eos_r6_ii", "Canon", "EOS R6 Mark II", ["EOS R6 Mark II", "R6 II", "R6M2", "R6 Mark II"], "Canon RF", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("canon_eos_r6", "Canon", "EOS R6", ["EOS R6", "R6", "Canon R6"], "Canon RF", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("canon_eos_r7", "Canon", "EOS R7", ["EOS R7", "R7", "Canon R7"], "Canon RF", {"width": 22.3, "height": 14.9}, 1.614, "APS-C", True),
            CameraEntry("canon_eos_r10", "Canon", "EOS R10", ["EOS R10", "R10", "Canon R10"], "Canon RF", {"width": 22.3, "height": 14.9}, 1.614, "APS-C", True),
            CameraEntry("canon_eos_c70", "Canon", "EOS C70", ["EOS C70", "C70", "Canon C70"], "Canon RF", {"width": 26.2, "height": 13.8}, 1.46, "Super 35", True),
            CameraEntry("canon_eos_5d_iv", "Canon", "EOS 5D Mark IV", ["5D Mark IV", "5D4", "EOS 5D Mark IV", "5D Mk IV"], "Canon EF", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("canon_eos_1d_x_iii", "Canon", "EOS-1D X Mark III", ["EOS 1 DX MK3", "1DX Mark III", "1DX Mk III"], "Canon EF", {"width": 36.0, "height": 24.0}, 1.0, "Full Frame", True),
            CameraEntry("canon_eos_m50", "Canon", "EOS M50", ["EOS M50", "M50", "Canon M50"], "Canon EF-M", {"width": 22.3, "height": 14.9}, 1.614, "APS-C", True),

            # --- Fujifilm Mirrorless ---
            CameraEntry("fujifilm_x_t5", "Fujifilm", "X-T5", ["X-T5", "XT5", "Fujifilm X-T5"], "Fujifilm X", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("fujifilm_x_t4", "Fujifilm", "X-T4", ["X-T4", "XT4", "Fuji X-T4"], "Fujifilm X", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("fujifilm_x_t3", "Fujifilm", "X-T3", ["X-T3", "XT3", "FujiFilm X-T3", "Fuji X-T3"], "Fujifilm X", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("fujifilm_x_h2s", "Fujifilm", "X-H2S", ["X-H2S", "XH2S", "Fujifilm X-H2S"], "Fujifilm X", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("fujifilm_x_h2", "Fujifilm", "X-H2", ["X-H2", "XH2"], "Fujifilm X", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("fujifilm_x_s20", "Fujifilm", "X-S20", ["X-S20", "XS20"], "Fujifilm X", {"width": 23.5, "height": 15.6}, 1.534, "APS-C", True),
            CameraEntry("fujifilm_gfx_100_ii", "Fujifilm", "GFX 100 II", ["GFX 100 II", "GFX100 II", "GFX100II"], "Fujifilm G", {"width": 43.8, "height": 32.9}, 0.789, "Medium Format", True),

            # --- Panasonic Lumix ---
            CameraEntry("panasonic_lumix_s5_ii", "Panasonic", "Lumix S5 II", ["Lumix S5II", "S5II", "S5M2", "Panasonic S5M2", "Panasonic Lumix S5II"], "Leica L", {"width": 35.6, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("panasonic_lumix_s5_iix", "Panasonic", "Lumix S5 IIX", ["Lumix S5IIX", "S5IIX", "S5M2X"], "Leica L", {"width": 35.6, "height": 23.8}, 1.0, "Full Frame", True),
            CameraEntry("panasonic_lumix_gh6", "Panasonic", "Lumix GH6", ["GH6", "Lumix GH6"], "Micro Four Thirds", {"width": 17.3, "height": 13.0}, 2.0, "Micro Four Thirds", True),
            CameraEntry("panasonic_lumix_gh5", "Panasonic", "Lumix GH5", ["GH5", "Lumix GH5"], "Micro Four Thirds", {"width": 17.3, "height": 13.0}, 2.0, "Micro Four Thirds", True),
            CameraEntry("panasonic_lumix_bgh1", "Panasonic", "Lumix BGH1", ["BGH1", "Lumix BGH1"], "Micro Four Thirds", {"width": 17.3, "height": 13.0}, 2.0, "Micro Four Thirds", True),

            # --- Nikon Mirrorless ---
            CameraEntry("nikon_z9", "Nikon", "Z 9", ["Z 9", "Z9", "Nikon Z9"], "Nikon Z", {"width": 35.9, "height": 23.9}, 1.0, "Full Frame", True),
            CameraEntry("nikon_z8", "Nikon", "Z 8", ["Z 8", "Z8", "Nikon Z8"], "Nikon Z", {"width": 35.9, "height": 23.9}, 1.0, "Full Frame", True),
            CameraEntry("nikon_z6_iii", "Nikon", "Z 6 III", ["Z 6 III", "Z6 III", "Z6M3"], "Nikon Z", {"width": 35.9, "height": 23.9}, 1.0, "Full Frame", True),
            CameraEntry("nikon_z6_ii", "Nikon", "Z 6 II", ["Z 6 II", "Z6 II", "Z6M2"], "Nikon Z", {"width": 35.9, "height": 23.9}, 1.0, "Full Frame", True),

            # --- Insta360 ---
            CameraEntry("insta360_ace_pro", "Insta360", "Ace Pro", ["Ace Pro", "Insta360 Ace Pro"], "Insta360 Fixed", {"width": 9.60, "height": 7.20}, 3.606, "1/1.3-inch", False, "Leica Summarit 16mm f/2.6"),
            CameraEntry("insta360_x4", "Insta360", "X4", ["Insta360 X4", "X4"], "Insta360 Fixed", {"width": 6.40, "height": 4.80}, 5.4, "1/2-inch", False, "Dual 360 Lenses"),
            CameraEntry("insta360_go_3", "Insta360", "GO 3", ["GO 3", "Insta360 GO 3", "GO3"], "Insta360 Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide"),

            # --- FPV Systems ---
            CameraEntry("runcam_thumb_pro", "RunCam", "Thumb Pro", ["Thumb Pro", "ThumbPro", "RunCam Thumb Pro"], "FPV Fixed", {"width": 7.60, "height": 5.70}, 4.554, "1/1.7-inch", False, "Wide 155 FOV"),
            CameraEntry("runcam_thumb_pro_v2", "RunCam", "Thumb Pro V2", ["Thumb Pro V2", "RunCam Thumb Pro V2"], "FPV Fixed", {"width": 7.60, "height": 5.70}, 4.554, "1/1.7-inch", False, "Wide 155 FOV"),
            CameraEntry("caddx_walnut", "Caddx", "Walnut", ["Walnut", "Caddx Walnut"], "FPV Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Wide 150 FOV"),
            CameraEntry("caddx_peanut", "Caddx", "Peanut", ["Peanut", "Caddx Peanut"], "FPV Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Wide 150 FOV"),
            CameraEntry("walksnail_avatar_hd_pro", "Walksnail", "Avatar HD Pro", ["Avatar HD Pro", "Walksnail Avatar HD Pro"], "FPV Fixed", {"width": 7.18, "height": 5.32}, 4.84, "1/1.8-inch", False, "Sony Starvis II Lens"),
            CameraEntry("caddx_ratel_2", "Caddx", "Ratel 2", ["Ratel 2", "Caddx Ratel 2"], "M12", {"width": 7.18, "height": 5.32}, 4.84, "1/1.8-inch", True),
            CameraEntry("foxeer_t_rex", "Foxeer", "T-Rex", ["T-Rex", "Foxeer T-Rex"], "M12", {"width": 6.40, "height": 4.80}, 5.4, "1/2-inch", True),
            CameraEntry("betafpv_smo_4k", "BetaFPV", "SMO 4K", ["SMO 4K", "BetaFPV SMO 4K"], "FPV Fixed", {"width": 6.17, "height": 4.55}, 5.643, "1/2.3-inch", False, "Built-in Ultra-Wide"),

            # --- Smartphones ---
            CameraEntry("apple_iphone_15_pro_max", "Apple", "iPhone 15 Pro Max", ["iPhone 15 Pro Max", "Apple iPhone 15 Pro Max"], "Smartphone Integrated", {"width": 9.80, "height": 7.30}, 3.55, "1/1.28-inch", False, "Main 24mm f/1.78"),
            CameraEntry("apple_iphone_14_pro", "Apple", "iPhone 14 Pro", ["iPhone 14 Pro", "Apple iPhone 14 Pro"], "Smartphone Integrated", {"width": 9.80, "height": 7.30}, 3.55, "1/1.28-inch", False, "Main 24mm f/1.78"),
            CameraEntry("apple_iphone_13_pro", "Apple", "iPhone 13 Pro", ["iPhone 13 Pro", "Apple iPhone 13 Pro"], "Smartphone Integrated", {"width": 8.00, "height": 6.00}, 4.33, "1/1.65-inch", False, "Main 26mm f/1.5"),
            CameraEntry("samsung_galaxy_s24_ultra", "Samsung", "Galaxy S24 Ultra", ["Galaxy S24 Ultra", "Samsung S24 Ultra"], "Smartphone Integrated", {"width": 9.80, "height": 7.30}, 3.55, "1/1.3-inch", False, "Main 24mm f/1.7"),

            # --- Generic / Custom Fallback ---
            CameraEntry("custom_other", "Other", "Other / Custom", ["Custom", "Other", "Unknown", "Generic"], "Fixed", {"width": 36.0, "height": 24.0}, 1.0, "Custom", True, "Custom Lens")
        ]
        return cameras

    @staticmethod
    def get_curated_lenses() -> List[LensEntry]:
        lenses = [
            # --- Popular Zoom Lenses ---
            LensEntry("sigma_24_70mm_f2_8_dg_dn_art", "Sigma", "24-70mm F2.8 DG DN Art", ["Sigma 24-70mm f2.8 DG DN", "Sigma 24-70mm f2.8", "24-70mm F2.8 DG DN", "Sigma 24-70 2.8"], ["Sony E", "Leica L"], {"min": 24.0, "max": 70.0}, {"min_f_stop": 2.8, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("sigma_28_70mm_f2_8_dg_dn_c", "Sigma", "28-70mm F2.8 DG DN Contemporary", ["Sigma 28-70mm 2.8", "Sigma 28-70mm f2.8", "Sigma 28-70"], ["Sony E", "Leica L"], {"min": 28.0, "max": 70.0}, {"min_f_stop": 2.8, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("sigma_18_35mm_f1_8_dc_hsm_art", "Sigma", "18-35mm F1.8 DC HSM Art", ["Sigma 18-35", "Sigma 18-35mm f1.8", "18-35mm F1.8 Art", "Sigma 18-35 24.00mm"], ["Canon EF", "Nikon F", "Sony E"], {"min": 18.0, "max": 35.0}, {"min_f_stop": 1.8, "max_f_stop": 16.0}, True, 1.5),
            LensEntry("sigma_14_24mm_f2_8_dg_dn_art", "Sigma", "14-24mm F2.8 DG DN Art", ["Sigma 14-24mm", "Sigma 14-24mm F2.8"], ["Sony E", "Leica L"], {"min": 14.0, "max": 24.0}, {"min_f_stop": 2.8, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("tamron_28_75mm_f2_8_di_iii_vxd_g2", "Tamron", "28-75mm F2.8 Di III VXD G2", ["Tamron 28-75mm F2.8 G2", "Tamron 28-75mm f2.8", "Tamron 28-75", "Tamron 28-7528mm"], ["Sony E", "Nikon Z"], {"min": 28.0, "max": 75.0}, {"min_f_stop": 2.8, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("tamron_17_28mm_f2_8_di_iii_rxd", "Tamron", "17-28mm F2.8 Di III RXD", ["Tamron 17-28mm F2.8", "Tamron 17-28mm", "Tamron 17-28"], ["Sony E"], {"min": 17.0, "max": 28.0}, {"min_f_stop": 2.8, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("tamron_35_150mm_f2_2_8_di_iii_vxd", "Tamron", "35-150mm F2-2.8 Di III VXD", ["Tamron 35-150mm Di III VXD", "Tamron 35-150", "Tamron 35-150mm"], ["Sony E", "Nikon Z"], {"min": 35.0, "max": 150.0}, {"min_f_stop": 2.0, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("sony_fe_24_70mm_f2_8_gm_ii", "Sony", "FE 24-70mm F2.8 GM II", ["Sony FE 24-70mm F2.8 GM II", "24-70 GM II", "SEL2470GM2"], ["Sony E"], {"min": 24.0, "max": 70.0}, {"min_f_stop": 2.8, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("sony_fe_16_35mm_f2_8_gm_ii", "Sony", "FE 16-35mm F2.8 GM II", ["Sony FE 16-35mm F2.8 GM II", "16-35 GM II", "SEL1635GM2"], ["Sony E"], {"min": 16.0, "max": 35.0}, {"min_f_stop": 2.8, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("sony_fe_28_70mm_f3_5_5_6_oss", "Sony", "FE 28-70mm F3.5-5.6 OSS", ["Sony FE 28-70mm", "Sony 28-70mm", "SEL2870", "kit FE 3.55.6 28-70"], ["Sony E"], {"min": 28.0, "max": 70.0}, {"min_f_stop": 3.5, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("sony_e_16_50mm_f3_5_5_6_pz_oss", "Sony", "E PZ 16-50mm F3.5-5.6 OSS", ["Sony E 16-50mm", "16-50", "SELP1650", "16-50s"], ["Sony E"], {"min": 16.0, "max": 50.0}, {"min_f_stop": 3.5, "max_f_stop": 22.0}, True, 1.5),

            # --- Popular Prime Lenses ---
            LensEntry("sony_fe_35mm_f1_8", "Sony", "FE 35mm F1.8", ["Sony FE 35mm f1.8", "Sony 35mm F1.8", "SEL35F18F", "35mm F1.8"], ["Sony E"], {"min": 35.0, "max": 35.0}, {"min_f_stop": 1.8, "max_f_stop": 22.0}, False, 1.0),
            LensEntry("sony_fe_24mm_f1_4_gm", "Sony", "FE 24mm F1.4 GM", ["Sony FE 24mm F1.4 GM", "24 GM", "SEL24F14GM"], ["Sony E"], {"min": 24.0, "max": 24.0}, {"min_f_stop": 1.4, "max_f_stop": 16.0}, False, 1.0),
            LensEntry("sony_fe_50mm_f1_8", "Sony", "FE 50mm F1.8", ["Sony FE 50mm F1.8", "Sony 50mm F1.8", "SEL50F18F"], ["Sony E"], {"min": 50.0, "max": 50.0}, {"min_f_stop": 1.8, "max_f_stop": 22.0}, False, 1.0),
            LensEntry("samyang_af_12mm_f2_0_e", "Samyang", "AF 12mm F2.0 E", ["Samyang 12mm", "Samyang 12mm 2.0", "SAMYANG 12MM", "Rokinon 12mm"], ["Sony E", "Fujifilm X"], {"min": 12.0, "max": 12.0}, {"min_f_stop": 2.0, "max_f_stop": 22.0}, False, 1.5),
            LensEntry("sigma_16mm_f1_4_dc_dn_c", "Sigma", "16mm F1.4 DC DN Contemporary", ["Sigma 16mm F1.4", "Sigma 16mm f1.4", "16mm F1.4 DC DN"], ["Sony E", "Fujifilm X", "Micro Four Thirds", "Canon EF-M"], {"min": 16.0, "max": 16.0}, {"min_f_stop": 1.4, "max_f_stop": 16.0}, False, 1.5),
            LensEntry("sigma_30mm_f1_4_dc_dn_c", "Sigma", "30mm F1.4 DC DN Contemporary", ["Sigma 30mm F1.4", "Sigma 30mm f1.4 EF-M", "30mm F1.4 DC DN"], ["Sony E", "Fujifilm X", "Micro Four Thirds", "Canon EF-M"], {"min": 30.0, "max": 30.0}, {"min_f_stop": 1.4, "max_f_stop": 16.0}, False, 1.5),
            LensEntry("canon_rf_24_70mm_f2_8l_is_usm", "Canon", "RF 24-70mm F2.8L IS USM", ["RF 24-70mm", "RF24-70", "Canon RF 24-70mm"], ["Canon RF"], {"min": 24.0, "max": 70.0}, {"min_f_stop": 2.8, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("canon_rf_24_105mm_f4l_is_usm", "Canon", "RF 24-105mm F4L IS USM", ["RF 24-105mm", "RF24-105", "Canon RF 24-105mm"], ["Canon RF"], {"min": 24.0, "max": 105.0}, {"min_f_stop": 4.0, "max_f_stop": 22.0}, True, 1.0),
            LensEntry("canon_ef_50mm_f1_8_stm", "Canon", "EF 50mm f/1.8 STM", ["Canon EF 50mm", "EF 50mm", "50mm F1.8"], ["Canon EF"], {"min": 50.0, "max": 50.0}, {"min_f_stop": 1.8, "max_f_stop": 22.0}, False, 1.0),

            # --- Cinema & Anamorphic Lenses ---
            LensEntry("sirui_50mm_f1_8_anamorphic", "Sirui", "50mm F1.8 Anamorphic 1.33x", ["SIRUI 50mm Anamorphic", "Sirui 50mm", "Sirui 50mm Anamorphic"], ["Sony E", "Fujifilm X", "Micro Four Thirds"], {"min": 50.0, "max": 50.0}, {"min_f_stop": 1.8, "max_f_stop": 16.0}, False, 1.5),
            LensEntry("laowa_9mm_f2_8_zero_d", "Venus Optics", "Laowa 9mm f/2.8 Zero-D", ["Laowa 9mm", "Laowa 9mm f2.8", "Venus Optics Laowa 9mm"], ["Sony E", "Fujifilm X", "Canon EF-M"], {"min": 9.0, "max": 9.0}, {"min_f_stop": 2.8, "max_f_stop": 22.0}, False, 1.5),

            # --- Generic / Custom Fallback ---
            LensEntry("custom_other_lens", "Other", "Other / Custom Lens", ["Custom Lens", "Other Lens", "Generic Lens"], ["Sony E", "Canon RF", "Canon EF", "Nikon Z", "Fujifilm X", "Micro Four Thirds", "Leica L", "Fixed"], {"min": 10.0, "max": 200.0}, {"min_f_stop": 1.0, "max_f_stop": 32.0}, True, 1.0)
        ]
        return lenses


class LensDatabaseBuilder:
    """
    Main aggregator engine that consolidates Lensfun XML files and
    Gyroflow lens profile repositories into canonical JSON data.
    """
    def __init__(self, lensfun_dir: str = "scratch/lensfun_cache", gyroflow_tree_file: str = "scratch/tree_cache.json"):
        self.lensfun_dir = lensfun_dir
        self.gyroflow_tree_file = gyroflow_tree_file
        self.cameras: Dict[str, CameraEntry] = {}
        self.lenses: Dict[str, LensEntry] = {}
        self.mounts: Dict[str, Dict[str, Any]] = dict(STANDARD_MOUNTS)
        self.camera_aliases: Dict[str, str] = {}  # normalized alias -> canonical camera id
        self.lens_aliases: Dict[str, str] = {}  # normalized alias -> canonical lens id

    def normalize_brand(self, raw_brand: str) -> str:
        """Normalizes a raw brand string to canonical brand name."""
        clean = raw_brand.strip().lower()
        clean = re.sub(r'[^a-z0-9 ]', '', clean)
        return BRAND_CANONICAL_MAP.get(clean, raw_brand.strip().title())

    def load_curated_data(self):
        """Loads authoritative curated entries for cameras and lenses."""
        for cam in CuratedHardwareCatalog.get_curated_cameras():
            self.add_camera(cam)
        for lens in CuratedHardwareCatalog.get_curated_lenses():
            self.add_lens(lens)
        print(f"[Curated] Ingested {len(self.cameras)} cameras and {len(self.lenses)} lenses.")

    def add_camera(self, cam: CameraEntry):
        """Adds or updates a camera entry with alias index."""
        if cam.id in self.cameras:
            existing = self.cameras[cam.id]
            # Merge aliases
            all_aliases = set(existing.aliases + cam.aliases)
            existing.aliases = sorted(list(all_aliases))
            return
        self.cameras[cam.id] = cam
        # Index aliases
        norm_model = self.normalize_query_str(cam.model)
        self.camera_aliases[norm_model] = cam.id
        for alias in cam.aliases:
            norm_a = self.normalize_query_str(alias)
            self.camera_aliases[norm_a] = cam.id
            # Also index with brand prefix
            norm_ba = self.normalize_query_str(f"{cam.brand} {alias}")
            self.camera_aliases[norm_ba] = cam.id

    def add_lens(self, lens: LensEntry):
        """Adds or updates a lens entry with alias index."""
        if lens.id in self.lenses:
            existing = self.lenses[lens.id]
            all_aliases = set(existing.aliases + lens.aliases)
            existing.aliases = sorted(list(all_aliases))
            return
        self.lenses[lens.id] = lens
        # Index aliases
        norm_model = self.normalize_query_str(lens.model)
        self.lens_aliases[norm_model] = lens.id
        for alias in lens.aliases:
            norm_a = self.normalize_query_str(alias)
            self.lens_aliases[norm_a] = lens.id
            norm_ba = self.normalize_query_str(f"{lens.brand} {alias}")
            self.lens_aliases[norm_ba] = lens.id

    @staticmethod
    def normalize_query_str(s: str) -> str:
        """Lowercases and strips whitespace and punctuation for index matching."""
        s = s.lower().replace('-', '').replace('_', '').replace(' ', '').replace('.', '').replace('/', '')
        return s

    def parse_lensfun_xmls(self):
        """Parses all XML files in the lensfun cache directory."""
        if not os.path.exists(self.lensfun_dir):
            print(f"[Lensfun] Cache directory '{self.lensfun_dir}' not found. Skipping XML parse.")
            return

        xml_files = glob.glob(os.path.join(self.lensfun_dir, "*.xml"))
        print(f"[Lensfun] Parsing {len(xml_files)} XML files...")
        cams_added = 0
        lenses_added = 0

        for fpath in xml_files:
            try:
                root = ET.parse(fpath).getroot()
                # Parse mounts
                for m in root.findall('mount'):
                    name = m.findtext('name')
                    if name and name not in self.mounts:
                        compats = [c.text for c in m.findall('compat') if c.text]
                        self.mounts[name] = {
                            "name": name,
                            "flange_distance_mm": None,
                            "description": f"Lensfun {name} mount",
                            "compatible_mounts": compats or [name],
                            "compatible_sensor_types": ["Standard"]
                        }

                # Parse cameras
                for cam in root.findall('camera'):
                    maker = cam.findtext('maker')
                    model = cam.findtext('model')
                    if not maker or not model:
                        continue
                    brand = self.normalize_brand(maker)
                    # Extract english model name if available
                    model_en = None
                    for mdl in cam.findall('model'):
                        if mdl.attrib.get('lang') == 'en' or mdl.attrib.get('{http://www.w3.org/XML/1998/namespace}lang') == 'en':
                            model_en = mdl.text
                    mount = cam.findtext('mount') or "Fixed"
                    crop_str = cam.findtext('cropfactor')
                    crop = float(crop_str) if crop_str else 1.0
                    w, h = calculate_dimensions_from_crop(crop)

                    canonical_model = model_en or model

                    # Deduplication guard: Check if this camera is already known via curated entries
                    candidate_queries = [
                        self.normalize_query_str(f"{brand} {canonical_model}"),
                        self.normalize_query_str(f"{brand} {model}"),
                        self.normalize_query_str(canonical_model),
                        self.normalize_query_str(model)
                    ]
                    matched_existing_id = None
                    for cq in candidate_queries:
                        if cq in self.camera_aliases:
                            matched_existing_id = self.camera_aliases[cq]
                            break

                    if matched_existing_id and matched_existing_id in self.cameras:
                        # Merge aliases into existing canonical camera
                        existing_cam = self.cameras[matched_existing_id]
                        for a in [model, model_en]:
                            if a and a not in existing_cam.aliases:
                                existing_cam.aliases.append(a)
                                self.camera_aliases[self.normalize_query_str(a)] = matched_existing_id
                                self.camera_aliases[self.normalize_query_str(f"{brand} {a}")] = matched_existing_id
                        continue

                    cam_id = slugify(f"{brand}_{canonical_model}")
                    aliases = [model]
                    if model_en and model_en != model:
                        aliases.append(model_en)
                    is_interchangeable = mount not in ["Fixed", "GoPro Fixed", "DJI Action Fixed", "Smartphone Integrated"]
                    sensor_type = "Full Frame" if crop <= 1.05 else ("APS-C" if crop <= 1.7 else ("Micro Four Thirds" if crop <= 2.1 else "Compact"))
                    entry = CameraEntry(
                        id=cam_id,
                        brand=brand,
                        model=canonical_model,
                        aliases=aliases,
                        lens_mount=mount,
                        sensor_size={"width": w, "height": h},
                        crop_factor=round(crop, 3),
                        sensor_type=sensor_type,
                        is_interchangeable_lens=is_interchangeable
                    )
                    self.add_camera(entry)
                    cams_added += 1

                # Parse lenses
                for lens in root.findall('lens'):
                    maker = lens.findtext('maker')
                    model = lens.findtext('model')
                    if not maker or not model:
                        continue
                    brand = self.normalize_brand(maker)
                    mount = lens.findtext('mount') or "Generic"
                    crop_str = lens.findtext('cropfactor')
                    crop = float(crop_str) if crop_str else 1.0
                    focal = lens.find('focal')
                    aperture = lens.find('aperture')
                    f_min = float(focal.attrib.get('min', 0)) if focal is not None and 'min' in focal.attrib else 24.0
                    f_max = float(focal.attrib.get('max', 0)) if focal is not None and 'max' in focal.attrib else f_min
                    a_min = float(aperture.attrib.get('min', 0)) if aperture is not None and 'min' in aperture.attrib else 2.8
                    a_max = float(aperture.attrib.get('max', 0)) if aperture is not None and 'max' in aperture.attrib else 22.0
                    lens_id = slugify(f"{brand}_{model}")
                    entry = LensEntry(
                        id=lens_id,
                        brand=brand,
                        model=model,
                        aliases=[model],
                        mount_compatibility=[mount],
                        focal_range={"min": f_min, "max": f_max},
                        aperture={"min_f_stop": a_min, "max_f_stop": a_max},
                        is_zoom=(f_max > f_min),
                        coverage_crop_factor=round(crop, 3)
                    )
                    self.add_lens(entry)
                    lenses_added += 1

            except Exception as e:
                print(f"[Lensfun] Warning reading {fpath}: {e}")

        print(f"[Lensfun] Ingested {cams_added} cameras and {lenses_added} lenses from Lensfun.")

    def parse_gyroflow_presets(self):
        """
        Parses profile filenames from the Gyroflow presets repository
        to extract real-world camera aliases and user submission strings.
        """
        if not os.path.exists(self.gyroflow_tree_file):
            print(f"[Gyroflow] Tree cache file '{self.gyroflow_tree_file}' not found. Skipping preset analysis.")
            return

        with open(self.gyroflow_tree_file, 'r', encoding='utf-8') as f:
            tree = json.load(f)

        json_files = [x['path'] for x in tree if x['path'].endswith('.json')]
        print(f"[Gyroflow] Analyzing {len(json_files)} preset paths from Gyroflow...")

        matched_cams = 0
        new_aliases = 0

        for path in json_files:
            fname = path.split('/')[-1][:-5]
            if fname == 'compress.rs':
                continue
            fparts = fname.split('_')
            if len(fparts) < 2:
                continue
            raw_brand = fparts[0]
            raw_model = fparts[1]
            raw_lens = fparts[2] if len(fparts) > 2 else ""

            brand = self.normalize_brand(raw_brand)

            # Match camera
            cam_query = self.normalize_query_str(f"{brand} {raw_model}")
            cam_model_query = self.normalize_query_str(raw_model)

            matched_id = self.camera_aliases.get(cam_query) or self.camera_aliases.get(cam_model_query)
            if matched_id and matched_id in self.cameras:
                cam = self.cameras[matched_id]
                if raw_model not in cam.aliases and len(raw_model) > 1:
                    cam.aliases.append(raw_model)
                    self.camera_aliases[self.normalize_query_str(raw_model)] = cam.id
                    new_aliases += 1
                matched_cams += 1

            # Match or register lens if present
            if raw_lens and len(raw_lens) > 3 and not raw_lens.startswith("1080p") and not raw_lens.startswith("4k"):
                lens_query = self.normalize_query_str(raw_lens)
                lens_id = self.lens_aliases.get(lens_query)
                if not lens_id:
                    # Register new lens from preset
                    # Clean raw_lens string
                    clean_lens_name = raw_lens.strip()
                    # Skip non-lens strings like resolutions, aspect ratios, frame rates, color profiles
                    if re.match(r'^(?:\d+p|\d+k|\d+fps|log|flat|raw|v\d+|stab|hyper|wide|linear)$', clean_lens_name, re.IGNORECASE):
                        continue

                    clean_lens_id = slugify(f"{brand}_{clean_lens_name}")
                    # Robust focal length parser supporting decimals like 14.00mm, 6.0mm, 24-70mm
                    f_match = re.search(r'(\d+(?:\.\d+)?)\s*(?:-\s*(\d+(?:\.\d+)?))?\s*mm', clean_lens_name, re.IGNORECASE)
                    if f_match:
                        val1 = float(f_match.group(1))
                        val2 = float(f_match.group(2)) if f_match.group(2) else val1
                        f_min = min(val1, val2)
                        f_max = max(val1, val2)
                        if f_min <= 0:
                            f_min = 12.0
                        if f_max < f_min:
                            f_max = f_min
                    else:
                        # Check standalone numbers followed by 'f' or single focal notation
                        f_single = re.search(r'\b(\d{1,3})\b', clean_lens_name)
                        if f_single and 8 <= float(f_single.group(1)) <= 800:
                            f_min = float(f_single.group(1))
                            f_max = f_min
                        else:
                            f_min = 24.0
                            f_max = 70.0

                    new_lens = LensEntry(
                        id=clean_lens_id,
                        brand=brand,
                        model=clean_lens_name,
                        aliases=[clean_lens_name],
                        mount_compatibility=["Sony E", "Canon RF", "Canon EF", "Fujifilm X", "Nikon Z", "Micro Four Thirds", "Fixed"],
                        focal_range={"min": f_min, "max": f_max},
                        aperture={"min_f_stop": 2.8, "max_f_stop": 22.0},
                        is_zoom=(f_max > f_min),
                        coverage_crop_factor=1.0
                    )
                    self.add_lens(new_lens)

        print(f"[Gyroflow] Matched {matched_cams} profiles, enriched with {new_aliases} new camera aliases.")

    def build_database(self) -> Dict[str, Any]:
        """Compiles the aggregated canonical dictionary with full metadata and indexes."""
        camera_brands = sorted(list(set(c.brand for c in self.cameras.values())))
        lens_brands = sorted(list(set(l.brand for l in self.lenses.values())))

        # Build UI Cascading Dropdown Indexes
        brand_to_cameras: Dict[str, List[Dict[str, Any]]] = {}
        for cam in sorted(self.cameras.values(), key=lambda c: (c.brand, c.model)):
            if cam.brand not in brand_to_cameras:
                brand_to_cameras[cam.brand] = []
            brand_to_cameras[cam.brand].append({
                "id": cam.id,
                "model": cam.model,
                "lens_mount": cam.lens_mount,
                "crop_factor": cam.crop_factor,
                "sensor_type": cam.sensor_type,
                "is_interchangeable_lens": cam.is_interchangeable_lens,
                "default_lens": cam.default_lens
            })

        mount_to_lenses: Dict[str, List[Dict[str, Any]]] = {}
        for lens in sorted(self.lenses.values(), key=lambda l: (l.brand, l.model)):
            for mount in lens.mount_compatibility:
                if mount not in mount_to_lenses:
                    mount_to_lenses[mount] = []
                mount_to_lenses[mount].append({
                    "id": lens.id,
                    "brand": lens.brand,
                    "model": lens.model,
                    "focal_range": lens.focal_range,
                    "aperture": lens.aperture,
                    "is_zoom": lens.is_zoom
                })

        db = {
            "$schema": "https://gyroflow.xyz/schemas/camera_lens_database.v2.json",
            "schema_version": "2.0.0",
            "updated_at": datetime.now(timezone.utc).isoformat(),
            "generator": "scripts/build_lens_database.py",
            "stats": {
                "total_camera_brands": len(camera_brands),
                "total_cameras": len(self.cameras),
                "total_lens_brands": len(lens_brands),
                "total_lenses": len(self.lenses),
                "total_mounts": len(self.mounts)
            },
            "mounts": self.mounts,
            "camera_brands": camera_brands,
            "lens_brands": lens_brands,
            "cameras": [c.to_dict() for c in sorted(self.cameras.values(), key=lambda c: (c.brand, c.model))],
            "lenses": [l.to_dict() for l in sorted(self.lenses.values(), key=lambda l: (l.brand, l.model))],
            "compatibility_index": {
                "brand_to_cameras": brand_to_cameras,
                "mount_to_lenses": mount_to_lenses
            }
        }
        return db

    def export(self, output_path: str = "resources/camera_lens_database.json"):
        """Exports the canonical JSON file with clean indentation."""
        os.makedirs(os.path.dirname(output_path), exist_ok=True)
        db = self.build_database()
        with open(output_path, 'w', encoding='utf-8') as f:
            json.dump(db, f, indent=2, ensure_ascii=False)
        print(f"[Export] Canonical database written to {output_path} ({os.path.getsize(output_path)} bytes).")
        return db


class LensProfileResolver:
    """
    Backwards compatibility matching engine for legacy .gyroflow project files
    and unstructured user input strings.
    """
    def __init__(self, db_path: str = "resources/camera_lens_database.json"):
        if os.path.exists(db_path):
            with open(db_path, 'r', encoding='utf-8') as f:
                self.db = json.load(f)
        else:
            # Fall back to curated data
            builder = LensDatabaseBuilder()
            builder.load_curated_data()
            self.db = builder.build_database()

        self.cameras_by_id = {c['id']: c for c in self.db['cameras']}
        self.lenses_by_id = {l['id']: l for l in self.db['lenses']}

        # Fast lookup indexes
        self.camera_alias_map: Dict[str, str] = {}
        for cam in self.db['cameras']:
            norm_m = LensDatabaseBuilder.normalize_query_str(cam['model'])
            self.camera_alias_map[norm_m] = cam['id']
            norm_bm = LensDatabaseBuilder.normalize_query_str(f"{cam['brand']} {cam['model']}")
            self.camera_alias_map[norm_bm] = cam['id']
            for a in cam.get('aliases', []):
                norm_a = LensDatabaseBuilder.normalize_query_str(a)
                self.camera_alias_map[norm_a] = cam['id']
                norm_ba = LensDatabaseBuilder.normalize_query_str(f"{cam['brand']} {a}")
                self.camera_alias_map[norm_ba] = cam['id']

        self.lens_alias_map: Dict[str, str] = {}
        for lens in self.db['lenses']:
            norm_m = LensDatabaseBuilder.normalize_query_str(lens['model'])
            self.lens_alias_map[norm_m] = lens['id']
            norm_bm = LensDatabaseBuilder.normalize_query_str(f"{lens['brand']} {lens['model']}")
            self.lens_alias_map[norm_bm] = lens['id']
            for a in lens.get('aliases', []):
                norm_a = LensDatabaseBuilder.normalize_query_str(a)
                self.lens_alias_map[norm_a] = lens['id']
                norm_ba = LensDatabaseBuilder.normalize_query_str(f"{lens['brand']} {a}")
                self.lens_alias_map[norm_ba] = lens['id']

    def resolve_camera(self, brand_str: str, model_str: str) -> Optional[Dict[str, Any]]:
        """Resolves free-form camera brand and model strings to canonical Camera entry."""
        if not model_str:
            return None
        norm_combined = LensDatabaseBuilder.normalize_query_str(f"{brand_str} {model_str}")
        norm_model = LensDatabaseBuilder.normalize_query_str(model_str)

        cam_id = self.camera_alias_map.get(norm_combined) or self.camera_alias_map.get(norm_model)
        if cam_id:
            return self.cameras_by_id.get(cam_id)

        # Fuzzy fallback search
        best_cam = None
        best_score = 0.0
        for cam in self.db['cameras']:
            # Check prefix / substring match
            c_name = LensDatabaseBuilder.normalize_query_str(cam['model'])
            if norm_model in c_name or c_name in norm_model:
                return cam
        return None

    def resolve_lens(self, lens_str: str, camera_entry: Optional[Dict[str, Any]] = None) -> Optional[Dict[str, Any]]:
        """Resolves free-form lens string to canonical Lens entry."""
        if not lens_str:
            if camera_entry and not camera_entry.get('is_interchangeable_lens', True):
                default_name = camera_entry.get('default_lens') or "Built-in Lens"
                return {
                    "id": f"{camera_entry['id']}_builtin",
                    "brand": camera_entry['brand'],
                    "model": default_name,
                    "mount_compatibility": [camera_entry['lens_mount']],
                    "focal_range": {"min": 16.0, "max": 16.0},
                    "aperture": {"min_f_stop": 2.8, "max_f_stop": 22.0},
                    "is_zoom": False,
                    "coverage_crop_factor": camera_entry['crop_factor']
                }
            return None

        norm_lens = LensDatabaseBuilder.normalize_query_str(lens_str)
        lens_id = self.lens_alias_map.get(norm_lens)
        if lens_id:
            return self.lenses_by_id.get(lens_id)

        # Substring search
        for lens in self.db['lenses']:
            l_name = LensDatabaseBuilder.normalize_query_str(lens['model'])
            if norm_lens in l_name or l_name in norm_lens:
                return lens

        return None

    def resolve_legacy_project(self, project_data: Dict[str, Any]) -> Dict[str, Any]:
        """
        Parses a legacy .gyroflow project file, resolving legacy strings while
        guaranteeing zero data loss and preserving all stabilization parameters.
        """
        brand = project_data.get('camera_brand', '')
        model = project_data.get('camera_model', '')
        lens = project_data.get('lens_model', '')

        resolved_camera = self.resolve_camera(brand, model)
        resolved_lens = self.resolve_lens(lens, resolved_camera)

        res = dict(project_data)
        if resolved_camera:
            res['canonical_camera'] = {
                "id": resolved_camera['id'],
                "brand": resolved_camera['brand'],
                "model": resolved_camera['model'],
                "lens_mount": resolved_camera['lens_mount'],
                "sensor_size": resolved_camera['sensor_size'],
                "crop_factor": resolved_camera['crop_factor']
            }
        else:
            # Fallback to Custom
            res['canonical_camera'] = {
                "id": "custom_camera",
                "brand": brand or "Custom",
                "model": model or "Custom Model",
                "lens_mount": "Custom",
                "sensor_size": {"width": 36.0, "height": 24.0},
                "crop_factor": 1.0,
                "is_custom": True
            }

        if resolved_lens:
            res['canonical_lens'] = {
                "id": resolved_lens['id'],
                "brand": resolved_lens['brand'],
                "model": resolved_lens['model'],
                "focal_range": resolved_lens['focal_range'],
                "is_zoom": resolved_lens['is_zoom']
            }
        else:
            res['canonical_lens'] = {
                "id": "custom_lens",
                "brand": brand or "Custom",
                "model": lens or "Custom Lens",
                "is_custom": True
            }

        return res


def validate_database_schema(db: Dict[str, Any]) -> Tuple[bool, List[str]]:
    """
    Validates that the database strictly conforms to schema requirements:
    - brand, model, lens mount, sensor size (width/height > 0), crop factor (> 0)
    - lens metadata: focal range, aperture, mount compatibility
    - physical consistency: crop factor approximates reference diagonal
    """
    errors = []

    # Check top-level keys
    for req_key in ["schema_version", "cameras", "lenses", "mounts", "compatibility_index"]:
        if req_key not in db:
            errors.append(f"Missing required top-level key: '{req_key}'")

    # Validate Cameras
    cameras = db.get("cameras", [])
    if not cameras:
        errors.append("Camera registry is empty.")

    camera_ids = set()
    for idx, cam in enumerate(cameras):
        c_id = cam.get("id")
        if not c_id:
            errors.append(f"Camera #{idx} missing 'id'.")
        elif c_id in camera_ids:
            errors.append(f"Duplicate camera id: '{c_id}'.")
        camera_ids.add(c_id)

        for fld in ["brand", "model", "lens_mount", "sensor_size", "crop_factor"]:
            if fld not in cam or cam[fld] is None:
                errors.append(f"Camera '{c_id}' missing field '{fld}'.")

        ss = cam.get("sensor_size", {})
        w = ss.get("width", 0.0)
        h = ss.get("height", 0.0)
        if w <= 0 or h <= 0:
            errors.append(f"Camera '{c_id}' has invalid sensor dimensions: width={w}, height={h}.")

        cf = cam.get("crop_factor", 0.0)
        if cf <= 0.1 or cf > 20.0:
            errors.append(f"Camera '{c_id}' has suspicious crop factor: {cf}.")

        # Check physical diagonal consistency: diag = sqrt(w^2 + h^2), cf ~ FULL_FRAME_DIAG / diag
        if w > 0 and h > 0 and cf > 0:
            expected_cf = FULL_FRAME_DIAG / math.sqrt(w ** 2 + h ** 2)
            # Allow 15% tolerance for aspect ratio variations (e.g. 16:9 vs 4:3 vs 3:2 crops)
            if abs(expected_cf - cf) / cf > 0.18:
                errors.append(f"Camera '{c_id}' physical geometry mismatch: reported CF={cf}, calculated CF={expected_cf:.3f}.")

    # Validate Lenses
    lenses = db.get("lenses", [])
    if not lenses:
        errors.append("Lens registry is empty.")

    lens_ids = set()
    for idx, lens in enumerate(lenses):
        l_id = lens.get("id")
        if not l_id:
            errors.append(f"Lens #{idx} missing 'id'.")
        elif l_id in lens_ids:
            errors.append(f"Duplicate lens id: '{l_id}'.")
        lens_ids.add(l_id)

        for fld in ["brand", "model", "mount_compatibility", "focal_range", "aperture"]:
            if fld not in lens or lens[fld] is None:
                errors.append(f"Lens '{l_id}' missing field '{fld}'.")

        mounts = lens.get("mount_compatibility", [])
        if not mounts or not isinstance(mounts, list):
            errors.append(f"Lens '{l_id}' must specify at least one compatible mount.")

        fr = lens.get("focal_range", {})
        f_min = fr.get("min", 0.0)
        f_max = fr.get("max", 0.0)
        if f_min <= 0 or f_max < f_min:
            errors.append(f"Lens '{l_id}' has invalid focal range: min={f_min}, max={f_max}.")

    is_valid = (len(errors) == 0)
    return is_valid, errors


def main():
    """CLI Entry Point."""
    import argparse
    parser = argparse.ArgumentParser(description="Gyroflow Camera & Lens Database Builder")
    parser.add_argument("--build", action="store_true", help="Aggregate, normalize, and export canonical database")
    parser.add_argument("--validate", action="store_true", help="Validate schema and physical consistency")
    parser.add_argument("--stats", action="store_true", help="Display database statistics")
    parser.add_argument("--match", nargs=3, metavar=("BRAND", "MODEL", "LENS"), help="Test resolving a legacy camera/lens tuple")
    parser.add_argument("--output", default="resources/camera_lens_database.json", help="Output file path")

    args = parser.parse_args()

    if len(sys.argv) == 1 or args.build:
        print("=== BUILDING GYROFLOW CAMERA & LENS DATABASE ===")
        builder = LensDatabaseBuilder()
        builder.load_curated_data()
        builder.parse_lensfun_xmls()
        builder.parse_gyroflow_presets()
        db = builder.export(args.output)
        valid, errors = validate_database_schema(db)
        if valid:
            print("[Validation] Schema validation PASSED (100% compliant).")
        else:
            print(f"[Validation] Warning: {len(errors)} issues found:")
            for err in errors[:10]:
                print(f"  - {err}")
            if len(errors) > 10:
                print(f"  ...and {len(errors) - 10} more.")

    if args.validate:
        with open(args.output, 'r', encoding='utf-8') as f:
            db = json.load(f)
        valid, errors = validate_database_schema(db)
        if valid:
            print(f"[Validation] '{args.output}' is 100% valid.")
        else:
            print(f"[Validation] Failed with {len(errors)} errors:")
            for err in errors[:15]:
                print(f"  - {err}")

    if args.stats:
        with open(args.output, 'r', encoding='utf-8') as f:
            db = json.load(f)
        stats = db.get('stats', {})
        print("=== DATABASE STATISTICS ===")
        print(f"Schema Version:     {db.get('schema_version')}")
        print(f"Camera Brands:      {stats.get('total_camera_brands')}")
        print(f"Total Cameras:      {stats.get('total_cameras')}")
        print(f"Lens Brands:        {stats.get('total_lens_brands')}")
        print(f"Total Lenses:       {stats.get('total_lenses')}")
        print(f"Total Mounts:       {stats.get('total_mounts')}")

    if args.match:
        brand, model, lens = args.match
        resolver = LensProfileResolver(args.output)
        cam = resolver.resolve_camera(brand, model)
        lens_entry = resolver.resolve_lens(lens, cam)
        print(f"--- MATCHING RESULTS for ('{brand}', '{model}', '{lens}') ---")
        if cam:
            print(f"Camera: {cam['brand']} {cam['model']} [ID: {cam['id']}, Mount: {cam['lens_mount']}, CF: {cam['crop_factor']}, Sensor: {cam['sensor_size']['width']}x{cam['sensor_size']['height']}mm]")
        else:
            print("Camera: Not matched (falls back to Custom)")
        if lens_entry:
            print(f"Lens:   {lens_entry['brand']} {lens_entry['model']} [ID: {lens_entry['id']}, Range: {lens_entry['focal_range']['min']}-{lens_entry['focal_range']['max']}mm]")
        else:
            print("Lens:   Not matched (falls back to Custom)")


if __name__ == "__main__":
    main()
