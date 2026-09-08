# Residual optical stabilization

This branch adds a second, spatially varying stabilization pass after the camera
transform, plus stricter optical tracking. It addresses the architecture requested
in [#45](https://github.com/gyroflow/gyroflow/issues/45#issuecomment-1765525467).
It is a review candidate, not a claim that every scene or supported GPU has been
validated. AI assistance was used to implement and review the change.

## Use

Load the video and its lens profile. In Stabilization, increase **Optical
stabilization** from its default of 0%, then select **Analyze optical motion**.
The whole clip is tracked. Existing valid gyro synchronization is preserved;
without usable synchronized gyro data, camera motion is estimated first.
The Synchronization panel selects the tracker. DIS does not require model
weights; the optional GMFlow tracker requires the [documented feature build](../gmflow/README.md).

Projects retain the normalized source tracks and strength. Lens, smoothing and
output changes reuse those observations and rebuild the correction. Presets can
set `stabilization.optical_stabilization_strength` in [0, 1]. For unattended
analysis, also set `synchronization.do_autosync: true`; missing observations then
trigger whole-video analysis in the render queue. Disable autosync to replay
existing project data without another analysis. Failed or cancelled analysis does
not publish partial tracks or replace camera motion.

## Model and integration

- DIS and GMFlow check forward/backward consistency and local photometric
  agreement, accounting for decoder row padding. GMFlow runs the network in both
  directions; older one-direction timing figures do not describe this version.
- Up to 256 spatially balanced tracks per frame pair are stored in normalized
  source coordinates with original decoded video timestamps. Sensor offsets and
  frame-rate overrides do not change that persisted time base.
- Tracks are projected into the camera-stabilized plane with the current lens,
  rolling shutter, camera smoothing and keyframed lens correction. A robust
  affine background estimate rejects motion outliers. Smooth local residuals
  define a 9 x 7 vertex displacement field.
- Vertex trajectories use a local linear temporal fit over a +/-0.5 second
  window. Corrections fade to identity at unsupported segment boundaries. Poor
  spatial support and missing frame pairs break a trajectory instead of holding
  a stale warp across a gap. This is offline processing, not a real-time method.
- Each correction is bounded to 4% of the image dimensions and a displacement
  Jacobian infinity norm of 0.2. Eight fixed-point iterations invert the resulting
  contraction. The forward map participates in adaptive crop calculation; CPU,
  WGSL, OpenCL and Qt preview apply its inverse before the existing camera/lens
  sampling. These bounds prevent folds; they do not guarantee perceptual quality.

The implementation is an independent regularized residual mesh, not a reproduction
of MeshFlow or a learned stabilization network. It assumes background observations
are sufficiently distributed and dominate the robust fit. A large moving subject,
occlusion, motion blur, low texture or unknown lens calibration can defeat that
assumption. Sparse support intentionally falls back to camera stabilization.
Changing scene appearance usually breaks tracking, but there is no semantic cut
detector. Adaptive crop and the correction bounds can limit high-shake reduction.

Tracks and prepared fields scale with clip duration. Decoder backlog is bounded,
but the visual-feature offset method retains images until its two-frame cache is
built; long clips using that offset method need separate memory validation.
Other offset methods release consumed DIS/GMFlow images earlier. Supported mobile
and desktop configurations require normal upstream integration testing.

## Reproduction

Use Python with NumPy and OpenCV (`opencv-python`), FFmpeg with libx264, and a
normal Gyroflow development runtime. No machine-specific directories are required.
The procedural fixtures contain camera shake, nonrigid image motion, a moving
subject, weak texture and a scene cut. Their camera and remap coefficients are
recorded. Synthetic input uses reflected edges, so it does not model newly
revealed real-world content at the border. Optional DAVIS frames must be obtained separately from the
[official dataset](https://davischallenge.org/davis2017/code.html); they are not
redistributed here. The DAVIS lens is unknown and 30 fps is an explicit evaluation
assumption, so those comparisons are diagnostics without camera-motion ground truth.

```text
python make_cases.py --ffmpeg <ffmpeg> --output-dir <data> --base-preset preset.gyroflow
python run_cases.py --gyroflow <gyroflow> --directory <data> --cases local-jitter moving-subject high-shake sparse-features scene-cut
python evaluate_cases.py <data> --cases local-jitter moving-subject high-shake sparse-features scene-cut --output <metrics.json>
```

`run_cases.py` first renders the local pass and saves its analysis, then replays
that exact camera analysis with strength zero for the camera-only comparison.
It records arguments, exit codes and wall time. Crop is recomputed for each
variant; the evaluator reports scale as well as registration coverage. To test
GMFlow, use `--variants gmflow-local gmflow-camera-only` and pass those variants
to the evaluator. Footage paths are local to the chosen data directory.

The generator uses the application's default `findEssentialMat` pose method (0).
Pass `--pose-method 3` to reproduce the separate homography comparison, or 1 for
Almeida. Keep results from different pose methods separate: camera estimation can
change the outcome substantially. Output folders use file URLs so generated
projects can also be opened in the GUI on Windows.

The independent evaluator uses SIFT correspondences and local affine background
patches. Acceleration and spatial residual metrics are proxies, not a perceptual
score. Cuts are evaluated as separate shots. Review the videos, crop and failed
registrations alongside the numbers. Do not use `resources/comparison1.mp4` as
raw input: it is already a before/after composite.

Run portable regression tests with:

```text
cargo test --manifest-path src/core/Cargo.toml --locked --lib --release
```

The new workflow is configured to run these tests on Linux, Windows and macOS. Optional OpenCV
and GMFlow tests require their documented native/model dependencies. The ignored
`residual_warp_matches_cpu_on_wgpu_with_lens_strength_rects_and_inversion` test
compares real float-coordinate rendering with the CPU implementation; run it
explicitly on graphics validation hosts. `GYROFLOW_TEST_GPU` selects a device
index from its printed inventory. `GYROFLOW_TEST_DUMP_DIR` exports binary fixtures
for a separate OpenCL-kernel comparison. With `use-opencl`, the corresponding
`residual_warp_matches_cpu_on_opencl_with_lens_strength_rects_and_inversion` test
exercises Rust buffer upload, shader flag specialization and OpenCL execution.
Neither a shader compile nor a parity
test alone establishes end-to-end video quality or neural inference compatibility.
