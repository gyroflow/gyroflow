# Experimental GMFlow GPU backend

This opt-in backend implements neural optical-flow matching for #831,
related to optical-only stabilization #45. The branch now integrates a separate
[residual optical pass](../optical_stabilization/README.md). The network estimates
image correspondences; it is not a rolling-shutter-aware neural motion model.
Default builds keep their existing optical-flow choices and do not import Burn.

## Reproducible model preparation

The architecture and pretrained parameters are from [UniMatch](https://github.com/autonomousvision/unimatch).
Use the pinned revision of [AdrianEddy's fork](https://github.com/AdrianEddy/unimatch)
below, retain the upstream license notices, and consult its model/data terms when
redistributing weights. This change does not commit a generated model or weights.

Create a Python environment and install CPU PyTorch 2.14.0 from its official CPU
wheel index, then install this directory's `requirements.txt`. These are the
versions used for the local export; other versions have not been validated.

```text
git clone https://github.com/AdrianEddy/unimatch.git <unimatch-dir>
git -C <unimatch-dir> checkout da140fac169d58fba4ffde9c4ef10c906fb5040b
python -m pip install torch==2.14.0 --index-url https://download.pytorch.org/whl/cpu
python -m pip install -r _scripts/gmflow/requirements.txt
python _scripts/gmflow/export.py --unimatch-dir <unimatch-dir> --output-dir <model-dir>
```

The exporter verifies the full SHA-256 of the official mixed-data checkpoint,
exports a fixed 320×576, three-channel, 0–255 input graph with opset 16, simplifies
it, and checks ONNX Runtime output against PyTorch. Gyroflow repeats luminance
into three channels because the existing optical-flow interface supplies luma.

Set `GYROFLOW_GMFLOW_ONNX` to the **absolute path** of the resulting `gmflow.onnx`,
then build the app with `--features gmflow`, or the core with `--features use-gmflow`.
The optional `gyroflow-gmflow` crate generates and compiles the network separately
from the application. It embeds the Burnpack record in the binary: it adds
approximately 150 MB and requires neither a runtime download nor Python on the
end user's machine. A production distribution strategy remains to be agreed.

Validation used Rust 1.98.1 explicitly. Select a toolchain that satisfies the
dependencies' Rust version requirements; changing the system default is unnecessary.

```text
cargo +1.98.1 test --manifest-path src/core/Cargo.toml --release --features use-gmflow --lib -p gyroflow-core -p gyroflow-gmflow --locked
cargo +1.98.1 test --manifest-path src/core/Cargo.toml --release --lib --locked
cargo +1.98.1 run --manifest-path src/core/Cargo.toml --release --features use-gmflow --example gmflow_pairs -- <width> <height> <stride> <gray8-sequence> <matches.json>
```

`gmflow_pairs` accepts at least two raw gray8 frames, with `stride * height` bytes
per frame. It calls the actual `OpticalFlowMethod` API (method 3) for each adjacent
pair and writes points and timing to JSON. No Qt, FFmpeg or OpenCV installation is
needed to run it. In the app, select **Synchronization → Advanced → Optical flow
method → GMFlow (GPU)**. The method is saved as `of_method: 3` and is also available
to the CLI and render queue. A build without the feature reports an explicit
error when a saved project requests it.

The generated network is expensive for the host compiler. On a memory-constrained
development machine, the following Cargo overrides keep the graph's host code
unoptimized while Burn and the rest of the app retain release optimization:

```text
--config profile.release.package.gyroflow-gmflow.opt-level=0
--config profile.release.package.gyroflow-gmflow.codegen-units=1
-j 1
```

These options affect host compilation; the GPU still executes the same network.
Report the build profile with measurements. No machine-specific paths, adapter
IDs, driver settings, or system-wide Rust changes are required by this backend.

## GPU loading and resource control

Burn 0.21.0 exports a native-boolean parameter for this graph. Burnpack preserves
its dtype while CubeCL requires GPU boolean storage. `GpuBoolAdapter` converts
only boolean snapshots to U32 storage through Burn's public adapter API. Integer
and floating-point parameters retain their original precision. No registry
patches or backend source changes are needed.

The generated model also exceeds the default Windows thread stack while loading.
A dedicated 16 MiB stack worker owns one model and serializes inference requests;
the bounded queue holds image references rather than allocating many GPU tensors.
The worker selects Wgpu's default high-power device; Burn's documented
`CUBECL_WGPU_DEFAULT_DEVICE` override remains available for evaluation. After 30
seconds without work, the worker drops the model and releases cached GPU memory.
The next request loads it again. Each completed inference also releases unused
workspace allocations while keeping model parameters loaded. This prevents the
allocator's retained workspace from competing with the subsequent video encoder
on systems where GPU allocations also consume host commit memory.

Each analysis owns cancellation and its first error. Cancelled queued pairs do
not launch inference. Already submitted GPU kernels finish before cancellation
can take effect; their results are discarded. Completion waits for feature and
pose jobs, preventing partially processed results from being published. Device
loss and backend panics fail the analysis and retain the original device error.
A failed compute client is not reused: another method remains available, while
retrying GMFlow requires an application restart.

Decoder failures cancel and drain the submitted analysis jobs. Failed analysis
stops its render-queue job before project export or video rendering. The command
line returns a nonzero status for failures, including invalid input and failed
queued jobs; explicit overwrite and successful jobs retain status zero.

Run the manual lifecycle check on a working GPU with:

```text
cargo +1.98.1 run --manifest-path src/core/Cargo.toml --release --features use-gmflow --example gmflow_lifecycle
```

It checks pre-cancellation, concurrent requests for the same pair, cached results,
and inference after the idle timeout. It does not simulate an in-flight driver hang.

Preprocessing crops decoder stride padding before aspect-preserving resizing and
edge padding. Flow is sampled bilinearly at pixel-center coordinates and mapped
back using separate, rounded x/y resize scales. Non-finite and out-of-image
endpoints are discarded, and flat patches do not become motion correspondences.
Forward/backward inference and a shared photometric consistency filter now reject
unreliable matches. This doubles network calls per uncached pair relative to the
earlier measurements below; it is not a learned occlusion-confidence model.

The shared pose pipeline also rejects non-finite rotations and angular velocity.
Degenerate homographies can otherwise introduce NaN samples into the synthesized
IMU stream, poison quaternion integration, and turn the rendered video black.
Valid neighboring estimates use the existing interpolation path. This guard
applies to every optical-flow method.

## Evidence and limits

Earlier standalone probe: Windows, RTX 5070 Ti, Burn/Wgpu 0.21.0, fixed 320×576. Five controlled
cases (identity, three translations, a two-degree rotation) measured warm medians
of 0.590–0.594 s per pair. The same cases with ONNX Runtime CPU, eight threads,
measured 4.82–5.52 s. Worst p99 component difference was 0.00016747 pixels and
maximum 0.00073649 pixels. Timing includes inference and output readback, excludes
model loading, warm-up, preprocessing, and the explicit workspace cleanup added
later. These results are not final application throughput or realtime performance.

The existing proposal [#1143](https://github.com/gyroflow/gyroflow/pull/1143) identified
the boolean GPU-loading blocker. Its reported CPU time was on different hardware
and is not used as a speedup baseline here. The exporter and backend in this
change were implemented independently, with AI assistance and local validation.

The packaged exporter was rerun from the pinned sources and produced the same
ONNX SHA-256, `ece9e754e2e2ae70af4bafbdef3c849b0a0972f0f6b39692aa83b463c25bd2ed`.
Export validation measured a maximum ONNX Runtime/PyTorch difference of
0.00030335 pixels on its deterministic test pair.

The NVIDIA RTX 5070 Ti completed inference. The tested integrated AMD Radeon
device (gfx1036, driver 26.8.1) lost its device during full-network execution on
both Vulkan and DX12. The cause is unresolved; AMD, Intel, macOS and Linux are
not validated targets. GPU support must be established by testing, not inferred
from Wgpu's supported APIs.

A full-app synthetic pinhole test improved measured image jitter with GMFlow,
and both DIS and GMFlow stopped producing black output after the shared finite-
pose guard. An exploratory run on `resources/comparison1.mp4` was rejected as
quality evidence: the source contains a wipe between two versions of the image,
not raw camera footage. That early result is excluded. Current second-pass
comparisons and raw-video diagnostics are documented with the residual optical
pass and must be assessed separately from these earlier backend measurements.

Model distribution and supported GPUs still need maintainer agreement. This
backend alone does not implement the local residual warp requested in #45 or
establish eligibility for its bounty. Do not present flow parity or throughput
as proof of complete optical-only stabilization quality.
