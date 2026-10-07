# LUT preview and export colors

This prototype adds a user-selected 3D `.cube` LUT to Gyroflow's normal export.
In **Color settings**, select **Choose LUT…**, pick a LUT, then export as usual.
**Clear** disables the LUT. Brightness and contrast run from −50% to +50%; zero
is neutral. Double-click either slider to reset it; **Reset adjustments** restores both to zero.
Color settings has its own collapsible section above Export settings.
Presets and Apply to all also group the LUT and adjustments under Color settings;
the saved `output` fields are unchanged.

The LUT is applied after stabilization and before encoding, in the same export.
The preview shows the selected LUT and color adjustments. **Preview colors**
switches the preview between adjusted and original colors without changing export.
Source video and motion data are not
modified. Use a LUT appropriate for the recording's color profile; the app does
not detect log footage or automatically choose a camera LUT. The exported video
already has the LUT applied, so avoid applying the same conversion again.

## Implementation

- `output.lut_url` is an optional file URL; old projects default to no LUT.
- `output.brightness` and `output.contrast` are normalized values from −0.5 to +0.5.
  Old projects default to zero. Projects, queues, presets and Apply to all retain
  these controls. The preview comparison toggle is only a view setting.
- After the LUT, both preview and export calculate
  `clamp((RGB - 0.5) * (1 + contrast) + 0.5 + brightness, 0, 1)`.
  These are simple brightness and contrast controls, not exposure in stops.
- A Qt Quick GPU layer colors the existing stabilized preview. The tetrahedral
  shader reads losslessly packed float32 LUT values from an opaque RGB atlas.
  Stabilization geometry and its processing code are unchanged.
- Standard text 3D `.cube` files are supported: size 2–128, complete finite
  entries, and a 0–1 input domain. 1D shapers, custom domains, and other LUT
  formats are rejected with a visible error that disables export. Input reads
  are limited to 64 MB. A canonical private cube gives the FFmpeg parser the
  same lattice values even when the input has a BOM or indented directives.
- The preview caches the chosen LUT while export reads it at render start.
  If the LUT file is edited externally, Clear and choose it again to refresh
  the preview. Keep a selected LUT file available until queued exports finish.
- Preview uses Gyroflow's existing RGBA8 display pipeline. Matching color math
  does not guarantee pixel-identical display and ten-bit export, HDR, or full
  display color management. A size-128 atlas needs a 1536×8192 GPU texture;
  large LUT performance and GPU limits on other platforms remain untested.
- Projects, render jobs, presets, and Apply to all include the selected LUT.
- Apple project export attempts to save a security-scoped bookmark for the LUT,
  using the existing bookmark mechanism. Sandboxed persistence remains untested.
- The renderer reads the selected file at render start and uses a private
  temporary copy for that render. FFmpeg receives the filename through its
  option API, avoiding filter-expression escaping of user paths.
- The filter uses float planar RGB, tetrahedral interpolation, then converts back
  to the frame's pixel format. Ten-bit precision and alpha are retained.
- FFmpeg must include `buffer`, `format`, `lut3d`, `geq`, and `buffersink`. Missing or
  unreadable LUTs and invalid cube files fail the render with an export LUT error.
- Export comments include `Gyroflow export LUT applied: <filename>` when a LUT
  is selected. Existing user comments are retained. Downstream tools can use
  this explicit marker to avoid repeating the conversion.
- Partial presets that do not include the LUT field preserve the current LUT.
  Clear removes its URL and its saved Apple bookmark.
- Export color processing is computed on the CPU. Stabilization and encoding can still use the GPU.

## Reproducible filter tests

With FFmpeg 9 development libraries installed:

```sh
cargo test --manifest-path tests/export-lut/Cargo.toml
```

The tests exercise identity and inversion transforms, timestamps, dimensions,
alpha, sample aspect ratio, graph reconstruction, malformed LUT rejection, and ten-bit YUV.
`tests/export-lut/examples/apply_raw_10bit.rs` also runs the production filter on
one packed limited-range BT.709 YUV420P10LE frame for comparison with FFmpeg:

```sh
cargo run --manifest-path tests/export-lut/Cargo.toml --example apply_raw_10bit -- \
  input.raw production.raw selected.cube 3840 2160
ffmpeg -f rawvideo -pixel_format yuv420p10le -video_size 3840x2160 \
  -color_range tv -colorspace bt709 -i input.raw \
  -vf 'format=gbrpf32le,lut3d=file=selected.cube:interp=tetrahedral,format=yuv420p10le' \
  -frames:v 1 -f rawvideo reference.raw
cmp production.raw reference.raw
```

