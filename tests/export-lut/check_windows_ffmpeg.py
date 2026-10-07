#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check the exact Windows shared FFmpeg bundle selected by FFMPEG_DIR.

Uses only Python's standard library. No installation, downloads, media writes,
or global PATH changes. Run: python check_windows_ffmpeg.py <FFMPEG_DIR>
Use --target-arch x64|arm64 in build recipes to reject the wrong target bundle.
Cross builds validate PE/import-library machines and header ABIs, explicitly
deferring runtime checks when target DLLs cannot load in host Python.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import re
import struct
import sys


ABIS = {"avfilter": 12, "avcodec": 63, "avformat": 63, "avdevice": 63,
        "avutil": 61, "swscale": 10, "swresample": 7}
MACHINES = {"x64": 0x8664, "arm64": 0xAA64}


def pe_machine(path):
    with Path(path).open("rb") as file:
        dos = file.read(64)
        if len(dos) != 64 or dos[:2] != b"MZ":
            raise RuntimeError(f"Invalid PE file: {path}")
        file.seek(struct.unpack_from("<I", dos, 60)[0])
        header = file.read(6)
        if len(header) != 6 or header[:4] != b"PE\0\0":
            raise RuntimeError(f"Invalid PE header: {path}")
        return struct.unpack_from("<H", header, 4)[0]


def import_machine(path):
    # Skip COFF archive linker/name tables to inspect the first import object.
    with Path(path).open("rb") as file:
        if file.read(8) != b"!<arch>\n":
            raise RuntimeError(f"Invalid MSVC import archive: {path}")
        while header := file.read(60):
            if len(header) != 60 or header[58:] != b"`\n":
                raise RuntimeError(f"Invalid COFF archive member: {path}")
            size = int(header[48:58])
            if size < 0 or size > 64 * 1024 * 1024:
                raise RuntimeError(f"Invalid COFF member size: {path}")
            start = file.tell()
            if header[:16].strip() not in (b"/", b"//", b"/SYM64/"):
                data = file.read(min(size, 8))
                offset = 6 if data[:4] == b"\0\0\xff\xff" else 0
                if len(data) < offset + 2:
                    raise RuntimeError(f"Truncated import object: {path}")
                return struct.unpack_from("<H", data, offset)[0]
            file.seek(start + size + size % 2)
    raise RuntimeError(f"Missing COFF import objects: {path}")


def check_bundle(root, target_arch=None):
    root = Path(root).resolve(strict=True)
    binary_dir = root / "bin"
    if not list(binary_dir.glob("avfilter-*.dll")):
        arch = target_arch or ("arm64" if os.environ.get("PROCESSOR_ARCHITECTURE") == "ARM64" else "x64")
        binary_dir /= arch
    dlls = {}
    for name in ABIS:
        matches = list(binary_dir.glob(name + "-*.dll"))
        if len(matches) != 1:
            raise RuntimeError(f"Expected one {name} DLL in {binary_dir}")
        dlls[name] = matches[0]
    target = MACHINES[target_arch] if target_arch else pe_machine(dlls["avfilter"])
    if target not in MACHINES.values():
        raise RuntimeError(f"Unsupported target PE machine: {target:#x}")
    for name, major in ABIS.items():
        if pe_machine(dlls[name]) != target:
            raise RuntimeError(f"Wrong DLL target architecture: {name}")
        library = root / "lib" / (name + ".lib")
        if import_machine(library) != target:
            raise RuntimeError(f"Wrong import-library target architecture: {name}")
        header = root / "include" / ("lib" + name) / "version_major.h"
        if not header.is_file():
            header = header.with_name("version.h")
        pattern = rf"^\s*#define\s+LIB{name.upper()}_VERSION_MAJOR\s+{major}\s*$"
        if not re.search(pattern, header.read_text(), re.MULTILINE):
            raise RuntimeError(f"Wrong or missing development header ABI: {name}")
    arch = next(name for name, machine in MACHINES.items() if machine == target)
    if pe_machine(sys.executable) != target:
        # The existing Windows ARM64 CI job builds on an x64 Windows host.
        # Loading target DLLs into that host Python is impossible, not a filter failure.
        return {"mode": "structural-only", "target_arch": arch,
                "header_abis": ABIS, "runtime_verified": False,
                "runtime_status": "Deferred: target DLL architecture differs from host Python"}
    # Pin DLL resolution to this bundle, including its transitive dependencies.
    with os.add_dll_directory(str(binary_dir)):
        def load(name):
            return ctypes.CDLL(str(dlls[name]))

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
        return {"mode": "native-runtime", "target_arch": arch, "runtime_verified": True,
                "versions": versions, "filters": required_filters, "encoders": required_encoders}


if __name__ == "__main__":
    try:
        if os.name != "nt":
            raise RuntimeError("Usage on Windows: python check_windows_ffmpeg.py <FFMPEG_DIR>")
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("bundle")
        parser.add_argument("--target-arch", choices=MACHINES)
        args = parser.parse_args()
        print(json.dumps(check_bundle(args.bundle, args.target_arch), sort_keys=True))
    except (OSError, RuntimeError, ValueError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
