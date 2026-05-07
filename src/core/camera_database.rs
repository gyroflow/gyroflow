// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow contributors

//! Loader and lookup helpers for `camera_database.json`.
//!
//! The JSON file lives under `resources/camera_database/camera_database.json`
//! and lists known camera brands / models with optional mount, crop factor and
//! sensor size. It is built from the gyroflow lens profile collection and the
//! LensFun database (see `resources/camera_database/build.py`).
//!
//! At runtime the file is loaded from disk next to the executable. If a newer
//! copy has been downloaded into the user's data directory it takes precedence,
//! mirroring the lens profile auto-update flow in `lens_profile_database`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Camera {
    pub brand: String,
    pub model: String,
    /// Lens mount label (free-form, may be empty when unknown).
    pub mount: String,
    /// 35mm crop factor. `None` when unknown.
    pub crop_factor: Option<f64>,
    /// `[width_mm, height_mm]` of the sensor. `None` when unknown.
    /// Currently derived from `crop_factor` assuming a 3:2 still-camera sensor.
    pub sensor_size_mm: Option<[f64; 2]>,
    /// Which upstream sources the entry came from (e.g. `"gyroflow"`, `"lensfun"`).
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraDatabaseFile {
    pub version: u32,
    #[serde(default)]
    pub sources: Vec<String>,
    pub cameras: Vec<Camera>,
}

#[derive(Debug, Clone, Default)]
pub struct CameraDatabase {
    pub version: u32,
    cameras: Vec<Camera>,
}

impl CameraDatabase {
    /// Locate the `camera_database.json` shipped with the application.
    ///
    /// Mirrors `LensProfileDatabase::get_path()` candidate ordering so that
    /// installed builds, source builds and the macOS bundle layout all work.
    pub fn get_path() -> PathBuf {
        let candidates = [
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            PathBuf::from("../Resources/camera_database/camera_database.json"),
            PathBuf::from("./resources/camera_database/camera_database.json"),
            PathBuf::from("./camera_database/camera_database.json"),
        ];
        let exe = std::env::current_exe().unwrap_or_default();
        let exe_parent = exe.parent();
        for path in &candidates {
            if let Ok(p) = std::fs::canonicalize(path) {
                if p.exists() {
                    return p;
                }
            }
            if let Ok(p) = std::fs::canonicalize(
                exe_parent.map(|x| x.join(path)).unwrap_or_default(),
            ) {
                if p.exists() {
                    return p;
                }
            }
        }
        std::fs::canonicalize(&candidates[0]).unwrap_or_default()
    }

    /// User-writable override path (used by the auto-update flow).
    pub fn user_path() -> PathBuf {
        crate::settings::data_dir()
            .join("camera_database")
            .join("camera_database.json")
    }

    /// Load and parse the database. Tries the user-writable override first,
    /// then falls back to the bundled copy.
    pub fn load() -> Self {
        for candidate in [Self::user_path(), Self::get_path()] {
            if !candidate.exists() {
                continue;
            }
            match std::fs::read(&candidate) {
                Ok(bytes) => match Self::parse(&bytes) {
                    Ok(db) => {
                        log::info!(
                            "Loaded camera database v{} ({} cameras) from {}",
                            db.version,
                            db.cameras.len(),
                            candidate.display()
                        );
                        return db;
                    }
                    Err(e) => log::error!(
                        "Error parsing camera database at {}: {e}",
                        candidate.display()
                    ),
                },
                Err(e) => log::error!(
                    "Error reading camera database at {}: {e}",
                    candidate.display()
                ),
            }
        }
        log::warn!("camera_database.json not found, using empty database");
        Self::default()
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let file: CameraDatabaseFile = serde_json::from_slice(bytes)?;
        let mut cameras = file.cameras;
        cameras.sort_by(|a, b| {
            a.brand
                .to_ascii_lowercase()
                .cmp(&b.brand.to_ascii_lowercase())
                .then_with(|| {
                    a.model
                        .to_ascii_lowercase()
                        .cmp(&b.model.to_ascii_lowercase())
                })
        });
        Ok(Self {
            version: file.version,
            cameras,
        })
    }

