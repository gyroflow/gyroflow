// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use walkdir::WalkDir;
use std::collections::HashMap;
use crate::LensProfile;
use itertools::Itertools;
use std::path::PathBuf;

#[derive(Default)]
pub struct LensProfileDatabase {
    map: HashMap<String, LensProfile>
}

impl LensProfileDatabase {
    pub fn get_path() -> PathBuf {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let path = PathBuf::from("../Resources/camera_presets/");
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        let path = PathBuf::from("./resources/camera_presets/");

        let candidates = [
            std::fs::canonicalize(&path).unwrap_or_default(),
            std::fs::canonicalize(std::env::current_exe().unwrap_or_default().parent().map(|x| x.join(&path)).unwrap_or_default()).unwrap_or_default()
        ];
        for x in candidates {
            if x.exists() {
                return x;
            }
        }

        std::fs::canonicalize(&path).unwrap_or_default()
    }

    pub fn load_all(&mut self) {
        log::info!("Lens profiles directory: {:?}", Self::get_path());

        let _time = std::time::Instant::now();
        
        WalkDir::new(Self::get_path()).into_iter().for_each(|e| {
            if let Ok(entry) = e {
                let f_name = entry.path().to_string_lossy().replace('\\', "/");
                if f_name.ends_with(".json") && !f_name.contains("/Legacy/") {
                    let mut data = std::fs::read_to_string(&f_name).unwrap();
                    match LensProfile::from_json(&mut data) {
                        Ok(mut v) => {
                            v.filename = f_name.clone();
                            let key = if !v.identifier.is_empty() { 
                                v.identifier.clone()
                            } else {
                                f_name
                            };
                            self.map.insert(key, v);
                        },
                        Err(e) => {
                            log::error!("Error parsing lens profile: {}: {:?}", f_name, e);
                        }
                    }
                }
            }
        });
        
        ::log::info!("Loaded lens profiles in {:.3}ms", _time.elapsed().as_micros() as f64 / 1000.0);
    }

    pub fn get_all_names(&self) -> Vec<(String, String)> {
        let mut ret = Vec::with_capacity(self.map.len());
        for v in self.map.values() {
            if !v.camera_brand.is_empty() && !v.camera_model.is_empty() {
                let strs = vec![&v.camera_brand, &v.camera_model, &v.lens_model, &v.camera_setting, &v.note].into_iter().filter(|x| !x.is_empty()).join(" ");

                ret.push((format!("{} {} {} {}x{}", self.cleanup_name(strs), v.get_size_str(), v.get_aspect_ratio(), v.calib_dimension.w, v.calib_dimension.h), v.filename.clone()));
            } else {
                log::debug!("Unknown camera model: {:?}", v);
            }
        }
        ret.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
        ret
    }
    pub fn cleanup_name(&self, name: String) -> String {
        name.replace(".json", "")
            .replace("4_3", "")
            .replace("4:3", "")
            .replace("4by3", "")
            .replace("16:9", "")
            .replace("169", "")
            .replace("16_9", "")
            .replace("16*9", "")
            .replace("16/9", "")
            .replace("16by9", "")
            .replace("2_7K", "")
            .replace("2,7K", "")
            .replace("2.7K", "")
            .replace("4K", "")
            .replace("5K", "")
            .replace('_', " ")
    }

    pub fn get_by_id(&self, id: &str) -> Option<&LensProfile> {
        self.map.get(id)
    }
}
