# Reproduce the rendered-video checks

The independent finite-pose patch is available [from its focused commit](https://github.com/antonioscafaro/gyroflow/commit/ed1c252228e7f1d6a1817bce78a62fa7f86a7b01.patch).

Use the upstream revision and patch identified in `../REVIEW.md`. The small
`finite-pose.patch` applies independently; the full patch
already contains it. Do not apply both to the same checkout.

```text
git checkout d918ab3594e539f25a67a9f1d2b8e042798f61f7
git apply --check <patch-file>
git apply <patch-file>
```

Build Gyroflow using the project's usual SDK instructions. GMFlow additionally
requires the pinned model export and `GYROFLOW_GMFLOW_ONNX` described in
`_scripts/gmflow/README.md`. It is enabled with `--features gmflow`. The independent
finite-pose patch requires no model, Python runtime, or additional dependency in
the application.

## Fixture and presets

The scripts require Python, NumPy and OpenCV. Validated versions were Python 3.12,
NumPy 2.5.3 and opencv-python-headless 5.0.0.93. Encoding requires an FFmpeg build
with libx264; the recorded fixture checksum used FFmpeg 9.0. Encoding with a
different build may change the file checksum while preserving the test scene.

```text
python -m pip install numpy==2.5.3 opencv-python-headless==5.0.0.93
python make_fixture.py --ffmpeg <ffmpeg-executable> --output-dir <evaluation-directory>
```

This produces a deterministic 90-frame pinhole video, camera parameters, known
rotations, and paired DIS/GMFlow presets. The script writes the supplied evaluation
directory into each preset, so there are no fixed machine paths. For the real-clip
checks, copy `resources/comparison1.mp4` from the pinned repository into that
directory. That source was later excluded from quality evidence because it is a
composite comparison with a wipe. The real-clip commands below reproduce that
rejected experiment only. Its lens preset is also approximate, not a calibration.

## Run the application

Run these commands in an environment where the built app can find its normal
Qt, FFmpeg, OpenCV and MDK runtime dependencies. Use the same binary for both flow
methods and retain all other preset settings.

```text
<gyroflow-executable> -f --no-gpu-decoding --preset <evaluation-directory>/known-camera-dis-preset.gyroflow --export-project 4 <evaluation-directory>/known-camera.mp4
<gyroflow-executable> -f --no-gpu-decoding --preset <evaluation-directory>/known-camera-gmflow-preset.gyroflow --export-project 4 <evaluation-directory>/known-camera.mp4

<gyroflow-executable> -f --no-gpu-decoding --preset <evaluation-directory>/comparison1-dis-essential-preset.gyroflow --export-project 4 <evaluation-directory>/comparison1.mp4
<gyroflow-executable> -f --no-gpu-decoding --preset <evaluation-directory>/comparison1-gmflow-essential-preset.gyroflow --export-project 4 <evaluation-directory>/comparison1.mp4
```

The real homography comparison uses the corresponding presets without
`-essential` in their filenames. `--no-gpu-decoding` disables hardware decoding;
the preset chooses CPU video encoding. The stabilization renderer may still use
OpenCL/Wgpu. These switches do not disable neural GPU inference.

To reproduce the black-video bug, use the DIS synthetic preset on the unmodified
upstream pipeline, then repeat after applying the finite-pose patch. Save the
earlier output before using `-f`, which explicitly overwrites an existing output.

## Independent registration metric

```text
python evaluate_video.py <evaluation-directory>/known-camera.mp4 <evaluation-directory>/known-camera.mp4 <evaluation-directory>/known-camera-dis.mp4 <evaluation-directory>/known-camera-gmflow.mp4 --output <evaluation-directory>/synthetic-metrics.json
python evaluate_video.py <evaluation-directory>/comparison1.mp4 <evaluation-directory>/comparison1.mp4 <evaluation-directory>/comparison1-dis-essential.mp4 <evaluation-directory>/comparison1-gmflow-essential.mp4 --pairwise --output <evaluation-directory>/real-metrics.json
```

The first positional file supplies the reference image. Repeating it in the
following video list also measures the input baseline, as shown above. The
synthetic scene uses fixed-reference registration; the real clip uses adjacent-
frame registration because the camera moves away from its initial view. Always
report registration failures alongside the metric. The score includes zoom/crop
effects, assumes a homography for registration, and does not measure residual
local deformation. Do not compare the two fixture scores directly.

The included JSON evidence and videos record the actual runs. See `../REVIEW.md`
for negative results, tested build profiles, and limits on interpretation.