    pub fn len(&self) -> usize {
        self.cameras.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cameras.is_empty()
    }

    pub fn cameras(&self) -> &[Camera] {
        &self.cameras
    }

    /// Sorted list of unique brand names.
    pub fn brands(&self) -> Vec<String> {
        let mut set = std::collections::BTreeSet::new();
        for c in &self.cameras {
            if !c.brand.is_empty() {
                set.insert(c.brand.clone());
            }
        }
        set.into_iter().collect()
    }

    /// Sorted list of unique models for the given brand (case-insensitive match).
    pub fn models_for_brand(&self, brand: &str) -> Vec<String> {
        let key = brand.to_ascii_lowercase();
        let mut set = std::collections::BTreeSet::new();
        for c in &self.cameras {
            if c.brand.to_ascii_lowercase() == key && !c.model.is_empty() {
                set.insert(c.model.clone());
            }
        }
        set.into_iter().collect()
    }

    /// Lookup a single camera by (brand, model). Case-insensitive.
    pub fn find(&self, brand: &str, model: &str) -> Option<&Camera> {
        let b = brand.to_ascii_lowercase();
        let m = model.to_ascii_lowercase();
        self.cameras.iter().find(|c| {
            c.brand.to_ascii_lowercase() == b && c.model.to_ascii_lowercase() == m
        })
    }

