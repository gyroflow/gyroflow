# Camera catalogue and profile review

The main lens panel and calibrator use a shared camera/lens catalogue. Each
dropdown supports filtering, including camera aliases such as Sony a6000 and
ILCE-6000. Selecting a brand, model or lens does not load a calibration until the
user chooses a result or presses Next.

The catalogue combines Gyroflow profile metadata with Lensfun names, mounts,
camera sensor crop factors and lens focal ranges. Lensfun-only entries are
identified as known lenses without a Gyroflow calibration. They are metadata
choices, not automatically converted distortion models.

The generator and pinned Lensfun source live in `gyroflow/lens_profiles`. A
bundled fallback is stored in `resources/camera_database.json`; the reserved
`__camera_database` entry in `profiles.cbor.gz` updates it through the existing
profile update mechanism. Older bundles retain the fallback; unsupported
catalogue schemas do not discard the profiles. Locally installed profiles are
added to the catalogue when loaded. See `resources/CAMERA_DATABASE.md` for data
attribution and the separate CC BY-SA 3.0 data licence.

## Compatibility and review

Camera suggestions require a shared mount and known sensor crop factors within
1%. Sensor dimensions are only derived when Lensfun supplies an aspect ratio;
otherwise the sensor diagonal is shown. Per-profile video crop factors are not
treated as physical sensor sizes. Lensfun mount adapter compatibility is
directional and is labelled separately from a native mount.

Suggested profiles remain labelled as coming from similar cameras. Recording
mode, digital correction, lens settings and framing must still be checked. A
suggestion never loads itself. Exact-camera results, favorites and matching video
aspect ratios are prioritized in the review list.

The review list retains alternate submissions with the same metadata identifier
and returns all matching submissions without the free-text search's 200-result
limit. Previous/Next changes the loaded calibration while preserving the chosen
camera setup. A profile can be hidden from the review list on this device and
restored with Show hidden profiles. This does not delete community data. The
existing Good/Bad rating controls remain available for reporting bad profiles.

## Calibration submissions

Video metadata fills the selectors where available. Other allows a new camera
or lens to be entered manually; changing the setup clears the old automatic
profile identifier. Community submissions require:

- A specific camera manufacturer, camera model, and lens model or built-in mode.
- A lens model rather than a bare focal-length label; generic FOV labels are
  rejected for cameras with a known interchangeable lens mount.
- A positive focal length for zoom lenses and valid focal/crop values when supplied.
- Positive calibration dimensions and frame rate.

The same checks run in the controller before an upload, including batch exports.
The interactive export path explains missing fields and can save an incomplete
profile locally. Existing calibrated-only upload restrictions and upload consent
remain in place.

## Verification

```sh
cargo test --manifest-path src/core/Cargo.toml --lib
cargo test --manifest-path src/core/Cargo.toml --lib --features bundle-lens-profiles
QT_QUICK_CONTROLS_STYLE=Basic qmltestrunner -input tests/qml -platform offscreen
cargo build
```

Core tests exercise catalogue replacement/fallback, aliases, mount direction,
sensor compatibility, local additions, alternate submissions, complete result
lists and submission validation. Qt tests exercise manual entry, filtering,
keyboard focus, large scrollable lists, metadata prefill, review and persistent
hide/restore. Standalone Qt tests do not embed the app's icon resources; the full
app build provides those resources.
