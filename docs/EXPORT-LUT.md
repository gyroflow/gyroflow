# Apply a LUT on export

This prototype adds a user-selected 3D `.cube` LUT to Gyroflow's normal export.
In **Export settings**, select **Choose LUT…**, pick a LUT, then export as usual.
**Clear** disables the LUT. There are no adjustment controls.

The LUT is applied after stabilization and before encoding, in the same export.
The preview retains its original colors. Source video and motion data are not
modified. Use a LUT appropriate for the recording's color profile; the app does
not detect log footage or automatically choose a camera LUT. The exported video
already has the LUT applied, so avoid applying the same conversion again.

## Implementation

- `output.lut_url` is an optional file URL; old projects default to no LUT.
- Projects, render jobs, presets, and Apply to all include the selected LUT.
- Apple project export attempts to save a security-scoped bookmark for the LUT,
  using the existing bookmark mechanism. Sandboxed persistence remains untested.
- The renderer reads the selected file at render start and uses a private
  temporary copy for that render. FFmpeg receives the filename through its
  option API, avoiding filter-expression escaping of user paths.
- The filter uses float planar RGB, tetrahedral interpolation, then converts back
  to the frame's pixel format. Ten-bit precision and alpha are retained.
- FFmpeg must include `buffer`, `format`, `lut3d`, and `buffersink`. Missing or
  unreadable LUTs and invalid cube files fail the render with an export LUT error.
- The LUT is computed on the CPU. Stabilization and encoding can still use the GPU.

## Reproducible filter tests

With FFmpeg 9 development libraries installed:

```sh
cargo test --manifest-path tests/export-lut/Cargo.toml
```

The tests exercise identity and inversion transforms, timestamps, dimensions,
alpha, graph reconstruction, malformed LUT rejection, and ten-bit YUV.
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

## Local verification (2026-10-07)

Tested on Apple M4 Max, macOS 27, Qt 6.11.2, Rust 1.99.0, and FFmpeg 9.0.1.
Two DJI O4 Pro recordings were 3840×2160, 59.94 fps, ten-bit HEVC, with usable
embedded motion data. Media and the DJI LUT are local test inputs and are not
included in this repository.

- Five automated filter tests passed.
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
- A saved project retained the selected LUT URL; invalid LUT export reported an
  explicit error rather than completing without color conversion.
- The Mac UI loaded the clip, opened the native LUT picker, accepted the DJI LUT,
  and displayed the Clear button. The preview remained unchanged.

This is a development prototype. Windows, Linux, mobile devices, sandboxed Store
builds, and the full range of export codecs have not been runtime tested.
The local Mac preview currently depends on the installed Homebrew libraries;
it is not a standalone distribution or an official Gyroflow release.
