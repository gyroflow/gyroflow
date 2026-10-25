// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::cmp::Ordering;
use std::collections::{ HashSet, HashMap, BTreeMap };
use crate::LensProfile;
use std::path::PathBuf;
use std::io::Read;

#[cfg(any(target_os = "android", target_os = "ios", feature = "bundle-lens-profiles"))]
static LENS_PROFILES_STATIC: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/camera_presets/profiles.cbor.gz"));

#[allow(dead_code)]
static CAMERA_DATABASE_STATIC: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/camera_database.json"));

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
    pub camera_db: crate::camera_database::CameraDatabase,
    pub loaded: bool,
    pub version: u32,
}
impl Clone for LensProfileDatabase {
    fn clone(&self) -> Self {
        Self { map: self.map.clone(), preset_map: self.preset_map.clone(), camera_db: self.camera_db.clone(), loaded: self.loaded, version: self.version, ..Default::default() }
    }
}

impl LensProfileDatabase {
    pub fn get_path() -> PathBuf {
        // return std::fs::canonicalize("D:/lens_review/").unwrap_or_default();

        let candidates = [
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            PathBuf::from("../Resources/camera_presets/"),
            PathBuf::from("./resources/camera_presets/"),
            PathBuf::from("../../resources/camera_presets/"),
            PathBuf::from("../resources/camera_presets/"),
            PathBuf::from("./resources/camera_presets/"),
            PathBuf::from("../../resources/camera_presets/"),
            PathBuf::from("../resources/camera_presets/"),
            PathBuf::from("./resources/camera_presets/"),
            PathBuf::from("../../resources/camera_presets/"),
            PathBuf::from("../resources/camera_presets/"),
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

        if let Ok(db) = crate::camera_database::CameraDatabase::from_json(CAMERA_DATABASE_STATIC) {
            self.camera_db = db;
        }
        for cam_db_path in [
            Self::get_path().join("camera_database.json"),
            crate::settings::data_dir().join("lens_profiles").join("camera_database.json"),
        ] {
            if cam_db_path.exists() {
                if let Ok(data) = std::fs::read_to_string(&cam_db_path) {
                    if let Ok(db) = crate::camera_database::CameraDatabase::from_json(&data) {
                        self.camera_db = db;
                    }
                }
            }
        }

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
                        if self.map.contains_key(&key) {
                            if !self.loaded {
                                log::warn!("Lens profile already present: {}, path_to_file: {} from {}", key, f_name, self.map.get(&key).unwrap().path_to_file);
                            }
                            if !f_name.is_empty() && f_name != key && !self.map.contains_key(f_name) {
                                self.map.insert(f_name.to_string(), profile);
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

        for v in self.map.values() {
            if !v.camera_brand.is_empty() && !v.camera_model.is_empty() {
                let norm_b = self.camera_db.normalize_brand(&v.camera_brand);
                let norm_m = self.camera_db.normalize_model(&norm_b, &v.camera_model);
                if self.camera_db.get_camera(&norm_b, &norm_m).is_none() {
                    self.camera_db.add_camera(crate::camera_database::CameraInfo {
                        brand: norm_b.clone(),
                        model: norm_m.clone(),
                        crop_factor: v.crop_factor,
                        ..Default::default()
                    });
                }
                if !v.lens_model.is_empty() {
                    self.camera_db.add_lens(crate::camera_database::LensInfo {
                        brand: norm_b.clone(),
                        model: v.lens_model.clone(),
                        crop_factor: v.crop_factor,
                        min_focal: v.focal_length,
                        max_focal: v.focal_length,
                        ..Default::default()
                    });
                }
            }
        }
        self.camera_db.rebuild_indices();

        ::log::info!("Loaded {} lens profiles in {:.3}ms", self.map.len(), _time.elapsed().as_micros() as f64 / 1000.0);
        self.loaded = true;
    }

    pub fn set_from_db(&mut self, b: Self) {
        self.map = b.map;
        self.preset_map = b.preset_map;
        self.loaded = b.loaded;
        self.camera_db = b.camera_db;
        self.version = b.version;
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
    pub fn get_brands(&self) -> Vec<String> {
        let mut set: HashSet<String> = self.camera_db.get_brands().into_iter().collect();
        for v in self.map.values() {
            if !v.camera_brand.is_empty() {
                set.insert(self.camera_db.normalize_brand(&v.camera_brand));
            }
        }
        let mut list: Vec<_> = set.into_iter().collect();
        list.sort();
        list
    }

    pub fn get_models(&self, brand: &str) -> Vec<String> {
        let norm_brand = self.camera_db.normalize_brand(brand);
        let mut set: HashSet<String> = self.camera_db.get_models(&norm_brand).into_iter().collect();
        let b_lower = norm_brand.to_ascii_lowercase();

        for v in self.map.values() {
            if v.camera_brand.to_ascii_lowercase() == b_lower && !v.camera_model.is_empty() {
                set.insert(self.camera_db.normalize_model(&norm_brand, &v.camera_model));
            }
        }
        let mut list: Vec<_> = set.into_iter().collect();
        list.sort();
        list
    }

    pub fn get_lenses(&self, brand: &str, model: &str) -> Vec<String> {
        let norm_brand = self.camera_db.normalize_brand(brand);
        let norm_model = self.camera_db.normalize_model(&norm_brand, model);
        let mut set: HashSet<String> = self.camera_db.get_lenses_for_camera(&norm_brand, &norm_model).into_iter().collect();

        let b_lower = norm_brand.to_ascii_lowercase();
        let m_lower = norm_model.to_ascii_lowercase();

        for v in self.map.values() {
            let vb = self.camera_db.normalize_brand(&v.camera_brand).to_ascii_lowercase();
            let vm = self.camera_db.normalize_model(&norm_brand, &v.camera_model).to_ascii_lowercase();
            if vb == b_lower && vm == m_lower && !v.lens_model.is_empty() {
                set.insert(v.lens_model.clone());
            }
        }
        let mut list: Vec<_> = set.into_iter().collect();
        list.sort();
        list
    }

    pub fn find_profiles(&self, brand: Option<&str>, model: Option<&str>, lens: Option<&str>) -> Vec<&LensProfile> {
        let b_norm = brand.map(|b| self.camera_db.normalize_brand(b).to_ascii_lowercase());
        let m_norm = model.map(|m| m.to_ascii_lowercase());
        let l_norm = lens.map(|l| l.to_ascii_lowercase());

        self.map.values().filter(|p| {
            if let Some(b) = &b_norm {
                let pb = self.camera_db.normalize_brand(&p.camera_brand).to_ascii_lowercase();
                if pb != *b && !pb.contains(b.as_str()) {
                    return false;
                }
            }
            if let Some(m) = &m_norm {
                let pm = self.camera_db.normalize_model(&p.camera_brand, &p.camera_model).to_ascii_lowercase();
                if pm != *m && !pm.contains(m.as_str()) {
                    return false;
                }
            }
            if let Some(l) = &l_norm {
                let pl = p.lens_model.to_ascii_lowercase();
                if pl != *l && !pl.contains(l.as_str()) {
                    return false;
                }
            }
            true
        }).collect()
    }

    pub fn get_compatible_cameras(&self, brand: &str, model: &str) -> Vec<String> {
        let norm_brand = self.camera_db.normalize_brand(brand);
        let norm_model = self.camera_db.normalize_model(&norm_brand, model);
        let Some(cam) = self.camera_db.get_camera(&norm_brand, &norm_model) else {
            return Vec::new();
        };
        let Some(mount) = &cam.mount else {
            return Vec::new();
        };
        let m_lower = mount.to_ascii_lowercase();
        let target_crop = cam.crop_factor;

        let mut compat: Vec<String> = self.camera_db.cameras.iter().filter(|c| {
            if c.brand.eq_ignore_ascii_case(&norm_brand) && c.model.eq_ignore_ascii_case(&norm_model) {
                return false;
            }
            let Some(cm) = &c.mount else { return false; };
            if !cm.eq_ignore_ascii_case(&m_lower) {
                return false;
            }
            match (target_crop, c.crop_factor) {
                (Some(tc), Some(cc)) => (tc - cc).abs() <= 0.15,
                _ => true,
            }
        }).map(|c| format!("{} {}", c.brand, c.model)).collect();
        compat.sort();
        compat.dedup();
        compat
    }

    pub fn get_submitted_profiles_for_setup(&self, brand: &str, model: &str, lens: &str, identifier: &str) -> Vec<(String, String, String, bool, f64)> {
        let norm_brand = self.camera_db.normalize_brand(brand);
        let norm_model = self.camera_db.normalize_model(&norm_brand, model);
        let b_lower = norm_brand.to_ascii_lowercase();
        let m_lower = norm_model.to_ascii_lowercase();
        let l_lower = lens.trim().to_ascii_lowercase();
        let id_trim = identifier.trim();

        let mut seen = HashSet::new();
        let mut results: Vec<(String, String, String, bool, f64)> = Vec::new();

        for (k, p) in &self.map {
            if p.is_copy { continue; }
            let id_match = !id_trim.is_empty() && p.identifier == id_trim;
            let pb = self.camera_db.normalize_brand(&p.camera_brand).to_ascii_lowercase();
            let pm = self.camera_db.normalize_model(&norm_brand, &p.camera_model).to_ascii_lowercase();
            let pl = p.lens_model.trim().to_ascii_lowercase();
            let setup_match = !b_lower.is_empty()
                && !m_lower.is_empty()
                && pb == b_lower
                && pm == m_lower
                && (l_lower.is_empty() || pl == l_lower || pl.contains(&l_lower) || l_lower.contains(&pl));

            if id_match || setup_match {
                let dedup_key = p.checksum.clone().unwrap_or_else(|| p.path_to_file.clone());
                if seen.insert(dedup_key) {
                    let load_key = if !p.path_to_file.is_empty() { p.path_to_file.clone() } else { k.clone() };
                    let author = if p.calibrated_by.is_empty() { "Community".to_string() } else { p.calibrated_by.clone() };
                    let label = if p.camera_setting.is_empty() {
                        format!("{} ({})", p.lens_model, author)
                    } else {
                        format!("{} - {} ({})", p.lens_model, p.camera_setting, author)
                    };
                    results.push((label, load_key, author, p.official, p.rating.unwrap_or(0.0)));
                }
            }
        }

        results.sort_by(|a, b| {
            b.3.cmp(&a.3)
                .then_with(|| b.4.partial_cmp(&a.4).unwrap_or(Ordering::Equal))
                .then_with(|| a.0.cmp(&b.0))
        });
        results
    }

    pub fn load_from_lensfun(&mut self, xml_str: &str) -> Result<usize, String> {
        let mut lf = crate::lensfun::LensfunDatabase::new();
        lf.parse_xml(std::io::Cursor::new(xml_str))?;

        lf.populate_camera_database(&mut self.camera_db);

        let profiles = lf.to_lens_profiles();
        let count = profiles.len();

        for profile in profiles {
            let key = if !profile.identifier.is_empty() {
                profile.identifier.clone()
            } else {
                profile.get_name()
            };

            if let Ok(json_str) = profile.get_json() {
                self.preset_map.insert(key.clone(), json_str);
            }
            self.map.insert(key, profile);
        }

        self.prepare_list_for_ui();
        Ok(count)
    }

    pub fn load_lensfun_file<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<usize, String> {
        let xml_str = std::fs::read_to_string(path).map_err(|e| format!("Failed to read file: {:?}", e))?;
        self.load_from_lensfun(&xml_str)
    }

    pub fn load_lensfun_dir<P: AsRef<std::path::Path>>(&mut self, dir: P) -> Result<usize, String> {
        let mut total = 0;
        for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("xml")) {
                if let Ok(count) = self.load_lensfun_file(p) {
                    total += count;
                }
            }
        }
        Ok(total)
    }

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
}
