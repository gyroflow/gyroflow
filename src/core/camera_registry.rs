// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::{LensProfile, lens_profile_database::LensProfileDatabase};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CameraRegistryFile {
    pub version: u32,
    pub cameras: Vec<CameraRegistryEntry>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CameraRegistryEntry {
    pub brand: String,
    pub model: String,
    pub lenses: Vec<String>,
    pub crop_factor: Option<f64>,
    pub sensor_size: Option<String>,
    pub source: Vec<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct CameraRegistryInfo {
    pub brand: String,
    pub model: String,
    pub lenses: Vec<String>,
    pub crop_factor: Option<f64>,
    pub sensor_size: Option<String>,
    pub source: Vec<String>,
    pub known_lens_count: usize,
}

#[derive(Clone, Debug, Default)]
struct CameraEntry {
    brand: String,
    model: String,
    lenses: BTreeSet<String>,
    crop_factor: Option<f64>,
    sensor_size: Option<String>,
    source: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CameraRegistry {
    cameras: BTreeMap<(String, String), CameraEntry>,
    brands: Vec<String>,
    models: BTreeMap<String, Vec<String>>,
    lenses: BTreeMap<(String, String), Vec<String>>,
}

impl CameraRegistry {
    pub fn from_lens_profile_database(profiles: &LensProfileDatabase) -> Self {
        Self::from_lens_profile_database_with_catalogs(profiles, true)
    }

    fn from_lens_profile_database_with_catalogs(
        profiles: &LensProfileDatabase,
        load_catalogs: bool,
    ) -> Self {
        let mut registry = Self::default();
        if load_catalogs {
            registry.merge_catalog_path(
                &crate::settings::data_dir()
                    .join("lens_profiles")
                    .join("camera_registry.json"),
            );
            registry.merge_catalog_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../resources/camera_registry.json"
            )));
        }

        for profile in profiles.iter_profiles() {
            registry.merge_profile(profile);
        }

        registry.rebuild_indexes();
        registry
    }

    pub fn brands(&self) -> Vec<String> {
        self.brands.clone()
    }

    pub fn models(&self, brand: &str) -> Vec<String> {
        self.models
            .get(&Self::key(brand))
            .cloned()
            .unwrap_or_default()
    }

    pub fn lenses(&self, brand: &str, model: &str) -> Vec<String> {
        self.lenses
            .get(&Self::camera_key(brand, model))
            .cloned()
            .unwrap_or_default()
    }

    pub fn compatible_models(&self, brand: &str, model: &str) -> Vec<String> {
        let Some(selected) = self.find_camera(brand, model) else {
            return Vec::new();
        };

        let selected_brand = Self::key(&selected.brand);
        self.cameras
            .values()
            .filter(|candidate| {
                Self::key(&candidate.brand) == selected_brand
                    && Self::camera_key(&candidate.brand, &candidate.model)
                        != Self::camera_key(&selected.brand, &selected.model)
                    && Self::sensor_compatible(selected, candidate)
            })
            .map(|candidate| format!("{} {}", candidate.brand, candidate.model))
            .collect()
    }

    pub fn selected_camera_keys(&self, brand: &str, model: &str) -> BTreeSet<(String, String)> {
        self.find_camera(brand, model)
            .map(|camera| BTreeSet::from([Self::camera_key(&camera.brand, &camera.model)]))
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
        let Some(selected) = self.find_camera(brand, model) else {
            return BTreeSet::new();
        };

        let selected_brand = Self::key(&selected.brand);
        self.cameras
            .values()
            .filter(|candidate| {
                Self::key(&candidate.brand) == selected_brand
                    && Self::camera_key(&candidate.brand, &candidate.model)
                        != Self::camera_key(&selected.brand, &selected.model)
                    && Self::sensor_compatible(selected, candidate)
            })
            .map(|camera| Self::camera_key(&camera.brand, &camera.model))
            .collect()
    }

    pub fn resolve_camera(&self, brand: &str, model: &str) -> Option<(String, String)> {
        if let Some(camera) = self.find_camera(brand, model) {
            return Some((camera.brand.clone(), camera.model.clone()));
        }

        let brand = self.resolve_brand(brand)?;
        Some((brand, Self::model_name(model)))
    }

    pub fn resolve_model(&self, brand: &str, model: &str) -> Option<String> {
        self.find_camera(brand, model)
            .map(|camera| camera.model.clone())
    }

    pub fn camera_info(&self, brand: &str, model: &str) -> Option<CameraRegistryInfo> {
        self.find_camera(brand, model)
            .map(|camera| CameraRegistryInfo {
                brand: camera.brand.clone(),
                model: camera.model.clone(),
                lenses: camera.lenses.iter().cloned().collect(),
                crop_factor: camera.crop_factor,
                sensor_size: camera.sensor_size.clone(),
                source: camera.source.iter().cloned().collect(),
                known_lens_count: camera.lenses.len(),
            })
    }

    pub fn to_catalog(&self) -> CameraRegistryFile {
        CameraRegistryFile {
            version: 1,
            cameras: self
                .cameras
                .values()
                .map(|camera| CameraRegistryEntry {
                    brand: camera.brand.clone(),
                    model: camera.model.clone(),
                    lenses: camera.lenses.iter().cloned().collect(),
                    crop_factor: camera.crop_factor,
                    sensor_size: camera.sensor_size.clone(),
                    source: camera.source.iter().cloned().collect(),
                })
                .collect(),
        }
    }

    fn merge_catalog_path(&mut self, path: &Path) {
        if let Ok(data) = std::fs::read_to_string(path) {
            self.merge_catalog_str(&data);
        }
    }

    fn merge_catalog_str(&mut self, data: &str) {
        if let Ok(file) = serde_json::from_str::<CameraRegistryFile>(data) {
            for camera in file.cameras {
                self.merge_catalog_camera(camera);
            }
        }
    }

    fn merge_catalog_camera(&mut self, camera: CameraRegistryEntry) {
        if camera.brand.trim().is_empty() || camera.model.trim().is_empty() {
            return;
        }

        let key = Self::camera_key(&camera.brand, &camera.model);
        let entry = self.cameras.entry(key).or_insert_with(|| CameraEntry {
            brand: Self::brand_name(&camera.brand),
            model: Self::model_name(&camera.model),
            ..Default::default()
        });

        entry.lenses.extend(
            camera
                .lenses
                .into_iter()
                .filter(|lens| !lens.trim().is_empty())
                .map(|lens| lens.trim().to_owned()),
        );
        if entry.crop_factor.is_none() {
            entry.crop_factor = camera.crop_factor;
        }
        if entry.sensor_size.is_none() {
            entry.sensor_size = camera
                .sensor_size
                .or_else(|| Self::sensor_size(entry.crop_factor).map(str::to_owned));
        }
        entry.source.extend(
            camera
                .source
                .into_iter()
                .filter(|source| !source.trim().is_empty()),
        );
    }

    fn merge_profile(&mut self, profile: &LensProfile) {
        if profile.is_copy
            || profile.camera_brand.trim().is_empty()
            || profile.camera_model.trim().is_empty()
        {
            return;
        }

        let key = Self::camera_key(&profile.camera_brand, &profile.camera_model);
        let entry = self.cameras.entry(key).or_insert_with(|| CameraEntry {
            brand: Self::brand_name(&profile.camera_brand),
            model: Self::model_name(&profile.camera_model),
            ..Default::default()
        });

        if !profile.lens_model.trim().is_empty() {
            entry.lenses.insert(profile.lens_model.trim().to_owned());
        }
        if entry.crop_factor.is_none() {
            entry.crop_factor = profile.crop_factor;
        }
        if entry.sensor_size.is_none() {
            entry.sensor_size = Self::sensor_size(entry.crop_factor).map(str::to_owned);
        }
        entry.source.insert("gyroflow".to_owned());
    }

    fn rebuild_indexes(&mut self) {
        let mut brands = BTreeSet::new();
        let mut models: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut lenses: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();

        for ((brand_key, model_key), camera) in &self.cameras {
            brands.insert(camera.brand.clone());
            models
                .entry(brand_key.clone())
                .or_default()
                .insert(camera.model.clone());
            lenses
                .entry((brand_key.clone(), model_key.clone()))
                .or_default()
                .extend(camera.lenses.iter().cloned());
        }

        self.brands = brands.into_iter().collect();
        self.models = models
            .into_iter()
            .map(|(brand, models)| (brand, models.into_iter().collect()))
            .collect();
        self.lenses = lenses
            .into_iter()
            .map(|(camera, lenses)| (camera, lenses.into_iter().collect()))
            .collect();
    }

    fn find_camera(&self, brand: &str, model: &str) -> Option<&CameraEntry> {
        self.cameras.get(&Self::camera_key(brand, model))
    }

    fn resolve_brand(&self, brand: &str) -> Option<String> {
        let brand_key = Self::key(brand);
        if brand_key.is_empty() {
            return None;
        }
        self.brands
            .iter()
            .find(|candidate| Self::key(candidate) == brand_key)
            .cloned()
            .or_else(|| {
                let name = Self::brand_name(brand);
                self.brands
                    .iter()
                    .find(|candidate| Self::key(candidate) == Self::key(&name))
                    .cloned()
            })
    }

    fn sensor_compatible(selected: &CameraEntry, candidate: &CameraEntry) -> bool {
        match (&selected.sensor_size, &candidate.sensor_size) {
            (Some(a), Some(b)) => Self::key(a) == Self::key(b),
            _ => match (selected.crop_factor, candidate.crop_factor) {
                (Some(a), Some(b)) => (a - b).abs() <= 0.15,
                _ => false,
            },
        }
    }

    pub fn camera_key(brand: &str, model: &str) -> (String, String) {
        (
            Self::key(&Self::brand_name(brand)),
            Self::key(&Self::model_name(model)),
        )
    }

    pub fn key(value: &str) -> String {
        value
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn brand_name(value: &str) -> String {
        match Self::key(value).as_str() {
            "akaso" => "AKASO".to_owned(),
            "arri" => "ARRI".to_owned(),
            "blackmagic design" => "Blackmagic".to_owned(),
            "casio computer co ltd" => "Casio".to_owned(),
            "dji" => "DJI".to_owned(),
            "fujifilm" | "fufifilm" => "Fujifilm".to_owned(),
            "gopro" | "go pro" => "GoPro".to_owned(),
            "lg mobile" | "lge" => "LG".to_owned(),
            "nikon corporation" => "Nikon".to_owned(),
            "olympus corporation" | "olympus imaging corp" | "olympus optical co ltd" => {
                "Olympus".to_owned()
            }
            "panasonic corporation" => "Panasonic".to_owned(),
            "pentax corporation" | "asahi optical co ltd" => "Pentax".to_owned(),
            "red digital cinema" => "RED".to_owned(),
            "runcam" => "RunCam".to_owned(),
            "sjcam" => "SJCam".to_owned(),
            "xiaomi communications co ltd" | "mi" => "Xiaomi".to_owned(),
            _ => value.split_whitespace().collect::<Vec<_>>().join(" "),
        }
    }

    fn model_name(value: &str) -> String {
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace("CInema", "Cinema")
            .replace("4k", "4K")
            .replace("6k", "6K")
    }

    fn sensor_size(crop_factor: Option<f64>) -> Option<&'static str> {
        let crop = crop_factor?;
        let size = if crop <= 1.1 {
            "Full frame"
        } else if crop <= 1.7 {
            "APS-C"
        } else if crop <= 2.2 {
            "Micro Four Thirds"
        } else if crop <= 3.0 {
            "1-inch"
        } else if crop <= 6.2 {
            "1/2.3-inch"
        } else {
            "Small sensor"
        };
        Some(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(brand: &str, model: &str, lens: &str, crop: Option<f64>) -> LensProfile {
        let mut profile = LensProfile::default();
        profile.camera_brand = brand.to_owned();
        profile.camera_model = model.to_owned();
        profile.lens_model = lens.to_owned();
        profile.crop_factor = crop;
        profile
    }

    fn registry_for_test(db: &LensProfileDatabase) -> CameraRegistry {
        CameraRegistry::from_lens_profile_database_with_catalogs(db, false)
    }

    #[test]
    fn builds_sorted_camera_and_lens_selectors() {
        let mut db = LensProfileDatabase::default();
        db.insert_for_test(
            "sony-a7iv.json",
            profile("Sony", "A7 IV", "Sony FE 24mm", Some(1.0)),
        );
        db.insert_for_test(
            "sony-a7iv-2.json",
            profile("Sony", "A7 IV", "Sony FE 24mm", Some(1.0)),
        );
        db.insert_for_test("gopro.json", profile("GoPro", "HERO12 Black", "Wide", None));

        let registry = registry_for_test(&db);

        assert_eq!(registry.brands(), vec!["GoPro", "Sony"]);
        assert_eq!(registry.models("Sony"), vec!["A7 IV"]);
        assert_eq!(registry.lenses("Sony", "A7 IV"), vec!["Sony FE 24mm"]);
    }

    #[test]
    fn canonicalizes_common_metadata_brand_names() {
        let mut db = LensProfileDatabase::default();
        db.insert_for_test(
            "olympus.json",
            profile("Olympus Imaging Corp.", "E-M1", "12mm", Some(2.0)),
        );

        let registry = registry_for_test(&db);

        assert_eq!(
            registry.resolve_camera("Olympus Imaging Corp.", "E-M1"),
            Some(("Olympus".to_owned(), "E-M1".to_owned()))
        );
        assert_eq!(registry.models("Olympus"), vec!["E-M1"]);
    }

    #[test]
    fn compatible_models_stay_in_same_brand_and_sensor_class() {
        let mut db = LensProfileDatabase::default();
        db.insert_for_test(
            "sony-a7iv.json",
            profile("Sony", "A7 IV", "24mm", Some(1.0)),
        );
        db.insert_for_test("sony-fx3.json", profile("Sony", "FX3", "35mm", Some(1.01)));
        db.insert_for_test(
            "sony-a6700.json",
            profile("Sony", "A6700", "35mm", Some(1.5)),
        );
        db.insert_for_test("canon-r5.json", profile("Canon", "R5", "35mm", Some(1.0)));

        let registry = registry_for_test(&db);

        assert_eq!(
            registry.compatible_models("Sony", "A7 IV"),
            vec!["Sony FX3"]
        );
        assert!(
            registry
                .compatible_camera_keys("Sony", "A7 IV")
                .contains(&CameraRegistry::camera_key("Sony", "FX3"))
        );
    }

    #[test]
    fn exports_registry_catalog_in_stable_order() {
        let mut db = LensProfileDatabase::default();
        db.insert_for_test(
            "sony-a7iv.json",
            profile("Sony", "A7 IV", "Sony FE 24mm", Some(1.0)),
        );

        let catalog = registry_for_test(&db).to_catalog();

        assert_eq!(catalog.version, 1);
        assert_eq!(catalog.cameras.len(), 1);
        assert_eq!(catalog.cameras[0].brand, "Sony");
        assert_eq!(catalog.cameras[0].model, "A7 IV");
        assert_eq!(catalog.cameras[0].lenses, vec!["Sony FE 24mm"]);
        assert_eq!(catalog.cameras[0].sensor_size.as_deref(), Some("Full frame"));
        assert_eq!(catalog.cameras[0].source, vec!["gyroflow"]);
    }
}
