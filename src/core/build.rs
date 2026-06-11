// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

fn main() {
    // Download lens profiles if not already present
    let project_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let resource_dir = format!("{project_dir}/../../resources/camera_presets");
    let db_path = format!("{resource_dir}/profiles.cbor.gz");
    if !std::path::Path::new(&db_path).exists() {
        std::fs::create_dir_all(&resource_dir).unwrap();
        if let Ok(mut body) = ureq::get("https://github.com/gyroflow/lens_profiles/releases/latest/download/profiles.cbor.gz").call().map(|x| x.into_body().into_reader()) {
            match std::fs::File::create(&db_path) {
                Ok(mut file) => { std::io::copy(&mut body, &mut file).unwrap(); },
                Err(e) => { panic!("Failed to create {db_path}: {e:?}"); }
            }
        }
    }

    let canonical_path = format!("{resource_dir}/canonical_lenses.json");
    if !std::path::Path::new(&canonical_path).exists() {
        std::fs::create_dir_all(&resource_dir).unwrap();
        if let Ok(mut body) = ureq::get("https://github.com/gyroflow/lens_profiles/releases/latest/download/canonical_lenses.json").call().map(|x| x.into_body().into_reader()) {
            match std::fs::File::create(&canonical_path) {
                Ok(mut file) => { std::io::copy(&mut body, &mut file).unwrap(); },
                Err(e) => { panic!("Failed to create {canonical_path}: {e:?}"); }
            }
        }
    }
}
