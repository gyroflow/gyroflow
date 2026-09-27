// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2021-2022 Adrian <adrian.eddy at gmail>

use std::path::PathBuf;

pub const CANONICAL_LENSES_FILENAME: &str = "canonical_lenses.json";

#[cfg(any(
    target_os = "android",
    target_os = "ios",
    feature = "bundle-lens-profiles"
))]
static CANONICAL_LENSES_STATIC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/camera_presets/canonical_lenses.json"
));

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CanonicalCameraDatabase {
    pub schema_version: u32,
    pub version: u32,
    pub sources: Vec<CanonicalSource>,
    pub mounts: Vec<CanonicalMount>,
    pub brands: Vec<CanonicalBrand>,
    pub setups: Vec<CanonicalSetup>,

    #[serde(skip)]
    pub loaded: bool,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CanonicalSource {
    pub id: String,
    pub name: String,
    pub commit: String,
    pub database_path: Option<String>,
    pub file_count: Option<usize>,
    pub profile_count: Option<usize>,
    pub skipped_profile_count: Option<usize>,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CanonicalMount {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub compatible_mount_ids: Vec<String>,
    pub sources: Vec<String>,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CanonicalBrand {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub sources: Vec<String>,
    pub cameras: Vec<CanonicalCamera>,
    pub lenses: Vec<CanonicalLens>,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CanonicalCamera {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub sources: Vec<String>,
    pub source_keys: Vec<String>,
    pub mount_ids: Vec<String>,
    pub crop_factor: Option<f64>,
    pub sensor: Option<CanonicalSensor>,
    pub profile_count: usize,
    pub observed_lens_ids: Vec<String>,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CanonicalSensor {
    pub name: String,
    pub crop_factor: f64,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CanonicalLens {
    pub id: String,
    pub name: String,
    pub maker: String,
    pub aliases: Vec<String>,
    pub sources: Vec<String>,
    pub source_keys: Vec<String>,
    pub mount_ids: Vec<String>,
    pub crop_factor: Option<f64>,
    pub lens_type: Option<String>,
    pub profile_count: usize,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CanonicalSetup {
    pub camera_id: String,
    pub lens_id: String,
    pub sources: Vec<String>,
    pub profile_count: usize,
    pub profile_keys: Vec<String>,
}

impl CanonicalCameraDatabase {
    pub fn get_path() -> PathBuf {
        crate::lens_profile_database::LensProfileDatabase::get_path()
    }

    pub fn user_path() -> PathBuf {
        crate::settings::data_dir()
            .join("lens_profiles")
            .join(CANONICAL_LENSES_FILENAME)
    }

    pub fn bundled_path() -> PathBuf {
        Self::get_path().join(CANONICAL_LENSES_FILENAME)
    }

    pub fn load_all(&mut self) {
        let _time = std::time::Instant::now();

        let user_path = Self::user_path();
        let loaded = if user_path.exists() {
            ::log::info!(
                "Loading canonical camera database from {}",
                user_path.display()
            );
            self.load_from_file(&user_path).is_ok()
        } else {
            let bundled_path = Self::bundled_path();
            if bundled_path.exists() {
                ::log::info!(
                    "Loading canonical camera database from {}",
                    bundled_path.display()
                );
                self.load_from_file(&bundled_path).is_ok()
            } else {
                false
            }
        };

        #[cfg(any(
            target_os = "android",
            target_os = "ios",
            feature = "bundle-lens-profiles"
        ))]
        let loaded = if loaded {
            true
        } else {
            self.load_from_str(CANONICAL_LENSES_STATIC).is_ok()
        };

        if loaded {
            ::log::info!(
                "Loaded canonical camera database: {} brands, {} mounts, {} setups in {:.3}ms",
                self.brands.len(),
                self.mounts.len(),
                self.setups.len(),
                _time.elapsed().as_micros() as f64 / 1000.0
            );
        } else {
            ::log::warn!("Canonical camera database not found.");
        }
        self.loaded = loaded;
    }

    pub fn load_from_file(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        let data = std::fs::read_to_string(path)?;
        self.load_from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    pub fn load_from_str(&mut self, data: &str) -> Result<(), serde_json::Error> {
        let mut parsed: Self = serde_json::from_str(data)?;
        parsed.loaded = true;
        *self = parsed;
        Ok(())
    }

    pub fn set_from_db(&mut self, db: Self) {
        *self = db;
    }

    pub fn find_brand(&self, id_or_name: &str) -> Option<&CanonicalBrand> {
        let needle = id_or_name.to_ascii_lowercase();
        self.brands.iter().find(|brand| {
            brand.id == needle
                || brand.name.eq_ignore_ascii_case(id_or_name)
                || brand
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(id_or_name))
        })
    }

    pub fn find_camera(&self, id_or_name: &str) -> Option<&CanonicalCamera> {
        let needle = id_or_name.to_ascii_lowercase();
        self.brands
            .iter()
            .flat_map(|brand| brand.cameras.iter())
            .find(|camera| {
                camera.id == needle
                    || camera.name.eq_ignore_ascii_case(id_or_name)
                    || camera
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(id_or_name))
            })
    }

    pub fn find_lens(&self, id_or_name: &str) -> Option<&CanonicalLens> {
        let needle = id_or_name.to_ascii_lowercase();
        self.brands
            .iter()
            .flat_map(|brand| brand.lenses.iter())
            .find(|lens| {
                lens.id == needle
                    || lens.name.eq_ignore_ascii_case(id_or_name)
                    || lens
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(id_or_name))
            })
    }

    pub fn setups_for_camera<'a>(
        &'a self,
        camera_id: &'a str,
    ) -> impl Iterator<Item = &'a CanonicalSetup> + 'a {
        self.setups
            .iter()
            .filter(move |setup| setup.camera_id == camera_id)
    }

    pub fn setups_for_lens<'a>(
        &'a self,
        lens_id: &'a str,
    ) -> impl Iterator<Item = &'a CanonicalSetup> + 'a {
        self.setups
            .iter()
            .filter(move |setup| setup.lens_id == lens_id)
    }

    pub fn camera_count(&self) -> usize {
        self.brands.iter().map(|brand| brand.cameras.len()).sum()
    }

    pub fn lens_count(&self) -> usize {
        self.brands.iter().map(|brand| brand.lenses.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_canonical_database_loads() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../resources/camera_presets/canonical_lenses.json");
        let mut db = CanonicalCameraDatabase::default();
        db.load_from_file(&path).unwrap();

        assert_eq!(db.schema_version, 1);
        assert!(db.find_brand("Sony").is_some());
        assert!(db.find_brand("Canon").is_some());
        assert!(db.find_brand("Nikon").is_some());
        assert!(db.find_brand("Panasonic").is_some());
        assert!(db.find_brand("DJI").is_some());
        assert!(db.camera_count() > 0);
        assert!(db.lens_count() > 0);
        assert!(!db.mounts.is_empty());
        assert!(!db.setups.is_empty());

        for setup in &db.setups {
            assert!(
                db.find_camera(&setup.camera_id).is_some(),
                "missing setup camera {}",
                setup.camera_id
            );
            assert!(
                db.find_lens(&setup.lens_id).is_some(),
                "missing setup lens {}",
                setup.lens_id
            );
        }
    }

    #[test]
    fn canonical_database_lookup_helpers_find_profiles() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../resources/camera_presets/canonical_lenses.json");
        let mut db = CanonicalCameraDatabase::default();
        db.load_from_file(&path).unwrap();

        let sony = db.find_brand("Sony").unwrap();
        let camera = sony
            .cameras
            .iter()
            .find(|camera| !camera.observed_lens_ids.is_empty())
            .unwrap();
        assert!(db.find_camera(&camera.id).is_some());
        assert!(db.setups_for_camera(&camera.id).count() > 0);

        let lens_id = &camera.observed_lens_ids[0];
        assert!(db.find_lens(lens_id).is_some());
        assert!(db.setups_for_lens(lens_id).count() > 0);
    }
}
