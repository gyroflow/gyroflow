use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrandInfo {
    pub name: String,
    pub models: Vec<String>,
    pub lenses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CameraDatabase {
    pub version: String,
    pub updated_at: String,
    pub brands: Vec<BrandInfo>,
}

impl CameraDatabase {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn load_default() -> Self {
        let json_bytes = include_bytes!("../../resources/camera_database.json");
        if let Ok(str_val) = std::str::from_utf8(json_bytes) {
            if let Ok(db) = Self::from_json(str_val) {
                return db;
            }
        }
        Self::default()
    }

    pub fn get_camera_brands(&self) -> Vec<String> {
        let mut brands: Vec<String> = self.brands.iter().map(|b| b.name.clone()).collect();
        brands.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        brands.dedup();
        brands
    }

    pub fn get_camera_models(&self, brand: &str) -> Vec<String> {
        let brand_lc = brand.trim().to_lowercase();
        let mut models = HashSet::new();
        for b in &self.brands {
            if brand_lc.is_empty() || b.name.to_lowercase() == brand_lc {
                for m in &b.models {
                    models.insert(m.clone());
                }
            }
        }
        let mut list: Vec<String> = models.into_iter().collect();
        list.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        list
    }

    pub fn get_lens_models(&self, brand: &str, _model: &str) -> Vec<String> {
        let brand_lc = brand.trim().to_lowercase();
        let mut lenses = HashSet::new();
        for b in &self.brands {
            if brand_lc.is_empty() || b.name.to_lowercase() == brand_lc {
                for l in &b.lenses {
                    lenses.insert(l.clone());
                }
            }
        }
        let mut list: Vec<String> = lenses.into_iter().collect();
        list.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_database_loading() {
        let db = CameraDatabase::load_default();
        assert!(!db.brands.is_empty());

        let brands = db.get_camera_brands();
        assert!(brands.contains(&"Sony".to_string()));
        assert!(brands.contains(&"GoPro".to_string()));

        let sony_models = db.get_camera_models("Sony");
        assert!(sony_models.contains(&"A7IV".to_string()));

        let sony_lenses = db.get_lens_models("Sony", "A7IV");
        assert!(sony_lenses.contains(&"FE 24mm F1.4 GM".to_string()));
    }
}