## Previous export-only verification (2026-10-07)

Tested on Apple M4 Max, macOS 27, Qt 6.11.2, Rust 1.99.0, and FFmpeg 9.0.1.
Three DJI O4 Pro recordings were 3840×2160, 59.94 fps, ten-bit HEVC, with usable
embedded motion data. Media and the DJI LUT are local test inputs and are not
included in this repository.

- Six automated filter tests passed, including non-square sample aspect ratio.
- A real 4K frame with the DJI O4 D-Log M to Rec.709 LUT matched the independent
  FFmpeg reference byte for byte.
- The full renderer exported the 8.125-second / 487-frame clip at 4K with Apple
  GPU HEVC encoding and the LUT, retaining ten-bit output.
- The 19.753-second / 1,184-frame clip exported with software HEVC encoding and
  the LUT at 720p. A separate export with the LUT disabled also completed.
- A lossless 720p comparison locked the same project, stabilization settings,
  lens profile, and input video. All 487 decoded frame hashes and timestamps
  matched between (a) LUT applied within Gyroflow and (b) LUT-off stabilized
  video with the equivalent color transform applied separately by FFmpeg.
  This verifies the LUT did not change stabilization or framing in that clip.
- The new 14.298-second / 857-frame moving-drone clip passed the same lossless
  comparison at full 3840×2160 resolution. All 857 decoded pixel hashes and
  timestamps matched exactly. This isolates the LUT from stabilization; it
  does not establish the cause of any vibration present in the recording.
- A real export carried the LUT comment marker. A downstream Easy Eject test
  created and verified a sharing copy without a second LUT, reused its matching
  receipt, and confirmed both input files were unchanged.
- The refined green status card displayed the selected LUT filename. A real
  UI save/Clear/save check confirmed the cleared project had no selected URL
  or stale bookmark, and the preserved original project reloaded the LUT.
- A saved project retained the selected LUT URL; invalid LUT export reported an
  explicit error rather than completing without color conversion.
- The Mac UI loaded the clip, opened the native LUT picker, accepted the DJI LUT,
  and displayed the Clear button. The preview remained unchanged.

This is a development prototype. Windows, Linux, mobile devices, sandboxed Store
builds, and the full range of export codecs have not been runtime tested.
The local Mac preview currently depends on the installed Homebrew libraries;
it is not a standalone distribution or an official Gyroflow release.

## Live-preview verification (2026-10-07)

- Eleven automated tests pass, including adjustment order, neutral defaults,
  timestamp/alpha preservation, all four adjustment extremes without a LUT,
  malformed cube rejection, atlas float-bit
  preservation, and an oversized stream that stops reading at the limit.
- The production Qt GPU shader and production FFmpeg filter matched exactly
  after RGB24 quantization for 16,384 deterministic input colors, with the DJI
  O4 D-Log M LUT, brightness +10%, and contrast +20%. This comparison supplies
  the same float RGB input to each pipeline; it tests color math and LUT
  interpolation, not the complete decoder/display color pipeline.
- The Mac UI visibly displayed the colored moving-drone clip, sliders, LUT
  filename/status, comparison toggle and reset. A real project save with
  comparison off retained brightness +10%, contrast +20% and the selected
  LUT, confirming comparison does not change export settings. At a paused frame,
  toggling colors preserved the same frame and zoom (119.17%). Adjustment-only
  preview also displayed correctly after clearing the LUT. The original
  user project was backed up and restored after the save test.
- A lossless 720p export comparison on the final build matched all 857 decoded
  frame hashes and timestamps exactly between color processing inside Gyroflow
  and the same LUT/adjustments applied independently to the color-off stabilized
  export. This isolates the new color controls from stabilization and framing.
- A complete moving-drone export with those controls retained 857 frames,
  14.297617 seconds, 59.94 fps and ten-bit HEVC output at 720p. Its comment
  retained the existing LUT marker and added the adjustment values, so the
  Easy Eject bridge can continue avoiding a second LUT.

`tests/export-lut/examples/preview_fixture.rs` writes a production export
reference and packed atlas for a supplied RGB24 image.
`tests/export-lut/preview-shader-check.qml` renders the exact production shader
with these PNG fixtures. Use a Qt 6 `qml` runner, then compare its captured
RGB pixels (accounting for the display device-pixel ratio) with the reference.
No user footage or proprietary LUT is included.
