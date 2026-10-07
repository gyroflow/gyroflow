# Windows LUT verification checkpoint

Tested 2026-10-07 on Windows 11 Home build 26300, Qt 6.7.3, Intel Arc 140T and
NVIDIA RTX 5070 Ti Laptop. The resumed release build, software
stabilization/export and portable startup are verified below. Complete Windows
visual UI checks and current NVIDIA encoding remain outstanding.

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

## Resumed Windows build

The official Microsoft Visual Studio Build Tools 2026 installation completed
successfully with MSVC 14.51.36231 and Windows SDK 10.0.26100.0. The normal
installation prompt was resolved by the user. No security setting was changed
by this work.

All **11 production parser/filter tests pass on Windows** with the pinned native
FFmpeg bundle and libclang 18.1.1:

```powershell
cargo test --locked --manifest-path tests/export-lut/Cargo.toml
```

These tests exercise the production Rust parser/filter code, including bounded
reads and errors, identity/float precision and timestamps, non-square geometry,
LUT/adjustment alpha, the four no-LUT adjustment extremes, ten-bit YUV properties
and graph size changes. Logs are retained in the ignored local build directory.
They do not replace full application stabilization/export checks.

OpenCV 4.14.0, FlatBuffers 25.12.19 and OpenCL installed successfully through the
local vcpkg checkout. The full release application build completed successfully,
using Rust 1.99.0, Qt 6.7.3 and the same pinned FFmpeg bundle. Existing upstream
unused-variable/dead-code warnings remain; no warning-free build is claimed. Build products stay
in the ignored checkout-local `ext/cargo-app` directory, outside OneDrive.
Copies of the supplied project have Windows file URLs and separate neutral and
colored output paths; original clip, LUT and project remain preserved.

The first application build exposed two compiler setup problems: the local
libclang wheel did not include a `clang.exe`, and LLVM 19.1.7 was rejected by the
new MSVC headers (`STL1000`, requires Clang 20 or newer). Windows setup now pins
LLVM 20.1.8 portable archives, verifies the official release SHA-256, extracts
only the compiler, matching libclang and resource headers, and checks the
compiler version before reuse. The package follows the **build host** architecture,
so an x64-to-ARM64 cross build uses x64 Clang. No installer or global PATH change
is required. The x64 archive hash is
`f229769f11d6a6edc8ada599c0cda964b7dee6ab1a08c6cf9dd7f513e85b107f`;
ARM64 is separately pinned to
`0df3e81e8fe26370dd2b60b9e009d81cd130d3fdc41b257434aa663c5d9f0c13`.
Actual x64 compiler execution and the complete build passed; ARM64 compiler
execution is untested. Recipe parsing and the version-readiness check also pass.
The local MDK 0.39.0 playback SDK was extracted separately and supplied with
`MDK_SDK`; its archive hash is
`5620e05359f692f6d9ecabc6e49bf62adf4e061f568a18355210bfac70f716a5`.

### Full application and export evidence

- The actual release application starts and reports its version (exit 0).
- Neutral and DJI LUT/+10% brightness/+20% contrast stabilized exports both
  complete all 857 frames of the supplied moving-drone clip with software
  lossless HEVC encoding at 1280x720. Both retain 60000/1001 fps, 14.297617 seconds,
  limited-range BT.709 and ten-bit YUV420.
- Independent FFmpeg color processing of the neutral stabilized export matches
  **every decoded frame hash and timestamp exactly** against the colored
  Gyroflow export. Both videos decode completely without errors. The colored
  export contains both LUT and adjustment metadata markers. This verifies that
  the new color processing preserves stabilization/framing for this clip.
- `tests/export-lut/check_app_exports.py` reproduces the comparison with
  user-supplied lossless outputs and a LUT. It requires an explicit expected
  frame count, uses a new output directory, and keeps copied fixtures and results
  outside tracked source. It is an opt-in integration check, not a default test.
- A malformed size-33 LUT with incomplete data is explicitly refused:
  `Could not apply the selected export LUT: The LUT is incomplete.` No completed
  output video is created. The inherited CLI still exits 0 for failed jobs;
  return code alone is not counted as export success.
- Production project export/reload retains the chosen LUT and +10%/+20%
  settings. Applying a separate preset to a queued project retains its LUT and
  +17%/-23% adjustments in the exported project.
- The original supplied clip and LUT still match their initial SHA-256 hashes.
- A separate portable app has Qt/QML plugins, matching FFmpeg/MDK/OpenCV DLLs,
  Microsoft runtime DLLs and official lens profiles v41. Its version check passes
  with only Windows system directories on PATH. A user-visible copy is prepared
  under the user's Documents folder, leaving the official app separate.
- Native UI accessibility confirms the app loaded the chosen project, LUT Clear
  control, brightness 10, contrast 20, Preview colors and Reset adjustments.
  Desktop input is currently denied (`GetCursorPos: Access is denied`), and the
  screenshot shows the screensaver rather than usable app pixels. **This is not
  visual preview/control proof.** No lock or display setting was changed.
- A fresh full-app NVIDIA HEVC test using encoder-supported P010 fails with
  `CUDA_ERROR_NO_DEVICE` / `no encode device`. A fresh standalone FFmpeg NVENC
  test fails the same way; NVIDIA-SMI reports insufficient permissions. Earlier
  synthetic NVIDIA dependency tests passed, but current full-app NVIDIA encoding
  is therefore **not verified**. No driver, security or graphics settings changed.

To repeat the actual frame comparison on explicit private fixtures:

```powershell
python tests/export-lut/check_app_exports.py <ffmpeg-bin> <neutral-lossless.mp4> <colored-lossless.mp4> <user-supplied.cube> <new-result-folder> --expected-frames 857
```

Required remaining gates: obtain usable native desktop access; visually verify
chosen/invalid/cleared LUT, reset, comparison/playback framing and project/preset/
job persistence; resolve the current NVIDIA device-access failure without
changing security settings; verify the final installed app and launcher through
the normal interactive desktop. Preserve the official app and user settings.

The Windows fixes through `034f118a56cfe43bce362bbf3d6a7ae68e15a0bd` were reviewed
and published through the authenticated Mac session. The feature branch remote
head was independently verified from Windows. Git Credential Manager sign-in on
this laptop is therefore not a current blocker. New changes after that commit
remain local until separately verified and published.
