# Export color processing performance

## Diagnosis and chosen change (2026-10-07)

Hardware video encoding compresses the frame after color processing. Keeping
Apple VideoToolbox or a Windows hardware encoder enabled does not move arbitrary
CPU FFmpeg filters onto the GPU. The previous pipeline retained the normal
encoder, but `geq` evaluated three brightness/contrast expressions per pixel.
[FFmpeg's source](https://www.ffmpeg.org/doxygen/trunk/vf__geq_8c_source.html)
shows `av_expr_eval` inside its float-plane pixel loop. A sample of the running
Mac export also showed that interpreter on active CPU stacks.

The export now uses direct arithmetic on the float RGB planes, distributed over
rows with the existing Rayon dependency. It retains the exact f64 operation
order and final f32 storage used by `geq`, including clipping. Alpha and padding
are excluded, frames are made writable before modification, and explicit
little-endian loads/stores remain portable. Invalid plane alignment/stride
returns an error instead of panicking. No production dependency was added.

The LUT still uses FFmpeg's tetrahedral `lut3d`. Adjustment processing is between
the RGB/LUT graph and the conversion back to the encoder's original format.
Both graphs are cached and rebuilt when input format, dimensions, matrix,
range or sample aspect ratio changes. The second conversion specifies the
original YUV matrix/range: simply splitting the graph lost that negotiation,
which the new independent reference tests caught before installation.

LUT-only processing and the completely neutral bypass retain their previous
paths. Stabilization, preview shaders, encoder selection, settings and metadata
markers are unchanged.

## Open source alternatives researched

| Option | Relevant capability | Decision |
| --- | --- | --- |
| FFmpeg direct filters | Existing float RGB/tetrahedral LUT pipeline; `geq` is flexible but interprets expressions per pixel. `colorlevels` uses simpler arithmetic but has different coefficient precision and range constraints. | Retain `lut3d`; replace only the costly adjustment interpreter with exact arithmetic. |
| [OpenColorIO](https://opencolorio.readthedocs.io/en/stable/concepts/overview/internal_architecture.html) | Optimizes sequences of color operations for CPU scanlines or generates GPU shaders and LUTs. | Good candidate for a larger color pipeline; not integrated here. It would introduce a C++ library/bridge and platform packaging work for two simple controls. |
| [FFmpeg libplacebo](https://www.ffmpeg.org/ffmpeg-filters.html#libplacebo) | GPU processing with custom shaders and LUT support in current FFmpeg source. | Possible future GPU path. The installed Mac FFmpeg has no libplacebo filter; adoption needs a Vulkan-capable build and verified frame/device interop, including Mac support. |

These GPU approaches are color processors alongside the codec engine. They are
not an automatic property of enabling hardware encoding. This change resolves
the adjustment bottleneck without claiming that LUT processing is now on the GPU.

## Measured Mac checks

Apple M4 Max, macOS 27, FFmpeg 9.0.1, release builds. A disposable one-second
trim of the supplied 4K/59.94 fps ten-bit O4 Pro flight used the same saved
stabilization, HEVC GPU-encoding option and 114 Mbps bitrate. The CLI planned
60 frames; the trim boundary produced 62 decoded frames in all final outputs.
Elapsed times include startup, input loading, stabilization, conversion and
encoding, and are individual runs rather than statistical benchmarks.

| Color settings | Previous build | Updated build |
| --- | ---: | ---: |
| None; adjustments zero | 2.54 s | 2.01 s |
| DJI LUT; adjustments zero | 3.33 s | 3.28 s |
| DJI LUT; brightness +10%, contrast +20% | 11.60 s | 3.26 s |

The adjusted export was about 3.6 times faster in this short test and now took
essentially the same time as LUT-only export. The neutral timing variation is
not evidence of a neutral-path optimization. LUT processing/conversion still
has measurable overhead; this does not promise neutral speed or a full-flight
speedup of the same factor.

- All 13 production parser/filter tests pass. New tests compare against an
  independently constructed FFmpeg `geq` graph, bit for bit. They exercise
  adjustment-only extremes and mixed values, odd widths and row padding,
  shared-frame reuse/ownership, unchanged alpha, P010LE/YUV420P10LE, limited
  BT.709, full BT.2020, unspecified color properties, timestamps, non-square
  pixels and format/size/color-property changes.
- A new lossless full-app 720p export of an eight-second moving-flight section
  matched the independent LUT plus `geq` transform of its neutral stabilized
  export for all 481 decoded ten-bit frames and timestamps. BT.709/limited-range
  stream properties and LUT/adjustment metadata markers also matched. This
  verifies unchanged stabilization and colors for that section.
- All three final 4K HEVC test outputs passed full decoding and retained
  ten-bit YUV, dimensions and the same decoded frame count. Lossy hardware
  exports are not claimed to have identical decoded hashes.

Private fixtures, LUT files, benchmark logs and generated videos remain local
under `_dev/export-performance-check/`, outside version control. The new native
adjustment path has not yet been runtime tested on Windows; earlier Windows
acceptance evidence in `WINDOWS-LUT-TESTING.md` applies to the previous path.
