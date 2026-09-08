# Gyroflow optical-flow contribution: review and evidence

The independent finite-pose patch is available [from its focused commit](https://github.com/antonioscafaro/gyroflow/commit/ed1c252228e7f1d6a1817bce78a62fa7f86a7b01.patch).

Reviewed against upstream `d918ab3594e539f25a67a9f1d2b8e042798f61f7` on 2026-09-08.

The work contains a general correctness fix and an experimental GPU optical-flow integration. It is **not a completed solution to issue #45**, and it has not earned a bounty. The local residual warp requested by the maintainer is still absent. No production-readiness or cross-vendor compatibility claim is made for GMFlow.

## General correctness fixes

Degenerate pose estimates can contain NaN values even when pose estimation returns success. Previously these values entered the synthesized IMU stream, poisoned quaternion integration, and made an entire rendered video black. The pose pipeline now accepts only finite rotations and finite angular velocity at a positive finite sample rate. Missing estimates retain the existing interpolation behavior.

On the deterministic 90-frame fixture, the earlier pipeline produced 90 black frames and 15 non-finite IMU samples. After the guard, all 90 frames remained visible: DIS produced 88 finite IMU samples and GMFlow produced 89. The included `videos/nonfinite-pose-fix.mp4` shows the failure and corrected DIS output. This is independent of choosing the new neural backend.

`finite-pose.patch` is independently applicable to the upstream revision. Its two regression tests cover invalid rotations/rates and valid angular velocity. It introduces no neural-model dependency. The larger integration additionally rejects invalid luminance dimensions and truncated planes instead of indexing past their storage, with a separate regression test.

The larger integration also fixes failure handling: analysis completion waits for submitted pose jobs; decoder and backend failures stop the render job; cancelled jobs do not publish a partial synthesized motion stream; command-line failures produce a nonzero exit status. Successful rendering and explicit overwrite retain exit status zero.

## Optional GMFlow integration

- Pinned UniMatch checkpoint and a checked export recipe; no weights or generated network files committed.
- Burn/Wgpu 0.21.0 inference behind an optional Cargo feature; existing methods remain the default.
- Boolean storage conversion through Burn's public adapter API, preserving float/integer precision and parameter metadata.
- Separate generated-model crate, one worker with a 16 MiB stack, bounded request queue, lazy loading, and idle model release.
- Decoder stride cropping, letterboxing, independent rounded x/y scales, pixel-center coordinates, and bilinear flow sampling.
- Invalid endpoints and flat image patches rejected; repeated pair requests share cached matches. Two-frame visual-offset analysis retains its source images until matching completes.
- Per-analysis cancellation and first-error reporting; failed GPU clients are not reused.
- Selection and saved-project support in the Qt UI, command line, and render queue, including an explicit missing-feature message.

No machine-specific adapter ID, path, registry patch, driver setting, or system-wide Rust configuration is part of the source change. The tested environment nevertheless remains limited to Windows and one working NVIDIA GPU.

## Numerical evidence

Model source: [AdrianEddy/UniMatch](https://github.com/AdrianEddy/unimatch), revision `da140fac169d58fba4ffde9c4ef10c906fb5040b`, using the upstream pretrained GMFlow scale-2/refinement-6 checkpoint. Input is 320×576, three-channel, 0–255. The current Gyroflow interface repeats luminance across RGB channels.

The packaged exporter was actually rerun from the pinned source and produced an identical ONNX SHA-256:

`ece9e754e2e2ae70af4bafbdef3c849b0a0972f0f6b39692aa83b463c25bd2ed`

ONNX Runtime/PyTorch maximum component difference: **0.00030335 pixels**. The manifest records the complete checkpoint checksum and software versions.

An earlier standalone Wgpu probe on an RTX 5070 Ti measured **0.590–0.594 seconds per warm pair**, against **4.82–5.52 seconds** with ONNX Runtime CPU using eight threads on the same computer. Across identity, three translations, and a two-degree rotation, the worst p99 component difference was **0.00016747 pixels**, maximum **0.00073649 pixels**. These timings exclude preprocessing, loading, warm-up, rendering, and the subsequently added explicit workspace cleanup. They are not final application throughput.

Tests through the actual core optical-flow API, including 32 bytes of decoder stride padding, produced median endpoint errors of **0.0058–0.0431 pixels** across those five controlled cases. The largest p95 endpoint error was **0.0734 pixels**. Two real DAVIS frame pairs produced 1,184 and 1,179 matches; without ground-truth flow these counts do not establish accuracy.

Those API checks were repeated after adding workspace cleanup: the controlled-case coordinates and endpoint errors were unchanged. One warm real-frame inference, including preprocessing and cleanup, took **0.665 seconds**; this single timing is not a benchmark distribution. The lifecycle check also passed after cleanup: pre-cancellation, four concurrent requests sharing one cached pair, and inference after a 32-second idle period. It does not prove interruption of an in-flight GPU kernel or quantify physical VRAM release.

The CPU time reported in [PR #1143](https://github.com/gyroflow/gyroflow/pull/1143) used different hardware and is not used as a speedup baseline. That PR is credited for identifying the boolean GPU-loading blocker. This integration and exporter were implemented independently with AI assistance and locally executed validation.

## Rendered-video evidence

Both methods use identical presets within each comparison. The metric is independent SIFT/RANSAC registration, evaluated at nine image positions. Values are RMS frame-to-frame velocity changes in **output pixels**; they include crop/zoom effects and are not a perceptual quality score or a measurement of local distortion. Lower is better under this limited metric.

| Fixture and pose model | Input | DIS | GMFlow |
| --- | ---: | ---: | ---: |
| Known pinhole, synthetic camera rotation; homography pose | 5.2678 | 3.7935 | 2.4291 |

The synthetic case uses 90 frames at 960×540 and 30 fps, known focal length 720 pixels, and known three-axis camera rotation. It uses fixed-reference registration with zero failed registrations. Its video checksum is `37ac40c59df729898e20634146dea4af54274521453b1d19c20ff68a8f006b17`; the published fixture generator reproduced it exactly with the tested FFmpeg build.

An exploratory run used the repository's `resources/comparison1.mp4`, 133 frames at 854×480, with an assumed pinhole focal length of 640 pixels. Inspection of the comparison video revealed a diagonal wipe between two versions of the image: it is an already composed comparison, **not raw camera footage**. It is excluded from stabilization-quality evidence. The retained experimental metrics (input 8.5886; DIS/GMFlow essential-pose 9.5742/9.7706; homography-pose 19.9910/19.9691) are disclosed only to document the rejected experiment. Zero adjacent-frame registration failures did not make this source suitable. The fixed-first-frame registration also failed after camera motion and is excluded. A proper real-video quality evaluation remains outstanding.

All measured corrected outputs decoded in full with zero black frames. An earlier complete CLI run took about 60.5 seconds for the 90-frame synthetic GMFlow case. This includes application startup, analysis, stabilization, and encoding; it predates the final workspace cleanup change.

## Remaining release work

1. Implement and validate the local residual warp required by [issue #45](https://github.com/gyroflow/gyroflow/issues/45), with confidence/occlusion handling and temporal consistency. The current backend does not implement forward/backward flow filtering.
2. Establish improvement on calibrated real footage, including moving subjects, depth changes, rolling shutter, and crop/zoom behavior.
3. Resolve the tested integrated AMD `gfx1036` device loss (driver 26.8.1, both Vulkan and DX12). Intel, Linux, and macOS are untested. Wgpu API coverage is not evidence of model compatibility.
4. Agree on model licensing/distribution and supported devices. Embedded weights add roughly 150 MB. No packaged binary is included.
5. Complete application cancellation/device-loss and memory-pressure coverage on additional systems. Submitted GPU kernels cannot be interrupted safely; cancellation discards their results after completion.

The source is substantial enough for a scoped maintainer review. Investing further in a production residual-warp pipeline before confirming its acceptance criteria and payment scope is not justified by the current evidence.

## Bounty status

[Issue #45](https://github.com/gyroflow/gyroflow/issues/45) remains open and the [Algora board](https://algora.io/gyroflow/bounties?status=open) advertises $500. This is not confirmation that a particular contribution qualifies, that funds remain payable, or that GitHub Sponsors is the applicable payout route. No explanation is established for other attempts not being paid. The relevant next step is a scoped technical proposal linked to reviewable source, with an explicit eligibility question rather than a completion claim.
