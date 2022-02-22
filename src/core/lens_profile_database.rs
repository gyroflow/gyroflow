// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use walkdir::WalkDir;
use std::collections::HashMap;
use crate::LensProfile;
use std::path::PathBuf;

#[derive(Default)]
pub struct LensProfileDatabase {
    map: HashMap<String, LensProfile>,
    loaded: bool
}

impl LensProfileDatabase {
    pub fn get_path() -> PathBuf {
        let candidates = [
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            PathBuf::from("../Resources/camera_presets/"),
            PathBuf::from("./resources/camera_presets/"),
            PathBuf::from("./camera_presets/"),
            PathBuf::from("./lens_profiles/")
        ];
        let exe = std::env::current_exe().unwrap_or_default();
        let exe_parent = exe.parent();
        for path in &candidates {
            if let Ok(path) = std::fs::canonicalize(&path) {
                if path.exists() {
                    return path;
                }
            }
            if let Ok(path) = std::fs::canonicalize(exe_parent.map(|x| x.join(&path)).unwrap_or_default()) {
                if path.exists() {
                    return path;
                }
            }
        }
        if let Ok(path) = std::fs::canonicalize(exe_parent.map(|x| x.join("./camera_presets/")).unwrap_or_default()) {
            if !path.exists() {
                let _ = std::fs::create_dir_all(&path);
            }
            return path;
        }

        log::warn!("Unknown lens directory: {:?}, exe: {:?}", candidates[0], exe_parent);

        std::fs::canonicalize(&candidates[0]).unwrap_or_default()
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
                            for profile in v.get_all_matching_profiles() {
                                let key = if !profile.identifier.is_empty() { 
                                    profile.identifier.clone()
                                } else {
                                    f_name.clone()
                                };
                                if self.map.contains_key(&key) {
                                    if !self.loaded {
                                        log::warn!("Lens profile already present: {}, filename: {}", key, f_name);
                                    }
                                } else {
                                    self.map.insert(key, profile);
                                }
                            }
                        },
                        Err(e) => {
                            log::error!("Error parsing lens profile: {}: {:?}", f_name, e);
                        }
                    }
                }
            }
        });
        
        ::log::info!("Loaded {} lens profiles in {:.3}ms", self.map.len(), _time.elapsed().as_micros() as f64 / 1000.0);
        self.loaded = true;
    }

    pub fn get_all_names(&self) -> Vec<(String, String)> {
        let mut ret = Vec::with_capacity(self.map.len());
        for (k, v) in &self.map {
            if !v.camera_brand.is_empty() && !v.camera_model.is_empty() {
                if !v.is_copy {
                    ret.push((v.get_display_name(), k.clone()));
                }
            } else {
                log::debug!("Unknown camera model: {:?}", v);
            }
        }
        ret.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
        ret
    }

    pub fn contains_id(&self, id: &str) -> bool {
        self.map.contains_key(id)
    }
    pub fn get_by_id(&self, id: &str) -> Option<&LensProfile> {
        self.map.get(id)
    }
}
