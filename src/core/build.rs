// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

fn main() {
    // Download lens profiles if not already present
    let project_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let db_path = format!("{project_dir}/../../resources/camera_presets/profiles.cbor.gz");
    if !std::path::Path::new(&db_path).exists() {
        std::fs::create_dir_all(&format!("{project_dir}/../../resources/camera_presets")).unwrap();
        if let Ok(mut body) = ureq::get("https://github.com/gyroflow/lens_profiles/releases/latest/download/profiles.cbor.gz").call().map(|x| x.into_body().into_reader()) {
            match std::fs::File::create(&db_path) {
                Ok(mut file) => { std::io::copy(&mut body, &mut file).unwrap(); },
                Err(e) => { panic!("Failed to create {db_path}: {e:?}"); }
            }
        }
    }
}
