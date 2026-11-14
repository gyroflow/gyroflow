#!/usr/bin/env python3
"""Fetch the MDK SDK into ext/mdk-sdk and build the Canon CRM plugin against it.

    python _scripts/mdk.py --os windows|linux|macos --arch x86_64|aarch64 [--crm-rev REV] [--no-crm]

MDK plugins use its C++ ABI, which changes between nightlies without notice, so a
plugin works only with the libmdk it was built against. This script downloads
the nightly SDK and its ABI headers once, builds mdk-crm
(https://github.com/gyroflow/mdk-crm) against them, and installs the plugin beside
MDK's own (mdk-braw, mdk-r3d). qml-video-rs builds against the same directory
through MDK_SDK and copies libmdk and the plugins next to Gyroflow from it.

MDK_CRM_DIR builds the plugin from a local checkout instead of the pinned one.
In release builds (CI, or IN_DOCKER=1 for Linux) a plugin that fails to build
fails the script; locally it warns and Gyroflow builds without it.
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tarfile
import time
import zipfile

NIGHTLY = "https://master.dl.sourceforge.net/project/mdk-sdk/nightly/"
SDK_ARCHIVE = {
    "windows": "mdk-sdk-windows-clang.7z",
    "linux": "mdk-sdk-linux.tar.xz",
    "macos": "mdk-sdk-macOS.tar.xz",
}
ABI_ARCHIVE = "mdk-abi-sdk.7z"
CRM_REPO = "https://github.com/gyroflow/mdk-crm.git"


def log(msg):
    print(f"[mdk] {msg}", flush=True)


def download(name, dst):
    # curl rather than urllib: some Python builds carry CA stores SourceForge's chain fails against.
    subprocess.run(["curl", "-fsSL", "--retry", "4", "--retry-delay", "5", "-o", dst, NIGHTLY + name + "?viasf=1"],
                   check=True)
    with open(dst, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def extract(archive, dst):
    if archive.endswith(".tar.xz"):
        with tarfile.open(archive) as t:
            # 'data': nothing outside dst (the framework's links are relative)
            t.extractall(dst, **({"filter": "data"} if hasattr(tarfile, "data_filter") else {}))
    else:
        subprocess.run(["7z", "x", "-y", f"-o{dst}", archive], check=True, stdout=subprocess.DEVNULL)


def fetch_sdk(target_os, ext_dir):
    """The SDK and the ABI headers are separate files, uploaded at different times by
    each nightly build. Fetching the headers before and after the SDK and requiring
    them to match keeps a build that lands in between from mixing two nightlies."""
    work = os.path.join(ext_dir, ".mdk-download")
    sdk_name = SDK_ARCHIVE[target_os]
    for attempt in range(3):
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(work)
        abi = os.path.join(work, ABI_ARCHIVE)
        sdk = os.path.join(work, sdk_name)
        before = download(ABI_ARCHIVE, abi)
        download(sdk_name, sdk)
        after = download(ABI_ARCHIVE, abi)
        if before == after:
            break
        log("a new nightly was being published during the download; fetching again")
        time.sleep(60)
    else:
        raise SystemExit("[mdk] the MDK nightly kept changing during the download")

    out = os.path.join(work, "out")
    extract(sdk, out)
    extract(abi, out)  # lays include/abi over the SDK
    new_sdk = os.path.join(out, "mdk-sdk")
    if not os.path.isfile(os.path.join(new_sdk, "include", "abi", "mdk", "FrameReader.h")):
        raise SystemExit("[mdk] the downloaded SDK has no ABI headers")

    # Fresh modification times: qml-video-rs reruns its build script, and copies
    # this libmdk again, when they change.
    now = time.time()
    for root, _, files in os.walk(new_sdk):
        for f in files:
            p = os.path.join(root, f)
            if not os.path.islink(p):
                os.utime(p, (now, now))

    dst = os.path.join(ext_dir, "mdk-sdk")
    old = os.path.join(ext_dir, ".mdk-sdk-old")
    shutil.rmtree(old, ignore_errors=True)
    if os.path.exists(dst):
        os.rename(dst, old)
    shutil.move(new_sdk, dst)
    shutil.rmtree(old, ignore_errors=True)
    shutil.rmtree(work, ignore_errors=True)
    return dst


def abi_info(sdk):
    text = open(os.path.join(sdk, "include", "abi", "mdk", "global.h"), encoding="utf-8").read()

    def define(name):
        for line in text.splitlines():
            parts = line.split()
            if len(parts) == 3 and parts[0] == "#define" and parts[1] == name:
                return parts[2]
        return "?"

    version = ".".join(define(f"MDK_{k}") for k in ("MAJOR", "MINOR", "MICRO"))
    return version, define("MDK_ABI_VERSION")


# ─────────────────────────────────────────────────────────────────────
# Building the plugin
# ─────────────────────────────────────────────────────────────────────

def crm_source(ext_dir, rev):
    local = os.environ.get("MDK_CRM_DIR")
    if local:
        log(f"mdk-crm from MDK_CRM_DIR: {local}")
        return local
    src = os.path.join(ext_dir, "mdk-crm")
    if not os.path.isdir(os.path.join(src, ".git")):
        shutil.rmtree(src, ignore_errors=True)
        subprocess.run(["git", "clone", "--quiet", CRM_REPO, src], check=True)
    else:
        subprocess.run(["git", "-C", src, "fetch", "--quiet", "origin"], check=True)
    ref = rev if rev != "master" else "origin/master"
    subprocess.run(["git", "-C", src, "checkout", "--quiet", "--detach", ref], check=True)
    head = subprocess.run(["git", "-C", src, "rev-parse", "--short", "HEAD"], check=True,
                          capture_output=True, text=True).stdout.strip()
    log(f"mdk-crm {head}")
    return src


def msvc_env():
    """The environment of a VS developer prompt for x64."""
    vswhere = os.path.join(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)"),
                           r"Microsoft Visual Studio\Installer\vswhere.exe")
    vs = subprocess.run([vswhere, "-latest", "-products", "*", "-requires",
                         "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"],
                        check=True, capture_output=True, text=True).stdout.strip()
    bat = os.path.join(vs, r"VC\Auxiliary\Build\vcvars64.bat")
    out = subprocess.run(f'cmd /s /c ""{bat}" >nul && set"', check=True, capture_output=True, text=True).stdout
    return dict(line.split("=", 1) for line in out.splitlines() if "=" in line)


def which(names, env, extra_dirs=()):
    path = os.pathsep.join([*extra_dirs, env.get("PATH") or env.get("Path") or ""])
    for n in names:
        p = shutil.which(n, path=path)
        if p:
            return p
    return None


def newest_clang(env):
    """The newest clang with a matching clang++; MDK's ABI headers need C++23."""
    for v in range(30, 16, -1):
        cc, cxx = which([f"clang-{v}"], env), which([f"clang++-{v}"], env)
        if cc and cxx:
            return cc, cxx
    return which(["clang"], env), which(["clang++"], env)


