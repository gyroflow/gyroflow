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
    pub updated_at: String,
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
            match serde_json::from_str::<CameraDatabaseFile>(&data) {
                Ok(file) => file,
                Err(e) => {
                    log::warn!("Failed to load camera database update: {:?}", e);
                    Self::bundled_file()
                }
            }
        } else {
            Self::bundled_file()
        };

        let mut db = Self::empty();
        for camera in file.cameras {
            db.merge_camera(camera);
        }
        for lens in file.lenses {
            db.merge_lens(lens);
        }
        db.rebuild_indexes();
        db
    }

    fn bundled_file() -> CameraDatabaseFile {
        match serde_json::from_str::<CameraDatabaseFile>(CAMERA_DATABASE_JSON) {
            Ok(file) => file,
            Err(e) => {
                log::warn!("Failed to load bundled camera database: {:?}", e);
                CameraDatabaseFile::default()
            }
        }
    }

    pub fn from_lens_profile_database(profiles: &LensProfileDatabase) -> Self {
        let mut db = Self::from_static();

        for profile in profiles.iter_profiles() {
            if profile.is_copy || profile.camera_brand.is_empty() || profile.camera_model.is_empty() {
                continue;
            }

            let camera = CameraEntry {
                brand: profile.camera_brand.clone(),
                model: profile.camera_model.clone(),
                crop_factor: profile.crop_factor,
                source: vec!["gyroflow".to_owned()],
                ..Default::default()
            };
            db.merge_camera(camera);

            if !profile.lens_model.is_empty() {
                let key = db.find_camera(&profile.camera_brand, &profile.camera_model)
                    .map(|(key, _)| key)
                    .unwrap_or_else(|| Self::camera_key(&profile.camera_brand, &profile.camera_model));
                db.camera_lenses.entry(key)
                    .or_default()
                    .insert(profile.lens_model.clone());
            }
        }

        db.rebuild_indexes();
        db
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
            .map(|(_, camera)| Self::camera_search_keys(camera))
            .unwrap_or_else(|| {
                let key = Self::camera_key(brand, model);
                if key.0.is_empty() || key.1.is_empty() {
                    BTreeSet::new()
                } else {
                    BTreeSet::from([key])
                }
            })
    }

    pub fn compatible_camera_keys(&self, brand: &str, model: &str) -> BTreeSet<(String, String)> {
        self.compatible_cameras(brand, model).into_iter()
            .flat_map(|camera| Self::camera_search_keys(&camera))
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

    fn empty() -> Self {
        Self {
            cameras: BTreeMap::new(),
            camera_lenses: BTreeMap::new(),
            mount_lenses: BTreeMap::new(),
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

        let mut key = Self::camera_key(&camera.brand, &camera.model);
        if !self.cameras.contains_key(&key) {
            let brand_key = key.0.clone();
            let model_key = key.1.clone();
            if let Some(existing_key) = self.cameras.iter()
                .find(|((candidate_brand, _), candidate)| {
                    candidate_brand == &brand_key &&
                    candidate.aliases.iter().any(|alias| Self::key(alias) == model_key)
                })
                .map(|(existing_key, _)| existing_key.clone())
            {
                key = existing_key;
            }
        }

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

        if entry.crop_factor.is_none() {
            entry.crop_factor = camera.crop_factor;
        }
        if entry.sensor_size.is_none() {
            entry.sensor_size = camera.sensor_size.or_else(|| Self::sensor_size(entry.crop_factor).map(str::to_owned));
        }
    }

    fn merge_lens(&mut self, lens: LensEntry) {
        let lens = LensEntry {
            brand: Self::brand_name(&lens.brand),
            ..lens
        };
        if lens.model.is_empty() {
            return;
        }

        let name = if lens.brand.is_empty() || lens.model.starts_with(&lens.brand) {
            lens.model.clone()
        } else {
            format!("{} {}", lens.brand, lens.model)
        };

        for mount in lens.mounts {
            self.mount_lenses.entry(Self::key(&mount)).or_default().insert(name.clone());
        }
    }

    fn rebuild_indexes(&mut self) {
        let mut brands = BTreeSet::new();
        let mut models: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut lenses = self.camera_lenses.clone();

        for ((brand_key, model_key), camera) in &self.cameras {
            brands.insert(camera.brand.clone());
            models.entry(brand_key.clone()).or_default().insert(camera.model.clone());

            let camera_lenses = lenses.entry((brand_key.clone(), model_key.clone())).or_default();
            for mount in camera.mounts.iter().chain(camera.compatible_mounts.iter()) {
                if let Some(mount_lenses) = self.mount_lenses.get(&Self::key(mount)) {
                    camera_lenses.extend(mount_lenses.iter().cloned());
                }
            }
        }

        self.brands = brands.into_iter().collect();
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

        let brand_key = Self::key(brand);
        let model_key = Self::key(model);
        self.cameras.iter()
            .find(|((candidate_brand, _), camera)| {
                (candidate_brand == &brand_key || camera.brand_aliases.iter().any(|alias| Self::key(alias) == brand_key)) &&
                (Self::key(&camera.model) == model_key || camera.aliases.iter().any(|alias| Self::key(alias) == model_key))
            })
            .map(|(key, camera)| (key.clone(), camera))
    }

    fn camera_search_keys(camera: &CameraEntry) -> BTreeSet<(String, String)> {
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
        match (&selected.sensor_size, &candidate.sensor_size) {
            (Some(a), Some(b)) if Self::key(a) == Self::key(b) => true,
            (Some(_), Some(_)) => false,
            _ => match (selected.crop_factor, candidate.crop_factor) {
                (Some(a), Some(b)) => (a - b).abs() <= 0.15,
                _ => true,
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

    fn sensor_size(crop_factor: Option<f64>) -> Option<&'static str> {
        let crop = crop_factor?;
        let size = if crop <= 1.1 {
            "Full frame"
        } else if crop <= 1.35 {
            "APS-H"
        } else if crop <= 1.7 {
            "APS-C"
        } else if crop <= 2.2 {
            "Micro Four Thirds"
        } else if crop <= 3.0 {
            "1-inch"
        } else if crop <= 4.8 {
            "1/1.7-inch"
        } else if crop <= 6.2 {
            "1/2.3-inch"
        } else {
            "Small sensor"
        };
        Some(size)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_database_loads() {
        let db = CameraDatabase::from_static();
        assert!(db.brands().len() > 10);
        assert!(db.models.values().any(|models| models.len() > 10));
        assert!(db.lenses.values().any(|lenses| lenses.len() > 10));
    }

    #[test]
    fn model_indexes_are_scoped_to_their_brand() {
        let db = CameraDatabase::from_static();
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
        let db = CameraDatabase::from_static();
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
        let mut db = CameraDatabase::empty();
        db.merge_camera(CameraEntry {
            brand: "Brand".to_owned(),
            model: "Primary".to_owned(),
            aliases: vec!["Primary Alias".to_owned()],
            mounts: vec!["Mount A".to_owned()],
            sensor_size: Some("Full frame".to_owned()),
            crop_factor: Some(1.0),
            ..Default::default()
        });
        db.merge_camera(CameraEntry {
            brand: "Brand".to_owned(),
            model: "Primary Alias".to_owned(),
            source: vec!["profiles".to_owned()],
            ..Default::default()
        });
        db.merge_camera(CameraEntry {
            brand: "Brand".to_owned(),
            model: "Same Mount".to_owned(),
            mounts: vec!["Mount A".to_owned()],
            sensor_size: Some("Full frame".to_owned()),
            crop_factor: Some(1.02),
            ..Default::default()
        });
        db.merge_camera(CameraEntry {
            brand: "Brand".to_owned(),
            model: "Different Sensor".to_owned(),
            mounts: vec!["Mount A".to_owned()],
            sensor_size: Some("APS-C".to_owned()),
            crop_factor: Some(1.5),
            ..Default::default()
        });
        db.merge_camera(CameraEntry {
            brand: "Brand".to_owned(),
            model: "Different Mount".to_owned(),
            mounts: vec!["Mount B".to_owned()],
            sensor_size: Some("Full frame".to_owned()),
            crop_factor: Some(1.0),
            ..Default::default()
        });
        db.merge_camera(CameraEntry {
            brand: "Brand".to_owned(),
            model: "Adapter Only".to_owned(),
            compatible_mounts: vec!["Mount A".to_owned()],
            sensor_size: Some("Full frame".to_owned()),
            crop_factor: Some(1.0),
            ..Default::default()
        });
        db.rebuild_indexes();

        assert_eq!(db.compatible_camera_names("Brand", "Primary"), vec!["Brand Same Mount"]);
        assert!(db.selected_camera_keys("Brand", "Primary").contains(&CameraDatabase::camera_key("Brand", "Primary Alias")));
        assert!(!db.models("Brand").contains(&"Primary Alias".to_owned()));
    }
}
