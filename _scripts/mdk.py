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

When the nightly's ABI headers and its SDK come from different builds, the SDK
already in ext/mdk-sdk is kept, if its own two halves matched, and the plugin
is built against that one. CI restores ext/ from its cache, so a release build
gets through such a day with the SDK its cache holds. With nothing to keep, or
when the plugin fails to build, a release build (CI, or IN_DOCKER=1 for Linux)
fails; a local build warns and Gyroflow builds without the plugin.
"""
import argparse
import email.utils
import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
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
# libmdk in each SDK, whose build time is compared with the ABI headers'
LIBMDK = {
    "windows": os.path.join("bin", "x64", "mdk.dll"),
    "linux": os.path.join("lib", "amd64", "libmdk.so.0"),
    "macos": os.path.join("lib", "mdk.framework", "Versions", "A", "mdk"),
}
CRM_REPO = "https://github.com/gyroflow/mdk-crm.git"

# Each nightly build uploads the ABI archive first and the SDKs 8-30 minutes later (2026-11-13: ABI 09:33:52 UT,
# Windows 09:42:14, Linux 09:42:41, macOS 10:02:24), and there are several builds a day. A pair is taken as one
# build's when the SDK is no older than the ABI headers and at most PAIR_GAP newer, both by the server's
# Last-Modified and by the archives' own file times (libmdk against the headers' global.h). The archives carry no
# build id, so two builds less than PAIR_GAP apart can't be told apart.
# Some builds upload the ABI archive and no SDK (2026-11-16: a build of another branch than master), and the two
# then stay apart until the next build that uploads both: see fetch_sdk for what is used meanwhile.
PAIR_GAP = 60 * 60
CLOCK_SLACK = 2 * 60  # the archives come from different CI runners
# How long after an ABI upload its SDK may still be on its way
PUBLISH_WAIT = 45 * 60
# What the plugin's note says of a mismatched pair in a gyroflow-mdk.json written before it had "pair"
MISMATCH_NOTE = "different nightly builds"


def log(msg):
    print(f"[mdk] {msg}", flush=True)


def strict():
    """Release builds fail instead of building without the plugin."""
    return bool(os.environ.get("CI")) or os.environ.get("IN_DOCKER") == "1"


def notice(msg):
    """A warning that, in a release build, is also printed as a GitHub Actions annotation, which the run's summary
    page shows. Every release build prints it: the Linux one runs in a container that doesn't see the runner's
    environment, so it can't tell whether it is on GitHub."""
    log(f"WARNING: {msg}")
    if strict():
        escaped = msg.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print(f"::warning title=MDK::{escaped}", flush=True)


def last_modified(headers):
    """The Last-Modified (epoch seconds) of the last response in curl's header dump: SourceForge redirects to a
    mirror, and the file that arrived is the final response's. None when it has none."""
    stamp = None
    for line in headers.splitlines():
        if line.upper().startswith("HTTP/"):
            stamp = None
        elif line.lower().startswith("last-modified:"):
            stamp = line.split(":", 1)[1].strip()
    if not stamp:
        return None
    try:
        return email.utils.parsedate_to_datetime(stamp).timestamp()
    except (TypeError, ValueError):
        return None


# Bounds for curl, so that a stalled connection can't hold a build for ever: a transfer that stays under 1 kB/s for a
# minute is given up, and so is one (with its retries) that takes longer than the total.
CURL_DOWNLOAD = ["--connect-timeout", "30", "--speed-limit", "1024", "--speed-time", "60", "--max-time", "1800",
                 "--retry", "4", "--retry-delay", "5", "--retry-max-time", "2400"]
CURL_HEAD = ["--connect-timeout", "30", "--max-time", "60", "--retry", "4", "--retry-delay", "5", "--retry-max-time", "180"]


