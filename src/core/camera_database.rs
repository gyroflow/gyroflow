// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

use std::collections::HashMap;

use serde::{ Serialize, Deserialize };

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct CameraDatabase {
    pub version: u32,
    pub brands: Vec<CameraBrand>,
    pub lenses: Vec<LensInfo>,
    pub mounts: Vec<MountInfo>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct CameraBrand {
    pub name: String,
    pub models: Vec<CameraModel>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct CameraModel {
    pub name: String,
    pub lens_profiles_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop_factor: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensor_width_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensor_height_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mount: Option<String>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct LensInfo {
    pub name: String,
    pub brands: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focal_length_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focal_length_max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop_factor: Option<f64>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct MountInfo {
    pub name: String,
    pub compatible_mounts: Vec<String>,
}

impl CameraDatabase {
    pub fn load_from_data(data: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(data)
    }

    pub fn load_bundled() -> Self {
        let data = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/camera_database.json"));
        Self::load_from_data(data).unwrap_or_default()
    }

    pub fn brand_names(&self) -> Vec<&str> {
        self.brands.iter().map(|b| b.name.as_str()).collect()
    }

    pub fn models_for_brand(&self, brand: &str) -> Vec<&str> {
        self.brands.iter()
            .find(|b| b.name.eq_ignore_ascii_case(brand))
            .map(|b| b.models.iter().map(|m| m.name.as_str()).collect())
            .unwrap_or_default()
    }

    pub fn lenses_for_brand(&self, brand: &str) -> Vec<&str> {
        self.lenses.iter()
            .filter(|l| l.brands.iter().any(|b| b.eq_ignore_ascii_case(brand)))
            .map(|l| l.name.as_str())
            .collect()
    }

    pub fn search_brands(&self, query: &str) -> Vec<&CameraBrand> {
        let query_lower = query.to_ascii_lowercase();
        self.brands.iter()
            .filter(|b| b.name.to_ascii_lowercase().contains(&query_lower))
            .collect()
    }

    pub fn search_models(&self, query: &str) -> Vec<(&CameraBrand, &CameraModel)> {
        let query_lower = query.to_ascii_lowercase();
        let mut results = Vec::new();
        for brand in &self.brands {
            for model in &brand.models {
                if model.name.to_ascii_lowercase().contains(&query_lower)
                    || brand.name.to_ascii_lowercase().contains(&query_lower)
                {
                    results.push((brand, model));
                }
            }
        }
        results
    }

    pub fn search_lenses(&self, query: &str) -> Vec<&LensInfo> {
        let query_lower = query.to_ascii_lowercase();
        self.lenses.iter()
            .filter(|l| l.name.to_ascii_lowercase().contains(&query_lower))
            .collect()
    }

    /// Build a lookup map from brand+model to crop factor.
    pub fn crop_factor_map(&self) -> HashMap<(String, String), f64> {
        let mut map = HashMap::new();
        for brand in &self.brands {
            for model in &brand.models {
                if let Some(cf) = model.crop_factor {
                    map.insert((brand.name.clone(), model.name.clone()), cf);
                }
            }
        }
        map
    }
}
