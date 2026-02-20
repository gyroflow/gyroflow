// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::{HashMap, HashSet, BTreeMap};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use crate::LensProfile;
use crate::lens_profile_database::LensProfileDatabase;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct CameraMetadata {
    pub brand: String,
    pub model: String,
    pub mount: Option<String>,
    pub sensor_width: Option<f64>,  // in mm
    pub sensor_height: Option<f64>, // in mm
    pub crop_factor: Option<f64>,
    pub full_frame: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct LensMetadata {
    pub brand: String,
    pub model: String,
    pub mount: Option<String>,
    pub min_focal_length: Option<f64>, // in mm
    pub max_focal_length: Option<f64>, // in mm
    pub is_zoom: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct CameraCompatibility {
    pub cameras: Vec<CameraMetadata>,
    pub lens_mounts: Vec<String>,
    pub sensor_sizes: Vec<SensorSize>,
    pub compatibility: Vec<CompatibilityRule>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct SensorSize {
    pub name: String,
    pub width: f64,  // in mm
    pub height: f64, // in mm
    pub crop_factor: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct CompatibilityRule {
    pub camera_brand: Option<String>,
    pub camera_model: Option<String>,
    pub mount: Option<String>,
    pub compatible_cameras: Vec<String>, // Camera model names or patterns
    pub compatible_mounts: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct CameraLensList {
    pub brands: Vec<String>,
    pub cameras: BTreeMap<String, Vec<String>>, // brand -> [models]
    pub lenses: BTreeMap<String, Vec<LensMetadata>>, // brand -> [lenses]
    pub version: u32,
}

#[derive(Default)]
pub struct CameraDatabase {
    compatibility: Option<CameraCompatibility>,
    camera_lens_list: Option<CameraLensList>,
    loaded: bool,
}

impl CameraDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_path() -> PathBuf {
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
            if let Some(exe_parent) = exe_parent {
                if let Ok(path) = std::fs::canonicalize(exe_parent.join(&path)) {
                    if path.exists() {
                        return path;
                    }
                }
            }
        }
        if let Some(exe_parent) = exe_parent {
            if let Ok(path) = std::fs::canonicalize(exe_parent.join("./camera_presets/")) {
                if !path.exists() {
                    let _ = std::fs::create_dir_all(&path);
                }
                return path;
            }
        }
        std::fs::canonicalize(&candidates[0]).unwrap_or_default()
    }

    pub fn load_all(&mut self) {
        let path = Self::get_path();
        log::info!("Camera database directory: {:?}", path);

        // Load compatibility database
        let compat_path = path.join("camera_compatibility.json");
        if compat_path.exists() {
            if let Ok(data) = std::fs::read_to_string(&compat_path) {
                if let Ok(compat) = serde_json::from_str::<CameraCompatibility>(&data) {
                    self.compatibility = Some(compat);
                    log::info!("Loaded camera compatibility database");
                } else {
                    log::warn!("Failed to parse camera_compatibility.json");
                }
            }
        }

        // Load camera/lens list
        let list_path = path.join("camera_lens_list.json");
        if list_path.exists() {
            if let Ok(data) = std::fs::read_to_string(&list_path) {
                if let Ok(mut list) = serde_json::from_str::<CameraLensList>(&data) {
                    // Try to merge LensFun data if available
                    if let Some(lensfun_path) = crate::lensfun_integration::find_lensfun_database() {
                        log::info!("Found LensFun database at: {:?}", lensfun_path);
                        // TODO: Parse LensFun XML and merge (requires XML parsing library)
                        // For now, this is a placeholder for future implementation
                    }
                    self.camera_lens_list = Some(list);
                    log::info!("Loaded camera/lens list database");
                } else {
                    log::warn!("Failed to parse camera_lens_list.json");
                }
            }
        }

        self.loaded = true;
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn get_brands(&self) -> Vec<String> {
        if let Some(ref list) = self.camera_lens_list {
            list.brands.clone()
        } else {
            Vec::new()
        }
    }

    pub fn get_camera_models(&self, brand: &str) -> Vec<String> {
        if let Some(ref list) = self.camera_lens_list {
            list.cameras.get(brand).cloned().unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    pub fn get_lenses(&self, brand: &str) -> Vec<LensMetadata> {
        if let Some(ref list) = self.camera_lens_list {
            list.lenses.get(brand).cloned().unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    pub fn get_compatible_cameras(&self, brand: &str, model: Option<&str>) -> Vec<String> {
        let mut compatible = Vec::new();
        
        if let Some(ref compat) = self.compatibility {
            for rule in &compat.compatibility {
                let matches = match (rule.camera_brand.as_ref(), rule.camera_model.as_ref()) {
                    (Some(b), Some(m)) if b == brand => {
                        model.map(|model| model == m).unwrap_or(true)
                    }
                    (Some(b), None) if b == brand => true,
                    _ => false,
                };

                if matches {
                    compatible.extend_from_slice(&rule.compatible_cameras);
                }
            }
        }

        // Also add cameras from the same brand
        if let Some(ref list) = self.camera_lens_list {
            if let Some(models) = list.cameras.get(brand) {
                compatible.extend(models.clone());
            }
        }

        // Remove duplicates and sort
        let mut unique: HashSet<String> = compatible.into_iter().collect();
        let mut result: Vec<String> = unique.into_iter().collect();
        result.sort();
        result
    }

    pub fn get_camera_metadata(&self, brand: &str, model: &str) -> Option<CameraMetadata> {
        if let Some(ref compat) = self.compatibility {
            compat.cameras.iter()
                .find(|c| c.brand == brand && c.model == model)
                .cloned()
        } else {
            None
        }
    }

    pub fn get_lens_metadata(&self, brand: &str, model: &str) -> Option<LensMetadata> {
        if let Some(ref list) = self.camera_lens_list {
            list.lenses.get(brand)
                .and_then(|lenses| lenses.iter().find(|l| l.model == model).cloned())
        } else {
            None
        }
    }

    pub fn search_cameras(&self, query: &str) -> Vec<(String, String)> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        if let Some(ref list) = self.camera_lens_list {
            for (brand, models) in &list.cameras {
                if brand.to_lowercase().contains(&query_lower) {
                    for model in models {
                        results.push((brand.clone(), model.clone()));
                    }
                } else {
                    for model in models {
                        if model.to_lowercase().contains(&query_lower) {
                            results.push((brand.clone(), model.clone()));
                        }
                    }
                }
            }
        }

        results
    }

    pub fn search_lenses(&self, query: &str) -> Vec<LensMetadata> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        if let Some(ref list) = self.camera_lens_list {
            for (brand, lenses) in &list.lenses {
                for lens in lenses {
                    if brand.to_lowercase().contains(&query_lower) ||
                       lens.model.to_lowercase().contains(&query_lower) {
                        results.push(lens.clone());
                    }
                }
            }
        }

        results
    }

    /// Extract camera and lens data from existing lens profile database
    pub fn extract_from_lens_profiles(db: &LensProfileDatabase) -> (CameraLensList, CameraCompatibility) {
        let mut brands = HashSet::new();
        let mut cameras: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut lenses: BTreeMap<String, Vec<LensMetadata>> = BTreeMap::new();
        let mut camera_metadata_list = Vec::new();
        let mut mounts = HashSet::new();

        // Extract from all lens profiles
        for profile in db.iter_profiles() {
            if profile.is_copy {
                continue; // Skip copies
            }

            let brand = profile.camera_brand.trim();
            let model = profile.camera_model.trim();
            let lens_model = profile.lens_model.trim();

            if !brand.is_empty() && !model.is_empty() {
                brands.insert(brand.to_string());
                
                let brand_entry = cameras.entry(brand.to_string()).or_insert_with(Vec::new);
                if !brand_entry.contains(&model.to_string()) {
                    brand_entry.push(model.to_string());
                }

                // Create camera metadata
                let camera_meta = CameraMetadata {
                    brand: brand.to_string(),
                    model: model.to_string(),
                    mount: None, // Will be filled from compatibility data
                    sensor_width: None,
                    sensor_height: None,
                    crop_factor: profile.crop_factor,
                    full_frame: profile.crop_factor.map(|cf| (cf - 1.0).abs() < 0.1).unwrap_or(false),
                };
                
                // Check if we already have this camera
                if !camera_metadata_list.iter().any(|c: &CameraMetadata| c.brand == brand && c.model == model) {
                    camera_metadata_list.push(camera_meta);
                }
            }

            if !brand.is_empty() && !lens_model.is_empty() {
                let brand_entry = lenses.entry(brand.to_string()).or_insert_with(Vec::new);
                
                // Check if lens already exists
                if !brand_entry.iter().any(|l| l.model == lens_model) {
                    let is_zoom = lens_model.to_lowercase().contains("zoom") ||
                                  lens_model.to_lowercase().contains("-") ||
                                  profile.focal_length.is_some();
                    
                    let lens_meta = LensMetadata {
                        brand: brand.to_string(),
                        model: lens_model.to_string(),
                        mount: None,
                        min_focal_length: profile.focal_length,
                        max_focal_length: profile.focal_length,
                        is_zoom,
                    };
                    brand_entry.push(lens_meta);
                }
            }
        }

        // Sort camera models and lenses
        for models in cameras.values_mut() {
            models.sort();
        }
        for lens_list in lenses.values_mut() {
            lens_list.sort_by(|a, b| a.model.cmp(&b.model));
        }

        let mut brand_vec: Vec<String> = brands.into_iter().collect();
        brand_vec.sort();

        let camera_lens_list = CameraLensList {
            brands: brand_vec,
            cameras,
            lenses,
            version: 1,
        };

        // Create basic compatibility structure
        let compatibility = CameraCompatibility {
            cameras: camera_metadata_list,
            lens_mounts: mounts.into_iter().collect(),
            sensor_sizes: vec![
                SensorSize { name: "Full Frame".to_string(), width: 36.0, height: 24.0, crop_factor: 1.0 },
                SensorSize { name: "APS-C".to_string(), width: 23.6, height: 15.7, crop_factor: 1.5 },
                SensorSize { name: "Micro Four Thirds".to_string(), width: 17.3, height: 13.0, crop_factor: 2.0 },
                SensorSize { name: "1 inch".to_string(), width: 13.2, height: 8.8, crop_factor: 2.7 },
            ],
            compatibility: Vec::new(),
        };

        (camera_lens_list, compatibility)
    }

    /// Save extracted data to JSON files
    pub fn save_extracted_data(list: &CameraLensList, compat: &CameraCompatibility) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::get_path();
        
        let list_path = path.join("camera_lens_list.json");
        let list_json = serde_json::to_string_pretty(list)?;
        std::fs::write(&list_path, list_json)?;
        log::info!("Saved camera/lens list to {:?}", list_path);

        let compat_path = path.join("camera_compatibility.json");
        let compat_json = serde_json::to_string_pretty(compat)?;
        std::fs::write(&compat_path, compat_json)?;
        log::info!("Saved camera compatibility to {:?}", compat_path);

        Ok(())
    }
}
