// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::{ BTreeMap, BTreeSet };
use std::path::PathBuf;

use crate::lens_profile_database::LensProfileDatabase;

static CAMERA_DATABASE_JSON: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/camera_database/camera_database.json"));

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CameraDatabaseFile {
    pub version: u32,
    pub cameras: Vec<CameraEntry>,
    pub lenses: Vec<LensEntry>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CameraEntry {
    pub brand: String,
    pub model: String,
    pub brand_aliases: Vec<String>,
    pub aliases: Vec<String>,
    pub fixed_lens: Option<bool>,
    pub mounts: Vec<String>,
    pub compatible_mounts: Vec<String>,
    pub sensor_size: Option<String>,
    pub crop_factor: Option<f64>,
    pub source: Vec<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LensEntry {
    pub brand: String,
    pub model: String,
    pub aliases: Vec<String>,
    pub mounts: Vec<String>,
    pub crop_factor: Option<f64>,
    pub source: Vec<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CameraInfo {
    pub brand: String,
    pub model: String,
    pub brand_aliases: Vec<String>,
    pub aliases: Vec<String>,
    pub fixed_lens: bool,
    pub mounts: Vec<String>,
    pub compatible_mounts: Vec<String>,
    pub sensor_size: Option<String>,
    pub crop_factor: Option<f64>,
    pub source: Vec<String>,
    pub known_lens_count: usize,
}

#[derive(Clone, Debug)]
pub struct CameraDatabase {
    cameras: BTreeMap<(String, String), CameraEntry>,
    camera_lenses: BTreeMap<(String, String), BTreeSet<String>>,
    mount_lenses: BTreeMap<String, BTreeSet<String>>,
    lens_names: BTreeMap<String, (String, bool)>,
    lens_aliases: BTreeMap<String, Option<String>>,
    observed_lens_keys: BTreeMap<String, BTreeSet<String>>,
    camera_aliases: BTreeMap<(String, String), Option<(String, String)>>,
    camera_profile_keys: BTreeMap<(String, String), BTreeSet<(String, String)>>,
    brand_search_keys: BTreeMap<String, BTreeSet<(String, String)>>,
    brands: Vec<String>,
    models: BTreeMap<String, Vec<String>>,
    lenses: BTreeMap<(String, String), Vec<String>>,
}

impl Default for CameraDatabase {
    fn default() -> Self {
        Self::from_static()
    }
}

impl CameraDatabase {
    pub fn get_path() -> PathBuf {
        crate::settings::data_dir().join("lens_profiles").join("camera_database.json")
    }

    pub fn from_static() -> Self {
        let file = if let Ok(data) = std::fs::read_to_string(Self::get_path()) {
            match Self::parse_file(&data) {
                Ok(file) => file,
                Err(e) => {
                    log::warn!("Failed to load camera database update: {:?}", e);
                    Self::bundled_file()
                }
            }
        } else {
            Self::bundled_file()
        };

        Self::from_file(file)
    }

    fn from_file(file: CameraDatabaseFile) -> Self {
        let mut db = Self::empty();
        for camera in file.cameras {
            if !Self::is_legacy_observation_source(&camera.source) {
                db.merge_camera(camera);
            }
        }
        for lens in file.lenses {
            if !Self::is_legacy_observation_source(&lens.source) {
                db.merge_lens(lens);
            }
        }
        db.rebuild_indexes();
        db
    }

    pub fn parse_file(data: &str) -> Result<CameraDatabaseFile, String> {
        let file: CameraDatabaseFile = serde_json::from_str(data).map_err(|e| e.to_string())?;
        if file.version != 1 || file.cameras.is_empty() || file.lenses.is_empty() {
            return Err("Unsupported or empty camera database".to_owned());
        }
        if file.cameras.iter().any(|camera| camera.brand.trim().is_empty() || camera.model.trim().is_empty()) {
            return Err("Camera database contains an unnamed camera".to_owned());
        }
        if file.cameras.iter().all(|camera| Self::is_legacy_observation_source(&camera.source)) {
            return Err("Camera database contains no sourced primary camera identities".to_owned());
        }
        if file.lenses.iter().any(|lens| lens.brand.trim().is_empty() || lens.model.trim().is_empty()) {
            return Err("Camera database contains a lens without a make and model".to_owned());
        }
        if file.lenses.iter().all(|lens| Self::is_legacy_observation_source(&lens.source)) {
            return Err("Camera database contains no sourced primary lens identities".to_owned());
        }
        Ok(file)
    }

    fn bundled_file() -> CameraDatabaseFile {
        match Self::parse_file(CAMERA_DATABASE_JSON) {
            Ok(file) => file,
            Err(e) => {
                log::warn!("Failed to load bundled camera database: {:?}", e);
                CameraDatabaseFile::default()
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn from_bundled() -> Self {
        Self::from_file(Self::bundled_file())
    }

    pub fn from_lens_profile_database(profiles: &LensProfileDatabase) -> Self {
        let mut db = Self::from_static();
        db.add_profile_observations(profiles.iter_profiles());
        db
    }

    fn add_profile_observations<'a>(&mut self, profiles: impl Iterator<Item = &'a crate::lens_profile::LensProfile>) {
        for profile in profiles {
            if profile.is_copy || profile.camera_brand.is_empty() || profile.camera_model.is_empty() {
                continue;
            }

            let Some((camera_key, _)) = self.find_camera(&profile.camera_brand, &profile.camera_model) else {
                continue;
            };

            if let Some(identity) = self.resolve_profile_lens_identity(&profile.lens_model) {
                let lens_name = &self.lens_names[&identity].0;
                self.camera_lenses.entry(camera_key).or_default().insert(lens_name.clone());
                let raw = Self::key(&profile.lens_model);
                if raw != identity {
                    self.observed_lens_keys.entry(identity).or_default().insert(raw);
                }
            }
        }

        self.rebuild_indexes();
    }

    pub fn brands(&self) -> Vec<String> {
        self.brands.clone()
    }

    pub fn models(&self, brand: &str) -> Vec<String> {
        self.models.get(&Self::key(brand)).cloned().unwrap_or_default()
    }

    pub fn lenses(&self, brand: &str, model: &str) -> Vec<String> {
        self.lenses.get(&Self::camera_key(brand, model)).cloned().unwrap_or_default()
    }

    pub fn selected_camera_keys(&self, brand: &str, model: &str) -> BTreeSet<(String, String)> {
        self.find_camera(brand, model)
            .map(|(key, _)| self.camera_profile_keys.get(&key).cloned().unwrap_or_default())
            .unwrap_or_else(|| {
                let key = Self::camera_key(brand, model);
                if key.0.is_empty() || key.1.is_empty() || self.camera_aliases.contains_key(&key) {
                    BTreeSet::new()
                } else {
                    BTreeSet::from([key])
                }
            })
    }

    /// Profile camera keys for every catalog model under a selected canonical brand.
    /// Brand aliases stay paired with their model so manufacturer names that map to
    /// both Pentax and Ricoh cannot make one brand's profiles appear under the other.
    pub fn brand_camera_keys(&self, brand: &str) -> BTreeSet<(String, String)> {
        let Some(brand) = self.resolve_brand(brand) else {
            return BTreeSet::new();
        };
        self.brand_search_keys.get(&Self::key(&brand)).cloned().unwrap_or_default()
    }

    pub fn compatible_camera_keys(&self, brand: &str, model: &str) -> BTreeSet<(String, String)> {
        self.compatible_cameras(brand, model).into_iter()
            .flat_map(|camera| self.camera_profile_keys.get(&Self::camera_key(&camera.brand, &camera.model)).cloned().unwrap_or_default())
            .collect()
    }

    pub fn compatible_camera_names(&self, brand: &str, model: &str) -> Vec<String> {
        self.compatible_cameras(brand, model).into_iter()
            .map(|camera| format!("{} {}", camera.brand, camera.model))
            .collect()
    }

    pub fn camera_info(&self, brand: &str, model: &str) -> Option<CameraInfo> {
        self.find_camera(brand, model).map(|(key, camera)| CameraInfo {
            brand: camera.brand.clone(),
            model: camera.model.clone(),
            brand_aliases: camera.brand_aliases.clone(),
            aliases: camera.aliases.clone(),
            mounts: camera.mounts.clone(),
            compatible_mounts: camera.compatible_mounts.clone(),
            sensor_size: camera.sensor_size.clone(),
            crop_factor: camera.crop_factor,
            source: camera.source.clone(),
            // Lensfun reserves lowercase mount names for built-in optics.
            fixed_lens: camera.fixed_lens.unwrap_or_else(|| !camera.mounts.is_empty() && camera.mounts.iter().all(|mount| {
                mount.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            })),
            known_lens_count: self.lenses.get(&key).map(|x| x.len()).unwrap_or_default(),
        })
    }

    pub fn resolve_model(&self, brand: &str, model: &str) -> Option<String> {
        self.find_camera(brand, model).map(|(_, camera)| camera.model.clone())
    }

    pub fn resolve_camera(&self, brand: &str, model: &str) -> Option<(String, String)> {
        if let Some((_, camera)) = self.find_camera(brand, model) {
            return Some((camera.brand.clone(), camera.model.clone()));
        }

        let brand = self.resolve_brand(brand)?;
        Some((brand.clone(), Self::model_name(&brand, model)))
    }

    /// Resolve a profile or video metadata lens string to an available canonical lens label.
    /// Exact canonical labels take precedence; other spellings must be an unambiguous catalog alias.
    pub fn resolve_lens(&self, brand: &str, model: &str, raw: &str) -> Option<String> {
        let (camera_key, _) = self.find_camera(brand, model)?;
        let available = self.lenses.get(&camera_key)?;
        let raw_key = Self::key(raw);
        if let Some(canonical) = available.iter().find(|lens| Self::key(lens) == raw_key) {
            return Some(canonical.clone());
        }

        let identity = self.resolve_profile_lens_identity(raw)?;
        let canonical = &self.lens_names.get(&identity)?.0;
        available.iter().find(|lens| Self::key(lens) == Self::key(canonical)).cloned()
    }

    /// Profile lens spellings that identify the selected canonical lens.
    /// `Some(empty)` means a known but ambiguous catalog alias; `None` means free text.
    pub fn lens_profile_keys(&self, brand: &str, model: &str, selected: &str) -> Option<BTreeSet<String>> {
        let (camera_key, _) = self.find_camera(brand, model)?;
        let available = self.lenses.get(&camera_key)?;
        let selected_key = Self::key(selected);

        let identity = if let Some(canonical) = available.iter().find(|lens| Self::key(lens) == selected_key) {
            Self::key(canonical)
        } else {
            match self.lens_aliases.get(&selected_key) {
                Some(Some(identity)) => identity.clone(),
                Some(None) => return Some(BTreeSet::new()),
                None => return None,
            }
        };
        let canonical = self.lens_names.get(&identity)?.0.as_str();
        if !available.iter().any(|lens| Self::key(lens) == Self::key(canonical)) {
            return None;
        }

        let mut keys = self.observed_lens_keys.get(&identity).cloned().unwrap_or_default();
        keys.insert(identity.clone());
        keys.extend(self.lens_aliases.iter().filter_map(|(alias, candidate)| {
            (candidate.as_deref() == Some(identity.as_str()) && !self.lens_names.contains_key(alias))
                .then(|| alias.clone())
        }));
        Some(keys)
    }

    fn empty() -> Self {
        Self {
            cameras: BTreeMap::new(),
            camera_lenses: BTreeMap::new(),
            mount_lenses: BTreeMap::new(),
            lens_names: BTreeMap::new(),
            lens_aliases: BTreeMap::new(),
            observed_lens_keys: BTreeMap::new(),
            camera_aliases: BTreeMap::new(),
            camera_profile_keys: BTreeMap::new(),
            brand_search_keys: BTreeMap::new(),
            brands: Vec::new(),
            models: BTreeMap::new(),
            lenses: BTreeMap::new(),
        }
    }

    fn merge_camera(&mut self, camera: CameraEntry) {
        let original_brand = camera.brand.clone();
        let camera = CameraEntry {
            brand: Self::camera_brand_name(&camera.brand, &camera.model),
            model: Self::model_name(&camera.brand, &camera.model),
            ..camera
        };
        if camera.brand.is_empty() || camera.model.is_empty() {
            return;
        }

        let key = Self::camera_key(&camera.brand, &camera.model);
        let entry = self.cameras.entry(key).or_insert_with(|| CameraEntry {
            brand: camera.brand.clone(),
            model: camera.model.clone(),
            ..Default::default()
        });

        if Self::key(&entry.brand) != Self::key(&original_brand) {
            Self::extend_unique(&mut entry.brand_aliases, std::slice::from_ref(&original_brand));
        }
        Self::extend_unique(&mut entry.brand_aliases, &camera.brand_aliases);
        if Self::key(&entry.model) != Self::key(&camera.model) {
            Self::extend_unique(&mut entry.aliases, std::slice::from_ref(&camera.model));
        }
        Self::extend_unique(&mut entry.aliases, &camera.aliases);
        Self::extend_unique(&mut entry.mounts, &camera.mounts);
        Self::extend_unique(&mut entry.compatible_mounts, &camera.compatible_mounts);
        Self::extend_unique(&mut entry.source, &camera.source);

        if camera.fixed_lens.is_some() {
            entry.fixed_lens = camera.fixed_lens;
        }
        if entry.crop_factor.is_none() {
            entry.crop_factor = camera.crop_factor;
        }
        if entry.sensor_size.is_none() {
            entry.sensor_size = camera.sensor_size;
        }
    }

    fn is_legacy_observation_source(sources: &[String]) -> bool {
        sources.len() == 1 && matches!(Self::key(&sources[0]).as_str(), "profiles" | "gyroflow")
    }

    fn merge_lens(&mut self, lens: LensEntry) {
        let lens = LensEntry {
            brand: Self::brand_name(&lens.brand),
            ..lens
        };
        if lens.model.is_empty() {
            return;
        }

        let name = if lens.brand.is_empty() || Self::key(&lens.model).starts_with(&Self::key(&lens.brand)) {
            lens.model.clone()
        } else {
            format!("{} {}", lens.brand, lens.model)
        };
        let identity = Self::key(&name);
        let is_lensfun = lens.source.iter().any(|source| Self::key(source) == "lensfun");
        match self.lens_names.get_mut(&identity) {
            Some((existing, existing_is_lensfun)) if is_lensfun && !*existing_is_lensfun => {
                *existing = name.clone();
                *existing_is_lensfun = true;
            }
            Some((existing, existing_is_lensfun)) if is_lensfun == *existing_is_lensfun && name.as_str() < existing.as_str() => {
                *existing = name.clone();
            }
            Some(_) => {}
            None => {
                self.lens_names.insert(identity.clone(), (name.clone(), is_lensfun));
            }
        }

        for alias in lens.aliases.iter().map(|alias| Self::key(alias)).filter(|alias| !alias.is_empty()) {
            use std::collections::btree_map::Entry;
            match self.lens_aliases.entry(alias) {
                Entry::Vacant(entry) => { entry.insert(Some(identity.clone())); }
                Entry::Occupied(mut entry) => {
                    if entry.get().as_deref() != Some(identity.as_str()) {
                        entry.insert(None);
                    }
                }
            }
        }

        for mount in lens.mounts {
            self.mount_lenses.entry(Self::key(&mount)).or_default().insert(name.clone());
        }
    }

    fn rebuild_indexes(&mut self) {
        let mut brands = BTreeSet::new();
        let mut models: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut lenses = self.camera_lenses.clone();
        let mut brand_search_keys: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();

        let mut alias_targets: BTreeMap<(String, String), BTreeSet<(String, String)>> = BTreeMap::new();
        for (key, camera) in &self.cameras {
            for profile_key in Self::all_camera_search_keys(camera) {
                alias_targets.entry(profile_key).or_default().insert(key.clone());
            }
        }
        let primary_keys = self.cameras.keys().cloned().collect::<BTreeSet<_>>();
        self.camera_aliases = alias_targets.into_iter().map(|(profile_key, targets)| {
            let target = if primary_keys.contains(&profile_key) {
                Some(profile_key.clone())
            } else if targets.len() == 1 {
                targets.into_iter().next()
            } else {
                None
            };
            (profile_key, target)
        }).collect();

        self.camera_profile_keys = self.cameras.iter().map(|(key, camera)| {
            let safe_keys = Self::all_camera_search_keys(camera).into_iter()
                .filter(|profile_key| self.camera_aliases.get(profile_key).and_then(Option::as_ref) == Some(key))
                .collect();
            (key.clone(), safe_keys)
        }).collect();

        for ((brand_key, model_key), camera) in &self.cameras {
            brands.insert(camera.brand.clone());
            models.entry(brand_key.clone()).or_default().insert(camera.model.clone());
            brand_search_keys.entry(brand_key.clone()).or_default()
                .extend(self.camera_profile_keys.get(&(brand_key.clone(), model_key.clone())).into_iter().flatten().cloned());

            let camera_lenses = lenses.entry((brand_key.clone(), model_key.clone())).or_default();
            for mount in camera.mounts.iter().chain(camera.compatible_mounts.iter()) {
                if let Some(mount_lenses) = self.mount_lenses.get(&Self::key(mount)) {
                    camera_lenses.extend(mount_lenses.iter().cloned());
                }
            }
        }

        self.brands = brands.into_iter().collect();
        self.brand_search_keys = brand_search_keys;
        self.models = models.into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect();
        self.lenses = lenses.into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect();
    }

    fn compatible_cameras(&self, brand: &str, model: &str) -> Vec<CameraEntry> {
        let Some((selected_key, selected)) = self.find_camera(brand, model) else {
            return Vec::new();
        };

        let selected_mounts = Self::mount_keys(selected);
        if selected_mounts.is_empty() {
            return Vec::new();
        }

        self.cameras.iter()
            .filter(|(key, camera)| {
                *key != &selected_key &&
                Self::mounts_overlap(&selected_mounts, camera) &&
                Self::sensor_compatible(selected, camera)
            })
            .map(|(_, camera)| camera.clone())
            .collect()
    }

    fn find_camera(&self, brand: &str, model: &str) -> Option<((String, String), &CameraEntry)> {
        let key = Self::camera_key(brand, model);
        if let Some(camera) = self.cameras.get(&key) {
            return Some((key, camera));
        }
        let key = self.camera_aliases.get(&key)?.as_ref()?.clone();
        self.cameras.get(&key).map(|camera| (key, camera))
    }

    fn all_camera_search_keys(camera: &CameraEntry) -> BTreeSet<(String, String)> {
        let brands = std::iter::once(&camera.brand).chain(camera.brand_aliases.iter()).collect::<Vec<_>>();
        let models = std::iter::once(&camera.model).chain(camera.aliases.iter()).collect::<Vec<_>>();
        let mut keys = BTreeSet::new();
        for brand in brands {
            for model in &models {
                keys.insert(Self::camera_key(brand, model));
            }
        }
        keys
    }

    fn mount_keys(camera: &CameraEntry) -> BTreeSet<String> {
        camera.mounts.iter()
            .map(|mount| Self::key(mount))
            .filter(|mount| !mount.is_empty())
            .collect()
    }

    fn mounts_overlap(selected_mounts: &BTreeSet<String>, camera: &CameraEntry) -> bool {
        Self::mount_keys(camera).iter().any(|mount| selected_mounts.contains(mount))
    }

    fn sensor_compatible(selected: &CameraEntry, candidate: &CameraEntry) -> bool {
        if let (Some(a), Some(b)) = (&selected.sensor_size, &candidate.sensor_size) {
            if Self::key(a) != Self::key(b) {
                return false;
            }
        }

        match (selected.crop_factor, candidate.crop_factor) {
            (Some(a), Some(b)) => a.is_finite() && b.is_finite() && a > 0.0 && b > 0.0 && (a - b).abs() <= 0.15,
            _ => match (&selected.sensor_size, &candidate.sensor_size) {
                (Some(a), Some(b)) => Self::key(a) == Self::key(b),
                _ => false,
            },
        }
    }

    fn extend_unique(target: &mut Vec<String>, values: &[String]) {
        let mut seen: BTreeSet<String> = target.iter().map(|x| Self::key(x)).collect();
        for value in values.iter().filter(|x| !x.is_empty()) {
            if seen.insert(Self::key(value)) {
                target.push(value.clone());
            }
        }
    }

    fn camera_key(brand: &str, model: &str) -> (String, String) {
        (Self::key(brand), Self::key(model))
    }

    fn key(value: &str) -> String {
        value.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()
    }

    fn brand_name(value: &str) -> String {
        match Self::key(value).replace('.', "").as_str() {
            "apple" => "Apple".to_owned(),
            "activeon" => "ACTIVEON".to_owned(),
            "akaso" => "AKASO".to_owned(),
            "arri" => "ARRI".to_owned(),
            "asahi optical co,ltd" => "Pentax".to_owned(),
            "asus" => "Asus".to_owned(),
            "betafpv" => "BetaFPV".to_owned(),
            "blackmagic" => "Blackmagic".to_owned(),
            "canon" => "Canon".to_owned(),
            "casio computer co,ltd" => "Casio".to_owned(),
            "casio computer co,ltd." => "Casio".to_owned(),
            "cooau" => "COOAU".to_owned(),
            "dji" => "DJI".to_owned(),
            "eastman kodak company" => "Kodak".to_owned(),
            "eken" => "EKEN".to_owned(),
            "feiyu-tech" => "Feiyu Tech".to_owned(),
            "fimi" => "FIMI".to_owned(),
            "fufifilm" => "Fujifilm".to_owned(),
            "fujifilm" => "Fujifilm".to_owned(),
            "gitup" => "GitUp".to_owned(),
            "gopro" => "GoPro".to_owned(),
            "google" => "Google".to_owned(),
            "huawei" => "Huawei".to_owned(),
            "iqoo" => "IQOO".to_owned(),
            "iqoo 9" => "IQOO".to_owned(),
            "insta360" => "Insta360".to_owned(),
            "lamax" => "LAMAX".to_owned(),
            "leica" => "Leica".to_owned(),
            "leica camera ag" => "Leica".to_owned(),
            "lg mobile" => "LG".to_owned(),
            "lge" => "LG".to_owned(),
            "mi" => "Xiaomi".to_owned(),
            "nikon" => "Nikon".to_owned(),
            "nikon corporation" => "Nikon".to_owned(),
            "olympus" => "Olympus".to_owned(),
            "olympus corporation" => "Olympus".to_owned(),
            "olympus imaging corp" => "Olympus".to_owned(),
            "olympus optical co,ltd" => "Olympus".to_owned(),
            "oneplus" => "OnePlus".to_owned(),
            "oppo" => "OPPO".to_owned(),
            "panasonic" => "Panasonic".to_owned(),
            "pentax" => "Pentax".to_owned(),
            "pentax corporation" => "Pentax".to_owned(),
            "red" => "RED".to_owned(),
            "ricoh" => "Ricoh".to_owned(),
            "runcam" => "RunCam".to_owned(),
            "samsung" => "Samsung".to_owned(),
            "samsung techwin" => "Samsung".to_owned(),
            "samsung techwin co" => "Samsung".to_owned(),
            "sjcam" => "SJCam".to_owned(),
            "sony" => "Sony".to_owned(),
            "volla" => "Volla".to_owned(),
            "vivo" => "Vivo".to_owned(),
            "wolfang" => "Wolfang".to_owned(),
            "wolfgang" => "Wolfang".to_owned(),
            "xiaomi" => "Xiaomi".to_owned(),
            _ => value.split_whitespace().collect::<Vec<_>>().join(" "),
        }
    }

    fn camera_brand_name(brand: &str, model: &str) -> String {
        if Self::key(brand).replace('.', "") == "ricoh imaging company, ltd" {
            let model = Self::key(model);
            if model.starts_with("k-") || model == "kf" || model == "kp" {
                return "Pentax".to_owned();
            }
            return "Ricoh".to_owned();
        }
        Self::brand_name(brand)
    }

    fn model_name(brand: &str, model: &str) -> String {
        let mut model = model.split_whitespace().collect::<Vec<_>>().join(" ").replace("CInema", "Cinema");
        if Self::key(brand) == "blackmagic" {
            model = model.replace("4k", "4K").replace("6k", "6K");
        }
        model
    }

    fn resolve_brand(&self, brand: &str) -> Option<String> {
        let brand_key = Self::key(brand);
        if brand_key.is_empty() {
            return None;
        }

        if let Some(brand) = self.brands.iter().find(|brand| Self::key(brand) == brand_key) {
            return Some(brand.clone());
        }

        let matches = self.cameras.values()
            .filter(|camera| camera.brand_aliases.iter().any(|alias| Self::key(alias) == brand_key))
            .map(|camera| camera.brand.clone())
            .collect::<BTreeSet<_>>();

        if matches.len() == 1 {
            matches.into_iter().next()
        } else {
            None
        }
    }

    fn resolve_profile_lens_identity(&self, raw: &str) -> Option<String> {
        let key = Self::key(raw);
        if self.lens_names.contains_key(&key) {
            return Some(key);
        }
        if let Some(alias) = self.lens_aliases.get(&key) {
            return alias.clone();
        }
        // Some profiles append the focal length used for the shot to the lens name.
        static SETTING: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||
            regex::Regex::new(r"(?i)(?:\s+|\s*@\s*)[0-9]{1,3}(?:\.[0-9]+)?\s*mm$").unwrap()
        );
        let model = &key[..SETTING.find(&key)?.start()];
        if self.lens_names.contains_key(model) {
            Some(model.to_owned())
        } else {
            self.lens_aliases.get(model).and_then(Clone::clone)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_primary_identity_wins_over_another_bodys_alias() {
        let mut db = CameraDatabase::empty();
        for (model, aliases, mounts) in [("Body A", vec!["Body B".into()], vec!["builtIn".into()]), ("Body B", vec![], vec!["Mount".into()])] {
            db.merge_camera(CameraEntry { brand: "Example".into(), model: model.into(), aliases, mounts, ..Default::default() });
        }
        assert!(db.camera_info("Example", "Body A").unwrap().fixed_lens);
        let second = db.camera_info("Example", "Body B").unwrap();
        assert_eq!(second.model, "Body B");
        assert!(!second.fixed_lens);
    }

    #[test]
    fn rejects_legacy_catalogs_that_treat_unidentified_lenses_as_models() {
        let legacy = r#"{"version":1,"cameras":[{"brand":"Sony","model":"Alpha 7C"}],"lenses":[{"model":"35mm"}]}"#;
        assert!(CameraDatabase::parse_file(legacy).is_err());

        let observations_only = r#"{"version":1,"cameras":[{"brand":"Sony","model":"Observed 7C","mounts":["E mount"],"source":["profiles"]}],"lenses":[{"brand":"Sony","model":"FE 35mm F1.4 GM","source":["gyroflow"]}]}"#;
        assert!(CameraDatabase::parse_file(observations_only).is_err());
        let stale_lenses = r#"{"version":1,"cameras":[{"brand":"Sony","model":"Alpha 7C","source":["lensfun"]}],"lenses":[{"brand":"Sony","model":"FE 35mm F1.4 GM","source":["profiles"]}]}"#;
        assert!(CameraDatabase::parse_file(stale_lenses).is_err());
        let generated_source_less = r#"{"version":1,"cameras":[{"brand":"Sony","model":"Alpha 7C"}],"lenses":[{"brand":"Sony","model":"FE 35mm F1.4 GM"}]}"#;
        assert!(CameraDatabase::parse_file(generated_source_less).is_ok());
    }

    #[test]
    fn rejects_incomplete_or_unsupported_updates() {
        for data in ["{}", "{\"version\":1}", "{\"version\":2,\"cameras\":[{}],\"lenses\":[{}]}"] {
            assert!(CameraDatabase::parse_file(data).is_err());
        }
        assert!(CameraDatabase::parse_file(r#"{"version":1,"cameras":[{"brand":"Sony","model":"FX3"}],"lenses":[{}]}"#).is_err());
        assert!(CameraDatabase::parse_file(CAMERA_DATABASE_JSON).is_ok());
    }

    #[test]
    fn unknown_sensor_is_not_a_compatibility_claim() {
        let known = CameraEntry { crop_factor: Some(1.0), ..Default::default() };
        assert!(!CameraDatabase::sensor_compatible(&known, &CameraEntry::default()));
        assert!(!CameraDatabase::sensor_compatible(&CameraEntry::default(), &CameraEntry::default()));
        for crop in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(!CameraDatabase::sensor_compatible(&known, &CameraEntry { crop_factor: Some(crop), ..Default::default() }));
        }
        let medium_format_645 = CameraEntry { sensor_size: Some("Medium format".to_owned()), crop_factor: Some(0.577), ..Default::default() };
        let medium_format_iq140 = CameraEntry { sensor_size: Some("Medium format".to_owned()), crop_factor: Some(0.789), ..Default::default() };
        assert!(!CameraDatabase::sensor_compatible(&medium_format_645, &medium_format_iq140));
        assert!(!CameraDatabase::sensor_compatible(
            &CameraEntry { sensor_size: Some("Full frame".to_owned()), crop_factor: Some(1.0), ..Default::default() },
            &CameraEntry { sensor_size: Some("APS-C".to_owned()), crop_factor: Some(1.5), ..Default::default() }
        ));
    }

    #[test]
    fn static_database_loads() {
        let db = CameraDatabase::from_file(CameraDatabase::bundled_file());
        assert!(db.brands().len() > 10);
        assert!(db.models.values().any(|models| models.len() > 10));
        assert!(db.lenses.values().any(|lenses| lenses.len() > 10));
    }

    #[test]
    fn model_indexes_are_scoped_to_their_brand() {
        let db = CameraDatabase::from_file(CameraDatabase::bundled_file());
        for brand in db.brands() {
            let brand_key = CameraDatabase::key(&brand);
            for model in db.models(&brand) {
                let camera = db.cameras.get(&CameraDatabase::camera_key(&brand, &model)).expect("model index must point to a camera");
                assert_eq!(CameraDatabase::key(&camera.brand), brand_key);
                assert_eq!(camera.model, model);
            }
        }
    }

    #[test]
    fn mount_based_cameras_expose_lens_options() {
        let db = CameraDatabase::from_file(CameraDatabase::bundled_file());
        let mut checked = 0;

        for camera in db.cameras.values() {
            let has_known_lens_mount = camera.mounts.iter()
                .chain(camera.compatible_mounts.iter())
                .any(|mount| db.mount_lenses.contains_key(&CameraDatabase::key(mount)));

            if has_known_lens_mount {
                checked += 1;
                assert!(!db.lenses(&camera.brand, &camera.model).is_empty());
            }
        }

        assert!(checked > 10);
    }

    #[test]
    fn local_profiles_only_attach_known_lenses_to_catalog_camera_aliases() {
        let mut db = CameraDatabase::from_file(CameraDatabaseFile {
            version: 1,
            cameras: vec![CameraEntry {
                brand: "Sony".to_owned(),
                model: "Alpha 7 IV".to_owned(),
                aliases: vec!["A7 IV".to_owned()],
                ..Default::default()
            }],
            lenses: vec![LensEntry {
                brand: "Sony".to_owned(),
                model: "FE 35mm F1.4 GM".to_owned(),
                aliases: vec!["FE 35mm F1.4 GM".to_owned()],
                source: vec!["lensfun".to_owned()],
                ..Default::default()
            }, LensEntry {
                brand: "Sony".to_owned(),
                model: "Old Profile Lens".to_owned(),
                mounts: vec!["E mount".to_owned()],
                source: vec!["profiles".to_owned()],
                ..Default::default()
            }],
            ..Default::default()
        });

        let profile = |brand: &str, model: &str, lens: &str| {
            let mut profile = crate::lens_profile::LensProfile::default();
            profile.camera_brand = brand.to_owned();
            profile.camera_model = model.to_owned();
            profile.lens_model = lens.to_owned();
            profile
        };
        let known_alias = profile("Sony", "A7 IV", "fe 35mm f1.4 gm");
        let unknown_model = profile("Sony", "New Body", "FE 35mm F1.4 GM");
        let unknown_brand = profile("NewBrand", "New Body", "FE 35mm F1.4 GM");
        let unknown_lens = profile("Sony", "Alpha 7 IV", "Unlisted Lens");
        db.add_profile_observations([&known_alias, &unknown_model, &unknown_brand, &unknown_lens].into_iter());

        assert_eq!(db.models("Sony"), vec!["Alpha 7 IV"]);
        assert!(db.brands().iter().all(|brand| brand != "NewBrand"));
        let lenses = db.lenses("Sony", "Alpha 7 IV");
        assert!(lenses.iter().any(|lens| lens == "Sony FE 35mm F1.4 GM"));
        assert!(!lenses.iter().any(|lens| lens == "Sony Old Profile Lens"));
        assert!(!lenses.iter().any(|lens| lens == "Unlisted Lens"));
    }

    #[test]
    fn lens_aliases_resolve_only_when_unambiguous_and_camera_scoped() {
        let mut db = CameraDatabase::from_file(CameraDatabaseFile {
            version: 1,
            cameras: vec![
                CameraEntry {
                    brand: "Sony".to_owned(),
                    model: "Alpha 7 IV".to_owned(),
                    mounts: vec!["E mount".to_owned()],
                    ..Default::default()
                },
                CameraEntry {
                    brand: "Canon".to_owned(),
                    model: "EOS R5".to_owned(),
                    mounts: vec!["RF mount".to_owned()],
                    ..Default::default()
                },
            ],
            lenses: vec![
                LensEntry {
                    brand: "Sony".to_owned(),
                    model: "FE 35mm F1.4 GM".to_owned(),
                    aliases: vec!["35 Sony GM".to_owned(), "35mm prime".to_owned()],
                    mounts: vec!["E mount".to_owned()],
                    ..Default::default()
                },
                LensEntry {
                    brand: "Sony".to_owned(),
                    model: "FE 50mm F1.2 GM".to_owned(),
                    aliases: vec!["50 GM".to_owned(), "35mm prime".to_owned()],
                    mounts: vec!["E mount".to_owned()],
                    ..Default::default()
                },
                LensEntry {
                    brand: "Canon".to_owned(),
                    model: "RF 35mm F1.8".to_owned(),
                    aliases: vec!["35 GM".to_owned(), "RF prime".to_owned()],
                    mounts: vec!["RF mount".to_owned()],
                    ..Default::default()
                },
            ],
        });
        db.rebuild_indexes();

        assert_eq!(db.resolve_lens("Sony", "Alpha 7 IV", "  35 Sony GM "), Some("Sony FE 35mm F1.4 GM".to_owned()));
        assert_eq!(db.resolve_lens("Sony", "Alpha 7 IV", "35 GM"), None);
        assert_eq!(db.resolve_lens("Sony", "Alpha 7 IV", "35mm prime"), None);
        assert_eq!(db.resolve_lens("Sony", "Alpha 7 IV", "Sony FE 35mm F1.4 GM"), Some("Sony FE 35mm F1.4 GM".to_owned()));
        assert_eq!(db.resolve_lens("Sony", "Alpha 7 IV", "RF prime"), None);
        assert_eq!(db.resolve_lens("Canon", "EOS R5", "RF prime"), Some("Canon RF 35mm F1.8".to_owned()));

        let keys = db.lens_profile_keys("Sony", "Alpha 7 IV", "Sony FE 35mm F1.4 GM").unwrap();
        assert!(keys.contains("sony fe 35mm f1.4 gm"));
        assert!(keys.contains("35 sony gm"));
        assert!(!keys.contains("35mm prime"));
    }

    #[test]
    fn known_brand_and_model_variants_are_canonicalized() {
        let mut db = CameraDatabase::empty();
        db.merge_camera(CameraEntry {
            brand: "Olympus Imaging Corp.".to_owned(),
            model: "E-M1".to_owned(),
            ..Default::default()
        });
        db.merge_camera(CameraEntry {
            brand: "OLYMPUS".to_owned(),
            model: "TG-6".to_owned(),
            ..Default::default()
        });
        db.merge_camera(CameraEntry {
            brand: "Blackmagic".to_owned(),
            model: "Pocket CInema Camera 4k".to_owned(),
            ..Default::default()
        });
        db.merge_camera(CameraEntry {
            brand: "LGE".to_owned(),
            model: "V40".to_owned(),
            ..Default::default()
        });
        db.rebuild_indexes();

        assert!(db.brands().contains(&"Olympus".to_owned()));
        assert!(!db.brands().contains(&"Olympus Imaging Corp.".to_owned()));
        assert!(db.models("Blackmagic").contains(&"Pocket Cinema Camera 4K".to_owned()));
        assert!(db.selected_camera_keys("Olympus", "E-M1").contains(&CameraDatabase::camera_key("Olympus Imaging Corp.", "E-M1")));
        assert_eq!(db.resolve_camera("Olympus Imaging Corp.", "E-M1"), Some(("Olympus".to_owned(), "E-M1".to_owned())));
        assert_eq!(db.resolve_camera("LGE", "Unknown"), Some(("LG".to_owned(), "Unknown".to_owned())));
    }

    #[test]
    fn compatible_cameras_require_shared_mount_and_sensor_class() {
        let db = CameraDatabase::from_file(CameraDatabaseFile {
            version: 1,
            cameras: vec![CameraEntry {
            brand: "Brand".to_owned(),
            model: "Primary".to_owned(),
            aliases: vec!["Primary Alias".to_owned()],
            mounts: vec!["Mount A".to_owned()],
            sensor_size: Some("Full frame".to_owned()),
            crop_factor: Some(1.0),
            ..Default::default()
            }, CameraEntry {
            brand: "Brand".to_owned(),
            model: "Primary Alias".to_owned(),
            mounts: vec!["Mount A".to_owned()],
            sensor_size: Some("APS-C".to_owned()),
            crop_factor: Some(1.5),
            source: vec!["profiles".to_owned()],
            ..Default::default()
            }, CameraEntry {
            brand: "Brand".to_owned(),
            model: "Same Mount".to_owned(),
            mounts: vec!["Mount A".to_owned()],
            sensor_size: Some("Full frame".to_owned()),
            crop_factor: Some(1.02),
            ..Default::default()
            }, CameraEntry {
            brand: "Brand".to_owned(),
            model: "Different Sensor".to_owned(),
            mounts: vec!["Mount A".to_owned()],
            sensor_size: Some("APS-C".to_owned()),
            crop_factor: Some(1.5),
            ..Default::default()
            }, CameraEntry {
            brand: "Brand".to_owned(),
            model: "Different Mount".to_owned(),
            mounts: vec!["Mount B".to_owned()],
            sensor_size: Some("Full frame".to_owned()),
            crop_factor: Some(1.0),
            ..Default::default()
            }, CameraEntry {
            brand: "Brand".to_owned(),
            model: "Adapter Only".to_owned(),
            compatible_mounts: vec!["Mount A".to_owned()],
            sensor_size: Some("Full frame".to_owned()),
            crop_factor: Some(1.0),
            ..Default::default()
            }],
            lenses: vec![],
        });

        assert_eq!(db.compatible_camera_names("Brand", "Primary"), vec!["Brand Same Mount"]);
        assert!(db.selected_camera_keys("Brand", "Primary").contains(&CameraDatabase::camera_key("Brand", "Primary Alias")));
        assert!(!db.models("Brand").contains(&"Primary Alias".to_owned()));
    }

    #[test]
    fn ambiguous_and_primary_colliding_camera_aliases_do_not_leak_profile_keys() {
        let db = CameraDatabase::from_file(CameraDatabaseFile {
            version: 1,
            cameras: vec![
                CameraEntry { brand: "Brand".into(), model: "Body A".into(), aliases: vec!["Body B".into(), "Shared Alias".into()], mounts: vec!["Mount".into()], sensor_size: Some("Full frame".into()), crop_factor: Some(1.0), ..Default::default() },
                CameraEntry { brand: "Brand".into(), model: "Body B".into(), aliases: vec!["Shared Alias".into()], mounts: vec!["Mount".into()], sensor_size: Some("Full frame".into()), crop_factor: Some(1.0), ..Default::default() },
                CameraEntry { brand: "Brand".into(), model: "Body C".into(), mounts: vec!["Mount".into()], sensor_size: Some("Full frame".into()), crop_factor: Some(1.0), ..Default::default() },
                CameraEntry { brand: "Brand".into(), model: "Body B".into(), source: vec!["profiles".into()], ..Default::default() },
                CameraEntry { brand: "Brand".into(), model: "Shared Alias".into(), source: vec!["profiles".into()], ..Default::default() },
            ],
            lenses: vec![],
        });

        let a_keys = db.selected_camera_keys("Brand", "Body A");
        assert!(!a_keys.contains(&CameraDatabase::camera_key("Brand", "Body B")));
        assert!(db.selected_camera_keys("Brand", "Body B").contains(&CameraDatabase::camera_key("Brand", "Body B")));
        assert!(db.selected_camera_keys("Brand", "Shared Alias").is_empty());
        assert!(!db.models("Brand").contains(&"Shared Alias".to_owned()));
        let compatible = db.compatible_camera_keys("Brand", "Body C");
        assert!(!compatible.contains(&CameraDatabase::camera_key("Brand", "Shared Alias")));
        assert!(compatible.contains(&CameraDatabase::camera_key("Brand", "Body B")));
    }

    #[test]
    fn observed_focal_suffix_resolves_to_catalog_lens_and_profile_key() {
        let mut db = CameraDatabase::from_file(CameraDatabaseFile {
            version: 1,
            cameras: vec![CameraEntry { brand: "Sony".into(), model: "Alpha 7 IV".into(), mounts: vec!["E mount".into()], ..Default::default() }],
            lenses: vec![LensEntry { brand: "Sony".into(), model: "FE 35mm F1.4 GM".into(), mounts: vec!["E mount".into()], source: vec!["lensfun".into()], ..Default::default() }],
        });
        let mut profile = crate::lens_profile::LensProfile::default();
        profile.camera_brand = "Sony".into();
        profile.camera_model = "Alpha 7 IV".into();
        profile.lens_model = "Sony FE 35mm F1.4 GM 50mm".into();
        db.add_profile_observations([&profile].into_iter());

        assert_eq!(db.resolve_lens("Sony", "Alpha 7 IV", &profile.lens_model), Some("Sony FE 35mm F1.4 GM".into()));
        let keys = db.lens_profile_keys("Sony", "Alpha 7 IV", "Sony FE 35mm F1.4 GM").unwrap();
        assert!(keys.contains("sony fe 35mm f1.4 gm 50mm"));
    }

    #[test]
    fn lens_brand_prefix_is_case_insensitive() {
        let mut db = CameraDatabase::empty();
        db.merge_lens(LensEntry { brand: "Leica".into(), model: "LEICA Q2 & compatibles".into(), ..Default::default() });

        assert_eq!(db.lens_names["leica q2 & compatibles"].0, "LEICA Q2 & compatibles");
    }
}
