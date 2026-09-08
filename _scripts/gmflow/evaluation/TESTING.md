# Validation record

The general finite-pose fix and the experimental GMFlow integration have different
release status. The finite-pose patch contains the same two regression tests and
guard used in the successful unit and rendered-video checks below.

| Check | Observed result |
| --- | --- |
| Core library, GMFlow enabled | 24 tests passed |
| Boolean storage adapter | 3 tests passed |
| Core library, default features | 24 tests passed |
| Pinned model exporter rerun | Same ONNX checksum; CPU parity passed |
| Actual GPU frame API, five controlled cases | Passed before and after workspace cleanup; unchanged coordinates |
| Actual GPU frame API, two DAVIS pairs | Finite, in-bounds correspondences; no ground-truth accuracy claim |
| Worker lifecycle after cleanup | Pre-cancel, four concurrent cache requests, and reload after idle passed |
| Full-app command line | Version and successful overwrite exit 0; missing input and untrackable video exit 1 |
| Full-app two-frame visual-offset analysis | Exit 0; rendered 26 frames after workspace cleanup |
| Two queued jobs with an unavailable GPU index | Exit 1; both report failure, no output produced |
| Full-resolution queue mode, final app build | Exit 0; output created from the 26-frame fixture |
| GUI | Final UI generation loaded a preset with GMFlow selected; normal close returned 0 |
| Demonstration videos | All frames decoded: 90, 90 and 133; representative frames visually inspected |

All seven CLI cases were rerun successfully against the final application build,
including full-resolution scaling, tightened luminance dimensions, workspace
cleanup, and error propagation. The GPU frame-API/lifecycle checks exercise the
final inference implementation.

The final successful unit rerun includes the tightened luminance-bound assertions:
24 core tests with GMFlow, 3 adapter tests, and 24 default-feature core tests passed.
An earlier rebuild failed with an `exr` metadata error during severe host commit-
memory pressure. The unchanged sources passed after memory became available.
The final complete application also built successfully.

## Compiler and resource configuration

Rust 1.98.1 was invoked explicitly, leaving the default toolchain unchanged. A
standard release build of the full application succeeded earlier. Later compiler
out-of-memory failures required task-local Cargo overrides:

- Final application: `profile.release.package.gyroflow.opt-level=1` and
  `profile.release.package.gyroflow-core.opt-level=1`; generated model and dependency
  release settings unchanged; one build job.
- Final GPU examples: model host code `opt-level=0`, `codegen-units=1`; core
  `opt-level=1`; other release settings unchanged; one build job.
- Final successful feature unit run: model host code `opt-level=0`,
  `codegen-units=1`; core `opt-level=1`. Earlier unit runs also passed with the
  core's normal release setting.

These are build-time resource settings, not a modified GPU graph, model precision,
or required end-user adapter configuration. No driver, TDR, pagefile, security,
or global Rust settings were changed. No binary is distributed.

The app used task-local Qt 6.7.3, OpenCV 4, FFmpeg 9.0 and MDK runtime SDKs on
Windows. Wgpu inference was validated on NVIDIA RTX 5070 Ti, driver 591.86. The
tested integrated AMD gfx1036 device failed full-model inference on both Vulkan
and DX12; that remains an unresolved release blocker for that device.

## Lints and source review

New Rust files were formatted with rustfmt; Python export/reproduction files with
Black. Whitespace checking uses `core.whitespace=cr-at-eol` because upstream stores
the existing source files with CRLF endings. Existing line endings were retained
to avoid unrelated whole-file diffs.

The standard repository Clippy run fails seven existing errors in
`gyro_source/splines.rs`, `lens_profile.rs`, `stabilization/cpu_undistort.rs`, and
`filesystem/mod.rs`. A scoped run allowing only those existing categories
(`absurd_extreme_comparisons`, `erasing_op`, `unused_io_amount`) completed, with
566 legacy warnings and no warnings in the added GMFlow/context/example modules
at that review point. The subsequent small memory-cleanup, CLI propagation and
full-resolution edits were build/runtime reviewed; a clean whole-repository
Clippy result is not claimed.

The independent finite-pose patch was checked for applicability to the pinned
upstream revision. The larger source patch contains that fix already.
