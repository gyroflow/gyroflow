# Camera catalogue attribution

`camera_database.json` is generated from the Gyroflow lens profiles (CC0-1.0)
and Lensfun camera/lens metadata (CC BY-SA 3.0). The combined catalogue is
licensed under CC BY-SA 3.0; see CAMERA_DATABASE_LICENSE.txt in this directory.
Lensfun data is copyright the Lensfun contributors. Names have been normalized,
aliases combined, source fields selected, focal ranges parsed and sensor sizes
calculated where the necessary data is available. Source revisions and licences
are recorded in the JSON.

The source data and reproducible generator are maintained at:
https://github.com/gyroflow/lens_profiles
https://github.com/lensfun/lensfun/tree/master/data

Copy a newly generated camera_database.json from the lens_profiles repository
into this directory when updating the bundled fallback. At runtime the app
reads updated catalogues from the reserved __camera_database entry in the
normal profiles.cbor.gz releases. Lensfun metadata in this catalogue does not
supply loadable Gyroflow distortion calibrations.
