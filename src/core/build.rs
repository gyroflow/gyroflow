// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

fn main() {
    // Download lens profiles if not already present
    let project_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let db_path = format!("{project_dir}/../../resources/camera_presets/profiles.cbor.gz");
    if !std::path::Path::new(&db_path).exists() {
        if let Ok(mut body) = ureq::get("https://github.com/gyroflow/lens_profiles/releases/latest/download/profiles.cbor.gz").call().map(|x| x.into_reader()) {
            let mut file = std::fs::File::create(&db_path).unwrap();
            std::io::copy(&mut body, &mut file).unwrap();
        }
    }
}