def curl(args, what, limit, **kwargs):
    """Runs curl with a time limit of its own on top of curl's; None when it ran out."""
    try:
        return subprocess.run(["curl", *args], timeout=limit, **kwargs)
    except subprocess.TimeoutExpired:
        log(f"{what} timed out")
        return None


def download(name, dst):
    """Downloads a nightly file; returns its sha256 and its Last-Modified (or None)."""
    # curl rather than urllib: some Python builds carry CA stores SourceForge's chain fails against.
    hdr = dst + ".headers"
    r = curl(["-fsSL", *CURL_DOWNLOAD, "-D", hdr, "-o", dst, NIGHTLY + name + "?viasf=1"], f"downloading {name}", 2700)
    if r is None or r.returncode != 0:
        raise SystemExit(f"[mdk] downloading {name} failed")
    with open(hdr, encoding="latin-1") as f:
        modified = last_modified(f.read())
    with open(dst, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest(), modified


def remote_modified(name):
    """The Last-Modified of a nightly file without downloading it (another mirror may answer)."""
    r = curl(["-fsSIL", *CURL_HEAD, NIGHTLY + name + "?viasf=1"], f"asking for {name}", 240,
             capture_output=True, text=True, encoding="latin-1")
    return last_modified(r.stdout) if r is not None and r.returncode == 0 else None


def when(t):
    return time.strftime("%Y-%m-%d %H:%M:%S UT", time.gmtime(t))


def pair_problem(what, abi_time, sdk_time):
    """Why an ABI archive and an SDK from these times can't be one build's, or None."""
    if abi_time is None or sdk_time is None:
        return None
    gap = sdk_time - abi_time
    if -CLOCK_SLACK <= gap <= PAIR_GAP:
        return None
    return (f"{what}: ABI headers {when(abi_time)}, SDK {when(sdk_time)}, "
            f"{'the SDK is older' if gap < 0 else 'over %d minutes apart' % (PAIR_GAP // 60)}")


def extract(archive, dst):
    if archive.endswith(".tar.xz"):
        with tarfile.open(archive) as t:
            # 'data': nothing outside dst (the framework's links are relative)
            t.extractall(dst, **({"filter": "data"} if hasattr(tarfile, "data_filter") else {}))
    else:
        subprocess.run(["7z", "x", "-y", f"-o{dst}", archive], check=True, stdout=subprocess.DEVNULL)


def matched_install(target_os, sdk):
    """The gyroflow-mdk.json of the SDK installed at `sdk`, when its ABI headers and its SDK were taken for one
    build's, so that the plugin can be built against it. None when nothing usable is installed, or its halves
    didn't match."""
    try:
        with open(os.path.join(sdk, "gyroflow-mdk.json"), encoding="utf-8") as f:
            info = json.load(f)
    except (OSError, ValueError):
        return None
    if not isinstance(info, dict):
        return None
    pair = info.get("pair")
    if pair is None:
        pair = "mismatched" if MISMATCH_NOTE in str(info.get("crm_plugin")) else "matched"
    if pair != "matched":
        return None
    for rel in (os.path.join("include", "abi", "mdk", "FrameReader.h"), LIBMDK[target_os]):
        if not os.path.isfile(os.path.join(sdk, rel)):
            return None
    return info


def fetch_sdk(target_os, ext_dir, need_pair=True):
    """Installs the nightly SDK with its ABI headers in ext/mdk-sdk. Returns (the SDK's path, mismatch, kept).

    mismatch: why the installed ABI headers and SDK look like different builds (see PAIR_GAP), when they do; the
    plugin must then not be built against them.

    kept: when the nightly's two archives don't match and the plugin is wanted (need_pair), the SDK already in
    ext/mdk-sdk stays if its own halves matched, and nothing is installed: `kept` is then (that SDK's
    gyroflow-mdk.json, what is wrong with the nightly). With nothing to keep, a release build fails and a local one
    installs the nightly, with `mismatch` set.

    The headers are downloaded before and after the SDK and must not change in between, which catches a build
    uploaded during the download. Their times against the SDK's catch the rest: a build that has uploaded its ABI
    archive but not yet its SDK, or one that uploads no SDK at all."""
    work = os.path.join(ext_dir, ".mdk-download")
    dst = os.path.join(ext_dir, "mdk-sdk")
    old = os.path.join(ext_dir, ".mdk-sdk-old")
    if os.path.exists(old):
        if os.path.exists(dst):
            shutil.rmtree(old)
        else:
            os.rename(old, dst)  # an earlier run stopped between the two renames at the end
    sdk_name = SDK_ARCHIVE[target_os]
    changes = 0
    start = time.time()
    while True:
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(work)
        abi = os.path.join(work, ABI_ARCHIVE)
        sdk = os.path.join(work, sdk_name)
        before, abi_time = download(ABI_ARCHIVE, abi)
        _, sdk_time = download(sdk_name, sdk)
        after, abi_time_after = download(ABI_ARCHIVE, abi)
        if before != after:
            changes += 1
            if changes >= 3:
                raise SystemExit("[mdk] the MDK nightly kept changing during the download")
            log("a new nightly was being published during the download; fetching again")
            time.sleep(60)
            continue
        abi_time = abi_time_after if abi_time is None else abi_time
        if need_pair and abi_time is not None and sdk_time is not None and sdk_time < abi_time - CLOCK_SLACK:
            # The ABI archive of a newer build is up. Its SDK, if that build uploads one, follows within PUBLISH_WAIT
            # of it; the whole wait is no longer than that either.
            deadline = min(start, abi_time) + PUBLISH_WAIT
            if time.time() < deadline:
                log(f"the nightly of {when(abi_time)} is being published; waiting for its SDK")
                arrived = False
                while not arrived and time.time() < deadline:
                    time.sleep(120)
                    t = remote_modified(sdk_name)
                    arrived = t is not None and t >= abi_time - CLOCK_SLACK
                if arrived:
                    continue
        break

    out = os.path.join(work, "out")
    extract(sdk, out)
    extract(abi, out)  # lays include/abi over the SDK
    new_sdk = os.path.join(out, "mdk-sdk")
    if not os.path.isfile(os.path.join(new_sdk, "include", "abi", "mdk", "FrameReader.h")):
        raise SystemExit("[mdk] the downloaded SDK has no ABI headers")

    def file_time(rel):
        p = os.path.join(new_sdk, rel)
        return os.stat(p).st_mtime if os.path.exists(p) else None

    problem = (pair_problem("by upload time", abi_time, sdk_time)
               or pair_problem("by build time", file_time(os.path.join("include", "abi", "mdk", "global.h")),
                               file_time(LIBMDK[target_os])))
    if problem:
        msg = f"the MDK ABI headers and SDK are from {MISMATCH_NOTE} ({problem})"
        if not need_pair:
            log(f"{msg}; no plugin is built for this target, so the SDK is used as it is")
        else:
            info = matched_install(target_os, dst)
            if info:
                shutil.rmtree(work, ignore_errors=True)
                return dst, None, (info, problem)
            if strict():
                raise SystemExit(f"[mdk] {msg}, and ext/mdk-sdk holds no earlier SDK to build the plugin against")
            log(f"WARNING: {msg}")

    # Fresh modification times: qml-video-rs reruns its build script, and copies
    # this libmdk again, when they change.
    now = time.time()
    for root, _, files in os.walk(new_sdk):
        for f in files:
            p = os.path.join(root, f)
            if not os.path.islink(p):
                os.utime(p, (now, now))

    had_old = os.path.exists(dst)
    if had_old:
        os.rename(dst, old)
    try:
        shutil.move(new_sdk, dst)
    except BaseException:
        if had_old:
            shutil.rmtree(dst, ignore_errors=True)  # what a failed move left
            os.rename(old, dst)
        raise
    shutil.rmtree(old, ignore_errors=True)
    shutil.rmtree(work, ignore_errors=True)
    return dst, problem, None


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

def remove_tree(path):
    """shutil.rmtree that also removes read-only files (git's objects on Windows)."""
    def make_writable(func, p, _exc):
        os.chmod(p, stat.S_IWRITE)
        func(p)
    shutil.rmtree(path, **{"onexc" if sys.version_info >= (3, 12) else "onerror": make_writable})


def crm_source(ext_dir, rev):
    local = os.environ.get("MDK_CRM_DIR")
    if local:
        log(f"mdk-crm from MDK_CRM_DIR: {local}")
        return local
    src = os.path.join(ext_dir, "mdk-crm")
    # A checkout git can't work in is cloned again. CI restores ext/ from its cache as the runner's user, and git
    # then refuses it ("dubious ownership") in the Linux build, which runs as root in a container.
    usable = os.path.isdir(os.path.join(src, ".git")) and subprocess.run(
        ["git", "-C", src, "rev-parse", "--verify", "--quiet", "HEAD"], capture_output=True).returncode == 0
    if usable:
        subprocess.run(["git", "-C", src, "fetch", "--quiet", "origin"], check=True)
    else:
        if os.path.lexists(src):
            log("cloning mdk-crm again: git can't use the checkout in ext/mdk-crm")
            remove_tree(src)
        subprocess.run(["git", "clone", "--quiet", CRM_REPO, src], check=True)
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
        r = curl(["-fsSL", *CURL_DOWNLOAD, "-o", archive,
                  "https://github.com/ninja-build/ninja/releases/download/v1.12.1/ninja-win.zip"], "downloading Ninja", 2700)
        if r is None or r.returncode != 0:
            raise RuntimeError("downloading Ninja failed")
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

    # The Canon SDK exists for Windows x64, Linux x64 and macOS (universal).
    supported = a.os == "macos" or a.arch == "x86_64"
    sdk, mismatch, kept = fetch_sdk(a.os, ext_dir, need_pair=supported and not a.no_crm)
    version, abi = abi_info(sdk)
    now = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    info = {"mdk": version, "abi": abi, "pair": mismatch or "matched", "crm_plugin": "not built yet", "fetched": now}
    if kept:
        info["fetched"] = kept[0].get("fetched", "at an unknown time")
        info["nightly_skipped"] = f"{now}: {kept[1]}"
        notice(f"today's MDK nightly was not used: its ABI headers and its SDK are from different builds ({kept[1]}). "
               f"Building with MDK {version} (ABI {abi}) fetched {info['fetched']}, which ext/mdk-sdk held")
    log(f"MDK {version}, ABI {abi} in {sdk}")

    def write_info():
        with open(os.path.join(sdk, "gyroflow-mdk.json"), "w", encoding="utf-8") as f:
            json.dump(info, f, indent=2)

    write_info()  # now, so that a plugin build that fails leaves what is known of the SDK

    if a.no_crm:
        note = "not built (--no-crm)"
    elif not supported:
        note = f"not built: no Canon SDK for {a.os} {a.arch}"
    elif mismatch:
        # fetch_sdk has already failed a release build for this
        note = f"not built: the ABI headers and the SDK are from {MISMATCH_NOTE} ({mismatch})"
        log("WARNING: the CRM plugin was not built; Gyroflow builds without .CRM playback. Run again later.")
    else:
        try:
            note = build_crm(a.os, ext_dir, sdk, a.crm_rev)
        except (RuntimeError, subprocess.CalledProcessError, OSError) as e:
            if strict():
                raise SystemExit(f"[mdk] building the CRM plugin failed: {e}")
            note = f"not built: {e}"
            log(f"WARNING: the CRM plugin was not built ({e}); Gyroflow builds without .CRM playback")
    log(f"CRM plugin: {note}")
    info["crm_plugin"] = note
    write_info()


if __name__ == "__main__":
    main()
