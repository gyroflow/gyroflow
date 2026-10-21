// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::cmp::Ordering;
use std::collections::{ HashSet, HashMap, BTreeMap };
use crate::LensProfile;
use crate::camera_registry::CameraRegistry;
use std::path::PathBuf;
use std::io::Read;

#[cfg(any(target_os = "android", target_os = "ios", feature = "bundle-lens-profiles"))]
static LENS_PROFILES_STATIC: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/camera_presets/profiles.cbor.gz"));

static CAMERA_REGISTRY_STATIC: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/camera_registry.json"));

#[allow(dead_code)]
enum DataSource {
    String(String),
    SerdeValue(serde_json::Value)
}

#[derive(Default)]
pub struct LensProfileDatabase {
    preset_map: HashMap<String, String>,
    map: HashMap<String, LensProfile>,
    loaded_callbacks: Vec<Box<dyn FnOnce(&Self) + Send + Sync + 'static>>,
    list_for_ui: Vec<(String, String, String, bool, f64, i32, String)>,
    pub loaded: bool,
    pub version: u32,
    pub camera_registry: CameraRegistry,
    hidden_profiles: HashSet<String>,
    review_groups: HashMap<String, Vec<String>>,
}
impl Clone for LensProfileDatabase {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            preset_map: self.preset_map.clone(),
            loaded: self.loaded,
            camera_registry: self.camera_registry.clone(),
            hidden_profiles: self.hidden_profiles.clone(),
            review_groups: self.review_groups.clone(),
            ..Default::default()
        }
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

        let mut load = |data: DataSource, f_name: &str| {
            if f_name.ends_with(".gyroflow") {
                let mut profile = LensProfile::default();
                profile.name = std::path::Path::new(f_name).file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
                profile.path_to_file = f_name.to_string();
                profile.checksum = Some(format!("{:08x}", crc32fast::hash(profile.path_to_file.as_bytes())));
                self.map.insert(f_name.to_string(), profile);
                match data {
                    DataSource::String(v)     => { self.preset_map.insert(f_name.to_string(), v); }
                    DataSource::SerdeValue(v) => { self.preset_map.insert(f_name.to_string(), serde_json::to_string(&v).unwrap()); }
                }
                return;
            }
            let parsed = match data {
                DataSource::String(x)     => LensProfile::from_json(&x),
                DataSource::SerdeValue(x) => LensProfile::from_value(x)
            };
            match parsed {
                Ok(mut v) => {
                    v.path_to_file = f_name.to_string();
                    for mut profile in v.get_all_matching_profiles() {
                        let key = if !profile.identifier.is_empty() {
                            profile.identifier.clone()
                        } else {
                            f_name.to_string()
                        };
                        if self.map.contains_key(&key) {
                            if !self.loaded {
                                log::warn!("Lens profile already present: {}, path_to_file: {} from {}", key, f_name, self.map.get(&key).unwrap().path_to_file);

                                // let prof = std::fs::read(&f_name).unwrap();
                                // let mut prof: serde_json::Value = serde_json::from_slice(&prof).unwrap();
                                // *prof.get_mut("identifier").unwrap() = serde_json::Value::String(String::new());
                                // std::fs::write(f_name, serde_json::to_string_pretty(&prof).unwrap()).unwrap();
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

        let mut bundle_loaded = false;

        let mut load_from_dir = |dir: PathBuf| {
            walkdir::WalkDir::new(dir).into_iter().for_each(|e| {
                if let Ok(entry) = e {
                    let f_name = entry.path().to_string_lossy().replace('\\', "/");
                    if f_name.ends_with(".json") || f_name.ends_with(".gyroflow") {
                        if let Ok(data) = std::fs::read_to_string(&f_name) {
                            load(DataSource::String(data), &f_name);
                        }
                    }
                    if !bundle_loaded && f_name.ends_with(".cbor.gz") {
                        if let Ok(data) = std::fs::read(&f_name) {
                            let mut e = flate2::read::GzDecoder::new(std::io::Cursor::new(data));
                            let mut decompressed = Vec::new();
                            if let Ok(_) = e.read_to_end(&mut decompressed) {
                                if let Ok(array) = ciborium::from_reader::<Vec<(String, serde_json::Value)>, _>(std::io::Cursor::new(decompressed)) {
                                    bundle_loaded = true;
                                    for (f_name, profile) in array {
                                        if f_name == "__version" { self.version = profile.as_u64().unwrap_or(0) as u32; continue; }
                                        if f_name.starts_with("__") { continue; }
                                        load(DataSource::SerdeValue(profile), &f_name);
                                    }
                                }
                            }
                        }
                    }
                }
            });
        };

        let preferred_path = crate::settings::data_dir().join("lens_profiles");
        if preferred_path.exists() && std::fs::read_dir(&preferred_path).map(|x| x.count()).unwrap_or(0) > 0 {
            ::log::info!("Loading lens profiles from {}", preferred_path.display());
            load_from_dir(preferred_path);
        }

        #[cfg(any(target_os = "android", target_os = "ios", feature = "bundle-lens-profiles"))]
        if !bundle_loaded {
            let mut e = flate2::read::GzDecoder::new(LENS_PROFILES_STATIC);
            let mut decompressed = Vec::new();
            if let Ok(_) = e.read_to_end(&mut decompressed) {
                if let Ok(array) = ciborium::from_reader::<Vec<(String, serde_json::Value)>, _>(std::io::Cursor::new(decompressed)) {
                    for (f_name, profile) in array {
                        if f_name == "__version" { self.version = profile.as_u64().unwrap_or(0) as u32; continue; }
                        if f_name.starts_with("__") { continue; }
                        load(DataSource::SerdeValue(profile), &f_name);
                    }
                }
            }
        }

        #[cfg(not(any(target_os = "android", target_os = "ios", feature = "bundle-lens-profiles")))]
        {
            load_from_dir(Self::get_path());
        }

        let copy = self.clone();
        for (_, v) in self.map.iter_mut() {
            v.resolve_interpolations(&copy);
        }

        self.load_camera_registry();
        self.build_review_groups();

        ::log::info!("Loaded {} lens profiles in {:.3}ms", self.map.len(), _time.elapsed().as_micros() as f64 / 1000.0);
        self.loaded = true;
    }

    fn load_camera_registry(&mut self) {
        if let Ok(registry) = CameraRegistry::load_from_json(std::str::from_utf8(CAMERA_REGISTRY_STATIC).unwrap_or("{}")) {
            self.camera_registry = registry;
            ::log::info!("Loaded camera registry with {} brands", self.camera_registry.brands.len());
        } else {
            ::log::warn!("Failed to load bundled camera registry");
        }

        let registry_path = Self::get_path().join("camera_registry.json");
        if registry_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&registry_path) {
                if let Ok(registry) = CameraRegistry::load_from_json(&content) {
                    if registry.version > self.camera_registry.version {
                        self.camera_registry = registry;
                        ::log::info!("Updated camera registry from file");
                    }
                }
            }
        }
    }

    fn build_review_groups(&mut self) {
        self.review_groups.clear();
        for (key, profile) in &self.map {
            if profile.path_to_file.ends_with(".gyroflow") {
                continue;
            }
            let group_key = format!(
                "{}|{}|{}|{}x{}",
                profile.camera_brand.to_lowercase(),
                profile.camera_model.to_lowercase(),
                profile.lens_model.to_lowercase(),
                profile.calib_dimension.w,
                profile.calib_dimension.h
            );
            self.review_groups.entry(group_key).or_default().push(key.clone());
        }
    }

    pub fn set_from_db(&mut self, b: Self) {
        self.map = b.map;
        self.preset_map = b.preset_map;
        self.loaded = b.loaded;
        self.version = b.version;
        self.camera_registry = b.camera_registry;
        self.review_groups = b.review_groups;
        if self.loaded {
            let cbs: Vec<_> = self.loaded_callbacks.drain(..).collect();
            for cb in cbs {
                cb(&self);
            }
        }
    }

    pub fn get_all_filenames(&self) -> Vec<String> {
        self.map.values().map(|x| {
            let mut path = x.path_to_file.clone();
            path = path.replace('\\', "/");
            if let Some(pos) = path.find("camera_presets/") {
                path = path[pos + 15..].to_string();
            }
            path
        }).collect()
    }

    pub fn prepare_list_for_ui(&mut self) {
        // (name, path_to_file, crc32, official, rating, aspect_ratio*1000, author)
        let mut set = HashSet::with_capacity(self.map.len());
        let mut checksum_map = HashMap::with_capacity(self.map.len());
        self.list_for_ui = Vec::with_capacity(self.map.len());
        for (k, v) in &self.map {
            if v.path_to_file.ends_with(".gyroflow") {
                self.list_for_ui.push((v.name.clone(), k.clone(), v.checksum.clone().unwrap_or_default(), v.official, v.rating.clone().unwrap_or_default(), 0, v.calibrated_by.clone()));
            } else if !v.camera_brand.is_empty() && !v.camera_model.is_empty() {
                if !v.is_copy {
                    let mut name = v.get_display_name();
                    let mut new_name = name.clone();
                    if set.contains(&new_name) {
                        if let Some((kk, vv, _, _, _, _, _)) = self.list_for_ui.iter_mut().find(|(k, _, _, _, _, _, _)| *k == new_name) {
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
                    self.list_for_ui.push((new_name, k.clone(), v.checksum.clone().unwrap_or_default(), v.official, v.rating.clone().unwrap_or_default(), aspect_ratio, v.calibrated_by.clone()));
                }
            } else {
                log::debug!("Unknown camera model: {:?}", v);
            }
            if let Some(dup) = checksum_map.get(&v.checksum) {
                log::error!("Duplicated lens profile! {} vs {}", dup, v.path_to_file);
            } else {
                checksum_map.insert(v.checksum.clone(), v.path_to_file.clone());
            }
        }
        self.list_for_ui.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
    }

    pub fn search(&self, text: &str, favorites: &HashSet<String>, aspect_ratio: i32, aspect_ratio_swapped: i32) -> Vec<(String, String, String, bool, f64, i32, String)> {
        let text = text.to_ascii_lowercase()
            .replace("bmpcc4k",  "blackmagic pocket cinema camera 4k")
            .replace("bmpcc6k",  "blackmagic pocket cinema camera 6k")
            .replace("bmpcc",    "blackmagic pocket cinema camera")
            .replace("gopro5",   "hero5 black") .replace("gopro 5",  "hero5 black")
            .replace("gopro6",   "hero6 black") .replace("gopro 6",  "hero6 black")
            .replace("gopro7",   "hero7 black") .replace("gopro 7",  "hero7 black")
            .replace("gopro8",   "hero8 black") .replace("gopro 8",  "hero8 black")
            .replace("gopro9",   "hero9 black") .replace("gopro 9",  "hero9 black")
            .replace("gopro10",  "hero10 black").replace("gopro 10", "hero10 black")
            .replace("gopro11",  "hero11 black").replace("gopro 11", "hero11 black")
            .replace("gopro12",  "hero11 black").replace("gopro 12", "hero11 black")
            .replace("gopro13",  "hero11 black").replace("gopro 13", "hero11 black")
            .replace("session5", "hero5 session") .replace("session 5", "hero5 session")
            .replace("a73",      "a7iii")
            .replace("a74",      "a7iv")
            .replace("a75",      "a7v")
            .replace("a7r3",     "a7riii")
            .replace("a7r4",     "a7riv")
            .replace("a7r5",     "a7rv")
            .replace("a7s2",     "a7sii")
            .replace("a7s3",     "a7siii")
            .replace(",", " ")
            .replace(";", " ");

        let words = text.split_ascii_whitespace().map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>();
        if words.is_empty() {
            return Vec::new();
        }

        let mut filtered = self.list_for_ui.iter().filter(|(name, _, _, _, _, _, author)| {
            let name = name.to_ascii_lowercase();
            let author = author.to_ascii_lowercase();
            for word in &words {
                if !name.contains(word) && !author.contains(word) {
                    return false;
                }
            }
            return true;
        }).collect::<Vec<_>>();

        filtered.sort_by(|a, b| {
            // Is preset or favorited
            let a_priority = a.1.ends_with(".gyroflow") || favorites.contains(&a.2);
            let b_priority = b.1.ends_with(".gyroflow") || favorites.contains(&b.2);
            if a_priority && !b_priority { return Ordering::Less; }
            if b_priority && !a_priority { return Ordering::Greater; }

            // Check aspect match
            let a_priority2 = a.5 != 0 && aspect_ratio == a.5;
            let b_priority2 = b.5 != 0 && aspect_ratio == b.5;
            if a_priority2 && !b_priority2 { return Ordering::Less; }
            if b_priority2 && !a_priority2 { return Ordering::Greater; }

            // Check swapped aspect match
            let a_priority3 = a.5 != 0 && aspect_ratio_swapped == a.5;
            let b_priority3 = b.5 != 0 && aspect_ratio_swapped == b.5;
            if a_priority3 && !b_priority3 { return Ordering::Less; }
            if b_priority3 && !a_priority3 { return Ordering::Greater; }

            a.0.cmp(&b.0)
        });

        filtered.into_iter().take(200).cloned().collect()
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
    pub fn get_preset_by_id(&self, id: &str) -> Option<String> {
        self.preset_map.get(id).cloned()
    }
    pub fn get_by_id(&self, id: &str) -> Option<&LensProfile> {
        self.map.get(id)
    }
    pub fn find(&self, filename_or_id: &str) -> Option<&LensProfile> {
        if let Some(l) = self.map.get(filename_or_id) {
            Some(l)
        } else {
            let path_normalized = filename_or_id.replace('\\', "/");
            self.map.iter().find(|(_, v)| v.path_to_file.replace('\\', "/").contains(&path_normalized)).map(|(_, v)| v)
        }
    }

    // -------------------------------------------------------------------
    // ---------------------- Maintenance functions ----------------------
    // -------------------------------------------------------------------

    pub fn list_all_metadata(&self) {
        fn q(s: &String) -> String { serde_json::to_string(&serde_json::Value::String(s.clone())).unwrap() }
        // fn qf(s: &Option<f64>) -> String { serde_json::to_string::<Option<f64>>(&s.clone().into()).unwrap() }
        // fn qb(s: bool) -> String { serde_json::to_string(&serde_json::Value::Bool(s)).unwrap() }

        let mut lines = Vec::new();
        let path = Self::get_path().to_string_lossy().replace('\\', "/");
        let mut coeffs_map = BTreeMap::new();
        for (_k, v) in &self.map {
            if !v.is_copy {
                let coeffs = format!("{:?}", v.fisheye_params.distortion_coeffs);
                if coeffs_map.contains_key(&coeffs) {
                    println!("Duplicate profile:\n{}\n{}\n", coeffs_map[&coeffs], v.path_to_file.replace(&path, ""))
                }
                coeffs_map.insert(coeffs, v.path_to_file.replace(&path, ""));
                lines.push(format!("[{:<50}, {:<50}, {:<50}, {:<80}, {}],", q(&v.camera_brand), q(&v.camera_model), q(&v.lens_model), q(&v.camera_setting), q(&v.path_to_file.replace(&path, ""))));
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

    // -------------------------------------------------------------------
    // ---------------------- Camera selector methods --------------------
    // -------------------------------------------------------------------

    pub fn get_camera_brands(&self) -> Vec<String> {
        let mut brands: HashSet<String> = self.camera_registry.get_brands()
            .into_iter()
            .map(|s| s.to_string())
            .collect();

        for profile in self.map.values() {
            if !profile.camera_brand.is_empty() {
                brands.insert(self.camera_registry.normalize_brand(&profile.camera_brand));
            }
        }

        let mut result: Vec<_> = brands.into_iter().collect();
        result.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        result
    }

    pub fn get_camera_models(&self, brand: &str) -> Vec<String> {
        let mut models: HashSet<String> = HashSet::new();

        for camera in self.camera_registry.get_cameras_for_brand(brand) {
            models.insert(camera.name.clone());
        }

        let brand_lower = brand.to_lowercase();
        for profile in self.map.values() {
            if profile.camera_brand.to_lowercase() == brand_lower && !profile.camera_model.is_empty() {
                models.insert(profile.camera_model.clone());
            }
        }

        let mut result: Vec<_> = models.into_iter().collect();
        result.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        result
    }

    pub fn get_lens_models(&self, brand: &str) -> Vec<String> {
        let mut lenses: HashSet<String> = HashSet::new();

        for lens in self.camera_registry.get_lenses_for_brand(brand) {
            lenses.insert(lens.name.clone());
        }

        let brand_lower = brand.to_lowercase();
        for profile in self.map.values() {
            if profile.camera_brand.to_lowercase() == brand_lower && !profile.lens_model.is_empty() {
                lenses.insert(profile.lens_model.clone());
            }
        }

        let mut result: Vec<_> = lenses.into_iter().collect();
        result.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        result
    }

    pub fn get_compatible_lenses(&self, brand: &str, model: &str) -> Vec<(String, String)> {
        let mut result = self.camera_registry.get_compatible_lenses(brand, model);

        let brand_lower = brand.to_lowercase();
        let model_lower = model.to_lowercase();
        for profile in self.map.values() {
            if profile.camera_brand.to_lowercase() == brand_lower
                && profile.camera_model.to_lowercase() == model_lower
                && !profile.lens_model.is_empty()
            {
                let lens_entry = (brand.to_string(), profile.lens_model.clone());
                if !result.contains(&lens_entry) {
                    result.push(lens_entry);
                }
            }
        }

        result.sort_by(|a, b| {
            let cmp = a.0.to_lowercase().cmp(&b.0.to_lowercase());
            if cmp == Ordering::Equal {
                a.1.to_lowercase().cmp(&b.1.to_lowercase())
            } else {
                cmp
            }
        });
        result
    }

    pub fn get_compatible_cameras(&self, brand: &str, model: &str) -> Vec<(String, String, f64)> {
        self.camera_registry.get_compatible_cameras(brand, model)
    }

    pub fn is_zoom_lens(&self, brand: &str, lens_model: &str) -> bool {
        self.camera_registry.is_zoom_lens(brand, lens_model)
    }

    pub fn is_fixed_lens_camera(&self, brand: &str, model: &str) -> bool {
        if let Some((_, camera)) = self.camera_registry.get_camera(brand, model) {
            return camera.fixed_lens;
        }
        false
    }

    pub fn search_by_camera(&self, brand: &str, model: &str, lens: &str, favorites: &HashSet<String>, aspect_ratio: i32, aspect_ratio_swapped: i32) -> Vec<(String, String, String, bool, f64, i32, String)> {
        let brand_lower = brand.to_lowercase();
        let model_lower = model.to_lowercase();
        let lens_lower = lens.to_lowercase();

        let mut filtered: Vec<_> = self.list_for_ui.iter().filter(|(name, key, checksum, _, _, _, _)| {
            if self.hidden_profiles.contains(checksum) {
                return false;
            }

            if let Some(profile) = self.map.get(key) {
                let matches_brand = brand.is_empty() || profile.camera_brand.to_lowercase() == brand_lower;
                let matches_model = model.is_empty() || profile.camera_model.to_lowercase() == model_lower;
                let matches_lens = lens.is_empty() || profile.lens_model.to_lowercase().contains(&lens_lower);

                return matches_brand && matches_model && matches_lens;
            }

            false
        }).collect();

        filtered.sort_by(|a, b| {
            let a_priority = a.1.ends_with(".gyroflow") || favorites.contains(&a.2);
            let b_priority = b.1.ends_with(".gyroflow") || favorites.contains(&b.2);
            if a_priority && !b_priority { return Ordering::Less; }
            if b_priority && !a_priority { return Ordering::Greater; }

            let a_priority2 = a.5 != 0 && aspect_ratio == a.5;
            let b_priority2 = b.5 != 0 && aspect_ratio == b.5;
            if a_priority2 && !b_priority2 { return Ordering::Less; }
            if b_priority2 && !a_priority2 { return Ordering::Greater; }

            let a_priority3 = a.5 != 0 && aspect_ratio_swapped == a.5;
            let b_priority3 = b.5 != 0 && aspect_ratio_swapped == b.5;
            if a_priority3 && !b_priority3 { return Ordering::Less; }
            if b_priority3 && !a_priority3 { return Ordering::Greater; }

            a.0.cmp(&b.0)
        });

        filtered.into_iter().take(200).cloned().collect()
    }

    // -------------------------------------------------------------------
    // ---------------------- Profile review methods ---------------------
    // -------------------------------------------------------------------

    pub fn get_review_group(&self, profile_key: &str) -> Vec<String> {
        if let Some(profile) = self.map.get(profile_key) {
            let group_key = format!(
                "{}|{}|{}|{}x{}",
                profile.camera_brand.to_lowercase(),
                profile.camera_model.to_lowercase(),
                profile.lens_model.to_lowercase(),
                profile.calib_dimension.w,
                profile.calib_dimension.h
            );
            if let Some(group) = self.review_groups.get(&group_key) {
                return group.clone();
            }
        }
        vec![profile_key.to_string()]
    }

    pub fn get_profiles_for_setup(&self, brand: &str, model: &str, lens: &str, width: usize, height: usize) -> Vec<&LensProfile> {
        let brand_lower = brand.to_lowercase();
        let model_lower = model.to_lowercase();
        let lens_lower = lens.to_lowercase();

        self.map.values().filter(|p| {
            p.camera_brand.to_lowercase() == brand_lower
                && p.camera_model.to_lowercase() == model_lower
                && p.lens_model.to_lowercase() == lens_lower
                && p.calib_dimension.w == width
                && p.calib_dimension.h == height
                && !self.hidden_profiles.contains(p.checksum.as_deref().unwrap_or(""))
        }).collect()
    }

    pub fn hide_profile(&mut self, checksum: &str) {
        self.hidden_profiles.insert(checksum.to_string());
        self.save_hidden_profiles();
    }

    pub fn unhide_profile(&mut self, checksum: &str) {
        self.hidden_profiles.remove(checksum);
        self.save_hidden_profiles();
    }

    pub fn is_profile_hidden(&self, checksum: &str) -> bool {
        self.hidden_profiles.contains(checksum)
    }

    pub fn get_hidden_profiles(&self) -> Vec<String> {
        self.hidden_profiles.iter().cloned().collect()
    }

    fn save_hidden_profiles(&self) {
        let path = crate::settings::data_dir().join("hidden_profiles.json");
        let hidden: Vec<_> = self.hidden_profiles.iter().collect();
        if let Ok(json) = serde_json::to_string(&hidden) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn load_hidden_profiles(&mut self) {
        let path = crate::settings::data_dir().join("hidden_profiles.json");
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(hidden) = serde_json::from_str::<Vec<String>>(&content) {
                    self.hidden_profiles = hidden.into_iter().collect();
                }
            }
        }
    }

    // -------------------------------------------------------------------
    // ---------------------- Calibrator validation ----------------------
    // -------------------------------------------------------------------

    pub fn validate_profile_for_upload(&self, profile: &LensProfile) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if profile.camera_brand.trim().is_empty() {
            errors.push("Camera brand is required".to_string());
        }

        if profile.camera_model.trim().is_empty() {
            errors.push("Camera model is required".to_string());
        }

        if !self.is_fixed_lens_camera(&profile.camera_brand, &profile.camera_model) {
            if profile.lens_model.trim().is_empty() {
                errors.push("Lens model is required for interchangeable lens cameras".to_string());
            }

            if self.is_zoom_lens(&profile.camera_brand, &profile.lens_model) {
                if profile.focal_length.is_none() || profile.focal_length.unwrap_or(0.0) <= 0.0 {
                    errors.push("Focal length is required for zoom lenses".to_string());
                }
            }
        }

        if profile.calib_dimension.w == 0 || profile.calib_dimension.h == 0 {
            errors.push("Valid calibration dimensions are required".to_string());
        }

        if profile.fps <= 0.0 {
            errors.push("Valid frame rate is required".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn normalize_profile_metadata(&self, profile: &mut LensProfile) {
        profile.camera_brand = self.camera_registry.normalize_brand(&profile.camera_brand);

        if let Some((_, camera)) = self.camera_registry.get_camera(&profile.camera_brand, &profile.camera_model) {
            profile.camera_model = camera.name.clone();
            if profile.crop_factor.is_none() {
                profile.crop_factor = camera.crop_factor;
            }
        }

        if let Some((_, lens)) = self.camera_registry.get_lens(&profile.camera_brand, &profile.lens_model) {
            profile.lens_model = lens.name.clone();
        }
    }
}
