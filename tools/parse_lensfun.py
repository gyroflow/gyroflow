#!/usr/bin/env python3
"""
Parse the LensFun XML database files and extract camera/lens information.

Reads all XML files from the LensFun data/db/ directory, extracts:
  - Camera makers, models, mounts, crop factors
  - Lens makers, models, focal length ranges, aperture ranges, mounts, types
  - Mount definitions and compatibility

Outputs a JSON structure compatible with the gyroflow camera_database format.

Usage:
    python3 parse_lensfun.py [--db-dir PATH] [--output PATH]
"""

import argparse
import json
import os
import re
import sys
import xml.etree.ElementTree as ET
from collections import defaultdict
from pathlib import Path


def parse_lensfun_xml(filepath):
    """Parse a single LensFun XML file and return cameras, lenses, and mounts."""
    cameras = []
    lenses = []
    mounts = []

    try:
        tree = ET.parse(filepath)
    except ET.ParseError as exc:
        print(f"  WARNING: Could not parse {filepath}: {exc}", file=sys.stderr)
        return cameras, lenses, mounts

    root = tree.getroot()

    # Parse mount definitions
    for mount_el in root.findall("mount"):
        name = mount_el.findtext("name", "").strip()
        compat_list = [c.text.strip() for c in mount_el.findall("compat") if c.text]
        if name:
            mounts.append({
                "name": name,
                "compat": compat_list,
            })

    # Parse camera entries
    for cam_el in root.findall("camera"):
        maker = ""
        maker_en = ""
        for m in cam_el.findall("maker"):
            lang = m.get("lang", "")
            text = (m.text or "").strip()
            if not lang:
                maker = text
            elif lang == "en":
                maker_en = text
        if not maker:
            maker = maker_en

        model = ""
        model_en = ""
        for m in cam_el.findall("model"):
            lang = m.get("lang", "")
            text = (m.text or "").strip()
            if not lang:
                model = text
            elif lang == "en":
                model_en = text

        mount = cam_el.findtext("mount", "").strip()
        cropfactor_str = cam_el.findtext("cropfactor", "").strip()
        cropfactor = None
        if cropfactor_str:
            try:
                cropfactor = float(cropfactor_str)
            except ValueError:
                pass

        variants = []
        for v in cam_el.findall("variant"):
            variants.append((v.text or "").strip())

        if maker or model:
            cam = {
                "maker": maker,
                "model": model,
                "model_en": model_en if model_en else None,
                "mount": mount if mount else None,
                "crop_factor": cropfactor,
            }
            if variants:
                cam["variants"] = variants
            cameras.append(cam)

    # Parse lens entries
    for lens_el in root.findall("lens"):
        maker = ""
        maker_en = ""
        for m in lens_el.findall("maker"):
            lang = m.get("lang", "")
            text = (m.text or "").strip()
            if not lang:
                maker = text
            elif lang == "en":
                maker_en = text
        if not maker:
            maker = maker_en

        model = ""
        model_en = ""
        for m in lens_el.findall("model"):
            lang = m.get("lang", "")
            text = (m.text or "").strip()
            if not lang:
                model = text
            elif lang == "en":
                model_en = text

        mount = lens_el.findtext("mount", "").strip()
        cropfactor_str = lens_el.findtext("cropfactor", "").strip()
        cropfactor = None
        if cropfactor_str:
            try:
                cropfactor = float(cropfactor_str)
            except ValueError:
                pass

        lens_type = lens_el.findtext("type", "").strip() or None
        aspect_ratio = lens_el.findtext("aspect-ratio", "").strip() or None

        # Focal length range from explicit <focal> tag
        focal_el = lens_el.find("focal")
        focal_min = None
        focal_max = None
        if focal_el is not None:
            val = focal_el.get("value")
            fmin = focal_el.get("min")
            fmax = focal_el.get("max")
            if val:
                try:
                    focal_min = focal_max = float(val)
                except ValueError:
                    pass
            else:
                if fmin:
                    try:
                        focal_min = float(fmin)
                    except ValueError:
                        pass
                if fmax:
                    try:
                        focal_max = float(fmax)
                    except ValueError:
                        pass

        # Aperture range
        aperture_el = lens_el.find("aperture")
        aperture_min = None
        aperture_max = None
        if aperture_el is not None:
            amin = aperture_el.get("min")
            amax = aperture_el.get("max")
            aval = aperture_el.get("value")
            if aval:
                try:
                    aperture_min = aperture_max = float(aval)
                except ValueError:
                    pass
            else:
                if amin:
                    try:
                        aperture_min = float(amin)
                    except ValueError:
                        pass
                if amax:
                    try:
                        aperture_max = float(amax)
                    except ValueError:
                        pass

        # If no explicit focal tag, try to extract from calibration distortion entries
        if focal_min is None and focal_max is None:
            cal_el = lens_el.find("calibration")
            if cal_el is not None:
                focals = set()
                for dist in cal_el.findall("distortion"):
                    f = dist.get("focal")
                    if f:
                        try:
                            focals.add(float(f))
                        except ValueError:
                            pass
                if focals:
                    focal_min = min(focals)
                    focal_max = max(focals)

        # Also try to parse focal from model name if still missing
        if focal_min is None and focal_max is None and model:
            m_range = re.search(r'(\d+(?:\.\d+)?)\s*-\s*(\d+(?:\.\d+)?)\s*mm', model)
            m_single = re.search(r'(\d+(?:\.\d+)?)\s*mm', model)
            if m_range:
                try:
                    focal_min = float(m_range.group(1))
                    focal_max = float(m_range.group(2))
                except ValueError:
                    pass
            elif m_single:
                try:
                    focal_min = focal_max = float(m_single.group(1))
                except ValueError:
                    pass

        if maker or model:
            lens = {
                "maker": maker,
                "model": model,
                "model_en": model_en if model_en else None,
                "mount": mount if mount else None,
                "crop_factor": cropfactor,
                "type": lens_type,
                "aspect_ratio": aspect_ratio,
                "focal_min": focal_min,
                "focal_max": focal_max,
                "aperture_min": aperture_min,
                "aperture_max": aperture_max,
            }
            lenses.append(lens)

    return cameras, lenses, mounts


