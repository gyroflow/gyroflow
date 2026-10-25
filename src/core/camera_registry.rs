// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024-2026 Adrian <adrian.eddy at gmail>

use crate::lens_profile::LensProfile;
use crate::lensfun::LensFunDatabase;
use serde::{ Deserialize, Serialize };
use std::collections::{ BTreeSet, HashMap };

static DEFAULT_CAMERA_REGISTRY_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/camera_registry.json"));

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MountSpec {
    pub name: String,
    pub flange_distance_mm: f64,
    pub compatible_mounts: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BrandSpec {
    pub name: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CameraSpec {
    pub brand: String,
    pub model: String,
    pub aliases: Vec<String>,
    pub mount: String,
    pub sensor_width_mm: f64,
    pub sensor_height_mm: f64,
    pub crop_factor: f64,
    pub fixed_lens: bool,
    pub lens_options: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LensSpec {
    pub brand: String,
    pub model: String,
    pub aliases: Vec<String>,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
    pub focal_min_mm: Option<f64>,
    pub focal_max_mm: Option<f64>,
    pub is_zoom: bool,
}

impl LensSpec {
    pub fn display_name(&self) -> String {
        if self.brand.is_empty()
            || self.model.to_ascii_lowercase().starts_with(&self.brand.to_ascii_lowercase())
        {
            self.model.clone()
        } else {
            format!("{} {}", self.brand, self.model)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraRegistry {
    pub version: u32,
    pub mounts: Vec<MountSpec>,
    pub brands: Vec<BrandSpec>,
    pub cameras: Vec<CameraSpec>,
    pub lenses: Vec<LensSpec>,

    #[serde(skip)]
    brand_lookup: HashMap<String, String>,
    #[serde(skip)]
    camera_lookup: HashMap<(String, String), usize>,
    #[serde(skip)]
    lens_lookup: HashMap<String, usize>,
    #[serde(skip)]
    mount_lookup: HashMap<String, usize>,
    #[serde(skip)]
    profile_lenses_by_camera: HashMap<(String, String), BTreeSet<String>>,
}

fn norm_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Detect whether a lens name describes a zoom focal range (e.g., "24-70mm", "16-35 mm", "18-50mm F2.8").
pub fn detect_zoom_from_name(name: &str) -> Option<(f64, f64)> {
    let bytes = name.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i].is_ascii_digit() {
            let start1 = i;
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let num1_str = &name[start1..i];
            while i < len && bytes[i] == b' ' {
                i += 1;
            }
            if i < len && bytes[i] == b'-' {
                i += 1;
                while i < len && bytes[i] == b' ' {
                    i += 1;
                }
                let start2 = i;
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                if start2 < i {
                    let num2_str = &name[start2..i];
                    let rest = name[i..].trim_start().to_ascii_lowercase();
                    if rest.starts_with("mm") || rest.starts_with("f/") || rest.starts_with("f1") || rest.starts_with("f2") || rest.starts_with("f3") || rest.starts_with("f4") || rest.starts_with("f5") {
                        if let (Ok(f1), Ok(f2)) = (num1_str.parse::<f64>(), num2_str.parse::<f64>()) {
                            if f1 >= 4.0 && f2 > f1 && f2 <= 2000.0 {
                                return Some((f1, f2));
                            }
                        }
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Detect if a string contains a specific single focal length (e.g., "24mm", "24 mm", "35.0mm", "@24mm").
pub fn has_explicit_focal_length_in_text(text: &str) -> bool {
    // Strip out zoom ranges like "24-70mm" first so we only check for standalone focal length markers
    let mut s = text.to_ascii_lowercase();
    while let Some((f1, f2)) = detect_zoom_from_name(&s) {
        let pat1 = format!("{:.0}-{:.0}mm", f1, f2);
        let pat2 = format!("{:.0}-{:.0} mm", f1, f2);
        let pat3 = format!("{:.0}-{:.0}", f1, f2);
        if s.contains(&pat1) {
            s = s.replacen(&pat1, "", 1);
        } else if s.contains(&pat2) {
            s = s.replacen(&pat2, "", 1);
        } else if s.contains(&pat3) {
            s = s.replacen(&pat3, "", 1);
        } else {
            break;
        }
    }

    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if let Ok(val) = s[start..i].parse::<f64>() {
                let rest = s[i..].trim_start();
                if rest.starts_with("mm") && val >= 1.0 && val <= 2000.0 {
                    return true;
                }
            }
        } else {
            i += 1;
        }
    }
    false
}

impl CameraRegistry {
    pub fn load_default() -> Self {
        Self::from_json(DEFAULT_CAMERA_REGISTRY_JSON).unwrap_or_default()
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let mut reg: Self = serde_json::from_str(json)?;
        reg.rebuild_indices();
        Ok(reg)
    }

    pub fn merge_json(&mut self, json: &str) -> Result<(), serde_json::Error> {
        let other: Self = serde_json::from_str(json)?;
        self.merge(other);
        Ok(())
    }

    pub fn merge(&mut self, other: Self) {
        if other.version > self.version {
            self.version = other.version;
        }
        for m in other.mounts {
            if let Some(&idx) = self.mount_lookup.get(&norm_key(&m.name)) {
                for compat in m.compatible_mounts {
                    if !self.mounts[idx].compatible_mounts.contains(&compat) {
                        self.mounts[idx].compatible_mounts.push(compat);
                    }
                }
            } else {
                self.mounts.push(m);
            }
        }
        for b in other.brands {
            let key = norm_key(&b.name);
            if let Some(existing_brand) = self.brand_lookup.get(&key).cloned() {
                if let Some(spec) = self.brands.iter_mut().find(|x| x.name == existing_brand) {
                    for alias in b.aliases {
                        if !spec.aliases.contains(&alias) {
                            spec.aliases.push(alias);
                        }
                    }
                }
            } else {
                self.brands.push(b);
            }
        }
        for c in other.cameras {
            let brand = self.normalize_brand(&c.brand);
            let key = (norm_key(&brand), norm_key(&c.model));
            if !self.camera_lookup.contains_key(&key) {
                self.cameras.push(c);
            }
        }
        for l in other.lenses {
            let key = norm_key(&l.display_name());
            if !self.lens_lookup.contains_key(&key) && !self.lens_lookup.contains_key(&norm_key(&l.model)) {
                self.lenses.push(l);
            }
        }
        self.rebuild_indices();
    }

    pub fn rebuild_indices(&mut self) {
        self.brand_lookup.clear();
        self.camera_lookup.clear();
        self.lens_lookup.clear();
        self.mount_lookup.clear();

        for (idx, m) in self.mounts.iter().enumerate() {
            self.mount_lookup.insert(norm_key(&m.name), idx);
        }

        for b in &self.brands {
            self.brand_lookup.insert(norm_key(&b.name), b.name.clone());
            for alias in &b.aliases {
                self.brand_lookup.insert(norm_key(alias), b.name.clone());
            }
        }

        for (idx, c) in self.cameras.iter().enumerate() {
            let b_key = norm_key(&c.brand);
            self.camera_lookup.insert((b_key.clone(), norm_key(&c.model)), idx);
            for alias in &c.aliases {
                self.camera_lookup.insert((b_key.clone(), norm_key(alias)), idx);
                // Also support alias when brand prefix is included inside model string
                let stripped = alias
                    .to_ascii_lowercase()
                    .replace(&c.brand.to_ascii_lowercase(), "");
                let stripped_key = norm_key(&stripped);
                if !stripped_key.is_empty() {
                    self.camera_lookup.insert((b_key.clone(), stripped_key), idx);
                }
            }
        }

        for (idx, l) in self.lenses.iter().enumerate() {
            self.lens_lookup.insert(norm_key(&l.model), idx);
            self.lens_lookup.insert(norm_key(&l.display_name()), idx);
            for alias in &l.aliases {
                self.lens_lookup.insert(norm_key(alias), idx);
            }
        }
    }

    pub fn ingest_lensfun(&mut self, lf: &LensFunDatabase) {
        for m in &lf.mounts {
            let key = norm_key(&m.name);
            if let Some(&idx) = self.mount_lookup.get(&key) {
                for c in &m.compat {
                    if !self.mounts[idx].compatible_mounts.contains(c) {
                        self.mounts[idx].compatible_mounts.push(c.clone());
                    }
                }
            } else {
                let mut compat = vec![m.name.clone()];
                compat.extend(m.compat.iter().cloned());
                self.mounts.push(MountSpec {
                    name: m.name.clone(),
                    flange_distance_mm: 0.0,
                    compatible_mounts: compat,
                });
            }
        }

        for cam in &lf.cameras {
            let brand = self.normalize_brand(&cam.maker);
            if self.brand_lookup.get(&norm_key(&brand)).is_none() {
                self.brands.push(BrandSpec {
                    name: brand.clone(),
                    aliases: vec![],
                });
            }
            let model_clean = if cam.model.to_ascii_lowercase().starts_with(&brand.to_ascii_lowercase()) {
                cam.model[brand.len()..].trim().to_string()
            } else {
                cam.model.clone()
            };
            let model = if model_clean.is_empty() { cam.model.clone() } else { model_clean.clone() };
            let key = (norm_key(&brand), norm_key(&model));
            if !self.camera_lookup.contains_key(&key) {
                let fixed_lens = cam.mount.to_ascii_lowercase().contains("fixed") || cam.mount.to_ascii_lowercase().contains("compact");
                self.cameras.push(CameraSpec {
                    brand,
                    model,
                    aliases: if cam.model != model_clean && !model_clean.is_empty() { vec![cam.model.clone()] } else { vec![] },
                    mount: if fixed_lens { "Fixed".to_string() } else { cam.mount.clone() },
                    sensor_width_mm: if cam.crop_factor > 0.0 { 36.0 / cam.crop_factor } else { 36.0 },
                    sensor_height_mm: if cam.crop_factor > 0.0 { 24.0 / cam.crop_factor } else { 24.0 },
                    crop_factor: if cam.crop_factor > 0.0 { cam.crop_factor } else { 1.0 },
                    fixed_lens,
                    lens_options: vec![],
                });
            }
        }

        for lens in &lf.lenses {
            let brand = self.normalize_brand(&lens.maker);
            let model_clean = if !brand.is_empty() && lens.model.to_ascii_lowercase().starts_with(&brand.to_ascii_lowercase()) {
                lens.model[brand.len()..].trim().to_string()
            } else {
                lens.model.clone()
            };
            let model = if model_clean.is_empty() { lens.model.clone() } else { model_clean };
            if !self.lens_lookup.contains_key(&norm_key(&model)) && !self.lens_lookup.contains_key(&norm_key(&lens.model)) {
                let zoom_detected = detect_zoom_from_name(&model);
                let is_zoom = lens.is_zoom() || zoom_detected.is_some();
                let (min_f, max_f) = match (lens.min_focal, lens.max_focal, zoom_detected) {
                    (Some(a), Some(b), _) => (Some(a), Some(b)),
                    (_, _, Some((a, b))) => (Some(a), Some(b)),
                    _ => (None, None),
                };
                self.lenses.push(LensSpec {
                    brand,
                    model,
                    aliases: vec![lens.model.clone()],
                    mounts: lens.mounts.clone(),
                    crop_factor: if lens.crop_factor > 0.0 { lens.crop_factor } else { 1.0 },
                    focal_min_mm: min_f,
                    focal_max_mm: max_f,
                    is_zoom,
                });
            }
        }

        self.rebuild_indices();
    }

    pub fn ingest_lens_profile(&mut self, profile: &LensProfile) {
        if profile.camera_brand.is_empty() || profile.camera_model.is_empty() || profile.camera_model == "LensFun" {
            return;
        }
        let brand = self.normalize_brand(&profile.camera_brand);
        let b_key = norm_key(&brand);
        if !b_key.is_empty() && !self.brand_lookup.contains_key(&b_key) {
            self.brands.push(BrandSpec {
                name: brand.clone(),
                aliases: vec![],
            });
            self.brand_lookup.insert(b_key.clone(), brand.clone());
        }

        let model = self.normalize_camera_model(&brand, &profile.camera_model);
        let m_key = norm_key(&model);
        if !b_key.is_empty() && !m_key.is_empty() && !self.camera_lookup.contains_key(&(b_key.clone(), m_key.clone())) {
            let idx = self.cameras.len();
            self.cameras.push(CameraSpec {
                brand: brand.clone(),
                model: model.clone(),
                aliases: vec![],
                mount: String::new(),
                sensor_width_mm: 0.0,
                sensor_height_mm: 0.0,
                crop_factor: profile.crop_factor.unwrap_or(1.0),
                fixed_lens: false,
                lens_options: vec![],
            });
            self.camera_lookup.insert((b_key.clone(), m_key.clone()), idx);
        }

        if !profile.lens_model.is_empty() {
            let lens = self.normalize_lens_model(&profile.lens_model);
            self.profile_lenses_by_camera
                .entry((b_key, m_key))
                .or_default()
                .insert(lens);
        }
    }

    pub fn normalize_brand(&self, brand: &str) -> String {
        let trimmed = brand.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        let key = norm_key(trimmed);
        if let Some(canon) = self.brand_lookup.get(&key) {
            return canon.clone();
        }
        trimmed.to_string()
    }

    pub fn normalize_camera_model(&self, brand: &str, model: &str) -> String {
        let canon_brand = self.normalize_brand(brand);
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        let b_key = norm_key(&canon_brand);
        let m_key = norm_key(trimmed);
        if let Some(&idx) = self.camera_lookup.get(&(b_key.clone(), m_key)) {
            return self.cameras[idx].model.clone();
        }
        // Try stripping brand prefix from model string
        if !canon_brand.is_empty() && trimmed.to_ascii_lowercase().starts_with(&canon_brand.to_ascii_lowercase()) {
            let without_brand = trimmed[canon_brand.len()..].trim();
            if let Some(&idx) = self.camera_lookup.get(&(b_key, norm_key(without_brand))) {
                return self.cameras[idx].model.clone();
            }
            if !without_brand.is_empty() {
                return without_brand.to_string();
            }
        }
        trimmed.to_string()
    }

    pub fn normalize_lens_model(&self, lens: &str) -> String {
        let trimmed = lens.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        let key = norm_key(trimmed);
        if let Some(&idx) = self.lens_lookup.get(&key) {
            return self.lenses[idx].display_name();
        }
        trimmed.to_string()
    }

    pub fn get_brands(&self) -> Vec<String> {
        let mut set: Vec<String> = self.brands.iter().map(|b| b.name.clone()).collect();
        set.sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
        set.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        set
    }

    pub fn get_camera_models(&self, brand: &str) -> Vec<String> {
        let canon_brand = self.normalize_brand(brand);
        let mut models: Vec<String> = self
            .cameras
            .iter()
            .filter(|c| c.brand.eq_ignore_ascii_case(&canon_brand))
            .map(|c| c.model.clone())
            .collect();
        models.sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
        models.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        models
    }

    pub fn get_camera_spec(&self, brand: &str, model: &str) -> Option<&CameraSpec> {
        let canon_brand = self.normalize_brand(brand);
        let canon_model = self.normalize_camera_model(&canon_brand, model);
        let key = (norm_key(&canon_brand), norm_key(&canon_model));
        self.camera_lookup.get(&key).map(|&idx| &self.cameras[idx])
    }

    pub fn is_fixed_lens(&self, brand: &str, model: &str) -> bool {
        self.get_camera_spec(brand, model)
            .map(|c| c.fixed_lens || c.mount.eq_ignore_ascii_case("Fixed"))
            .unwrap_or(false)
    }

    pub fn is_mount_compatible(&self, body_mount: &str, lens_mount: &str) -> bool {
        if body_mount.is_empty() || lens_mount.is_empty() {
            return false;
        }
        if norm_key(body_mount) == norm_key(lens_mount) {
            return true;
        }
        if let Some(&idx) = self.mount_lookup.get(&norm_key(body_mount)) {
            let m = &self.mounts[idx];
            if m.compatible_mounts.iter().any(|cm| norm_key(cm) == norm_key(lens_mount)) {
                return true;
            }
        }
        false
    }

    pub fn get_compatible_lenses(&self, brand: &str, model: &str) -> Vec<String> {
        let canon_brand = self.normalize_brand(brand);
        let canon_model = self.normalize_camera_model(&canon_brand, model);
        let mut result = BTreeSet::new();

        if let Some(cam) = self.get_camera_spec(&canon_brand, &canon_model) {
            if cam.fixed_lens || cam.mount.eq_ignore_ascii_case("Fixed") {
                for opt in &cam.lens_options {
                    result.insert(opt.clone());
                }
            } else if !cam.mount.is_empty() {
                for lens in &self.lenses {
                    let mount_ok = lens.mounts.iter().any(|lm| self.is_mount_compatible(&cam.mount, lm));
                    // Full-frame lenses work on APS-C/MFT bodies; APS-C lenses work on APS-C/MFT bodies or FF in Super35 crop mode
                    if mount_ok {
                        result.insert(lens.display_name());
                    }
                }
            }
        }

        // Also include any lenses recorded in loaded profiles for this camera or same-mount cameras
        let cam_key = (norm_key(&canon_brand), norm_key(&canon_model));
        if let Some(prof_lenses) = self.profile_lenses_by_camera.get(&cam_key) {
            for l in prof_lenses {
                result.insert(l.clone());
            }
        }

        result.into_iter().collect()
    }

    /// Find cameras whose profiles are compatible with `(brand, model)` based on mount & sensor size.
    /// Returns `Vec<(brand, model, requires_adapter)>`.
    pub fn get_compatible_cameras(
        &self,
        brand: &str,
        model: &str,
        include_adapted_mounts: bool,
    ) -> Vec<(String, String, bool)> {
        let Some(target) = self.get_camera_spec(brand, model) else {
            return Vec::new();
        };
        if target.fixed_lens || target.mount.is_empty() || target.mount.eq_ignore_ascii_case("Fixed") {
            return Vec::new();
        }

        let mut compatible = Vec::new();
        for cam in &self.cameras {
            if cam.fixed_lens || cam.mount.is_empty() || cam.mount.eq_ignore_ascii_case("Fixed") {
                continue;
            }
            if cam.brand.eq_ignore_ascii_case(&target.brand) && cam.model.eq_ignore_ascii_case(&target.model) {
                continue;
            }
            // Sensor crop factor must match closely (within ~12%, e.g. all full-frame 1.0x cameras or all APS-C 1.5x cameras)
            if (cam.crop_factor - target.crop_factor).abs() > 0.12 {
                continue;
            }

            let same_mount = norm_key(&cam.mount) == norm_key(&target.mount);
            if same_mount {
                compatible.push((cam.brand.clone(), cam.model.clone(), false));
            } else if include_adapted_mounts
                && (self.is_mount_compatible(&target.mount, &cam.mount)
                    || self.is_mount_compatible(&cam.mount, &target.mount))
            {
                compatible.push((cam.brand.clone(), cam.model.clone(), true));
            }
        }

        // Prioritize same brand + native mount first, then same mount, then adapted mounts
        compatible.sort_by(|a, b| {
            a.2.cmp(&b.2)
                .then_with(|| {
                    let a_same_brand = a.0.eq_ignore_ascii_case(&target.brand);
                    let b_same_brand = b.0.eq_ignore_ascii_case(&target.brand);
                    b_same_brand.cmp(&a_same_brand)
                })
                .then_with(|| a.0.cmp(&b.0))
                .then_with(|| a.1.cmp(&b.1))
        });

        compatible
    }

    pub fn is_zoom_lens(&self, lens_model: &str) -> bool {
        let key = norm_key(lens_model);
        if let Some(&idx) = self.lens_lookup.get(&key) {
            if self.lenses[idx].is_zoom {
                return true;
            }
        }
        detect_zoom_from_name(lens_model).is_some()
    }

    /// Validate whether a newly calibrated or loaded `LensProfile` is unambiguous and safe to upload.
    /// Returns `Ok(())` if valid, or `Err(Vec<String>)` with human-readable reasons if ambiguous.
    pub fn validate_profile_for_upload(&self, profile: &LensProfile) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        let brand = profile.camera_brand.trim();
        let model = profile.camera_model.trim();
        let lens = profile.lens_model.trim();
        let setting = profile.camera_setting.trim();

        let invalid_placeholders = ["", "other", "unknown", "none", "n/a", "---", "camera", "lens", "default", "test"];
        if invalid_placeholders.contains(&brand.to_ascii_lowercase().as_str()) {
            errors.push("Camera brand is missing or ambiguous.".to_string());
        }
        if invalid_placeholders.contains(&model.to_ascii_lowercase().as_str()) {
            errors.push("Camera model is missing or ambiguous.".to_string());
        }

        let fixed = self.is_fixed_lens(brand, model);
        if !fixed && invalid_placeholders.contains(&lens.to_ascii_lowercase().as_str()) {
            errors.push("Lens model is required for interchangeable-lens cameras.".to_string());
        }

        // If a zoom lens is selected, require an explicit focal length unless the profile has multi-focal interpolations
        if self.is_zoom_lens(lens) && !profile.has_interpolations() && profile.interpolations.is_none() {
            let has_fl_field = profile.focal_length.is_some_and(|fl| fl > 0.0);
            let has_fl_text = has_explicit_focal_length_in_text(lens)
                || has_explicit_focal_length_in_text(setting)
                || has_explicit_focal_length_in_text(&profile.note);
            if !has_fl_field && !has_fl_text {
                errors.push("Focal length must be specified when calibrating a zoom lens (e.g. 24mm).".to_string());
            }
        }

        if profile.calib_dimension.w == 0 || profile.calib_dimension.h == 0 {
            errors.push("Calibration dimensions are missing.".to_string());
        }
        if profile.fisheye_params.camera_matrix.len() != 3 || profile.fisheye_params.distortion_coeffs.is_empty() {
            errors.push("Calibration parameters (camera matrix or distortion coefficients) are incomplete.".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lens_profile::{ CameraParams, Dimensions };

    #[test]
    fn test_camera_registry_normalization_and_compatibility() {
        let reg = CameraRegistry::load_default();
        assert!(!reg.get_brands().is_empty());

        // Brand and model normalization
        assert_eq!(reg.normalize_brand("sony"), "Sony");
        assert_eq!(reg.normalize_brand("Blackmagic Design"), "Blackmagic");
        assert_eq!(reg.normalize_camera_model("Sony", "ILCE-7SM3"), "A7S III");
        assert_eq!(reg.normalize_camera_model("Sony", "a74"), "A7 IV");
        assert_eq!(reg.normalize_camera_model("Blackmagic", "Pocker Cinema Camera 4k"), "Pocket Cinema Camera 4K");

        // Fixed lens vs interchangeable lens detection
        assert!(reg.is_fixed_lens("GoPro", "HERO11 Black"));
        assert!(!reg.is_fixed_lens("Sony", "A7S III"));

        // Mount compatibility: EF on RF, E-mount on Nikon Z
        assert!(reg.is_mount_compatible("Canon RF", "Canon EF"));
        assert!(reg.is_mount_compatible("Sony E", "Canon EF"));
        assert!(reg.is_mount_compatible("Nikon Z", "Sony E"));
        assert!(!reg.is_mount_compatible("Canon EF", "Sony E"));

        // Compatible cameras for Sony A7S III (Issue #742 item 4)
        let compat = reg.get_compatible_cameras("Sony", "A7S III", true);
        let native_models: Vec<&str> = compat.iter().filter(|(_, _, adapted)| !*adapted).map(|(_, m, _)| m.as_str()).collect();
        assert!(native_models.contains(&"FX3"));
        assert!(native_models.contains(&"FX6"));
        assert!(native_models.contains(&"FX9"));
        assert!(native_models.contains(&"ZV-E1"));
        assert!(native_models.contains(&"A7 IV"));
        assert!(native_models.contains(&"A7C"));
        assert!(native_models.contains(&"A7C II"));
        assert!(native_models.contains(&"A1"));
        assert!(native_models.contains(&"A9"));

        // Adapted full-frame Nikon Z cameras
        let adapted_brands: Vec<&str> = compat.iter().filter(|(_, _, adapted)| *adapted).map(|(b, _, _)| b.as_str()).collect();
        assert!(adapted_brands.contains(&"Nikon"));
    }

    #[test]
    fn test_zoom_detection_and_upload_validation() {
        let reg = CameraRegistry::load_default();
        assert!(reg.is_zoom_lens("FE 24-70mm F2.8 GM"));
        assert!(reg.is_zoom_lens("Sigma 18-50mm F2.8 DC DN"));
        assert!(!reg.is_zoom_lens("FE 24mm F1.4 GM"));

        let mut profile = LensProfile {
            camera_brand: "Sony".to_string(),
            camera_model: "A7S III".to_string(),
            lens_model: "FE 24-70mm F2.8 GM".to_string(),
            calib_dimension: Dimensions { w: 3840, h: 2160 },
            fisheye_params: CameraParams {
                RMS_error: 0.25,
                camera_matrix: vec![[2000.0, 0.0, 1920.0], [0.0, 2000.0, 1080.0], [0.0, 0.0, 1.0]],
                distortion_coeffs: vec![0.01, -0.02, 0.001, 0.0],
                radial_distortion_limit: None,
            },
            ..Default::default()
        };

        // Missing focal length on zoom lens -> blocked!
        let res = reg.validate_profile_for_upload(&profile);
        assert!(res.is_err());

        // Specify focal length -> allowed!
        profile.focal_length = Some(24.0);
        assert!(reg.validate_profile_for_upload(&profile).is_ok());

        // Ambiguous brand "Other" -> blocked!
        profile.camera_brand = "Other".to_string();
        assert!(reg.validate_profile_for_upload(&profile).is_err());
    }
}
