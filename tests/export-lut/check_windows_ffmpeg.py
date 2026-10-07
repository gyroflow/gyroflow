#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check the exact Windows shared FFmpeg bundle selected by FFMPEG_DIR.

Uses only Python's standard library. No installation, downloads, media writes,
or global PATH changes. Run: python check_windows_ffmpeg.py <FFMPEG_DIR>
"""
import ctypes
import json
import os
from pathlib import Path
import sys


def check_bundle(root):
    root = Path(root).resolve(strict=True)
    binary_dir = root / "bin"
    if not list(binary_dir.glob("avfilter-*.dll")):
        arch = "arm64" if os.environ.get("PROCESSOR_ARCHITECTURE") == "ARM64" else "x64"
        binary_dir /= arch
    # Pin DLL resolution to this bundle, including its transitive dependencies.
    with os.add_dll_directory(str(binary_dir)):
        def load(name):
            matches = list(binary_dir.glob(name + "-*.dll"))
            if len(matches) != 1:
                raise RuntimeError(f"Expected one {name} DLL in {binary_dir}")
            return ctypes.CDLL(str(matches[0]))

        filters, codecs, util = load("avfilter"), load("avcodec"), load("avutil")
        versions = {}
        for library, name, major in ((filters, "avfilter", 12), (codecs, "avcodec", 63), (util, "avutil", 61)):
            version = getattr(library, name + "_version")
            version.restype = ctypes.c_uint
            packed = version()
            versions[name] = [packed >> 16, (packed >> 8) & 255, packed & 255]
            if versions[name][0] != major:
                raise RuntimeError(f"{name} ABI mismatch: {versions[name]}, expected major {major}")
        filters.avfilter_get_by_name.argtypes = [ctypes.c_char_p]
        filters.avfilter_get_by_name.restype = ctypes.c_void_p
        codecs.avcodec_find_encoder_by_name.argtypes = [ctypes.c_char_p]
        codecs.avcodec_find_encoder_by_name.restype = ctypes.c_void_p
        required_filters = ["buffer", "format", "lut3d", "geq", "buffersink"]
        required_encoders = ["libx264", "libx265", "prores_ks", "ffv1"]
        missing = [name for name in required_filters if not filters.avfilter_get_by_name(name.encode())]
        missing += [name for name in required_encoders if not codecs.avcodec_find_encoder_by_name(name.encode())]
        if missing:
            raise RuntimeError("Missing required FFmpeg filters/encoders: " + ", ".join(missing))
        for name in ("avfilter", "avcodec", "avformat", "avutil", "swscale", "swresample"):
            if not (root / "lib" / (name + ".lib")).is_file():
                raise RuntimeError(f"Missing MSVC import library: {name}.lib")
            if not (root / "include" / ("lib" + name)).is_dir():
                raise RuntimeError(f"Missing development headers: lib{name}")
        return {"versions": versions, "filters": required_filters, "encoders": required_encoders}


if __name__ == "__main__":
    try:
        if os.name != "nt" or len(sys.argv) != 2:
            raise RuntimeError("Usage on Windows: python check_windows_ffmpeg.py <FFMPEG_DIR>")
        print(json.dumps(check_bundle(sys.argv[1]), sort_keys=True))
    except (OSError, RuntimeError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
