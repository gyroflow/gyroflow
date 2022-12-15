// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::{ HashSet, HashMap, BTreeMap };
use crate::LensProfile;
use std::path::PathBuf;

#[cfg(target_os = "android")]
static LENS_PROFILES_STATIC: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/../../resources/camera_presets/");

#[derive(Default)]
pub struct LensProfileDatabase {
    map: HashMap<String, LensProfile>,
    loaded_callbacks: Vec<Box<dyn FnOnce(&Self) + Send + Sync + 'static>>,
    loaded: bool
}
impl Clone for LensProfileDatabase {
    fn clone(&self) -> Self {
        Self { map: self.map.clone(), loaded: self.loaded, ..Default::default() }
    }
}

impl LensProfileDatabase {
    pub fn get_path() -> PathBuf {
        // return std::fs::canonicalize("D:/lens_review/").unwrap_or_default();

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

        let mut load = |data: &str, f_name: &str| {
            if f_name.ends_with(".gyroflow") {
                let mut profile = LensProfile::default();
                profile.name = std::path::Path::new(f_name).file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
                profile.filename = f_name.to_string();
                profile.checksum = Some(format!("{:08x}", crc32fast::hash(profile.filename.as_bytes())));
                self.map.insert(f_name.to_string(), profile);
                return;
            }
            match LensProfile::from_json(data) {
                Ok(mut v) => {
                    v.filename = f_name.to_string();
                    for mut profile in v.get_all_matching_profiles() {
                        let key = if !profile.identifier.is_empty() {
                            profile.identifier.clone()
                        } else {
                            f_name.to_string()
                        };
                        if self.map.contains_key(&key) {
                            if !self.loaded {
                                log::warn!("Lens profile already present: {}, filename: {} from {}", key, f_name, self.map.get(&key).unwrap().filename);
                            }
                        } else {
                            (|| -> Option<()> {
                                let to_checksum = format!("{}|{}{}|{:.8}{:.8}|{:.8}{:.8}|{:.8}{:.8}{:.8}{:.8}",
                                    profile.identifier,

                                    profile.calib_dimension.w,
                                    profile.calib_dimension.h,

                                    profile.fisheye_params.camera_matrix.get(0)?.get(0)?,
                                    profile.fisheye_params.camera_matrix.get(1)?.get(1)?,
                                    profile.fisheye_params.camera_matrix.get(0)?.get(2)?,
                                    profile.fisheye_params.camera_matrix.get(1)?.get(2)?,

                                    profile.fisheye_params.distortion_coeffs.get(0).unwrap_or(&0.0),
                                    profile.fisheye_params.distortion_coeffs.get(1).unwrap_or(&0.0),
                                    profile.fisheye_params.distortion_coeffs.get(2).unwrap_or(&0.0),
                                    profile.fisheye_params.distortion_coeffs.get(3).unwrap_or(&0.0)
                                );

                                profile.checksum = Some(format!("{:08x}", crc32fast::hash(to_checksum.as_bytes())));
                                Some(())
                            })();
                            self.map.insert(key, profile);
                        }
                    }
                },
                Err(e) => {
                    log::error!("Error parsing lens profile: {}: {:?}", f_name, e);
                }
            }
        };

        #[cfg(target_os = "android")]
        for entry in LENS_PROFILES_STATIC.find("**/*").unwrap() {
            if let Some(data) = entry.as_file().and_then(|x| x.contents_utf8()) {
                load(data, &entry.path().display().to_string());
            }
        }

        #[cfg(not(target_os = "android"))]
        walkdir::WalkDir::new(Self::get_path()).into_iter().for_each(|e| {
            if let Ok(entry) = e {
                let f_name = entry.path().to_string_lossy().replace('\\', "/");
                if f_name.ends_with(".json") || f_name.ends_with(".gyroflow") {
                    if let Ok(data) = std::fs::read_to_string(&f_name) {
                        load(&data, &f_name);
                    }
                }
            }
        });

        let copy = self.clone();
        for (_, v) in self.map.iter_mut() {
            v.resolve_interpolations(&copy);
        }

        ::log::info!("Loaded {} lens profiles in {:.3}ms", self.map.len(), _time.elapsed().as_micros() as f64 / 1000.0);
        self.loaded = true;
    }

    pub fn set_from_db(&mut self, b: Self) {
        self.map = b.map;
        self.loaded = b.loaded;
        if self.loaded {
            let cbs: Vec<_> = self.loaded_callbacks.drain(..).collect();
            for cb in cbs {
                cb(&self);
            }
        }
    }

