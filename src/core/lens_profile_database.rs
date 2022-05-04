// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use walkdir::WalkDir;
use std::collections::{ HashSet, HashMap };
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
                if f_name.ends_with(".json") {
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
                                        log::warn!("Lens profile already present: {}, filename: {} from {}", key, f_name, self.map.get(&key).unwrap().filename);
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
        let mut set = HashSet::with_capacity(self.map.len());
        let mut ret = Vec::with_capacity(self.map.len());
        for (k, v) in &self.map {
            if !v.camera_brand.is_empty() && !v.camera_model.is_empty() {
                if !v.is_copy {
                    let name = v.get_display_name();
                    let mut new_name = name.clone();
                    let mut i = 2;
                    while set.contains(&new_name) {
                        new_name = format!("{} - {}", name, i);
                        i += 1;
                    }
                    set.insert(new_name.clone());
                    ret.push((new_name, k.clone()));
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

    // -------------------------------------------------------------------
    // ---------------------- Maintenance functions ----------------------
    // -------------------------------------------------------------------

    pub fn list_all_metadata(&self) {
        fn q(s: &String) -> String {
            serde_json::to_string(&serde_json::Value::String(s.clone())).unwrap()
        }

        let mut lines = Vec::new();
        let path = Self::get_path().to_string_lossy().replace('\\', "/");
        for (_k, v) in &self.map {
            lines.push(format!("[{:<50}, {:<50}, {:<50}, {:<80}, {}],", q(&v.camera_brand), q(&v.camera_model), q(&v.lens_model), q(&v.camera_setting), q(&v.filename.replace(&path, ""))));
        }
        lines.sort_by(|a, b| a.to_lowercase().trim().cmp(&b.to_lowercase().trim()));
        let mut out: String = "[\n".into();
        for l in lines {
            out.push_str(&l);
            out.push('\n');
        }
        out.push(']');
        std::fs::write(path + "/_metadata.json", out).unwrap();
    }

    pub fn process_adjusted_metadata(&self) {
        let mut path = Self::get_path();
        path.push("_metadata.json");
        let content = std::fs::read(path).unwrap();
        let content: serde_json::Value = serde_json::from_slice(&content).unwrap();

        for x in content.as_array().unwrap() {
            let x: Vec<String> = x.as_array().unwrap().into_iter().map(|v| v.as_str().unwrap().to_string()).collect();
            let (brand, model, lens_model, camera_setting, fname) = (&x[0], &x[1], &x[2], &x[3], &x[4]);

            let mut old_path = Self::get_path();
            old_path.push(&fname[1..]);

            let mut cam_setting = LensProfile::cleanup_name(camera_setting.clone()).trim().to_string();

            if let Ok(prof) = std::fs::read(&old_path) {
                let parsed = LensProfile::from_json(&String::from_utf8_lossy(&prof)).unwrap();
                if parsed.calibrated_by == "Eddy" {
                    // These are solid, skip any changes
                    continue;
                }

                cam_setting = cam_setting
                    .replace(&format!("{}x{}", parsed.calib_dimension.w, parsed.calib_dimension.h), "")
                    .replace(&format!("{}p", parsed.calib_dimension.h), "")
                    .replace(&parsed.get_aspect_ratio(), "")
                    .replace(parsed.get_size_str(), "")
                    .replace("1080", "")
                    .replace("2160", "")
                    .replace("C4K", "")
                    .replace("UHD", "");
                if parsed.fps > 0.0 {
                    cam_setting = cam_setting
                        .replace(&format!("{:.0}p", parsed.fps), "")
                        .replace(&format!("{:.2}p", parsed.fps), "")
                        .replace(&format!("{:.3}p", parsed.fps), "")
                        .replace(&format!("{:.0}fps", parsed.fps), "")
                        .replace(&format!("{:.2}fps", parsed.fps), "")
                        .replace(&format!("{:.3}fps", parsed.fps), "")
                        .replace(&format!("{:.0} fps", parsed.fps), "")
                        .replace(&format!("{:.2} fps", parsed.fps), "")
                        .replace(&format!("{:.3} fps", parsed.fps), "")
                        .replace(&format!("{:.0}P", parsed.fps), "")
                        .replace(&format!("{:.2}P", parsed.fps), "")
                        .replace(&format!("{:.3}P", parsed.fps), "")
                        .replace(&format!("{:.0}FPS", parsed.fps), "")
                        .replace(&format!("{:.2}FPS", parsed.fps), "")
                        .replace(&format!("{:.3}FPS", parsed.fps), "")
                        .replace(&format!("{:.0} FPS", parsed.fps), "")
                        .replace(&format!("{:.2} FPS", parsed.fps), "")
                        .replace(&format!("{:.3} FPS", parsed.fps), "");
                }
                cam_setting = cam_setting.trim().to_string();

                let mut prof: serde_json::Value = serde_json::from_slice(&prof).unwrap();
                *prof.get_mut("camera_brand")  .unwrap() = serde_json::Value::String(brand.clone());
                *prof.get_mut("camera_model")  .unwrap() = serde_json::Value::String(model.clone());
                *prof.get_mut("lens_model")    .unwrap() = serde_json::Value::String(lens_model.clone());
                *prof.get_mut("camera_setting").unwrap() = serde_json::Value::String(cam_setting);

                let parsed = LensProfile::from_json(&serde_json::to_string_pretty(&prof).unwrap()).unwrap();
                *prof.get_mut("name").unwrap() = serde_json::Value::String(parsed.get_name());
                
                let new_prof = serde_json::to_string_pretty(&prof).unwrap();
                //dbg!(new_prof);
                let new_filename = parsed.get_name().chars().filter(|c| c.is_ascii()).collect::<String>()
                    .replace("\n", "")
                    .replace("|", "_")
                    .replace("*", "_")
                    .replace(":", "_")
                    .replace("<", "")
                    .replace("\"", "")
                    .replace(">", "")
                    .replace("/", "")
                    .replace("\\", "")
                    .trim().to_string();

                let mut new_path = old_path.with_file_name(format!("{}.json", new_filename));
                let mut i = 2;
                if new_path != old_path {
                    while new_path.exists() {
                        new_path = old_path.with_file_name(format!("{} - {}.json", new_filename, i));
                        i += 1;
                    }
                }
                
                if std::fs::write(&new_path, new_prof).is_ok() {
                    if new_path != old_path {
                        if std::fs::remove_file(&old_path).is_err() {
                            println!("Remove error {:?}", old_path);
                        }
                    }
                } else {
                    println!("Write error {:?}", new_path);
                }
            }
        }
    }
}
