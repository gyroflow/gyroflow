# Camera and lens catalogue

This catalogue describes camera bodies, lens names, aliases, mounts and crop
factors. It is not a distortion calibration. Only records loaded from Gyroflow
lens profiles are selectable for stabilization.

## Regeneration

Run the dependency-free generator against local source checkouts:

```sh
python3 _scripts/generate_camera_catalog.py \
  --profiles /path/to/lens_profiles \
  --lensfun /path/to/lensfun/data/db \
  --profiles-revision 886fc616c9c2c788856f54c21d451b9cb7fc4f44 \
  --lensfun-revision bbd4332a9ec566fd9aa548c9e0d8ced238c56261 \
  --output resources/camera_catalog.json
```

The generated document records both revision IDs and a digest of the consumed
input files. Unknown mounts, crop factors and physical sensor dimensions remain
unknown. A crop-factor-derived equivalent sensor diagonal is labelled as such;
it is not asserted to be a measured sensor width or height. A lens's maker is
never guessed from the camera manufacturer.

Put the same JSON at the root of `gyroflow/lens_profiles` as
`__camera_catalog.json`. The existing compression/release process includes JSON
files in `profiles.cbor.gz`; the new loader consumes this reserved metadata
entry while older clients already skip `__` entries. This updates metadata with
the same profile download, without additional HTTP requests or changing the
first release asset used by legacy clients. Invalid/incompatible metadata falls
back to the bundled catalogue. A loose cached `__camera_catalog.json` is also
supported and is not parsed as a lens profile.

Lensfun metadata comes from https://github.com/lensfun/lensfun. Its database is
licensed under Creative Commons Attribution-ShareAlike 3.0. Retain the copied
`resources/LENSFUN_DATABASE_LICENSE.txt` when redistributing derived catalogue
data. Gyroflow metadata comes from https://github.com/gyroflow/lens_profiles.
The generated record's `sources` lists identify the inputs and the generator
normalizes whitespace and merges aliases. No Lensfun distortion coefficients
are imported by this feature.

## Use

The main lens menu selects brand, camera model and lens before listing actual
calibrations. Previous/Next load each matching submission for visual review.
Hide/Restore changes only a local preference, never deletes the shared database
or posts a rating. Show hidden makes these choices reversible. Legacy search
remains available for presets and historical names.

The calibrator uses the same selectors and provides Other text fields for
unknown equipment. Video metadata pre-fills only exact names/aliases; ambiguous
or unknown names are preserved for manual correction. A refresh retains the
current selection rather than replacing it with stale video metadata.

Potentially compatible cameras must share an explicit mount and crop factor.
This is a browsing suggestion, not a guarantee that two recording modes have
the same calibrated geometry. Lenses known only to Lensfun appear as names but
cannot be loaded as nonexistent Gyroflow calibrations.

Both interactive and batch exports validate upload metadata at the controller
boundary. Specific camera brand/model and lens description are required. A zoom
lens also requires the focal length actually used. Incomplete local exports are
still allowed when upload is disabled; the interactive dialog offers that path.

## Focused checks

```sh
node _scripts/camera_catalog_checks.cjs
python3 -m unittest discover -s _scripts -p test_camera_catalog_generator.py
node _scripts/camera_catalog_checks.cjs --benchmark
```

The benchmark compares exact-selection lookup against a linear scan over a
synthetic catalogue and verifies identical match counts. It measures indexing
and lookup only, not full-application rendering or video stabilization. Rust
module tests cover malformed catalogues and upload requirements. A desktop
application run is still needed to validate native UI interaction and video
preview.
