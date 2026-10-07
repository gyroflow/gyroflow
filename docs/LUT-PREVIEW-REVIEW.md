# Adversarial review: LUT preview and color controls

Reviewed 2026-10-07 against export-only commit 29195d4b9fd8970a3284252f77ed8111fee4c459.
This is a development feature tested on the local Mac, not a public release.

## Findings resolved

- **Misleading status on invalid LUT:** hide the green applied card when parsing
  fails; retain the visible error and disable export. No silent fallback to a
  successful uncolored export.
- **Oversized input allocation:** both preview and export use a bounded reader
  before parsing. A stream test proves reading stops at 64 MB plus one byte.
- **Parser differences between preview and export:** validate one bounded cube
  subset and write a canonical private cube for FFmpeg. This handles accepted
  BOM/whitespace forms without letting FFmpeg silently ignore them.
- **Preview/export interpolation and adjustment order:** production GPU/FFmpeg
  comparison matched all RGB components for 16,384 input colors. Brightness and
  contrast are applied after the LUT with the same formula and clamp.
- **Potential stabilization/frame ownership regression:** KEEP_REF remains in
  place. On the final build, all 857 decoded frame hashes/timestamps matched
  between an adjusted export and the same transform applied independently to
  a color-off stabilized export. At a paused preview frame, comparison preserved
  the same frame and zoom. Source video hash is unchanged.
- **Comparison toggle changing output:** the toggle is excluded from export
  settings. An actual UI save with comparison off retained LUT/+10%/+20%.
- **Old projects and partial presets:** serde defaults and full-project UI loads
  use zero adjustments. Partial presets without these fields preserve current
  settings. Controls are included in the preset/Apply to all selector.
- **Clipping and alpha:** float tests cover neutral behavior, all four adjustment
  extremes with no LUT, LUT-before-adjustment ordering, alpha and timestamps.
- **User path expression injection:** only a private temporary filename is passed
  through FFmpeg's option API; adjustment expressions use bounded finite numbers.
  Image bytes are owned through QImage encoding. No user path is shader source.
- **Easy Eject integration:** the existing explicit LUT-applied comment marker is
  retained. Color adjustment metadata is appended without replacing comments.

## Visual verification

The final Mac build loaded the moving-drone project, displayed the chosen LUT,
brightness and contrast fields/sliders, comparison and reset controls, and played
with stabilization active. Color on/off preserved framing at the same paused
frame. The adjustment-only preview also worked after Clear. The original user
project was restored after save testing. The development app was rebuilt and
ad-hoc signature checked; the official Gyroflow application was not replaced.

## Windows source compatibility follow-up

The preview package includes HLSL shader model 5.0 for Direct3D. Re-baked with
`qsb --qsbversion 64` to match the existing shader serialization format (6)
and remain compatible with the repository's Windows Qt 6.7.3 build. The same
16,384-color Mac GPU comparison still matches exactly. This removes a package
version mismatch; Windows compilation/runtime remain unverified.

## Remaining limits

- Windows/Linux/mobile and sandboxed persistence need runtime validation.
- Preview uses the existing RGBA8 display path; ten-bit export remains ten-bit.
  Same color math is not an HDR/display color-management guarantee.
- LUT file changes outside the app require Clear/reselection to refresh preview;
  queued exports read the file at render start. This behavior is documented.
- Large cube selection/PNG preparation is synchronous; 128-point LUT GPU limits
  and responsiveness on other hardware remain untested. The tested DJI LUT is 33.
- Unsupported LUT formats/domains are intentionally rejected consistently by
  preview and export. This narrows the previous export-only FFmpeg format range.
- Full-range codec and GPU-backend coverage remains open. The local Mac app
  depends on Homebrew runtime libraries and is not a distributable release.