    pub fn get_all_info(&self) -> Vec<(String, String, String, bool, f64, i32)> {
        // (name, filename, crc32, official, rating, aspect_ratio*1000)
        let mut set = HashSet::with_capacity(self.map.len());
        let mut checksum_map = HashMap::with_capacity(self.map.len());
        let mut ret = Vec::with_capacity(self.map.len());
        for (k, v) in &self.map {
            if v.filename.ends_with(".gyroflow") {
                ret.push((v.name.clone(), k.clone(), v.checksum.clone().unwrap_or_default(), v.official, v.rating.clone().unwrap_or_default(), 0));
            } else if !v.camera_brand.is_empty() && !v.camera_model.is_empty() {
                if !v.is_copy {
                    let mut name = v.get_display_name();
                    let mut new_name = name.clone();
                    if set.contains(&new_name) {
                        if let Some((kk, vv, _, _, _, _)) = ret.iter_mut().find(|(k, _, _, _, _, _)| *k == new_name) {
                            set.remove(kk);
                            *kk = format!("{} by {}", *kk, self.map[vv].calibrated_by);
                            set.insert(kk.clone());
                        }
                        name = format!("{} by {}", name, v.calibrated_by);
                        new_name = name.clone();
                    }
                    let mut i = 2;
                    while set.contains(&new_name) {
                        new_name = format!("{} - {}", name, i);
                        i += 1;
                    }
                    set.insert(new_name.clone());

                    let hstretch = if v.input_horizontal_stretch > 0.01 { v.input_horizontal_stretch } else { 1.0 };
                    let vstretch = if v.input_vertical_stretch   > 0.01 { v.input_vertical_stretch   } else { 1.0 };

                    let aspect_ratio = (((v.calib_dimension.w as f64 / hstretch) / (v.calib_dimension.h.max(1) as f64 / vstretch)) * 1000.0).round() as i32;
                    ret.push((new_name, k.clone(), v.checksum.clone().unwrap_or_default(), v.official, v.rating.clone().unwrap_or_default(), aspect_ratio));
                }
            } else {
                log::debug!("Unknown camera model: {:?}", v);
            }
            if let Some(dup) = checksum_map.get(&v.checksum) {
                log::error!("Duplicated lens profile! {} vs {}", dup, v.filename);
            } else {
                checksum_map.insert(v.checksum.clone(), v.filename.clone());
            }
        }
        ret.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
        ret
    }

    pub fn set_profile_ratings(&mut self, json: &str) {
        if let Ok(serde_json::Value::Object(v)) = serde_json::from_str(json) as serde_json::Result<serde_json::Value> {
            let final_ratings: HashMap<String, f64> = v.into_iter().filter_map(|(k, arr)| {
                if let serde_json::Value::Array(arr) = arr {
                    if arr.len() == 3 {
                        let _good = arr[0].as_i64().unwrap_or_default();
                        let _bad = arr[1].as_i64().unwrap_or_default();
                        let final_rating = arr[2].as_f64().unwrap_or_default();
                        return Some((k, final_rating));
                    }
                }
                None
            }).collect();

            for (_, v) in self.map.iter_mut() {
                if let Some(crc) = &v.checksum {
                    v.rating = final_ratings.get(crc).copied();
                }
            }
        }
    }

    pub fn on_loaded<F: FnOnce(&Self) + Send + Sync + 'static>(&mut self, cb: F) {
        if self.loaded {
            cb(self);
        } else {
            self.loaded_callbacks.push(Box::new(cb));
        }
    }
    pub fn contains_id(&self, id: &str) -> bool {
        self.map.contains_key(id)
    }
    pub fn get_by_id(&self, id: &str) -> Option<&LensProfile> {
        self.map.get(id)
    }
    pub fn find(&self, filename_or_id: &str) -> Option<&LensProfile> {
        if let Some(l) = self.map.get(filename_or_id) {
            Some(l)
        } else {
            self.map.iter().find(|(_, v)| v.filename.contains(filename_or_id)).map(|(_, v)| v)
        }
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
        let mut coeffs_map = BTreeMap::new();
        for (_k, v) in &self.map {
            if !v.is_copy {
                let coeffs = format!("{:?}", v.fisheye_params.distortion_coeffs);
                if coeffs_map.contains_key(&coeffs) {
                    println!("Duplicate profile:\n{}\n{}\n", coeffs_map[&coeffs], v.filename.replace(&path, ""))
                }
                coeffs_map.insert(coeffs, v.filename.replace(&path, ""));
                lines.push(format!("[{:<50}, {:<50}, {:<50}, {:<80}, {}],", q(&v.camera_brand), q(&v.camera_model), q(&v.lens_model), q(&v.camera_setting), q(&v.filename.replace(&path, ""))));
            }
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
            if fname.is_empty() { continue; }
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
                let mut calibrated_by = prof.get("calibrated_by").and_then(|x| x.as_str().map(|x| x.to_string())).unwrap_or_default();
                if !calibrated_by.chars().all(|c| c.is_ascii()) {
                    // println!("Non-ascii author: {}", calibrated_by);
                    calibrated_by.clear();
                }

                let new_prof = serde_json::to_string_pretty(&prof).unwrap();
                //dbg!(new_prof);
                let mut new_filename = parsed.get_name().chars().filter(|c| c.is_ascii()).collect::<String>()
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
                    if !calibrated_by.is_empty() && new_path.exists() {
                        new_filename.push_str(" - ");
                        new_filename.push_str(&calibrated_by);
                        new_path = old_path.with_file_name(format!("{}.json", new_filename));
                    }
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