def crop_factor_to_sensor_size(crop_factor):
    """
    Estimate sensor dimensions from crop factor.
    Full-frame reference: 36mm x 24mm (diagonal ~43.27mm).
    Assumes 3:2 aspect ratio.
    """
    if not crop_factor or crop_factor <= 0:
        return None, None
    ff_w, ff_h = 36.0, 24.0
    sensor_w = ff_w / crop_factor
    sensor_h = ff_h / crop_factor
    return round(sensor_w, 2), round(sensor_h, 2)


# Comprehensive maker normalization: lowered key -> canonical name
_MAKER_CANONICAL = {
    "canon": "Canon",
    "nikon": "Nikon",
    "nikon corporation": "Nikon",
    "sony": "Sony",
    "fujifilm": "Fujifilm",
    "fuji": "Fujifilm",
    "fuji photo film co ltd": "Fujifilm",
    "panasonic": "Panasonic",
    "olympus": "Olympus",
    "olympus corporation": "Olympus",
    "olympus imaging corp.": "Olympus",
    "olympus optical co.,ltd": "Olympus",
    "om digital solutions": "OM System",
    "om system": "OM System",
    "pentax": "Pentax",
    "pentax corporation": "Pentax",
    "asahi optical co.,ltd": "Pentax",
    "samsung": "Samsung",
    "samsung techwin": "Samsung",
    "samsung techwin co.": "Samsung",
    "sigma": "Sigma",
    "tamron": "Tamron",
    "tokina": "Tokina",
    "zeiss": "Zeiss",
    "carl zeiss": "Zeiss",
    "hasselblad": "Hasselblad",
    "leica": "Leica",
    "leica camera ag": "Leica",
    "ricoh": "Ricoh",
    "ricoh imaging company": "Ricoh",
    "ricoh imaging company, ltd.": "Ricoh",
    "ricoh imaging company, ltd": "Ricoh",
    "gopro": "GoPro",
    "dji": "DJI",
    "samyang": "Samyang",
    "rokinon": "Samyang",
    "vivitar": "Vivitar",
    "schneider": "Schneider",
    "schneider-kreuznach": "Schneider",
    "kodak": "Kodak",
    "eastman kodak company": "Kodak",
    "casio": "Casio",
    "casio computer co.,ltd": "Casio",
    "casio computer co.,ltd.": "Casio",
    "konica minolta": "Konica Minolta",
    "konica minolta camera, inc.": "Konica Minolta",
    "minolta": "Minolta",
    "minolta co., ltd.": "Minolta",
    "konica": "Konica",
    "gitup": "Gitup",
    "aee": "AEE",
    "aee dv": "AEE",
    "soligor": "Soligor",
    "seiko epson corp.": "Epson",
    "yi technology": "YI Technology",
    "lg mobile": "LG",
    "lg": "LG",
    "apple": "Apple",
    "nokia": "Nokia",
    "huawei": "Huawei",
    "honor": "Honor",
    "microsoft": "Microsoft",
    "phase one": "Phase One",
    "rolleiflex": "Rollei",
    "rollei": "Rollei",
    "mamiya": "Mamiya",
    "contax": "Contax",
    "cosina": "Cosina",
    "voigtländer": "Voigtlander",
    "kmz": "KMZ",
    "mto": "MTO",
    "viltrox": "Viltrox",
    "venus": "Venus",
    "yongnuo": "Yongnuo",
    "meike": "Meike",
    "meke": "Meike",
    "ttartisan": "TTArtisan",
    "7artisans": "7Artisans",
    "pergear": "Pergear",
    "slr magic": "SLR Magic",
    "irix": "Irix",
    "kipon": "Kipon",
    "mitakon": "Mitakon",
    "toshiba": "Toshiba",
    "generic": "Generic",
    "sun": "Sun",
    "fotasy": "Fotasy",
    "fujian": "Fujian",
    "astrhori": "AstrHori",
    "opteka": "Opteka",
    "quantaray": "Quantaray",
    "petri": "Petri",
    "miranda": "Miranda",
    "pentacon": "Pentacon",
    "chinon": "Chinon",
    "beroflex": "Beroflex",
    "steinheil münchen": "Steinheil",
    "yashica": "Yashica",
    "arsenal": "Arsenal",
    "zenit": "Zenit",
    "meyer-optik görlitz": "Meyer-Optik",
}


