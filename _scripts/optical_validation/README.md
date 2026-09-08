# Optical stabilization validation

These original fixture generators and the core tests exercise the optical-only camera estimate and the optional residual correction. Build the normal release application with the prerequisites in the main build documentation first.

```sh
cargo test --manifest-path src/core/Cargo.toml --features use-opencv --lib
cargo test --manifest-path src/core/Cargo.toml --no-default-features --lib
python -m pip install -r _scripts/optical_validation/requirements.txt
python _scripts/optical_validation/generate_parallax_demo.py --ffmpeg ffmpeg --output demo
```

The generated scene includes depth, straight-line references, deliberately jittery camera motion, a slow pan and an independently moving foreground box. It contains no gyro stream and uses no third-party imagery. The accompanying lens file defines its exact perspective camera.

Example render (POSIX shell quoting):

```sh
gyroflow demo/parallax-input.mp4 demo/lens.json \
  --no-gpu-decoding --processing-device -1 --suffix _residual --export-project 4 \
  --sync-params '{"of_method":2,"pose_method":3,"every_nth_frame":1,"search_size":5,"time_per_syncpoint":1,"max_sync_points":1,"do_autosync":true,"auto_sync_points":false,"processing_resolution":360}' \
  --preset '{"version":2,"stabilization":{"optical_stabilization":true,"fov":1,"max_zoom":130}}' \
  -p '{"codec":"H.264/AVC","use_gpu":false,"audio":false,"pixel_format":"yuv420p","bitrate":8,"output_width":640,"output_height":360,"interpolation":"Bilinear"}'
```

Use the same array of arguments through `subprocess.run` on shells that alter nested JSON quoting. To test the full frame decoder at an unusual row width, `generate_motion_fixture.py` produces a 318×238 translated texture. Adding `--cut-at 60` inserts an independent scene at frame 60 of 120. The expected saved discontinuity is 2,000,000 microseconds. A successful run decodes all 120 frames and has no correspondence across that edit.

`analyze_motion.py` reports central phase-correlation motion at 640×360, retaining every adjacent frame pair. It is independent of DIS and PyrLK, but its acceleration value is only a motion proxy: rotation, zoom, parallax, moving subjects and encoding can affect it. It is not a general perceptual-quality score.

For a controlled comparison, save the analyzed project, then render both cases from that same motion data. The camera-only control uses an equivalent internal camera zoom allowance and FOV keyframes matching the candidate's extra reserve. `optical_diagnostics` exposes the per-frame base FOVs, margins and grid bounds so the crop equality can be checked:

```sh
cargo run --release --manifest-path src/core/Cargo.toml --no-default-features \
  --example optical_diagnostics -- demo/parallax-input.gyroflow demo/diagnostics.json
```

Load the lens profile before analysis. Run Autosync again after changing calibration or replacing the video. Saved projects preserve the optical pairs and, when applicable, the estimated gyro and unsupported-transition boundaries. They retain Gyroflow's existing association with the source video path; this change does not provide a cryptographic identity check of a replaced file.

The residual option is experimental and off by default. Crop, low-confidence fallback and deformation limits trade stabilization strength against field of view and shape preservation. CPU/OpenCL/wgpu rendering, project reload, deterministic motion/cut tests and partial-input failure were exercised on Windows. Other platforms and the standalone Rust SPIR-V target require their normal platform checks. Qt preview shaders are regenerated with `_scripts/compile_qt_shaders.py --root . --qsb /path/to/qsb`.