def build_crm(target_os, ext_dir, sdk, rev):
    src = crm_source(ext_dir, rev)
    build = os.path.join(src, "build-gyroflow")
    shutil.rmtree(build, ignore_errors=True)  # compilers or the SDK may have changed
    env = dict(os.environ)
    # The Rust core's cargo cache stays in ext/, also when building from MDK_CRM_DIR.
    args = ["-S", src, "-B", build, "-DCMAKE_BUILD_TYPE=RelWithDebInfo", f"-DMDKSDK={sdk}",
            f"-DCORE_TARGET_DIR={os.path.join(ext_dir, 'mdk-crm-target')}"]

    if target_os == "windows":
        env = msvc_env()
        llvm = [d for d in (os.environ.get("LIBCLANG_PATH"), os.path.join(ext_dir, "llvm", "bin"),
                            r"C:\Program Files\LLVM\bin") if d]
        # clang-cl: MSVC 17.x rejects a constexpr in MDK's headers, and libmdk itself is built with clang-cl.
        clang_cl = which(["clang-cl"], env, llvm)
        if not clang_cl:
            raise RuntimeError("clang-cl not found (install LLVM, or run install-deps)")
        args += [f"-DCMAKE_C_COMPILER={clang_cl}", f"-DCMAKE_CXX_COMPILER={clang_cl}"]
    elif target_os == "linux":
        cc, cxx = newest_clang(env)
        if not cxx:
            raise RuntimeError("clang++ not found; the plugin needs clang and libc++ (libc++-dev, libc++abi-dev)")
        args += [f"-DCMAKE_C_COMPILER={cc}", f"-DCMAKE_CXX_COMPILER={cxx}"]
    else:
        subprocess.run(["rustup", "target", "add", "aarch64-apple-darwin", "x86_64-apple-darwin"], check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        args += ["-DCMAKE_OSX_ARCHITECTURES=arm64;x86_64",
                 f"-DCMAKE_OSX_DEPLOYMENT_TARGET={os.environ.get('MACOSX_DEPLOYMENT_TARGET', '10.15')}"]

    cmake = which(["cmake"], env)
    if not cmake:
        raise RuntimeError("cmake not found")
    ninja = which(["ninja"], env, [os.path.join(ext_dir, "ninja")])
    if not ninja and target_os == "windows":
        # clang-cl needs Ninja (or a VS installation with its ClangCL toolset)
        os.makedirs(os.path.join(ext_dir, "ninja"), exist_ok=True)
        archive = os.path.join(ext_dir, "ninja", "ninja-win.zip")
        subprocess.run(["curl", "-fsSL", "--retry", "4", "-o", archive,
                        "https://github.com/ninja-build/ninja/releases/download/v1.12.1/ninja-win.zip"], check=True)
        with zipfile.ZipFile(archive) as z:
            z.extract("ninja.exe", os.path.join(ext_dir, "ninja"))
        ninja = which(["ninja"], env, [os.path.join(ext_dir, "ninja")])
    if ninja:
        args += ["-G", "Ninja", f"-DCMAKE_MAKE_PROGRAM={ninja}"]
    elif target_os == "windows":
        raise RuntimeError("ninja not found")
    # The caller's target dir would otherwise also hold the plugin's cargo build.
    env.pop("CARGO_TARGET_DIR", None)
    subprocess.run([cmake, *args], check=True, env=env)
    subprocess.run([cmake, "--build", build, "--parallel"], check=True, env=env)

    if target_os == "windows":
        files, dst = ["mdk-crm.dll", "mdk-crm.pdb"], os.path.join(sdk, "bin", "x64")
    elif target_os == "linux":
        files, dst = ["libmdk-crm.so"], os.path.join(sdk, "lib", "amd64")
    else:
        files, dst = ["libmdk-crm.dylib"], os.path.join(sdk, "lib", "mdk.framework", "Versions", "A")
    if not os.path.exists(os.path.join(build, files[0])):
        raise RuntimeError(f"the build produced no {files[0]}")
    for f in files:
        p = os.path.join(build, f)
        if os.path.exists(p):
            shutil.copy2(p, dst)
    return os.path.join(dst, files[0])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--os", required=True, choices=sorted(SDK_ARCHIVE))
    ap.add_argument("--arch", default="x86_64")
    ap.add_argument("--ext", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "ext"))
    ap.add_argument("--crm-rev", default="master", help="mdk-crm commit, tag or 'master'")
    ap.add_argument("--no-crm", action="store_true", help="only fetch the SDK")
    a = ap.parse_args()
    ext_dir = os.path.abspath(a.ext)
    os.makedirs(ext_dir, exist_ok=True)

    sdk = fetch_sdk(a.os, ext_dir)
    version, abi = abi_info(sdk)
    log(f"MDK {version}, ABI {abi} in {sdk}")

    # The Canon SDK exists for Windows x64, Linux x64 and macOS (universal).
    supported = a.os == "macos" or a.arch == "x86_64"
    if a.no_crm:
        note = "not built (--no-crm)"
    elif not supported:
        note = f"not built: no Canon SDK for {a.os} {a.arch}"
    else:
        try:
            note = build_crm(a.os, ext_dir, sdk, a.crm_rev)
        except (RuntimeError, subprocess.CalledProcessError, OSError) as e:
            if os.environ.get("CI") or os.environ.get("IN_DOCKER") == "1":
                raise SystemExit(f"[mdk] building the CRM plugin failed: {e}")
            note = f"not built: {e}"
            log(f"WARNING: the CRM plugin was not built ({e}); Gyroflow builds without .CRM playback")
    log(f"CRM plugin: {note}")

    with open(os.path.join(sdk, "gyroflow-mdk.json"), "w", encoding="utf-8") as f:
        json.dump({"mdk": version, "abi": abi, "crm_plugin": note,
                   "fetched": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}, f, indent=2)


if __name__ == "__main__":
    main()
