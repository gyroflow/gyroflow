# Windows LUT verification checkpoint

Tested 2026-10-07 on Windows 11 Home build 26300, Qt 6.7.3, Intel Arc 140T and
NVIDIA RTX 5070 Ti Laptop. The resumed release build, software and NVIDIA
stabilization/export, interactive LUT controls, restart persistence and portable
desktop shortcut are verified below. ARM64 runtime verification remains open.

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
- Initial desktop input was denied and captures showed the screensaver; those
  observations were not counted as visual proof. Later usable desktop access
  allowed the interactive checks below without changing security settings.
- Initial full-app NVIDIA HEVC and standalone NVENC tests failed with
  `CUDA_ERROR_NO_DEVICE`. The installed G-Helper utility was not running, so its
  saved automatic GPU preference had not enabled the NVIDIA device. Starting
  that existing utility applied the saved preference; the device became present
  and both standalone NVENC and a full-app export succeeded. The latter completed
  all 857 frames in 51.462 seconds at 1280x720, 60000/1001 fps, ten-bit HEVC/P010.
  Independent probing counted 857 frames and full FFmpeg decoding passed with
  `-v error -xerror`. This hardware export uses lossy encoding; the exact frame
  hash proof above applies to the software lossless exports. The charger was
  reported as 70 W by the user, who accepts battery supplementation. No driver
  or security setting was changed, and no new G-Helper autostart was installed.

### Interactive installation and preview correction

The portable executable must keep the upstream name **`gyroflow.exe`** (the
standard packaging recipe uses `Gyroflow.exe`). A custom local installation had
renamed it `GyroflowLUTPreview.exe`, triggering a large MDK QR overlay in the
video preview. Restoring the upstream filename, with identical executable bytes
and unchanged embedded upstream key, removed the overlay. Playback then worked
without the obstruction. Keep the separate folder and desktop shortcut named
"Gyroflow LUT Preview" to distinguish this build; do not rename its executable.
The corrected desktop shortcut was launched and verified interactively after
closing the earlier instance. The incorrectly named copy was moved to the
private test-fixture backup directory.

Actual installed-app UI checks passed:

- The selected official user-supplied DJI LUT appears by name with its green
  export/preview status. Playback advances through the moving clip without a QR
  overlay. On the same paused frame (793/857), switching Preview colors off and
  back on visibly changes the colors while preserving frame position, zoom and
  framing; the export LUT remains selected.
- Reset adjustments changes both fields to zero, disables itself and retains
  the LUT. Clear removes the LUT status and returns to the ungraded preview.
  Selecting the disposable incomplete LUT shows the red `The LUT is incomplete.`
  error. Selecting the valid LUT again clears that error and restores its status.
- Editing brightness to +17% and contrast to -23% visibly updates the preview.
  Saving the private project writes `brightness: 0.17`, `contrast: -0.23` and the
  exact chosen LUT file URL. Reopening that file through the GUI after restarting
  restores all three controls.
- Create settings preset includes the checked LUT and brightness/contrast
  selections. Save to file preserves those values and leaves `output_filename`
  empty. After restarting, clearing/resetting the current colors and opening the
  saved preset through the GUI restores the LUT and +17%/-23% adjustments.
- Adding a disposable job to the queue leaves it unrendered. Closing the app and
  launching its corrected shortcut presents the unfinished-queue prompt. Opening
  that queue and choosing Edit restores the same LUT and +17%/-23% adjustments.
  No originals or existing output videos were overwritten.

To repeat the actual frame comparison on explicit private fixtures:

```powershell
python tests/export-lut/check_app_exports.py <ffmpeg-bin> <neutral-lossless.mp4> <colored-lossless.mp4> <user-supplied.cube> <new-result-folder> --expected-frames 857
```

These Windows x64 LUT acceptance gates are now passed on this laptop. This does
not establish ARM64 runtime support, partial-alpha GPU correctness or broad
camera/hardware coverage. Preserve the official app and user settings.

The Windows fixes through `034f118a56cfe43bce362bbf3d6a7ae68e15a0bd` were reviewed
and published through the authenticated Mac session. The feature branch remote
head was independently verified from Windows. Git Credential Manager sign-in on
this laptop is therefore not a current blocker. New changes after that commit
remain local until separately verified and published.
