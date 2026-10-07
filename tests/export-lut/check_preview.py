#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Render the production QSB, using a caller-supplied 128x128 RGB24 fixture.

Requires Pillow and Qt's qml executable. No downloads or global settings.
Arguments: qml.exe fixture-directory output-directory [D3D adapter index]
The fixture directory contains input.png, atlas.png and expected.rgb generated
by preview_fixture with the DJI size-33 LUT, brightness .1 and contrast .2.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from PIL import Image


def main():
    if len(sys.argv) not in (4, 5):
        raise RuntimeError(__doc__)
    qml, fixture, output = (Path(value).resolve() for value in sys.argv[1:4])
    output.mkdir(parents=True, exist_ok=True)
    repository = Path(__file__).resolve().parents[2]
    shader = repository / "src/qt_gpu/compiled/color_preview.frag.qsb"
    template = (Path(__file__).parent / "preview-shader-check.qml").read_text()
    image = Image.open(fixture / "input.png").convert("RGB")
    if image.size != (128, 128):
        raise RuntimeError("Expected a 128x128 source fixture")
    rgb = image.tobytes()
    reference = (fixture / "expected.rgb").read_bytes()
    if len(reference) != len(rgb):
        raise RuntimeError("Fixture reference size mismatch")
    env = os.environ.copy()
    env.update(QSG_RHI_BACKEND="d3d11", QSG_INFO="1",
               QT_FORCE_STDERR_LOGGING="1", QT_LOGGING_RULES="qt.rhi.*=true")
    if len(sys.argv) == 5:
        env["QT_D3D_ADAPTER_INDEX"] = sys.argv[4]
    cases = [("lut", 33, .1, .2), ("neutral", 0, 0, 0)]
    cases += [(f"extreme-{b}-{c}", 0, b, c) for b in (-.5, .5) for c in (-.5, .5)]
    results = []
    for name, size, brightness, contrast in cases:
        with tempfile.TemporaryDirectory(prefix="gyroflow-shader-") as directory:
            directory = Path(directory)
            image.save(directory / "input.png")
            shutil.copyfile(fixture / "atlas.png", directory / "atlas.png")
            harness = template.replace("../../src/qt_gpu/compiled/color_preview.frag.qsb", shader.as_uri())
            harness = harness.replace("brightness: 0.1;", f"brightness: {brightness};")
            harness = harness.replace("contrast: 0.2;", f"contrast: {contrast};")
            harness = harness.replace("lutSize: 33;", f"lutSize: {size};")
            (directory / "check.qml").write_text(harness)
            run = subprocess.run([str(qml), str(directory / "check.qml")],
                                 cwd=directory, env=env, capture_output=True, timeout=30)
            log = (run.stdout + run.stderr).decode(errors="replace")
            (output / (name + ".log")).write_text(log)
            if run.returncode or "compilation failed" in log or "Failed to build" in log:
                raise RuntimeError(f"Shader render failed for {name}; see saved log")
            actual = Image.open(directory / "actual.png").convert("RGBA")
            actual.save(output / (name + ".png"))
            grab_size = actual.size
            actual = actual.resize((128, 128), Image.Resampling.NEAREST)
            if actual.getextrema()[3] != (255, 255):
                raise RuntimeError(f"Nonopaque shader result for {name}")
            actual_rgb = actual.convert("RGB").tobytes()
            expected = reference if size else bytes(
                int(min(1, max(0, (value / 255 - .5) * (1 + contrast) + .5 + brightness)) * 255 + .5)
                for value in rgb)
            difference = [abs(a - b) for a, b in zip(actual_rgb, expected)]
            maximum = max(difference)
            results.append({"case": name, "components": len(difference),
                            "different_components": sum(value != 0 for value in difference),
                            "max_difference": maximum, "grab_size": grab_size})
            if maximum > 1:
                raise RuntimeError(f"Color mismatch for {name}: maximum {maximum}, allowed 1")
    (output / "results.json").write_text(json.dumps(results, indent=2))
    print(json.dumps(results))


if __name__ == "__main__":
    main()
