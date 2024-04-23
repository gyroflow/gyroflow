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

    // Serialize to a compressed binary file, only if the profile is deploy
    if let Ok(out_dir) = std::env::var("OUT_DIR") {
        if out_dir.contains("\\deploy\\build\\") || out_dir.contains("/deploy/build/") || cfg!(any(target_os = "android", target_os = "ios", feature = "bundle-lens-profiles")) {
            let binary_path = format!("{project_dir}/../../resources/camera_presets/profiles.cbor.gz");
            if !Path::new(&binary_path).exists() {
                let mut all_profiles = Vec::new();
                walkdir::WalkDir::new(&format!("{project_dir}/../../resources/camera_presets/")).into_iter().for_each(|e| {
                    if let Ok(entry) = e {
                        let f_name = entry.path().to_string_lossy().replace('\\', "/");
                        if f_name.ends_with(".json") || f_name.ends_with(".gyroflow") {
                            if let Ok(data) = std::fs::read_to_string(&f_name) {
                                let parsed = serde_json::from_str::<serde_json::Value>(&data).unwrap();
                                let pos = f_name.find("camera_presets/").unwrap();
                                all_profiles.push((f_name[pos + 15..].to_owned(), parsed));
                            }
                        }
                    }
                });
                if !all_profiles.is_empty() {
                    use std::io::Write;
                    let mut file = std::fs::File::create(&binary_path).unwrap();
                    let mut data = Vec::<u8>::new();
                    ciborium::into_writer(&all_profiles, &mut data).unwrap();

                    let mut e = flate2::write::GzEncoder::new(&mut file, flate2::Compression::best());
                    e.write_all(&data).unwrap();
                }
            }
        }
    }
}
