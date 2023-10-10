// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use std::path::{ Path, PathBuf };

fn main() {
    // Download lens profiles if not already present
    let project_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    if !Path::new(&format!("{project_dir}/../../resources/camera_presets")).exists() {
        if let Ok(body) = ureq::get("https://github.com/gyroflow/lens_profiles/archive/refs/heads/main.tar.gz").call().map(|x| x.into_reader()) {
            let target_dir = PathBuf::from(format!("{project_dir}/../../resources/camera_presets"));

            let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(body));
            for mut file in archive.entries().unwrap().flatten() {
                let mut final_path = target_dir.clone();
                final_path.push(file.path().unwrap().components().skip(1).collect::<PathBuf>());
                if file.path().unwrap().to_string_lossy().ends_with('/') {
                    std::fs::create_dir_all(&final_path).unwrap();
                } else {
                    file.unpack(&final_path).unwrap();
                }
            }
        }
    }
}
