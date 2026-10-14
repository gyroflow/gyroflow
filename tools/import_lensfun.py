#!/usr/bin/env python3
"""Convert Lensfun XML entries into Gyroflow lens profile JSON files.

This is intentionally a small importer: it supports Lensfun's PTLens-style
`<distortion model="ptlens" ...>` entries because Gyroflow already ships a
matching `ptlens` distortion model. Unsupported distortion models are skipped so
that generated profiles do not silently use the wrong math.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


SAFE_NAME_RE = re.compile(r"[^A-Za-z0-9._-]+")


@dataclass(frozen=True)
class LensfunCamera:
    maker: str
    model: str
    cropfactor: float | None


@dataclass(frozen=True)
class LensfunLens:
    maker: str
    model: str
    cropfactor: float | None
    min_focal: float | None
    max_focal: float | None
    distortion_coeffs: tuple[float, float, float]


def _text(element: ET.Element, name: str) -> str:
    child = element.find(name)
    return (child.text or "").strip() if child is not None else ""


def _float(value: str | None) -> float | None:
    if value is None or value == "":
        return None
    try:
        return float(value)
    except ValueError:
        return None


def _focal_range(value: str) -> tuple[float | None, float | None]:
    """Parse Lensfun focal values like `24`, `24-70`, or `18.0 - 55.0`."""
    parts = [p for p in re.split(r"\s*-\s*", value.strip()) if p]
    if not parts:
        return None, None
    if len(parts) == 1:
        focal = _float(parts[0])
        return focal, focal
    return _float(parts[0]), _float(parts[-1])


def _iter_xml_files(path: Path) -> Iterable[Path]:
    if path.is_file():
        yield path
        return
    yield from sorted(path.rglob("*.xml"))


def parse_lensfun(path: Path) -> tuple[list[LensfunCamera], list[LensfunLens]]:
    cameras: list[LensfunCamera] = []
    lenses: list[LensfunLens] = []

    for xml_file in _iter_xml_files(path):
        root = ET.parse(xml_file).getroot()
        for camera in root.findall("camera"):
            maker = _text(camera, "maker")
            model = _text(camera, "model")
            if maker and model:
                cameras.append(
                    LensfunCamera(
                        maker=maker,
                        model=model,
                        cropfactor=_float(_text(camera, "cropfactor")),
                    )
                )

        for lens in root.findall("lens"):
            maker = _text(lens, "maker")
            model = _text(lens, "model")
            if not model:
                continue

            distortion = next(
                (
                    node
                    for node in lens.findall("calibration/distortion")
                    if node.attrib.get("model") == "ptlens"
                ),
                None,
            )
            if distortion is None:
                continue

            coeffs = tuple(
                _float(distortion.attrib.get(name)) or 0.0
                for name in ("a", "b", "c")
            )
            min_focal, max_focal = _focal_range(_text(lens, "focal"))
            lenses.append(
                LensfunLens(
                    maker=maker,
                    model=model,
                    cropfactor=_float(_text(lens, "cropfactor")),
                    min_focal=min_focal,
                    max_focal=max_focal,
                    distortion_coeffs=coeffs,  # type: ignore[arg-type]
                )
            )

    return cameras, lenses


def _profile_filename(camera: LensfunCamera, lens: LensfunLens) -> str:
    raw = f"Lensfun_{camera.maker}_{camera.model}_{lens.maker}_{lens.model}.json"
    return SAFE_NAME_RE.sub("_", raw).strip("_")


def _profile(camera: LensfunCamera, lens: LensfunLens) -> dict:
    focal_length = lens.min_focal if lens.min_focal == lens.max_focal else None
    return {
        "name": f"Lensfun {camera.maker} {camera.model} {lens.maker} {lens.model}",
        "note": "Imported from Lensfun database",
        "calibrated_by": "Lensfun",
        "camera_brand": camera.maker,
        "camera_model": camera.model,
        "lens_model": " ".join(part for part in [lens.maker, lens.model] if part).strip(),
        "calib_dimension": {"w": 0, "h": 0},
        "orig_dimension": {"w": 0, "h": 0},
        "official": True,
        "fisheye_params": {
            "RMS_error": 0.0,
            "camera_matrix": [],
            "distortion_coeffs": list(lens.distortion_coeffs),
            "radial_distortion_limit": None,
        },
        "distortion_model": "ptlens",
        "focal_length": focal_length,
        "crop_factor": camera.cropfactor or lens.cropfactor,
        "compatible_settings": [],
    }


def convert(path: Path, output_dir: Path) -> int:
    cameras, lenses = parse_lensfun(path)
    output_dir.mkdir(parents=True, exist_ok=True)

    count = 0
    for camera in cameras:
        for lens in lenses:
            # Lensfun compatibility can be mount-driven. Until Gyroflow has a
            # mount graph, emit conservative same-crop profiles only.
            if camera.cropfactor and lens.cropfactor and abs(camera.cropfactor - lens.cropfactor) > 0.05:
                continue
            profile = _profile(camera, lens)
            (output_dir / _profile_filename(camera, lens)).write_text(
                json.dumps(profile, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            count += 1
    return count


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lensfun", type=Path, help="Lensfun XML file or database directory")
    parser.add_argument("output", type=Path, help="Directory for generated Gyroflow JSON profiles")
    args = parser.parse_args()

    count = convert(args.lensfun, args.output)
    print(f"Generated {count} Gyroflow Lensfun profile(s) in {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
