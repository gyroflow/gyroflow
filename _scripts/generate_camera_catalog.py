#!/usr/bin/env python3
"""Build a deterministic camera/lens catalogue from local, pinned source trees.

No network requests. Lensfun records are metadata, never Gyroflow calibrations.
Pass the upstream revision IDs so the generated document records its inputs.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import math
import os
import tempfile
from pathlib import Path
import xml.etree.ElementTree as ET


def text(value: object) -> str:
    return " ".join(value.split()) if isinstance(value, str) else ""


def key(brand: str, model: str) -> str:
    return json.dumps([text(brand).casefold(), text(model).casefold()], ensure_ascii=False)


def number(value: object) -> float | None:
    try:
        result = float(value) if not isinstance(value, bool) else float("nan")
        return result if math.isfinite(result) and result > 0 else None
    except (TypeError, ValueError):
        return None


def base_name(element: ET.Element, tag: str) -> str:
    nodes = element.findall(tag)
    return next((text(n.text) for n in nodes if not n.get("lang") and text(n.text)),
                next((text(n.text) for n in nodes if text(n.text)), ""))


def add_record(records: dict, brand: str, model: str, **fields: object) -> None:
    brand, model = text(brand), text(model)
    if not brand or not model:
        return
    record = records.setdefault(key(brand, model), {"brand": brand, "model": model,
                                                   "aliases": [], "mounts": [], "sources": []})
    for name, value in fields.items():
        if name == "sampled_focal_lengths":
            record[name] = sorted(set(record.get(name, [])) | set(value))
        elif isinstance(value, list):
            record[name] = sorted(set(record.get(name, [])) | {text(x) for x in value if text(x)})
        elif value is not None and name not in record:
            record[name] = value


def generate(profiles: Path, lensfun: Path, profile_revision: str, lensfun_revision: str) -> dict:
    cameras: dict = {}
    lenses: dict = {}
    mounts: dict = {}
    digests: list[tuple[str, str]] = []
    profile_count = 0
    xml_count = 0
    # Lensfun supplies canonical maker/model names, aliases, mounts and crop factors.
    for path in sorted(lensfun.rglob("*.xml")):
        raw = path.read_bytes()
        root = ET.fromstring(raw)
        if root.tag != "lensdatabase":
            continue
        xml_count += 1
        digests.append(("lensfun/" + path.relative_to(lensfun).as_posix(), hashlib.sha256(raw).hexdigest()))
        for mount in root.findall("mount"):
            name = base_name(mount, "name")
            if name:
                mounts[name] = sorted(set(mounts.get(name, [])) | {text(n.text) for n in mount.findall("compat") if text(n.text)})
        for camera in root.findall("camera"):
            crop = number(camera.findtext("cropfactor"))
            add_record(cameras, base_name(camera, "maker"), base_name(camera, "model"),
                       aliases=[text(n.text) for n in camera.findall("model")],
                       mounts=[text(n.text) for n in camera.findall("mount")],
                       crop_factor=crop,
                       # A diagonal equivalence is not an asserted physical width/height.
                       equivalent_sensor_diagonal_mm=round(math.hypot(36, 24) / crop, 6) if crop else None,
                       sources=["lensfun"])
        for lens in root.findall("lens"):
            focal = lens.find("focal")
            low = number(focal.get("min")) if focal is not None else None
            high = number(focal.get("max")) if focal is not None else None
            if focal is not None and number(focal.get("value")):
                low = high = number(focal.get("value"))
            # Some Lensfun entries specify focal lengths only in calibration records.
            points = [number(n.get("focal")) for n in lens.findall("calibration/*")]
            points = [n for n in points if n is not None]
            add_record(lenses, base_name(lens, "maker"), base_name(lens, "model"),
                       aliases=[text(n.text) for n in lens.findall("model")],
                       mounts=[text(n.text) for n in lens.findall("mount")],
                       crop_factor=number(lens.findtext("cropfactor")),
                       min_focal_length=low, max_focal_length=high,
                       sampled_focal_lengths=points, sources=["lensfun"])
    # Merge profile aliases into their unambiguous canonical Lensfun body.
    # A duplicate alias body would otherwise obscure metadata prefill at runtime.
    camera_aliases: dict[str, str | None] = {}
    for identity, camera in cameras.items():
        for alias in [camera["model"], *camera["aliases"]]:
            alias_key = key(camera["brand"], alias)
            if alias_key not in camera_aliases:
                camera_aliases[alias_key] = identity
            elif camera_aliases[alias_key] != identity:
                camera_aliases[alias_key] = None
    # Gyroflow profiles add names missing from Lensfun without inventing mount data.
    for path in sorted(profiles.rglob("*.json")):
        if path.name.startswith("__") or path.name == "camera_catalog.json":
            continue
        raw = path.read_bytes()
        try:
            data = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            continue
        if not isinstance(data, dict) or "calib_dimension" not in data:
            continue
        profile_count += 1
        digests.append(("profiles/" + path.relative_to(profiles).as_posix(), hashlib.sha256(raw).hexdigest()))
        brand, model = text(data.get("camera_brand")), text(data.get("camera_model"))
        canonical = camera_aliases.get(key(brand, model))
        if canonical:
            brand, model = cameras[canonical]["brand"], cameras[canonical]["model"]
        add_record(cameras, brand, model, sources=["gyroflow"],
                   sensor_width_mm=number(data.get("sensor_width_mm")),
                   sensor_height_mm=number(data.get("sensor_height_mm")))
        # A lens label is not necessarily made by the camera manufacturer. Retain
        # it as a profile association; do not infer a lens manufacturer or mount.
    if not xml_count or not profile_count:
        raise ValueError("Both a nonempty Lensfun database and Gyroflow profile tree are required")
    digest = hashlib.sha256(json.dumps(digests, separators=(",", ":")).encode()).hexdigest()
    return {"schema_version": 1, "sources": {"gyroflow_revision": profile_revision,
            "lensfun_revision": lensfun_revision, "input_sha256": digest,
            "profile_count": profile_count, "lensfun_xml_count": xml_count},
            "cameras": [cameras[k] for k in sorted(cameras)],
            "lenses": [lenses[k] for k in sorted(lenses)], "mounts": dict(sorted(mounts.items()))}



def write_catalog(output: Path, document: dict, profiles: Path, lensfun: Path) -> None:
    """Write a generated catalogue without destroying source inputs or old output."""
    output_resolved = output.resolve()
    profile_root = profiles.resolve()
    lensfun_root = lensfun.resolve()

    # The profile repository intentionally accepts this one reserved metadata
    # file. Ordinary profile JSONs and all Lensfun XMLs are source data.
    reserved = profile_root / "__camera_catalog.json"
    if output_resolved.is_relative_to(lensfun_root) or (
        output_resolved.is_relative_to(profile_root) and output_resolved != reserved
    ):
        raise ValueError("Refusing to overwrite catalogue input tree with generated metadata")

    # Also protect external targets reached through input symlinks. A symlink
    # source may live outside the nominal tree but is still an input.
    for root, glob in ((profiles, "*.json"), (lensfun, "*.xml")):
        for source in root.rglob(glob):
            if root == profiles and (
                source.name.startswith("__") or source.name == "camera_catalog.json"
            ):
                continue
            if source.resolve() == output_resolved:
                raise ValueError(f"Refusing to overwrite input: {source}")

    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        # A failed/disk-full write must leave an earlier working catalogue in
        # place. The temporary file is created on the same filesystem.
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=output.parent,
            prefix=".camera-catalog-", suffix=".tmp", delete=False,
        ) as stream:
            temporary = Path(stream.name)
            json.dump(document, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, output)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--lensfun", type=Path, required=True)
    parser.add_argument("--profiles-revision", required=True)
    parser.add_argument("--lensfun-revision", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.profiles.is_dir() or not args.lensfun.is_dir():
        parser.error("Input directories must exist")
    try:
        document = generate(args.profiles, args.lensfun, args.profiles_revision, args.lensfun_revision)
    except (ValueError, OSError, ET.ParseError) as exc:
        parser.exit(1, f"Catalogue generation failed: {exc}\n")
    try:
        write_catalog(args.output, document, args.profiles, args.lensfun)
    except (OSError, ValueError) as exc:
        parser.exit(1, f"Catalogue output failed: {exc}\n")
    print(json.dumps({"cameras": len(document["cameras"]), "lenses": len(document["lenses"]),
                      "sources": document["sources"]}))


if __name__ == "__main__":
    main()
