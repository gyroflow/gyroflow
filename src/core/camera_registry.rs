// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, BTreeMap};
use std::io::Read;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CameraRegistry {
    pub version: u32,
    pub brands: Vec<CameraBrand>,
    pub mounts: Vec<MountInfo>,
    #[serde(default)]
    pub lensfun_lenses: Vec<LensfunLens>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CameraBrand {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub cameras: Vec<CameraModel>,
    #[serde(default)]
    pub lenses: Vec<LensModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CameraModel {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub mount: String,
    #[serde(default)]
    pub sensor_width_mm: Option<f64>,
    #[serde(default)]
    pub sensor_height_mm: Option<f64>,
    #[serde(default)]
    pub crop_factor: Option<f64>,
    #[serde(default)]
    pub fixed_lens: bool,
    #[serde(default)]
    pub fixed_lens_name: Option<String>,
    #[serde(default)]
    pub from_lensfun: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LensModel {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub mounts: Vec<String>,
    #[serde(default)]
    pub min_focal_mm: Option<f64>,
    #[serde(default)]
    pub max_focal_mm: Option<f64>,
    #[serde(default)]
    pub is_zoom: bool,
    #[serde(default)]
    pub from_lensfun: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LensfunLens {
    pub maker: String,
    pub model: String,
    #[serde(default)]
    pub mounts: Vec<String>,
    #[serde(default)]
    pub min_focal: Option<f64>,
    #[serde(default)]
    pub max_focal: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MountInfo {
    pub name: String,
    #[serde(default)]
    pub compatible_mounts: Vec<String>,
    #[serde(default)]
    pub flange_distance_mm: Option<f64>,
}

impl CameraRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn load_from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        Ok(Self::load_from_json(&content)?)
    }

    pub fn load_from_cbor_gz(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let mut decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(data));
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;
        Ok(ciborium::from_reader(std::io::Cursor::new(decompressed))?)
    }

    pub fn get_brands(&self) -> Vec<&str> {
        self.brands.iter().map(|b| b.name.as_str()).collect()
    }

    pub fn get_brand_by_name(&self, name: &str) -> Option<&CameraBrand> {
        let name_lower = name.to_lowercase();
        self.brands.iter().find(|b| {
            b.name.to_lowercase() == name_lower
                || b.aliases.iter().any(|a| a.to_lowercase() == name_lower)
        })
    }

    pub fn get_cameras_for_brand(&self, brand: &str) -> Vec<&CameraModel> {
        self.get_brand_by_name(brand)
            .map(|b| b.cameras.iter().collect())
            .unwrap_or_default()
    }

    pub fn get_lenses_for_brand(&self, brand: &str) -> Vec<&LensModel> {
        self.get_brand_by_name(brand)
            .map(|b| b.lenses.iter().collect())
            .unwrap_or_default()
    }

    pub fn get_camera(&self, brand: &str, model: &str) -> Option<(&CameraBrand, &CameraModel)> {
        let brand_obj = self.get_brand_by_name(brand)?;
        let model_lower = model.to_lowercase();
        let camera = brand_obj.cameras.iter().find(|c| {
            c.name.to_lowercase() == model_lower
                || c.aliases.iter().any(|a| a.to_lowercase() == model_lower)
        })?;
        Some((brand_obj, camera))
    }

    pub fn get_lens(&self, brand: &str, lens_model: &str) -> Option<(&CameraBrand, &LensModel)> {
        let brand_obj = self.get_brand_by_name(brand)?;
        let lens_lower = lens_model.to_lowercase();
        let lens = brand_obj.lenses.iter().find(|l| {
            l.name.to_lowercase() == lens_lower
                || l.aliases.iter().any(|a| a.to_lowercase() == lens_lower)
        })?;
        Some((brand_obj, lens))
    }

    pub fn get_mount(&self, name: &str) -> Option<&MountInfo> {
        let name_lower = name.to_lowercase();
        self.mounts.iter().find(|m| m.name.to_lowercase() == name_lower)
    }

    pub fn get_compatible_mounts(&self, mount: &str) -> HashSet<String> {
        let mut result = HashSet::new();
        result.insert(mount.to_string());

        if let Some(mount_info) = self.get_mount(mount) {
            for compat in &mount_info.compatible_mounts {
                result.insert(compat.clone());
            }
        }

        for m in &self.mounts {
            if m.compatible_mounts.iter().any(|c| c.to_lowercase() == mount.to_lowercase()) {
                result.insert(m.name.clone());
            }
        }

        result
    }

    pub fn get_compatible_cameras(&self, brand: &str, model: &str) -> Vec<(String, String, f64)> {
        let mut result = Vec::new();

        if let Some((_, camera)) = self.get_camera(brand, model) {
            let camera_mount = &camera.mount;
            let camera_crop = camera.crop_factor.unwrap_or(1.0);

            if camera_mount.is_empty() || camera.fixed_lens {
                return result;
            }

            let compatible_mounts = self.get_compatible_mounts(camera_mount);

            for brand_obj in &self.brands {
                for cam in &brand_obj.cameras {
                    if cam.fixed_lens || cam.mount.is_empty() {
                        continue;
                    }
                    if brand_obj.name == brand && cam.name == model {
                        continue;
                    }

                    let cam_crop = cam.crop_factor.unwrap_or(1.0);
                    let crop_diff = (camera_crop - cam_crop).abs();

                    if compatible_mounts.contains(&cam.mount) && crop_diff < 0.05 {
                        result.push((brand_obj.name.clone(), cam.name.clone(), cam_crop));
                    }
                }
            }
        }

        result
    }

    pub fn get_compatible_lenses(&self, brand: &str, model: &str) -> Vec<(String, String)> {
        let mut result = Vec::new();

        if let Some((_, camera)) = self.get_camera(brand, model) {
            if camera.fixed_lens {
                if let Some(ref lens_name) = camera.fixed_lens_name {
                    result.push((brand.to_string(), lens_name.clone()));
                }
                return result;
            }

            let compatible_mounts = self.get_compatible_mounts(&camera.mount);

            for brand_obj in &self.brands {
                for lens in &brand_obj.lenses {
                    if lens.mounts.iter().any(|m| compatible_mounts.contains(m)) {
                        result.push((brand_obj.name.clone(), lens.name.clone()));
                    }
                }
            }

            for lens in &self.lensfun_lenses {
                if lens.mounts.iter().any(|m| compatible_mounts.contains(m)) {
                    result.push((lens.maker.clone(), lens.model.clone()));
                }
            }
        }

        result
    }

    pub fn is_zoom_lens(&self, brand: &str, lens_model: &str) -> bool {
        if let Some((_, lens)) = self.get_lens(brand, lens_model) {
            return lens.is_zoom;
        }

        for lens in &self.lensfun_lenses {
            if lens.maker.to_lowercase() == brand.to_lowercase()
                && lens.model.to_lowercase() == lens_model.to_lowercase()
            {
                if let (Some(min), Some(max)) = (lens.min_focal, lens.max_focal) {
                    return (max - min).abs() > 0.1;
                }
            }
        }

        lens_model.contains('-') && lens_model.contains("mm")
    }

    pub fn normalize_brand(&self, brand: &str) -> String {
        if let Some(b) = self.get_brand_by_name(brand) {
            return b.name.clone();
        }
        brand.to_string()
    }

    pub fn normalize_model(&self, brand: &str, model: &str) -> String {
        if let Some((_, c)) = self.get_camera(brand, model) {
            return c.name.clone();
        }
        model.to_string()
    }

    pub fn search_brands(&self, query: &str) -> Vec<&str> {
        let query_lower = query.to_lowercase();
        self.brands
            .iter()
            .filter(|b| {
                b.name.to_lowercase().contains(&query_lower)
                    || b.aliases.iter().any(|a| a.to_lowercase().contains(&query_lower))
            })
            .map(|b| b.name.as_str())
            .collect()
    }

    pub fn search_cameras(&self, brand: &str, query: &str) -> Vec<&str> {
        let query_lower = query.to_lowercase();
        self.get_cameras_for_brand(brand)
            .into_iter()
            .filter(|c| {
                c.name.to_lowercase().contains(&query_lower)
                    || c.aliases.iter().any(|a| a.to_lowercase().contains(&query_lower))
            })
            .map(|c| c.name.as_str())
            .collect()
    }

    pub fn search_lenses(&self, brand: &str, query: &str) -> Vec<&str> {
        let query_lower = query.to_lowercase();
        self.get_lenses_for_brand(brand)
            .into_iter()
            .filter(|l| {
                l.name.to_lowercase().contains(&query_lower)
                    || l.aliases.iter().any(|a| a.to_lowercase().contains(&query_lower))
            })
            .map(|l| l.name.as_str())
            .collect()
    }
}

#[derive(Default)]
pub struct CameraRegistryBuilder {
    brands: BTreeMap<String, CameraBrandBuilder>,
    mounts: BTreeMap<String, MountInfo>,
}

#[derive(Default)]
struct CameraBrandBuilder {
    name: String,
    aliases: HashSet<String>,
    cameras: BTreeMap<String, CameraModel>,
    lenses: BTreeMap<String, LensModel>,
}

impl CameraRegistryBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_brand(&mut self, name: &str) -> &mut Self {
        let normalized = Self::normalize_brand_name(name);
        self.brands.entry(normalized.clone()).or_insert_with(|| CameraBrandBuilder {
            name: normalized,
            ..Default::default()
        });
        self
    }

    pub fn add_brand_alias(&mut self, brand: &str, alias: &str) -> &mut Self {
        let normalized = Self::normalize_brand_name(brand);
        if let Some(b) = self.brands.get_mut(&normalized) {
            b.aliases.insert(alias.to_string());
        }
        self
    }

    pub fn add_camera(&mut self, brand: &str, model: &str, mount: &str, crop_factor: Option<f64>) -> &mut Self {
        let normalized_brand = Self::normalize_brand_name(brand);
        self.add_brand(&normalized_brand);

        if let Some(b) = self.brands.get_mut(&normalized_brand) {
            let key = model.to_lowercase();
            b.cameras.entry(key).or_insert_with(|| CameraModel {
                name: model.to_string(),
                mount: mount.to_string(),
                crop_factor,
                ..Default::default()
            });
        }
        self
    }

    pub fn add_camera_alias(&mut self, brand: &str, model: &str, alias: &str) -> &mut Self {
        let normalized_brand = Self::normalize_brand_name(brand);
        if let Some(b) = self.brands.get_mut(&normalized_brand) {
            let key = model.to_lowercase();
            if let Some(c) = b.cameras.get_mut(&key) {
                if !c.aliases.contains(&alias.to_string()) {
                    c.aliases.push(alias.to_string());
                }
            }
        }
        self
    }

    pub fn add_lens(&mut self, brand: &str, name: &str, mounts: &[&str], is_zoom: bool) -> &mut Self {
        let normalized_brand = Self::normalize_brand_name(brand);
        self.add_brand(&normalized_brand);

        if let Some(b) = self.brands.get_mut(&normalized_brand) {
            let key = name.to_lowercase();
            b.lenses.entry(key).or_insert_with(|| LensModel {
                name: name.to_string(),
                mounts: mounts.iter().map(|s| s.to_string()).collect(),
                is_zoom,
                ..Default::default()
            });
        }
        self
    }

    pub fn add_mount(&mut self, name: &str, compatible: &[&str]) -> &mut Self {
        self.mounts.insert(name.to_lowercase(), MountInfo {
            name: name.to_string(),
            compatible_mounts: compatible.iter().map(|s| s.to_string()).collect(),
            flange_distance_mm: None,
        });
        self
    }

    pub fn build(self) -> CameraRegistry {
        CameraRegistry {
            version: 1,
            brands: self.brands.into_values().map(|b| CameraBrand {
                name: b.name,
                aliases: b.aliases.into_iter().collect(),
                cameras: b.cameras.into_values().collect(),
                lenses: b.lenses.into_values().collect(),
            }).collect(),
            mounts: self.mounts.into_values().collect(),
            lensfun_lenses: Vec::new(),
        }
    }

    fn normalize_brand_name(name: &str) -> String {
        match name.to_lowercase().as_str() {
            "gopro" | "go pro" => "GoPro".to_string(),
            "dji" => "DJI".to_string(),
            "sony" | "ilce" | "ilme" => "Sony".to_string(),
            "canon" => "Canon".to_string(),
            "nikon" => "Nikon".to_string(),
            "panasonic" | "lumix" => "Panasonic".to_string(),
            "fujifilm" | "fuji" => "Fujifilm".to_string(),
            "blackmagic" | "blackmagic design" | "bmpcc" => "Blackmagic".to_string(),
            "insta360" => "Insta360".to_string(),
            "red" | "red digital cinema" => "RED".to_string(),
            "apple" => "Apple".to_string(),
            "samsung" => "Samsung".to_string(),
            "olympus" | "om system" | "om-system" => "Olympus".to_string(),
            "leica" => "Leica".to_string(),
            "hasselblad" => "Hasselblad".to_string(),
            "runcam" => "RunCam".to_string(),
            "caddx" => "Caddx".to_string(),
            "arri" => "ARRI".to_string(),
            _ => name.to_string()
        }
    }
}

pub fn generate_from_lens_profiles(profiles: &[(String, String, String, String)]) -> CameraRegistry {
    let mut builder = CameraRegistryBuilder::new();

    builder
        .add_mount("Sony E", &[])
        .add_mount("Sony A", &["Sony E"])
        .add_mount("Canon EF", &[])
        .add_mount("Canon EF-S", &["Canon EF"])
        .add_mount("Canon RF", &["Canon EF", "Canon EF-S"])
        .add_mount("Nikon F", &[])
        .add_mount("Nikon Z", &["Nikon F"])
        .add_mount("Micro Four Thirds", &[])
        .add_mount("Fujifilm X", &[])
        .add_mount("Fujifilm GFX", &[])
        .add_mount("Leica L", &[])
        .add_mount("Leica M", &[]);

    for (brand, model, lens, _setting) in profiles {
        if brand.is_empty() || model.is_empty() {
            continue;
        }

        let normalized_brand = CameraRegistryBuilder::normalize_brand_name(brand);
        builder.add_brand(&normalized_brand);

        let mount = infer_mount(&normalized_brand, model);
        let crop = infer_crop_factor(&normalized_brand, model);

        builder.add_camera(&normalized_brand, model, &mount, crop);

        if !lens.is_empty() {
            let is_zoom = lens.contains('-') && lens.to_lowercase().contains("mm");
            builder.add_lens(&normalized_brand, lens, &[&mount], is_zoom);
        }
    }

    builder.build()
}

fn infer_mount(brand: &str, model: &str) -> String {
    let model_lower = model.to_lowercase();

    match brand {
        "Sony" => {
            if model_lower.contains("a7") || model_lower.contains("a9") || model_lower.starts_with("fx") {
                "Sony E".to_string()
            } else if model_lower.contains("a6") || model_lower.contains("zv-e") {
                "Sony E".to_string()
            } else {
                "Sony E".to_string()
            }
        }
        "Canon" => {
            if model_lower.contains("r3") || model_lower.contains("r5") || model_lower.contains("r6")
                || model_lower.contains("r7") || model_lower.contains("r8") || model_lower.contains("r10")
                || model_lower.starts_with("eos r")
            {
                "Canon RF".to_string()
            } else {
                "Canon EF".to_string()
            }
        }
        "Nikon" => {
            if model_lower.starts_with("z") || model_lower.contains("z5") || model_lower.contains("z6")
                || model_lower.contains("z7") || model_lower.contains("z8") || model_lower.contains("z9")
            {
                "Nikon Z".to_string()
            } else {
                "Nikon F".to_string()
            }
        }
        "Panasonic" => {
            if model_lower.contains("s1") || model_lower.contains("s5") {
                "Leica L".to_string()
            } else {
                "Micro Four Thirds".to_string()
            }
        }
        "Olympus" => "Micro Four Thirds".to_string(),
        "Fujifilm" => {
            if model_lower.contains("gfx") {
                "Fujifilm GFX".to_string()
            } else {
                "Fujifilm X".to_string()
            }
        }
        "GoPro" | "DJI" | "Insta360" | "RunCam" | "Caddx" => String::new(),
        _ => String::new()
    }
}

fn infer_crop_factor(brand: &str, model: &str) -> Option<f64> {
    let model_lower = model.to_lowercase();

    match brand {
        "Sony" => {
            if model_lower.contains("a7") || model_lower.contains("a9")
                || model_lower.starts_with("fx3") || model_lower.starts_with("fx6")
            {
                Some(1.0)
            } else if model_lower.contains("a6") || model_lower.contains("zv-e") {
                Some(1.5)
            } else {
                None
            }
        }
        "Canon" => {
            if model_lower.contains("5d") || model_lower.contains("6d")
                || model_lower.contains("r3") || model_lower.contains("r5") || model_lower.contains("r6")
                || model_lower.contains("r8") || model_lower.starts_with("1d")
            {
                Some(1.0)
            } else if model_lower.contains("7d") || model_lower.contains("r7") || model_lower.contains("r10")
                || model_lower.contains("90d") || model_lower.contains("80d") || model_lower.contains("77d")
            {
                Some(1.6)
            } else {
                None
            }
        }
        "Nikon" => {
            if model_lower.starts_with("d8") || model_lower.starts_with("d7") || model_lower.starts_with("d6")
                || model_lower.starts_with("z5") || model_lower.starts_with("z6") || model_lower.starts_with("z7")
                || model_lower.starts_with("z8") || model_lower.starts_with("z9")
            {
                Some(1.0)
            } else if model_lower.starts_with("d5") || model_lower.starts_with("d3") || model_lower.starts_with("z30")
                || model_lower.starts_with("z50") || model_lower.starts_with("zfc")
            {
                Some(1.5)
            } else {
                None
            }
        }
        "Panasonic" | "Olympus" => {
            if model_lower.contains("s1") || model_lower.contains("s5") {
                Some(1.0)
            } else {
                Some(2.0)
            }
        }
        "Fujifilm" => {
            if model_lower.contains("gfx") {
                Some(0.79)
            } else {
                Some(1.5)
            }
        }
        "GoPro" => Some(1.0),
        "DJI" => {
            if model_lower.contains("mavic 3") || model_lower.contains("inspire") {
                Some(1.0)
            } else {
                None
            }
        }
        _ => None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brand_normalization() {
        assert_eq!(CameraRegistryBuilder::normalize_brand_name("gopro"), "GoPro");
        assert_eq!(CameraRegistryBuilder::normalize_brand_name("SONY"), "Sony");
        assert_eq!(CameraRegistryBuilder::normalize_brand_name("ILCE"), "Sony");
        assert_eq!(CameraRegistryBuilder::normalize_brand_name("DJI"), "DJI");
        assert_eq!(CameraRegistryBuilder::normalize_brand_name("Blackmagic Design"), "Blackmagic");
        assert_eq!(CameraRegistryBuilder::normalize_brand_name("BMPCC"), "Blackmagic");
        assert_eq!(CameraRegistryBuilder::normalize_brand_name("fuji"), "Fujifilm");
        assert_eq!(CameraRegistryBuilder::normalize_brand_name("lumix"), "Panasonic");
    }

    #[test]
    fn test_registry_builder() {
        let mut builder = CameraRegistryBuilder::new();
        builder
            .add_brand("Sony")
            .add_camera("Sony", "A7 III", "Sony E", Some(1.0))
            .add_camera_alias("Sony", "A7 III", "ILCE-7M3")
            .add_lens("Sony", "FE 24-70mm f/2.8 GM", &["Sony E"], true)
            .add_mount("Sony E", &[]);

        let registry = builder.build();

        assert!(registry.get_brand_by_name("Sony").is_some());
        assert!(registry.get_camera("Sony", "A7 III").is_some());
        assert!(registry.get_camera("Sony", "ILCE-7M3").is_some());
        assert!(registry.get_lens("Sony", "FE 24-70mm f/2.8 GM").is_some());
    }

    #[test]
    fn test_compatible_mounts() {
        let mut builder = CameraRegistryBuilder::new();
        builder
            .add_mount("Sony E", &[])
            .add_mount("Sony A", &["Sony E"]);

        let registry = builder.build();
        let compat = registry.get_compatible_mounts("Sony A");

        assert!(compat.contains("Sony A"));
        assert!(compat.contains("Sony E"));
    }

    #[test]
    fn test_is_zoom_lens() {
        let mut builder = CameraRegistryBuilder::new();
        builder
            .add_brand("Sony")
            .add_lens("Sony", "FE 24-70mm f/2.8 GM", &["Sony E"], true)
            .add_lens("Sony", "FE 50mm f/1.2 GM", &["Sony E"], false);

        let registry = builder.build();

        assert!(registry.is_zoom_lens("Sony", "FE 24-70mm f/2.8 GM"));
        assert!(!registry.is_zoom_lens("Sony", "FE 50mm f/1.2 GM"));
        assert!(registry.is_zoom_lens("Unknown", "24-70mm lens"));
    }

    #[test]
    fn test_compatible_cameras() {
        let mut builder = CameraRegistryBuilder::new();
        builder
            .add_mount("Sony E", &[])
            .add_brand("Sony")
            .add_camera("Sony", "A7 III", "Sony E", Some(1.0))
            .add_camera("Sony", "A7R IV", "Sony E", Some(1.0))
            .add_camera("Sony", "A6600", "Sony E", Some(1.5));

        let registry = builder.build();
        let compat = registry.get_compatible_cameras("Sony", "A7 III");

        assert_eq!(compat.len(), 1);
        assert!(compat.iter().any(|(b, m, _)| b == "Sony" && m == "A7R IV"));
        assert!(!compat.iter().any(|(b, m, _)| b == "Sony" && m == "A6600"));
    }

    #[test]
    fn test_json_load() {
        let json = r#"{
            "version": 1,
            "brands": [{
                "name": "Sony",
                "aliases": ["ILCE"],
                "cameras": [
                    {"name": "A7 III", "aliases": ["ILCE-7M3"], "mount": "Sony E", "crop_factor": 1.0}
                ],
                "lenses": [
                    {"name": "FE 24-70mm f/2.8 GM", "mounts": ["Sony E"], "is_zoom": true}
                ]
            }],
            "mounts": [
                {"name": "Sony E", "compatible_mounts": []}
            ],
            "lensfun_lenses": []
        }"#;

        let registry = CameraRegistry::load_from_json(json).unwrap();
        
        assert_eq!(registry.brands.len(), 1);
        assert!(registry.get_brand_by_name("Sony").is_some());
        assert!(registry.get_brand_by_name("ILCE").is_some());
        assert!(registry.get_camera("Sony", "A7 III").is_some());
        assert!(registry.get_camera("Sony", "ILCE-7M3").is_some());
    }

    #[test]
    fn test_fixed_lens_camera() {
        let json = r#"{
            "version": 1,
            "brands": [{
                "name": "GoPro",
                "aliases": [],
                "cameras": [
                    {"name": "HERO11 Black", "mount": "", "fixed_lens": true}
                ],
                "lenses": []
            }],
            "mounts": [],
            "lensfun_lenses": []
        }"#;

        let registry = CameraRegistry::load_from_json(json).unwrap();
        
        if let Some((_, cam)) = registry.get_camera("GoPro", "HERO11 Black") {
            assert!(cam.fixed_lens);
        } else {
            panic!("Camera not found");
        }
    }

    #[test]
    fn test_compatible_lenses() {
        let mut builder = CameraRegistryBuilder::new();
        builder
            .add_mount("Sony E", &[])
            .add_mount("Sony A", &["Sony E"])
            .add_brand("Sony")
            .add_camera("Sony", "A7 III", "Sony E", Some(1.0))
            .add_lens("Sony", "FE 24-70mm f/2.8 GM", &["Sony E"], true)
            .add_lens("Sony", "Zeiss 55mm f/1.8", &["Sony E"], false)
            .add_brand("Sigma")
            .add_lens("Sigma", "24-70mm f/2.8 DG DN Art", &["Sony E"], true);

        let registry = builder.build();
        let lenses = registry.get_compatible_lenses("Sony", "A7 III");

        assert!(lenses.iter().any(|(b, l)| b == "Sony" && l == "FE 24-70mm f/2.8 GM"));
        assert!(lenses.iter().any(|(b, l)| b == "Sigma" && l == "24-70mm f/2.8 DG DN Art"));
    }

    #[test]
    fn test_search_brands() {
        let mut builder = CameraRegistryBuilder::new();
        builder
            .add_brand("Sony")
            .add_brand_alias("Sony", "ILCE")
            .add_brand("Canon")
            .add_brand("Nikon");

        let registry = builder.build();
        
        let results = registry.search_brands("son");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "Sony");
        
        let results = registry.search_brands("ilce");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "Sony");
    }

    #[test]
    fn test_infer_mount() {
        assert_eq!(infer_mount("Sony", "A7 III"), "Sony E");
        assert_eq!(infer_mount("Sony", "A6600"), "Sony E");
        assert_eq!(infer_mount("Canon", "EOS R5"), "Canon RF");
        assert_eq!(infer_mount("Canon", "5D Mark IV"), "Canon EF");
        assert_eq!(infer_mount("Nikon", "Z8"), "Nikon Z");
        assert_eq!(infer_mount("Nikon", "D850"), "Nikon F");
        assert_eq!(infer_mount("Panasonic", "S5"), "Leica L");
        assert_eq!(infer_mount("Panasonic", "GH6"), "Micro Four Thirds");
        assert_eq!(infer_mount("GoPro", "HERO11"), "");
    }

    #[test]
    fn test_infer_crop_factor() {
        assert_eq!(infer_crop_factor("Sony", "A7 III"), Some(1.0));
        assert_eq!(infer_crop_factor("Sony", "A6600"), Some(1.5));
        assert_eq!(infer_crop_factor("Canon", "EOS R5"), Some(1.0));
        assert_eq!(infer_crop_factor("Canon", "EOS R7"), Some(1.6));
        assert_eq!(infer_crop_factor("Panasonic", "GH6"), Some(2.0));
        assert_eq!(infer_crop_factor("Fujifilm", "GFX 100"), Some(0.79));
        assert_eq!(infer_crop_factor("Fujifilm", "X-T5"), Some(1.5));
    }

    #[test]
    fn test_generate_from_profiles() {
        let profiles = vec![
            ("Sony".to_string(), "A7 III".to_string(), "FE 24-70mm f/2.8 GM".to_string(), "".to_string()),
            ("Canon".to_string(), "EOS R5".to_string(), "RF 24-70mm f/2.8".to_string(), "".to_string()),
            ("GoPro".to_string(), "HERO11 Black".to_string(), "".to_string(), "Wide".to_string()),
        ];

        let registry = generate_from_lens_profiles(&profiles);

        assert!(registry.get_brand_by_name("Sony").is_some());
        assert!(registry.get_brand_by_name("Canon").is_some());
        assert!(registry.get_brand_by_name("GoPro").is_some());
        assert!(registry.get_camera("Sony", "A7 III").is_some());
    }
}
