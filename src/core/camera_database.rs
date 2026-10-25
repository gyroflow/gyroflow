// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

use std::collections::{HashMap, HashSet, BTreeMap, BTreeSet};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct MountInfo {
    pub id: String,
    pub name: String,
    pub flange_distance: Option<f64>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct CameraInfo {
    pub brand: String,
    pub model: String,
    pub mount: Option<String>,
    pub sensor_type: Option<String>,
    pub crop_factor: Option<f64>,
    pub sensor_width: Option<f64>,
    pub sensor_height: Option<f64>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct LensInfo {
    pub brand: String,
    pub model: String,
    pub mount: Option<String>,
    pub sensor_coverage: Option<String>,
    pub crop_factor: Option<f64>,
    pub min_focal: Option<f64>,
    pub max_focal: Option<f64>,
    pub aspect_ratio: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct CameraDatabaseRaw {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub mounts: Vec<MountInfo>,
    #[serde(default)]
    pub brand_aliases: HashMap<String, String>,
    #[serde(default)]
    pub model_aliases: HashMap<String, String>,
    #[serde(default)]
    pub cameras: Vec<CameraInfo>,
    #[serde(default)]
    pub lenses: Vec<LensInfo>,
}

fn default_version() -> u32 { 1 }

#[derive(Clone, Default, Debug)]
pub struct CameraDatabase {
    pub version: u32,
    pub mounts: Vec<MountInfo>,
    pub cameras: Vec<CameraInfo>,
    pub lenses: Vec<LensInfo>,
    pub raw_model_aliases: HashMap<String, String>,

    // Indices for O(1) and O(log N) lookups
    brand_aliases: HashMap<String, String>,
    model_aliases: HashMap<(String, String), String>, // (brand_lower, model_alias_lower) -> canonical_model
    cameras_by_brand: BTreeMap<String, Vec<usize>>,
    camera_map: HashMap<(String, String), usize>, // (brand_lower, model_lower) -> index
    lenses_by_mount: HashMap<String, Vec<usize>>, // mount_lower -> lens indices
    lenses_by_brand: BTreeMap<String, Vec<usize>>,
    mount_compat: HashMap<String, HashSet<String>>, // mount_lower -> set of compatible lens mounts (lower)
}

impl CameraDatabase {
    pub fn new() -> Self {
        let mut db = Self::default();
        db.init_mount_compatibility();
        db
    }

    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        let raw: CameraDatabaseRaw = serde_json::from_str(json_str)?;
        let mut db = Self {
            version: raw.version,
            mounts: raw.mounts,
            cameras: raw.cameras,
            lenses: raw.lenses,
            brand_aliases: raw.brand_aliases,
            ..Default::default()
        };
        db.rebuild_indices();
        Ok(db)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let raw = CameraDatabaseRaw {
            version: self.version,
            mounts: self.mounts.clone(),
            brand_aliases: self.brand_aliases.clone(),
            model_aliases: self.raw_model_aliases.clone(),
            cameras: self.cameras.clone(),
            lenses: self.lenses.clone(),
        };
        serde_json::to_string_pretty(&raw)
    }

    pub fn rebuild_indices(&mut self) {
        self.init_mount_compatibility();

        self.cameras_by_brand.clear();
        self.camera_map.clear();
        self.model_aliases.clear();
        self.lenses_by_mount.clear();
        self.lenses_by_brand.clear();

        // Index cameras
        for (k, v) in &self.raw_model_aliases {
            let parts: Vec<&str> = k.splitn(2, ':').collect();
            if parts.len() == 2 {
                self.model_aliases.insert((parts[0].to_ascii_lowercase(), parts[1].to_ascii_lowercase()), v.clone());
            }
        }

        for (i, cam) in self.cameras.iter().enumerate() {
            let b_lower = cam.brand.to_ascii_lowercase();
            let m_lower = cam.model.to_ascii_lowercase();
            self.camera_map.insert((b_lower.clone(), m_lower.clone()), i);
            self.cameras_by_brand.entry(cam.brand.clone()).or_default().push(i);

            for alias in &cam.aliases {
                self.model_aliases.insert((b_lower.clone(), alias.to_ascii_lowercase()), cam.model.clone());
            }
        }

        // Index lenses
        for (i, lens) in self.lenses.iter().enumerate() {
            self.lenses_by_brand.entry(lens.brand.clone()).or_default().push(i);
            if let Some(mount) = &lens.mount {
                let m_lower = mount.to_ascii_lowercase();
                self.lenses_by_mount.entry(m_lower).or_default().push(i);
            }
        }
    }

    fn init_mount_compatibility(&mut self) {
        let mut compat = HashMap::new();

        let add_compat = |map: &mut HashMap<String, HashSet<String>>, cam_m: &str, lens_ms: &[&str]| {
            let set = map.entry(cam_m.to_ascii_lowercase()).or_insert_with(HashSet::new);
            set.insert(cam_m.to_ascii_lowercase());
            for &lm in lens_ms {
                set.insert(lm.to_ascii_lowercase());
            }
        };

        // Sony E mount cameras accept Sony E, Sony FE, and with adapter A-mount
        add_compat(&mut compat, "Sony E", &["Sony FE", "Sony E-mount", "E-mount", "FE-mount", "Minolta A", "Sony A"]);
        add_compat(&mut compat, "Sony FE", &["Sony E", "Sony FE", "Sony E-mount", "FE-mount"]);

        // Canon RF mount cameras accept Canon RF and Canon EF (via adapter)
        add_compat(&mut compat, "Canon RF", &["Canon RF", "RF-mount", "Canon EF", "EF-mount", "Canon EF-S"]);
        add_compat(&mut compat, "Canon EF", &["Canon EF", "EF-mount", "Canon EF-S"]);
        add_compat(&mut compat, "Canon EF-M", &["Canon EF-M", "EF-M", "Canon EF"]);

        // Nikon Z mount cameras accept Nikon Z and Nikon F (via FTZ adapter)
        add_compat(&mut compat, "Nikon Z", &["Nikon Z", "Z-mount", "Nikon F", "F-mount"]);
        add_compat(&mut compat, "Nikon F", &["Nikon F", "F-mount"]);

        // Micro Four Thirds standard
        add_compat(&mut compat, "Micro Four Thirds", &["Micro Four Thirds", "MFT", "M4/3", "Micro 4/3", "m43", "Four Thirds"]);
        add_compat(&mut compat, "MFT", &["Micro Four Thirds", "MFT", "M4/3", "Micro 4/3", "Four Thirds"]);

        // Fujifilm X and G mounts
        add_compat(&mut compat, "Fujifilm X", &["Fujifilm X", "Fuji X", "X-mount", "FX"]);
        add_compat(&mut compat, "Fujifilm G", &["Fujifilm G", "G-mount", "GFX"]);

        // Leica L-mount alliance (Leica, Panasonic, Sigma, DJI)
        add_compat(&mut compat, "Leica L", &["Leica L", "L-mount", "L Mount"]);
        add_compat(&mut compat, "Leica M", &["Leica M", "M-mount", "M Mount"]);

        // ARRI PL
        add_compat(&mut compat, "ARRI PL", &["ARRI PL", "PL", "PL-mount"]);
        add_compat(&mut compat, "PL", &["ARRI PL", "PL", "PL-mount"]);

        // M12 / M8 action camera lenses
        add_compat(&mut compat, "M12", &["M12", "M12x0.5", "S-mount"]);
        add_compat(&mut compat, "M8", &["M8", "M8x0.5"]);

        // Fixed lenses
        add_compat(&mut compat, "Fixed Lens", &["Fixed Lens", "Fixed", "Integrated", "Built-in"]);

        self.mount_compat = compat;
    }

    /// Add or update a camera in the database
    pub fn add_camera(&mut self, camera: CameraInfo) {
        let b_lower = camera.brand.to_ascii_lowercase();
        let m_lower = camera.model.to_ascii_lowercase();

        if let Some(&idx) = self.camera_map.get(&(b_lower.clone(), m_lower.clone())) {
            self.cameras[idx] = camera;
        } else {
            let idx = self.cameras.len();
            self.cameras.push(camera.clone());
            self.camera_map.insert((b_lower.clone(), m_lower.clone()), idx);
            self.cameras_by_brand.entry(camera.brand.clone()).or_default().push(idx);

            for alias in &camera.aliases {
                self.model_aliases.insert((b_lower.clone(), alias.to_ascii_lowercase()), camera.model.clone());
            }
        }
    }

    /// Add or update a lens in the database
    pub fn add_lens(&mut self, lens: LensInfo) {
        let idx = self.lenses.len();
        self.lenses_by_brand.entry(lens.brand.clone()).or_default().push(idx);
        if let Some(mount) = &lens.mount {
            let m_lower = mount.to_ascii_lowercase();
            self.lenses_by_mount.entry(m_lower).or_default().push(idx);
        }
        self.lenses.push(lens);
    }

    /// Add a brand alias
    pub fn add_brand_alias(&mut self, alias: &str, canonical: &str) {
        self.brand_aliases.insert(alias.to_ascii_lowercase(), canonical.to_string());
    }

    /// Add a model alias
    pub fn add_model_alias(&mut self, brand: &str, alias: &str, canonical: &str) {
        let canonical_brand = self.normalize_brand(brand);
        let key = format!("{}:{}", canonical_brand.to_ascii_lowercase(), alias.to_ascii_lowercase());
        self.raw_model_aliases.insert(key, canonical.to_string());
        self.model_aliases.insert((canonical_brand.to_ascii_lowercase(), alias.to_ascii_lowercase()), canonical.to_string());
    }

    /// Normalize brand name using alias dictionary
    pub fn normalize_brand(&self, brand: &str) -> String {
        let trimmed = brand.trim();
        if trimmed.is_empty() { return String::new(); }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(canonical) = self.brand_aliases.get(&lower) {
            return canonical.clone();
        }
        // Capitalize first letter if not in alias dict
        let mut chars = trimmed.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        }
    }

    /// Normalize model name for a brand
    pub fn normalize_model(&self, brand: &str, model: &str) -> String {
        let norm_brand = self.normalize_brand(brand);
        let b_lower = norm_brand.to_ascii_lowercase();
        let m_trim = model.trim();
        let m_lower = m_trim.to_ascii_lowercase();

        // Check explicit alias
        if let Some(canonical) = self.model_aliases.get(&(b_lower.clone(), m_lower.clone())) {
            return canonical.clone();
        }

        // Check if directly matches a known camera
        if let Some(&idx) = self.camera_map.get(&(b_lower, m_lower)) {
            return self.cameras[idx].model.clone();
        }

        m_trim.to_string()
    }

    /// Normalize camera brand and model together
    pub fn normalize_camera(&self, brand: &str, model: &str) -> (String, String) {
        let b = self.normalize_brand(brand);
        let m = self.normalize_model(&b, model);
        (b, m)
    }

    /// Get all known camera brands
    pub fn get_brands(&self) -> Vec<String> {
        let mut brands: BTreeSet<String> = BTreeSet::new();
        for cam in &self.cameras {
            if !cam.brand.is_empty() {
                brands.insert(cam.brand.clone());
            }
        }
        brands.into_iter().collect()
    }

    /// Get all models for a specific camera brand
    pub fn get_models(&self, brand: &str) -> Vec<String> {
        let norm_brand = self.normalize_brand(brand);
        let b_lower = norm_brand.to_ascii_lowercase();

        let mut models = Vec::new();
        for cam in &self.cameras {
            if cam.brand.to_ascii_lowercase() == b_lower {
                models.push(cam.model.clone());
            }
        }
        models.sort();
        models.dedup();
        models
    }

    /// Look up camera info
    pub fn get_camera(&self, brand: &str, model: &str) -> Option<&CameraInfo> {
        let norm_brand = self.normalize_brand(brand);
        let norm_model = self.normalize_model(&norm_brand, model);
        let b_lower = norm_brand.to_ascii_lowercase();
        let m_lower = norm_model.to_ascii_lowercase();

        self.camera_map.get(&(b_lower, m_lower)).map(|&idx| &self.cameras[idx])
    }

    /// Get all compatible lenses for a given camera brand and model
    pub fn get_lenses_for_camera(&self, brand: &str, model: &str) -> Vec<String> {
        let mut lens_names = BTreeSet::new();

        if let Some(cam) = self.get_camera(brand, model) {
            let compatible_lenses = self.get_compatible_lenses(cam);
            for l in compatible_lenses {
                lens_names.insert(l.model.clone());
            }
        } else {
            // Fallback: return lenses with the same brand
            let norm_brand = self.normalize_brand(brand);
            let b_lower = norm_brand.to_ascii_lowercase();
            for l in &self.lenses {
                if l.brand.to_ascii_lowercase() == b_lower {
                    lens_names.insert(l.model.clone());
                }
            }
        }

        lens_names.into_iter().collect()
    }

    /// Get compatible lenses for a camera based on mount compatibility
    pub fn get_compatible_lenses(&self, camera: &CameraInfo) -> Vec<&LensInfo> {
        let mut result = Vec::new();

        if let Some(cam_mount) = &camera.mount {
            let m_lower = cam_mount.to_ascii_lowercase();
            let empty_set = HashSet::new();
            let compat_mounts = self.mount_compat.get(&m_lower).unwrap_or(&empty_set);

            for lens in &self.lenses {
                if let Some(l_mount) = &lens.mount {
                    let lm_lower = l_mount.to_ascii_lowercase();
                    if lm_lower == m_lower || compat_mounts.contains(&lm_lower) {
                        result.push(lens);
                    }
                }
            }
        }

        result
    }

    /// Search cameras by free-form text
    pub fn find_cameras(&self, search: &str) -> Vec<&CameraInfo> {
        let query = search.to_ascii_lowercase();
        let words: Vec<&str> = query.split_ascii_whitespace().collect();
        if words.is_empty() { return Vec::new(); }

        self.cameras.iter().filter(|cam| {
            let brand = cam.brand.to_ascii_lowercase();
            let model = cam.model.to_ascii_lowercase();
            let aliases: String = cam.aliases.join(" ").to_ascii_lowercase();
            words.iter().all(|w| brand.contains(w) || model.contains(w) || aliases.contains(w))
        }).collect()
    }

    /// Search lenses by free-form text
    pub fn find_lenses(&self, search: &str) -> Vec<&LensInfo> {
        let query = search.to_ascii_lowercase();
        let words: Vec<&str> = query.split_ascii_whitespace().collect();
        if words.is_empty() { return Vec::new(); }

        self.lenses.iter().filter(|lens| {
            let brand = lens.brand.to_ascii_lowercase();
            let model = lens.model.to_ascii_lowercase();
            let mount = lens.mount.as_deref().unwrap_or("").to_ascii_lowercase();
            let aliases: String = lens.aliases.join(" ").to_ascii_lowercase();
            words.iter().all(|w| brand.contains(w) || model.contains(w) || mount.contains(w) || aliases.contains(w))
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_database_basic() {
        let mut db = CameraDatabase::new();
        db.add_brand_alias("sony", "Sony");
        db.add_brand_alias("bmpcc", "Blackmagic");

        assert_eq!(db.normalize_brand("sony"), "Sony");
        assert_eq!(db.normalize_brand("bmpcc"), "Blackmagic");
        assert_eq!(db.normalize_brand("Nikon"), "Nikon");

        let cam = CameraInfo {
            brand: "Sony".into(),
            model: "a7 III".into(),
            mount: Some("Sony E".into()),
            crop_factor: Some(1.0),
            aliases: vec!["ILCE-7M3".into(), "a73".into()],
            ..Default::default()
        };
        db.add_camera(cam);

        let lens = LensInfo {
            brand: "Sony".into(),
            model: "FE 24-70mm F2.8 GM".into(),
            mount: Some("Sony E".into()),
            min_focal: Some(24.0),
            max_focal: Some(70.0),
            crop_factor: Some(1.0),
            ..Default::default()
        };
        db.add_lens(lens);
        db.rebuild_indices();

        assert_eq!(db.normalize_model("Sony", "ILCE-7M3"), "a7 III");
        assert_eq!(db.normalize_model("Sony", "a73"), "a7 III");

        let models = db.get_models("Sony");
        assert_eq!(models, vec!["a7 III"]);

        let lenses = db.get_lenses_for_camera("Sony", "a7 III");
        assert_eq!(lenses, vec!["FE 24-70mm F2.8 GM"]);

        let found_lenses = db.find_lenses("24-70");
        assert_eq!(found_lenses.len(), 1);
    }

    #[test]
    fn test_camera_database_mount_compatibility() {
        let mut db = CameraDatabase::new();
        let cam_mft = CameraInfo {
            brand: "Panasonic".into(),
            model: "GH5".into(),
            mount: Some("Micro Four Thirds".into()),
            crop_factor: Some(2.0),
            ..Default::default()
        };
        let lens_mft = LensInfo {
            brand: "Panasonic".into(),
            model: "Lumix G 12-35mm F2.8".into(),
            mount: Some("MFT".into()),
            ..Default::default()
        };
        let lens_sony = LensInfo {
            brand: "Sony".into(),
            model: "FE 24-70mm F2.8 GM".into(),
            mount: Some("Sony E".into()),
            ..Default::default()
        };
        db.add_camera(cam_mft.clone());
        db.add_lens(lens_mft);
        db.add_lens(lens_sony);
        db.rebuild_indices();

        let compat = db.get_compatible_lenses(&cam_mft);
        assert_eq!(compat.len(), 1);
        assert_eq!(compat[0].model, "Lumix G 12-35mm F2.8");
    }
}