def normalize_maker(raw):
    """Normalize a camera/lens maker name."""
    if not raw:
        return ""
    s = raw.strip()
    key = s.lower()
    if key in _MAKER_CANONICAL:
        return _MAKER_CANONICAL[key]
    # Fallback
    return s.title() if s == s.lower() or s == s.upper() else s


def build_lensfun_database(db_dir):
    """Parse all LensFun XML files and build a unified database."""
    db_path = Path(db_dir)
    xml_files = sorted(db_path.glob("*.xml"))

    if not xml_files:
        print(f"ERROR: No XML files found in {db_dir}", file=sys.stderr)
        sys.exit(1)

    all_cameras = []
    all_lenses = []
    all_mounts = {}

    for xml_file in xml_files:
        cameras, lenses, mounts = parse_lensfun_xml(str(xml_file))
        all_cameras.extend(cameras)
        all_lenses.extend(lenses)
        for m in mounts:
            all_mounts[m["name"]] = m

    print(f"Parsed {len(xml_files)} XML files")
    print(f"  Raw cameras: {len(all_cameras)}")
    print(f"  Raw lenses:  {len(all_lenses)}")
    print(f"  Mounts:      {len(all_mounts)}")

    # Normalize and deduplicate cameras
    camera_map = {}
    for cam in all_cameras:
        maker = normalize_maker(cam["maker"])
        model = cam["model"]
        if not maker and not model:
            continue
        key = (maker, model)
        if key not in camera_map:
            camera_map[key] = {
                "maker": maker,
                "model": model,
                "model_en": cam.get("model_en"),
                "mount": cam.get("mount"),
                "crop_factor": cam.get("crop_factor"),
            }
        else:
            existing = camera_map[key]
            if not existing.get("mount") and cam.get("mount"):
                existing["mount"] = cam["mount"]
            if not existing.get("crop_factor") and cam.get("crop_factor"):
                existing["crop_factor"] = cam["crop_factor"]
            if not existing.get("model_en") and cam.get("model_en"):
                existing["model_en"] = cam["model_en"]

    # Normalize and deduplicate lenses
    lens_map = {}
    for lens in all_lenses:
        maker = normalize_maker(lens["maker"])
        model = lens["model"]
        if not maker and not model:
            continue
        key = (maker, model)
        if key not in lens_map:
            lens_map[key] = {
                "maker": maker,
                "model": model,
                "model_en": lens.get("model_en"),
                "mount": lens.get("mount"),
                "crop_factor": lens.get("crop_factor"),
                "type": lens.get("type"),
                "aspect_ratio": lens.get("aspect_ratio"),
                "focal_min": lens.get("focal_min"),
                "focal_max": lens.get("focal_max"),
                "aperture_min": lens.get("aperture_min"),
                "aperture_max": lens.get("aperture_max"),
            }
        else:
            existing = lens_map[key]
            for field in ["mount", "crop_factor", "type", "aspect_ratio",
                          "focal_min", "focal_max", "aperture_min", "aperture_max",
                          "model_en"]:
                if not existing.get(field) and lens.get(field):
                    existing[field] = lens[field]

    # Build output structure organized by brand
    brands_cameras = defaultdict(list)
    for (maker, model), cam in camera_map.items():
        sensor_w, sensor_h = crop_factor_to_sensor_size(cam.get("crop_factor"))
        entry = {
            "name": cam.get("model_en") or model,
            "model_id": model,
            "mount": cam.get("mount"),
            "crop_factor": cam.get("crop_factor"),
        }
        if sensor_w and sensor_h:
            entry["sensor_width_mm"] = sensor_w
            entry["sensor_height_mm"] = sensor_h
        entry = {k: v for k, v in entry.items() if v is not None}
        brands_cameras[maker].append(entry)

    brands_lenses = defaultdict(list)
    for (maker, model), lens in lens_map.items():
        entry = {
            "name": lens.get("model_en") or model,
            "model_id": model,
            "mount": lens.get("mount"),
            "crop_factor": lens.get("crop_factor"),
        }
        if lens.get("focal_min") is not None:
            entry["focal_length_min"] = lens["focal_min"]
        if lens.get("focal_max") is not None:
            entry["focal_length_max"] = lens["focal_max"]
        if lens.get("aperture_min") is not None:
            entry["aperture_min"] = lens["aperture_min"]
        if lens.get("aperture_max") is not None:
            entry["aperture_max"] = lens["aperture_max"]
        if lens.get("type"):
            entry["type"] = lens["type"]
        if lens.get("aspect_ratio"):
            entry["aspect_ratio"] = lens["aspect_ratio"]
        entry = {k: v for k, v in entry.items() if v is not None}
        brands_lenses[maker].append(entry)

    all_brand_names = sorted(set(list(brands_cameras.keys()) + list(brands_lenses.keys())))

    brands_list = []
    for brand_name in all_brand_names:
        brand_entry = {"name": brand_name}
        cams = brands_cameras.get(brand_name, [])
        if cams:
            cams.sort(key=lambda x: x.get("name", ""))
            brand_entry["cameras"] = cams
        lns = brands_lenses.get(brand_name, [])
        if lns:
            lns.sort(key=lambda x: x.get("name", ""))
            brand_entry["lenses"] = lns
        brands_list.append(brand_entry)

    mounts_list = []
    for name in sorted(all_mounts.keys()):
        m = all_mounts[name]
        mounts_list.append({
            "name": m["name"],
            "compatible_mounts": m["compat"],
        })

    database = {
        "source": "lensfun",
        "version": 2,
        "brands": brands_list,
        "mounts": mounts_list,
    }

    # Statistics
    total_cameras = sum(len(b.get("cameras", [])) for b in brands_list)
    total_lenses = sum(len(b.get("lenses", [])) for b in brands_list)
    total_brands = len(brands_list)

    print(f"\n=== LensFun Database Extraction ===")
    print(f"Total brands:  {total_brands}")
    print(f"Total cameras: {total_cameras}")
    print(f"Total lenses:  {total_lenses}")
    print(f"Total mounts:  {len(mounts_list)}")

    cams_with_cf = sum(
        1 for b in brands_list
        for c in b.get("cameras", [])
        if c.get("crop_factor") is not None
    )
    print(f"Cameras with crop_factor: {cams_with_cf}/{total_cameras}")

    lenses_with_focal = sum(
        1 for b in brands_list
        for l in b.get("lenses", [])
        if l.get("focal_length_min") is not None
    )
    print(f"Lenses with focal info:   {lenses_with_focal}/{total_lenses}")

    # Top brands
    brand_cam_counts = [
        (b["name"], len(b.get("cameras", [])))
        for b in brands_list if b.get("cameras")
    ]
    brand_cam_counts.sort(key=lambda x: -x[1])
    print(f"\nTop 10 brands by camera count:")
    for name, count in brand_cam_counts[:10]:
        print(f"  {name}: {count} cameras")

    brand_lens_counts = [
        (b["name"], len(b.get("lenses", [])))
        for b in brands_list if b.get("lenses")
    ]
    brand_lens_counts.sort(key=lambda x: -x[1])
    print(f"\nTop 10 brands by lens count:")
    for name, count in brand_lens_counts[:10]:
        print(f"  {name}: {count} lenses")

    return database


def main():
    parser = argparse.ArgumentParser(description="Parse LensFun XML database")
    parser.add_argument(
        "--db-dir",
        default="/Users/omermac/Desktop/moneymaking2/workspace/lensfun_data/data/db",
        help="Path to LensFun data/db/ directory",
    )
    parser.add_argument(
        "--output",
        default="/tmp/lensfun_data.json",
        help="Output JSON file path",
    )
    args = parser.parse_args()

    if not os.path.isdir(args.db_dir):
        print(f"ERROR: LensFun database directory not found: {args.db_dir}", file=sys.stderr)
        sys.exit(1)

    database = build_lensfun_database(args.db_dir)

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    with open(output_path, "w", encoding="utf-8") as f:
        json.dump(database, f, indent=2, ensure_ascii=False)

    print(f"\nDatabase written to: {output_path}")
    print(f"File size: {output_path.stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
