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
enum DataSource {
    String(String),
    SerdeValue(serde_json::Value)
}

pub type LensProfileUiItem = (String, String, String, bool, f64, i32, String);
/// A loaded lens profile file: its path as loaded, the number of components of its path in the lens profile folder it was
/// loaded from, and the key of the file's own profile in the map
#[derive(Clone)]
struct ProfileFile {
    path: String,
    depth: usize,
    key: String,
}

#[derive(Default)]
pub struct LensProfileDatabase {
    preset_map: HashMap<String, String>,
    map: HashMap<String, LensProfile>,
    // In the order they were loaded
    files: Vec<ProfileFile>,
    loaded_callbacks: Vec<Box<dyn FnOnce(&Self) + Send + Sync + 'static>>,
    list_for_ui: Vec<LensProfileUiItem>,
    pub loaded: bool,
    pub version: u32,
}
impl Clone for LensProfileDatabase {
    fn clone(&self) -> Self {
        Self { map: self.map.clone(), preset_map: self.preset_map.clone(), files: self.files.clone(), loaded: self.loaded, ..Default::default() }
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

        // `relative` is the file's path in the lens profile folder
        let mut load = |data: DataSource, f_name: &str, relative: &str| {
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
                            if !profile.is_copy {
                                let depth = relative.split('/').filter(|x| !x.is_empty()).count();
                                self.files.push(ProfileFile { path: f_name.to_string(), depth, key: key.clone() });
                            }
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
            walkdir::WalkDir::new(&dir).into_iter().for_each(|e| {
                if let Ok(entry) = e {
                    let f_name = entry.path().to_string_lossy().replace('\\', "/");
                    if f_name.ends_with(".json") || f_name.ends_with(".gyroflow") {
                        // The camera catalog is installed beside profiles but is not a profile.
                        if entry.file_name() != "camera_database.json" {
                            if let Ok(data) = std::fs::read_to_string(&f_name) {
                                let relative = entry.path().strip_prefix(&dir).map(|x| x.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| f_name.clone());
                                load(DataSource::String(data), &f_name, &relative);
                            }
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
                                        load(DataSource::SerdeValue(profile), &f_name, &f_name);
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
                        load(DataSource::SerdeValue(profile), &f_name, &f_name);
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

        ::log::info!("Loaded {} lens profiles in {:.3}ms", self.map.len(), _time.elapsed().as_micros() as f64 / 1000.0);
        self.loaded = true;
    }

    pub fn set_from_db(&mut self, b: Self) {
        self.map = b.map;
        self.preset_map = b.preset_map;
        self.files = b.files;
        self.loaded = b.loaded;
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

    pub fn iter_profiles(&self) -> impl Iterator<Item = &LensProfile> {
        self.map.values()
    }

    pub fn prepare_list_for_ui(&mut self) {
        // (name, path_to_file, crc32, official, rating, aspect_ratio*1000, author)
        let mut set = HashSet::with_capacity(self.map.len());
        let mut checksum_map = HashMap::with_capacity(self.map.len());
        self.list_for_ui = Vec::with_capacity(self.map.len());
        // In key order, so profiles of the same name get the same " by …" and " - 2" every time
        let mut entries: Vec<_> = self.map.iter().collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
        for (k, v) in entries {
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

    pub fn search(&self, text: &str, favorites: &HashSet<String>, aspect_ratio: i32, aspect_ratio_swapped: i32) -> Vec<LensProfileUiItem> {
        let text = Self::normalize_search_text(text);
        let words = text.split_ascii_whitespace().map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>();
        if words.is_empty() {
            return Vec::new();
        }

        let mut filtered = self.list_for_ui.iter().filter(|(name, _, _, _, _, _, author)| {
            Self::matches_words(name, author, &words)
        }).collect::<Vec<_>>();

        Self::sort_ui_items(&mut filtered, favorites, aspect_ratio, aspect_ratio_swapped);

        filtered.into_iter().take(200).cloned().collect()
    }

    fn profile_is_hidden(profile: Option<&LensProfile>, path: &str, checksum: &str, hidden: &HashSet<String>) -> bool {
        [Some(path), Some(checksum), profile.map(|value| value.path_to_file.as_str())]
            .into_iter()
            .flatten()
            .any(|value| !value.is_empty() && hidden.contains(value))
    }

    fn lens_matches(profile: Option<&LensProfile>, requested: &str) -> bool {
        let Some(candidate) = profile.map(|value| Self::key(&value.lens_model)).filter(|value| !value.is_empty()) else {
            return false;
        };
        candidate == Self::key(requested)
    }

    fn lens_matches_keys(profile: Option<&LensProfile>, keys: &HashSet<String>) -> bool {
        let Some(candidate) = profile.map(|value| Self::key(&value.lens_model)).filter(|value| !value.is_empty()) else {
            return false;
        };
        keys.contains(&candidate)
    }

    pub fn search_by_camera(
        &self,
        brand: &str,
        model: &str,
        lens: &str,
        text: &str,
        selected_camera_keys: &HashSet<(String, String)>,
        brand_camera_keys: &HashSet<(String, String)>,
        compatible_camera_keys: &HashSet<(String, String)>,
        hidden_profiles: &HashSet<String>,
        favorites: &HashSet<String>,
        aspect_ratio: i32,
        aspect_ratio_swapped: i32
    ) -> Vec<LensProfileUiItem> {
        self.search_by_camera_with_lens_keys(
            brand, model, lens, text, selected_camera_keys, brand_camera_keys,
            compatible_camera_keys, hidden_profiles, favorites, aspect_ratio,
            aspect_ratio_swapped, None
        )
    }

    /// Search camera profiles using an optional set of normalized profile lens keys.
    /// `None` preserves exact free-text filtering; `Some(empty)` intentionally matches none.
    pub fn search_by_camera_with_lens_keys(
        &self,
        brand: &str,
        model: &str,
        lens: &str,
        text: &str,
        selected_camera_keys: &HashSet<(String, String)>,
        brand_camera_keys: &HashSet<(String, String)>,
        compatible_camera_keys: &HashSet<(String, String)>,
        hidden_profiles: &HashSet<String>,
        favorites: &HashSet<String>,
        aspect_ratio: i32,
        aspect_ratio_swapped: i32,
        lens_profile_keys: Option<&HashSet<String>>
    ) -> Vec<LensProfileUiItem> {
        let text = Self::normalize_search_text(text);
        let words = text.split_ascii_whitespace().map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>();
        let brand_key = Self::key(brand);
        let model_key = Self::key(model);
        let lens_key = Self::key(lens);
        let has_selector_filter = !brand_key.is_empty() || !model_key.is_empty() || !lens_key.is_empty();
        if !has_selector_filter && words.is_empty() {
            return Vec::new();
        }

        let mut camera_keys = selected_camera_keys.clone();
        camera_keys.extend(compatible_camera_keys.iter().cloned());

        let mut filtered = self.list_for_ui.iter().filter(|item| {
            let (name, path, checksum, _, _, _, author) = item;
            let profile = self.map.get(path);
            if Self::profile_is_hidden(profile, path, checksum, hidden_profiles) {
                return false;
            }

            if !brand_key.is_empty() {
                let Some(profile) = profile else {
                    return false;
                };

                if !model_key.is_empty() {
                    let profile_key = Self::profile_camera_key(profile);
                    if !camera_keys.contains(&profile_key) {
                        return false;
                    }
                } else {
                    let profile_key = Self::profile_camera_key(profile);
                    if !brand_camera_keys.contains(&profile_key) && Self::key(&profile.camera_brand) != brand_key {
                        return false;
                    }
                }
            }

            if !lens_key.is_empty() {
                let matches = match lens_profile_keys {
                    Some(keys) => Self::lens_matches_keys(profile, keys),
                    None => Self::lens_matches(profile, &lens_key),
                };
                if !matches { return false; }
            }

            words.is_empty() || Self::matches_words(name, author, &words)
        }).collect::<Vec<_>>();

        filtered.sort_by(|a, b| {
            let a_exact = self.map.get(&a.1).map(|profile| selected_camera_keys.contains(&Self::profile_camera_key(profile))).unwrap_or(false);
            let b_exact = self.map.get(&b.1).map(|profile| selected_camera_keys.contains(&Self::profile_camera_key(profile))).unwrap_or(false);
            if a_exact && !b_exact { return Ordering::Less; }
            if b_exact && !a_exact { return Ordering::Greater; }

            Self::compare_ui_items(a, b, favorites, aspect_ratio, aspect_ratio_swapped)
        });

        // A selected setup is also the review list; do not silently omit its profiles.
        let limit = if model_key.is_empty() { 200 } else { usize::MAX };
        filtered.into_iter().take(limit).cloned().collect()
    }

    fn normalize_search_text(text: &str) -> String {
        text.to_ascii_lowercase()
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
            .replace(";", " ")
    }

    fn matches_words(name: &str, author: &str, words: &[&str]) -> bool {
        let name = name.to_ascii_lowercase();
        let author = author.to_ascii_lowercase();
        words.iter().all(|word| name.contains(word) || author.contains(word))
    }

    fn sort_ui_items(filtered: &mut Vec<&LensProfileUiItem>, favorites: &HashSet<String>, aspect_ratio: i32, aspect_ratio_swapped: i32) {
        filtered.sort_by(|a, b| Self::compare_ui_items(a, b, favorites, aspect_ratio, aspect_ratio_swapped));
    }

    fn compare_ui_items(a: &LensProfileUiItem, b: &LensProfileUiItem, favorites: &HashSet<String>, aspect_ratio: i32, aspect_ratio_swapped: i32) -> Ordering {
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
    }

    fn profile_camera_key(profile: &LensProfile) -> (String, String) {
        (Self::key(&profile.camera_brand), Self::key(&profile.camera_model))
    }

    fn key(value: &str) -> String {
        value.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()
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
    /// The profile a file's lens profile reference names (.gcsv `lensprofile`, Gyroflow protobuf `lens_profile`): the
    /// profile of that identifier or key, else that of a lens profile file (not a preset) by its path in the lens profile
    /// folder or by the end of that path (`Brand/name.json`, `Brand/name`, `name`), with or without the extension. That's
    /// the file's own profile, not one of its compatible settings. Of several files, the one with the fewest folders the
    /// reference doesn't give, then the one loaded first, as for identifiers. No part of a name matches, so a lens name that
    /// names no profile, like "unknown" from a RED camera, finds none
    pub fn find(&self, filename_or_id: &str) -> Option<&LensProfile> {
        if filename_or_id.trim().is_empty() { return None; }
        if let Some(l) = self.map.get(filename_or_id) { return Some(l); }
        let path = filename_or_id.replace('\\', "/");
        if let Some(l) = self.map.get(&path) { return Some(l); }

        let wanted: Vec<&str> = path.split('/').filter(|x| !x.is_empty() && *x != ".").collect();
        let (name, folders) = wanted.split_last()?;
        self.files.iter().filter_map(|file| {
            let mut components = file.path.rsplit('/').filter(|x| !x.is_empty());
            let file_name = components.next()?;
            if file_name != *name && file_name.strip_suffix(".json") != Some(*name) { return None; }
            for folder in folders.iter().rev() {
                if components.next()? != *folder { return None; }
            }
            Some((file.depth.saturating_sub(wanted.len()), file))
        }).min_by_key(|(unmatched_folders, _)| *unmatched_folders).and_then(|(_, file)| self.map.get(&file.key))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_database::CameraDatabase;

    fn profile(brand: &str, model: &str, lens: &str, checksum: &str) -> LensProfile {
        let mut profile = LensProfile::default();
        profile.camera_brand = brand.to_owned();
        profile.camera_model = model.to_owned();
        profile.lens_model = lens.to_owned();
        profile.checksum = Some(checksum.to_owned());
        profile
    }

    fn item(name: &str, path: &str, checksum: &str) -> LensProfileUiItem {
        (name.to_owned(), path.to_owned(), checksum.to_owned(), false, 0.0, 0, String::new())
    }

    #[test]
    fn selected_setup_review_includes_every_matching_profile() {
        let mut db = LensProfileDatabase::default();
        for i in 0..205 {
            let path = format!("profile-{i}");
            db.map.insert(path.clone(), profile("Maker", "Body", "Prime", &path));
            db.list_for_ui.push(item("Maker Body Prime", &path, &path));
        }
        let selected = HashSet::from([("maker".to_owned(), "body".to_owned())]);
        assert_eq!(db.search_by_camera(
            "Maker", "Body", "Prime", "", &selected, &HashSet::new(), &HashSet::new(), &HashSet::new(), &HashSet::new(), 0, 0
        ).len(), 205);
    }

    #[test]
    fn structured_search_includes_compatible_profiles_and_hides_rejected_ones() {
        let mut db = LensProfileDatabase::default();
        db.map.insert("selected.json".to_owned(), profile("Brand", "Primary", "Wide", "selected"));
        db.map.insert("compatible.json".to_owned(), profile("Brand", "Compatible", "Wide", "compatible"));
        db.map.insert("hidden.json".to_owned(), profile("Brand", "Compatible", "Wide", "hidden"));
        db.map.insert("other.json".to_owned(), profile("Brand", "Other", "Wide", "other"));
        db.list_for_ui = vec![
            item("Brand Primary Wide", "selected.json", "selected"),
            item("Brand Compatible Wide", "compatible.json", "compatible"),
            item("Brand Compatible Hidden", "hidden.json", "hidden"),
            item("Brand Other Wide", "other.json", "other"),
        ];

        let results = db.search_by_camera(
            "Brand",
            "Primary",
            "",
            "",
            &HashSet::from([(String::from("brand"), String::from("primary"))]),
            &HashSet::from([(String::from("brand"), String::from("primary"))]),
            &HashSet::from([(String::from("brand"), String::from("compatible"))]),
            &HashSet::from([String::from("hidden")]),
            &HashSet::new(),
            0,
            0
        );

        let result_checksums = results.into_iter().map(|item| item.2).collect::<Vec<_>>();
        assert_eq!(result_checksums, vec!["selected", "compatible"]);
    }

    #[test]
    fn brand_search_resolves_model_scoped_aliases_without_cross_brand_matches() {
        let camera_db = CameraDatabase::from_bundled();
        let pentax_keys = HashSet::from_iter(camera_db.brand_camera_keys("Pentax"));
        assert!(pentax_keys.contains(&("ricoh imaging company, ltd.".to_owned(), "k-1".to_owned())));
        assert!(!pentax_keys.contains(&("ricoh imaging company, ltd.".to_owned(), "gr iii".to_owned())));

        let mut db = LensProfileDatabase::default();
        db.map.insert("pentax.json".to_owned(), profile("Ricoh Imaging Company, Ltd.", "K-1", "Prime", "pentax"));
        db.map.insert("ricoh.json".to_owned(), profile("Ricoh Imaging Company, Ltd.", "GR III", "Prime", "ricoh"));
        db.list_for_ui = vec![
            item("Pentax K-1 Prime", "pentax.json", "pentax"),
            item("Ricoh GR III Prime", "ricoh.json", "ricoh"),
        ];

        let results = db.search_by_camera(
            "Pentax", "", "", "", &HashSet::new(), &pentax_keys,
            &HashSet::new(), &HashSet::new(), &HashSet::new(), 0, 0,
        );
        assert_eq!(results.iter().map(|item| item.1.as_str()).collect::<Vec<_>>(), vec!["pentax.json"]);
    }

    #[test]
    fn structured_search_honors_the_profile_resolved_path() {
        let mut db = LensProfileDatabase::default();
        let mut candidate = profile("Maker", "Body", "Prime", "candidate");
        candidate.path_to_file = "profiles/resolved.json".to_owned();
        db.map.insert("catalog-key".to_owned(), candidate);
        db.list_for_ui.push(item("Maker Body Prime", "catalog-key", "candidate"));

        let hidden = HashSet::from(["profiles/resolved.json".to_owned()]);
        let selected = HashSet::from([("maker".to_owned(), "body".to_owned())]);
        let matches = db.search_by_camera(
            "Maker", "Body", "Prime", "", &selected, &HashSet::new(), &HashSet::new(), &hidden, &HashSet::new(), 0, 0
        );

        assert!(matches.is_empty());
    }

    #[test]
    fn structured_search_requires_lens_metadata_for_a_lens_filter() {
        let mut db = LensProfileDatabase::default();
        db.map.insert("unnamed".to_owned(), profile("Maker", "Body", "", "unnamed"));
        db.map.insert("prime".to_owned(), profile("Maker", "Body", "Prime 24mm", "prime"));
        db.list_for_ui.extend([
            item("Maker Body", "unnamed", "unnamed"),
            item("Maker Body Prime", "prime", "prime"),
        ]);

        let selected = HashSet::from([("maker".to_owned(), "body".to_owned())]);
        let checksums = db.search_by_camera(
            "Maker", "Body", "Prime 24mm", "", &selected, &HashSet::new(), &HashSet::new(), &HashSet::new(), &HashSet::new(), 0, 0
        ).into_iter().map(|entry| entry.2).collect::<Vec<_>>();

        assert_eq!(checksums, ["prime"]);
    }

    #[test]
    fn selected_lens_matches_its_exact_identity_only() {
        let mut db = LensProfileDatabase::default();
        for (path, lens) in [
            ("35mm", "FE 35mm F1.4 GM"),
            ("135mm", "FE 135mm F1.8 GM"),
            ("35mm-ii", "FE 35mm F1.4 GM II"),
        ] {
            db.map.insert(path.to_owned(), profile("Sony", "A7 IV", lens, path));
            db.list_for_ui.push(item(&format!("Sony A7 IV {lens}"), path, path));
        }

        let selected = HashSet::from([("sony".to_owned(), "a7 iv".to_owned())]);
        let results = db.search_by_camera(
            "Sony", "A7 IV", "fe 35mm f1.4 gm", "", &selected, &selected,
            &HashSet::new(), &HashSet::new(), &HashSet::new(), 0, 0,
        );

        assert_eq!(results.iter().map(|item| item.1.as_str()).collect::<Vec<_>>(), vec!["35mm"]);
    }

    #[test]
    fn selected_catalog_lens_searches_legacy_alias_profiles() {
        let mut db = LensProfileDatabase::default();
        db.map.insert("canonical".to_owned(), profile("Sony", "A7 IV", "Sony FE 35mm F1.4 GM", "canonical"));
        db.map.insert("legacy-alias".to_owned(), profile("Sony", "A7 IV", "35 GM", "legacy-alias"));
        db.map.insert("other".to_owned(), profile("Sony", "A7 IV", "FE 135mm F1.8 GM", "other"));
        db.list_for_ui = vec![
            item("Sony A7 IV FE 35mm F1.4 GM", "canonical", "canonical"),
            item("Sony A7 IV 35 GM", "legacy-alias", "legacy-alias"),
            item("Sony A7 IV FE 135mm F1.8 GM", "other", "other"),
        ];

        let selected = HashSet::from([("sony".to_owned(), "a7 iv".to_owned())]);
        let keys = HashSet::from(["sony fe 35mm f1.4 gm".to_owned(), "35 gm".to_owned()]);
        let results = db.search_by_camera_with_lens_keys(
            "Sony", "A7 IV", "Sony FE 35mm F1.4 GM", "", &selected, &HashSet::new(),
            &HashSet::new(), &HashSet::new(), &HashSet::new(), 0, 0, Some(&keys),
        );

        let result_paths = results.iter().map(|item| item.1.as_str()).collect::<HashSet<_>>();
        assert_eq!(result_paths, HashSet::from(["canonical", "legacy-alias"]));

        let ambiguous = HashSet::new();
        assert!(db.search_by_camera_with_lens_keys(
            "Sony", "A7 IV", "35mm", "", &selected, &HashSet::new(),
            &HashSet::new(), &HashSet::new(), &HashSet::new(), 0, 0, Some(&ambiguous),
        ).is_empty());
    }
}