    /// Multi-word, case-insensitive substring search across brand+model.
    /// Used to power UI selectors and combobox typeahead.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Camera> {
        let query = query.to_ascii_lowercase();
        let words: Vec<&str> = query.split_whitespace().filter(|s| !s.is_empty()).collect();
        if words.is_empty() {
            return self.cameras.iter().take(limit).collect();
        }
        self.cameras
            .iter()
            .filter(|c| {
                let haystack = format!(
                    "{} {} {}",
                    c.brand.to_ascii_lowercase(),
                    c.model.to_ascii_lowercase(),
                    c.mount.to_ascii_lowercase()
                );
                words.iter().all(|w| haystack.contains(w))
            })
            .take(limit)
            .collect()
    }

    /// Cameras grouped by brand. Useful for UI population.
    pub fn grouped_by_brand(&self) -> BTreeMap<String, Vec<&Camera>> {
        let mut map: BTreeMap<String, Vec<&Camera>> = BTreeMap::new();
        for c in &self.cameras {
            map.entry(c.brand.clone()).or_default().push(c);
        }
        map
    }

    /// URL the application should fetch updated copies of `camera_database.json`
    /// from. Mirrors the lens_profile auto-update endpoint.
    pub const REMOTE_URL: &'static str =
        "https://github.com/gyroflow/lens_profiles/releases/latest/download/camera_database.json";

    /// Validate a payload (e.g. just downloaded from `REMOTE_URL`) and, if the
    /// remote version is newer than the one we currently have loaded, write it
    /// to the user-writable copy. Returns `true` on a successful update.
    ///
    /// This is split from any HTTP-fetching code so that core stays free of
    /// network dependencies; the GUI controller drives the actual download.
    pub fn store_update(payload: &[u8], current_version: u32) -> bool {
        let parsed: CameraDatabaseFile = match serde_json::from_slice(payload) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("camera_database: remote payload not valid JSON: {e}");
                return false;
            }
        };

        if parsed.version <= current_version {
            log::info!(
                "camera_database: remote v{} not newer than local v{}, skipping",
                parsed.version,
                current_version
            );
            return false;
        }

        let dst = Self::user_path();
        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::error!("camera_database: cannot create {}: {e}", parent.display());
                return false;
            }
        }
        if let Err(e) = std::fs::write(&dst, payload) {
            log::error!("camera_database: cannot write {}: {e}", dst.display());
            return false;
        }

        log::info!(
            "camera_database: updated v{} -> v{} ({} bytes written to {})",
            current_version,
            parsed.version,
            payload.len(),
            dst.display()
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "version": 1,
        "sources": ["gyroflow_lens_profiles", "lensfun"],
        "cameras": [
            {"brand": "Sony", "model": "ILCE-7M4", "mount": "Sony E", "crop_factor": 1.0, "sensor_size_mm": [35.6, 23.8], "sources": ["lensfun"]},
            {"brand": "Sony", "model": "FX3", "mount": "Sony E", "crop_factor": 1.0, "sensor_size_mm": null, "sources": ["gyroflow", "lensfun"]},
            {"brand": "GoPro", "model": "HERO11 Black", "mount": "goProHero11bl", "crop_factor": 5.54, "sensor_size_mm": [6.5, 4.3], "sources": ["gyroflow"]},
            {"brand": "Canon", "model": "EOS R5", "mount": "Canon RF", "crop_factor": 1.0, "sensor_size_mm": null, "sources": ["lensfun"]}
        ]
    }"#;

    #[test]
    fn parses_sample() {
        let db = CameraDatabase::parse(SAMPLE.as_bytes()).expect("parse");
        assert_eq!(db.version, 1);
        assert_eq!(db.len(), 4);
    }

    #[test]
    fn brands_are_sorted_and_unique() {
        let db = CameraDatabase::parse(SAMPLE.as_bytes()).unwrap();
        let brands = db.brands();
        assert_eq!(brands, vec!["Canon", "GoPro", "Sony"]);
    }

    #[test]
    fn models_for_brand_case_insensitive() {
        let db = CameraDatabase::parse(SAMPLE.as_bytes()).unwrap();
        let models = db.models_for_brand("sony");
        assert_eq!(models, vec!["FX3", "ILCE-7M4"]);
    }

    #[test]
    fn find_round_trips() {
        let db = CameraDatabase::parse(SAMPLE.as_bytes()).unwrap();
        let cam = db.find("GoPro", "HERO11 Black").expect("found");
        assert_eq!(cam.mount, "goProHero11bl");
        assert!((cam.crop_factor.unwrap() - 5.54).abs() < 1e-6);
        assert!(db.find("nope", "nope").is_none());
    }

    #[test]
    fn search_matches_multi_word_substrings() {
        let db = CameraDatabase::parse(SAMPLE.as_bytes()).unwrap();
        let hits = db.search("sony fx", 100);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].model, "FX3");

        let hits = db.search("hero11", 100);
        assert_eq!(hits.len(), 1);

        let empty = db.search("nonexistent xyz", 100);
        assert!(empty.is_empty());
    }

    #[test]
    fn search_limit_is_respected() {
        let db = CameraDatabase::parse(SAMPLE.as_bytes()).unwrap();
        let hits = db.search("", 2);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn grouped_by_brand_works() {
        let db = CameraDatabase::parse(SAMPLE.as_bytes()).unwrap();
        let grouped = db.grouped_by_brand();
        assert_eq!(grouped.get("Sony").map(|v| v.len()), Some(2));
        assert_eq!(grouped.get("GoPro").map(|v| v.len()), Some(1));
    }

    #[test]
    fn store_update_rejects_older_versions() {
        let payload = br#"{"version": 1, "cameras": []}"#;
        assert!(
            !CameraDatabase::store_update(payload, 5),
            "remote older than local should not update"
        );
        assert!(
            !CameraDatabase::store_update(payload, 1),
            "equal version should not update"
        );
    }

    #[test]
    fn store_update_rejects_invalid_payload() {
        let payload = b"not json";
        assert!(!CameraDatabase::store_update(payload, 0));
    }

    #[test]
    fn missing_optional_fields_default_to_none() {
        let json = r#"{
            "version": 1,
            "cameras": [
                {"brand": "X", "model": "Y"}
            ]
        }"#;
        let db = CameraDatabase::parse(json.as_bytes()).unwrap();
        assert_eq!(db.len(), 1);
        let cam = &db.cameras()[0];
        assert!(cam.crop_factor.is_none());
        assert!(cam.sensor_size_mm.is_none());
        assert_eq!(cam.mount, "");
        assert!(cam.sources.is_empty());
    }
}
