# Windows LUT verification checkpoint

Tested 2026-10-07 on Windows 11 Home build 26300, Qt 6.7.3, Intel Arc 140T and
NVIDIA RTX 5070 Ti Laptop. This checkpoint is **not** a completed Windows app
build, installation or stabilization test.

## Fixed and verified

- Commit `8c21f04e76719dca9290bf3ac617d3f53ceb689f` fixes Direct3D HLSL error X3500
  from dynamically indexed vector writes. The QSB retains serialization version
  6 and GLSL 150/300 ES, HLSL 5.0 and MSL 1.2.
- Six actual Qt shader renders pass on each GPU: a size-33 DJI LUT at +10%
  brightness/+20% contrast, neutral/no LUT, and all four adjustment extremes
  without a LUT. The LUT render differs from the supplied production reference
  by at most one RGB24 level (83 Intel / 1,087 NVIDIA components out of 49,152).
  Neutral and extremes match exactly. Fully opaque alpha is verified; partial
  alpha is not. The 150% DPI grab is normalized with nearest-neighbor sampling.
- The previous avbuild GPL-lite bundle has neither `lut3d` nor `geq`. Its actual
  shared DLLs fail the new preflight, confirming that it cannot supply this feature.
- Windows now selects BtbN's pinned FFmpeg 9.0.2 GPL shared bundle from
  `autobuild-2026-10-06-13-06`, with SHA-256 verification before extraction.
  The x64 archive hash is
  `9d971d67fea48100b623e65d3ffba480215244931492b23a41374cec7511b1ce`.
  ARM64 has a separately pinned archive/hash; its archive hash, seven target DLL
  PE machines, seven MSVC import-library COFF machines and header ABI majors pass
  structural checks on this x64 laptop. ARM64 runtime is untested.
- The replacement bundle's actual x64 DLLs expose FFmpeg 9 ABIs (avfilter 12,
  avcodec 63, avutil 61), required filters, libx264/libx265, ProRes and FFV1,
  with development headers and MSVC `.lib` import libraries.
- Its standalone CLI processed all 857 frames of the supplied DJI clip through
  the LUT/+10%/+20% filter chain, writing a 1280x720 ten-bit FFV1 MKV. Full decode
  completed without errors; output duration is 14.298 seconds. This is an
  external FFmpeg dependency test, **not Gyroflow stabilization/export proof**.
- Synthetic 16-frame 720p encodes and full decodes pass for libx264 ten-bit,
  libx265 ten-bit, NVIDIA H.264 eight-bit and NVIDIA HEVC ten-bit. Listing an
  encoder alone was not counted as runtime proof.
- Source clip and supplied LUT SHA-256 hashes remain unchanged. Private fixtures,
  generated media, logs, tools and build caches remain outside tracked source.
- Dependency setup reuses its local vcpkg checkout and retains caches. The old
  rooted `\vcpkg` removal and unnecessary recursive cache cleanup are removed.

## Reproduce the completed checks

Python's standard library suffices for the DLL preflight. On a matching native
host it loads the actual DLLs and requires all filters/encoders. The existing
ARM64 CI job runs on x64 Windows, so cross builds instead validate the target's
PE/COFF machines, import libraries and header ABIs, and explicitly report
`structural-only`, `runtime_verified: false`. They do not attempt to load ARM64
DLLs in x64 Python. The install recipe supplies `--target-arch` to reject an
incorrectly selected bundle. Native runtime checks remain strict:

```powershell
python tests/export-lut/check_windows_ffmpeg.py $env:FFMPEG_DIR
```

On this laptop, the real pinned ARM64 bundle passes structural checks with
`--target-arch arm64`. Passing `--target-arch x64` to that same bundle fails with
`Wrong DLL target architecture`; the native x64 GPL-lite bundle still fails on
missing `lut3d`/`geq`. No ARM64 runtime or complete ARM64 CI build is claimed.

For actual Direct3D shader checks, install Pillow in an isolated environment and
provide the private fixture folder containing `input.png`, `atlas.png`, and
`expected.rgb` produced by `preview_fixture` for a size-33 LUT at .1/.2. The
helper uses Qt's actual shader renderer, records its adapter log and images,
checks opaque alpha and rejects RGB differences greater than one level:

```powershell
python tests/export-lut/check_preview.py <qml.exe> <fixture-folder> <output-folder> 0
python tests/export-lut/check_preview.py <qml.exe> <fixture-folder> <output-folder> 1
```

Adapter indices here are this laptop's Intel and NVIDIA respectively; consult
the saved logs on other machines. Selection is limited to each test process.
`just -f _scripts/windows.just --dry-run install-deps` parses the new recipe;
x64 and ARM64 variable evaluation select their matching pinned packages/hashes.
The entire dependency recipe has not been run successfully.

## Current blocker and resume

Rust 1.99.0, Qt 6.7.3, the pinned FFmpeg bundle, Just 1.58.0, a local Python
environment with libclang 18.1.1, and a bootstrapped local vcpkg checkout are ready.
OpenCV is not built yet. The Microsoft Visual Studio Build Tools 2026 installer
timed out at Windows' administrator prompt (exit 1602). The production command
`cargo test --locked --manifest-path tests/export-lut/Cargo.toml` fails with
`linker link.exe not found` (exit 101); **no Rust tests passed on Windows yet**.

Once the user is present to approve Windows' normal administrator prompt, resume
with the same official Microsoft package, without changing security settings:

```powershell
winget install --id Microsoft.VisualStudio.BuildTools --exact --version 18.10.2 --source winget --accept-source-agreements --accept-package-agreements --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --locale en-US"
```

Use a fresh Visual Studio developer terminal. In the isolated source checkout,
add the prepared Cargo/Just/Qt/local Python directories to that terminal's PATH,
set `FFMPEG_DIR` to the pinned bundle and `FFMPEG_ARCH=x64`, and set
`LIBCLANG_PATH` to the local environment's `Lib/site-packages/clang/native`.
Keep `CARGO_TARGET_DIR` in the checkout's ignored `ext/` directory, outside
OneDrive. Run the 11 production parser/filter tests, finish OpenCV dependencies,
build the full release app, and package a separate portable **Gyroflow LUT Preview**
using its matching runtime DLLs. Existing official Gyroflow and user settings
must remain preserved.

Required remaining gates: execute the full app under current Windows protections;
verify its actual GPU/backend and controls; chosen/invalid/cleared LUT and reset;
comparison/playback framing; project, preset and job persistence; actual Gyroflow
neutral-versus-colored stabilized frame/timestamp comparison on the supplied
clip; full output decode/precision/metadata; then install and verify the portable
app. This environment's computer-use surface currently does not expose native
app control, so visual UI evidence also remains outstanding. Do not substitute
the completed external-tool checks for those gates.

Git push on this laptop waits for Git Credential Manager sign-in. The bounded
local commits can instead be transferred as a git bundle for review/publishing
through the already authenticated Mac session. Until that session confirms a
push, local commits must not be described as published.
