// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Gyroflow contributors

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use log::{info, warn, error};
use once_cell::sync::Lazy;

const REMOTE_CAMERA_DB_URL: &str = "https://raw.githubusercontent.com/gyroflow/lens_profiles/main/camera_database.json";
const LOCAL_DB_FILENAME: &str = "camera_database.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountInfo {
    pub flange_distance_mm: f64,
    #[serde(default)]
    pub full_frame: bool,
    #[serde(default)]
    pub fixed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CameraInfo {
    pub brand: String,
    pub model: String,
    pub mount: String,
    pub sensor_type: String,
    pub crop_factor: f64,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensInfo {
    pub brand: String,
    pub model: String,
    pub mount: String,
    pub sensor_format: String,
    pub focal_min: f64,
    pub focal_max: f64,
    pub is_zoom: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CameraDatabase {
    pub version: u32,
    pub mounts: HashMap<String, MountInfo>,
    pub cameras: Vec<CameraInfo>,
    pub lenses: Vec<LensInfo>,
}

pub static CAMERA_DB: Lazy<RwLock<CameraDatabase>> = Lazy::new(|| {
    RwLock::new(CameraDatabase::load_default())
});

impl CameraDatabase {
    pub fn load_default() -> Self {
        // 1. Try user directory / app data cached file
        if let Some(user_db) = Self::load_from_user_data() {
            return user_db;
        }

        // 2. Try embedded resource bundled in binary or local resources/ folder
        if let Ok(content) = fs::to_string(Path::new("resources").join(LOCAL_DB_FILENAME)) {
            if let Ok(db) = serde_json::from_str::<CameraDatabase>(&content) {
                return db;
            }
        }

        // 3. Fallback to compile-time resource
        let embedded = include_str!("../../../resources/camera_database.json");
        serde_json::from_str(embedded).unwrap_or_default()
    }

    fn user_cache_path() -> Option<PathBuf> {
        dirs::data_dir().map(|p| p.join("gyroflow").join(LOCAL_DB_FILENAME))
    }

    fn load_from_user_data() -> Option<Self> {
        let path = Self::user_cache_path()?;
        if path.exists() {
            let data = fs::to_string(path).ok()?;
            serde_json::from_str(&data).ok()
        } else {
            None
        }
    }

    pub fn update_from_remote() {
        std::thread::spawn(|| {
            info!("Checking for camera database updates from {}", REMOTE_CAMERA_DB_URL);
            match ureq::get(REMOTE_CAMERA_DB_URL).timeout(std::time::Duration::from_secs(10)).call() {
                Ok(response) => {
                    if let Ok(text) = response.into_string() {
                        if let Ok(new_db) = serde_json::from_str::<CameraDatabase>(&text) {
                            let mut current = CAMERA_DB.write().unwrap();
                            if new_db.version > current.version {
                                info!("Updating camera database to version {}", new_db.version);
                                if let Some(cache_path) = CameraDatabase::user_cache_path() {
                                    if let Some(parent) = cache_path.parent() {
                                        let _ = fs::create_dir_all(parent);
                                    }
                                    let _ = fs::write(&cache_path, &text);
                                }
                                *current = new_db;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch remote camera database: {}", e);
                }
            }
        });
    }

    pub fn get_brands(&self) -> Vec<String> {
        let mut brands: Vec<String> = self.cameras.iter().map(|c| c.brand.clone()).collect();
        brands.sort();
        brands.dedup();
        brands
    }

    pub fn get_models_for_brand(&self, brand: &str) -> Vec<String> {
        let mut models: Vec<String> = self.cameras
            .iter()
            .filter(|c| c.brand.eq_ignore_ascii_case(brand))
            .map(|c| c.model.clone())
            .collect();
        models.sort();
        models.dedup();
        models
    }

    pub fn find_camera(&self, brand: &str, model: &str) -> Option<&CameraInfo> {
        self.cameras.iter().find(|c| {
            c.brand.eq_ignore_ascii_case(brand) && (c.model.eq_ignore_ascii_case(model)
                || c.aliases.iter().any(|a| a.eq_ignore_ascii_case(model)))
        })
    }

    pub fn match_camera_by_name(&self, query: &str) -> Option<&CameraInfo> {
        let q_lower = query.to_lowercase();
        self.cameras.iter().find(|c| {
            let full_name = format!("{} {}", c.brand, c.model).to_lowercase();
            if full_name == q_lower || c.model.to_lowercase() == q_lower {
                return true;
            }
            c.aliases.iter().any(|a| a.to_lowercase() == q_lower)
        })
    }

    pub fn get_compatible_cameras(&self, camera: &CameraInfo) -> Vec<&CameraInfo> {
        let mount = &camera.mount;
        let crop = camera.crop_factor;

        self.cameras
            .iter()
            .filter(|c| {
                if &c.mount != mount {
                    return false;
                }
                // Compatible if same mount and compatible sensor area (crop factor <= camera or same)
                (c.crop_factor - crop).abs() < 0.05 || c.crop_factor <= crop
            })
            .collect()
    }

    pub fn get_lenses_for_camera(&self, camera: &CameraInfo) -> Vec<&LensInfo> {
        self.lenses
            .iter()
            .filter(|l| l.mount.eq_ignore_ascii_case(&camera.mount))
            .collect()
    }
}
