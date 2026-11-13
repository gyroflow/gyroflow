// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023-2026 Adrian <adrian.eddy at gmail>

//! Standardized camera brand, camera model, lens mount, sensor size / crop factor catalog,
//! compatible camera recommender, calibrator submission validator, and lens profile review session (#742).

use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use crate::lens_profile::LensProfile;
use crate::lens_profile_database::LensProfileDatabase;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraMountSpec {
    pub mount_id: String,
    pub display_name: String,
    pub flange_focal_distance_mm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraModelSpec {
    pub brand: String,
    pub model: String,
    pub mount: String,
    pub sensor_width_mm: f64,
    pub sensor_height_mm: f64,
    pub crop_factor: f64,
    pub is_interchangeable_lens: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensModelSpec {
    pub brand: String,
    pub model: String,
    pub compatible_mounts: Vec<String>,
    pub min_focal_mm: Option<f64>,
    pub max_focal_mm: Option<f64>,
    pub is_zoom: bool,
    pub max_sensor_crop_factor: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CameraLensCatalog {
    pub brands: BTreeSet<String>,
    pub mounts: BTreeMap<String, CameraMountSpec>,
    pub cameras: Vec<CameraModelSpec>,
    pub lenses: Vec<LensModelSpec>,
}

impl CameraLensCatalog {
    pub fn with_builtin_mounts() -> Self {
        let mut mounts = BTreeMap::new();
        let builtin = [
            ("sony_e", "Sony E / FE", 18.0),
            ("canon_rf", "Canon RF", 20.0),
            ("canon_ef", "Canon EF", 44.0),
            ("nikon_z", "Nikon Z", 16.0),
            ("fujifilm_x", "Fujifilm X", 17.7),
            ("mft", "Micro Four Thirds", 19.25),
            ("l_mount", "Leica / Panasonic / Sigma L-Mount", 20.0),
            ("fixed", "Fixed / Action Cam Integrated Lens", 0.0),
        ];
        for (id, name, ffd) in builtin {
            mounts.insert(
                id.to_string(),
                CameraMountSpec {
                    mount_id: id.to_string(),
                    display_name: name.to_string(),
                    flange_focal_distance_mm: ffd,
                },
            );
        }
        Self {
            brands: BTreeSet::new(),
            mounts,
            cameras: Vec::new(),
            lenses: Vec::new(),
        }
    }

    pub fn infer_mount_and_interchangeable(brand: &str, model: &str) -> (String, bool) {
        let b = brand.trim().to_ascii_lowercase();
        let m = model.trim().to_ascii_lowercase();
        if b == "sony" && (m.starts_with("ilce") || m.starts_with("ilme") || m.starts_with("a7") || m.starts_with("fx") || m.starts_with("zv-e")) {
            return ("sony_e".to_string(), true);
        }
        if b == "canon" && m.starts_with("eos r") {
            return ("canon_rf".to_string(), true);
        }
        if b == "nikon" && m.starts_with('z') {
            return ("nikon_z".to_string(), true);
        }
        if b == "fujifilm" && m.starts_with("x-") {
            return ("fujifilm_x".to_string(), true);
        }
        if b == "panasonic" && (m.starts_with("dc-s") || m.starts_with("s5") || m.starts_with("s1")) {
            return ("l_mount".to_string(), true);
        }
        if (b == "panasonic" || b == "olympus" || b == "om system") && (m.starts_with("gh") || m.starts_with("g9") || m.starts_with("om-")) {
            return ("mft".to_string(), true);
        }
        ("fixed".to_string(), false)
    }

    pub fn parse_focal_range_mm(lens_model: &str) -> (Option<f64>, Option<f64>, bool) {
        let lower = lens_model.to_ascii_lowercase();
        if let Some(mm_idx) = lower.find("mm") {
            let prefix = &lower[..mm_idx];
            let token = prefix.split_whitespace().last().unwrap_or("");
            if let Some((min_s, max_s)) = token.split_once('-') {
                if let (Ok(min_f), Ok(max_f)) = (min_s.parse::<f64>(), max_s.parse::<f64>()) {
                    if min_f > 0.0 && max_f >= min_f {
                        return (Some(min_f), Some(max_f), (max_f - min_f).abs() > 1e-3);
                    }
                }
            } else if let Ok(single_f) = token.parse::<f64>() {
                if single_f > 0.0 {
                    return (Some(single_f), Some(single_f), false);
                }
            }
        }
        (None, None, false)
    }

    /// Populates the standardized catalog from a `LensProfileDatabase`.
    pub fn from_database(db: &LensProfileDatabase) -> Self {
        let mut catalog = Self::with_builtin_mounts();
        let mut seen_cameras = BTreeSet::new();
        let mut seen_lenses = BTreeSet::new();

        for profile in db.get_all().values() {
            let brand = profile.camera_brand.trim();
            let model = profile.camera_model.trim();
            if !brand.is_empty() {
                catalog.brands.insert(brand.to_string());
            }
            if !brand.is_empty() && !model.is_empty() {
                let cam_key = format!("{}:{}", brand.to_ascii_lowercase(), model.to_ascii_lowercase());
                if seen_cameras.insert(cam_key) {
                    let (mount, is_ilc) = Self::infer_mount_and_interchangeable(brand, model);
                    let crop = profile.crop_factor.filter(|&c| c > 0.0).unwrap_or(1.0);
                    let sensor_w = 36.0 / crop;
                    let sensor_h = 24.0 / crop;
                    catalog.cameras.push(CameraModelSpec {
                        brand: brand.to_string(),
                        model: model.to_string(),
                        mount: mount.clone(),
                        sensor_width_mm: (sensor_w * 100.0).round() / 100.0,
                        sensor_height_mm: (sensor_h * 100.0).round() / 100.0,
                        crop_factor: crop,
                        is_interchangeable_lens: is_ilc,
                    });
                }
            }
            let lens_name = profile.lens_model.trim();
            if !lens_name.is_empty() {
                let lens_key = lens_name.to_ascii_lowercase();
                if seen_lenses.insert(lens_key) {
                    let (mount, _) = Self::infer_mount_and_interchangeable(brand, model);
                    let (min_f, max_f, is_zoom) = Self::parse_focal_range_mm(lens_name);
                    let crop = profile.crop_factor.filter(|&c| c > 0.0).unwrap_or(1.0);
                    catalog.lenses.push(LensModelSpec {
                        brand: brand.to_string(),
                        model: lens_name.to_string(),
                        compatible_mounts: vec![mount],
                        min_focal_mm: min_f,
                        max_focal_mm: max_f,
                        is_zoom,
                        max_sensor_crop_factor: crop,
                    });
                }
            }
        }
        catalog
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }

    /// Recommends all compatible cameras in the catalog that share the same lens mount
    /// and a compatible sensor crop factor (e.g., all Sony full-frame E-mount cameras).
    pub fn find_compatible_cameras(&self, brand: &str, model: &str) -> Vec<CameraModelSpec> {
        let Some(target) = self.cameras.iter().find(|c| {
            c.brand.eq_ignore_ascii_case(brand.trim()) && c.model.eq_ignore_ascii_case(model.trim())
        }) else {
            return Vec::new();
        };
        if !target.is_interchangeable_lens || target.mount == "fixed" {
            return vec![target.clone()];
        }
        self.cameras
            .iter()
            .filter(|c| {
                c.mount == target.mount
                    && c.is_interchangeable_lens
                    && (c.crop_factor - target.crop_factor).abs() <= 0.15
            })
            .cloned()
            .collect()
    }

    /// Enforces calibrator lens profile metadata completeness before submission (#742):
    /// - Rejects missing `camera_brand` or `camera_model`
    /// - Rejects interchangeable-lens / mirrorless cameras without `lens_model`
    /// - Rejects zoom lenses (e.g. `24-70mm`) that do not specify the calibrated focal length
    pub fn validate_profile_for_submission(
        &self,
        profile: &LensProfile,
        allow_other_manual: bool,
    ) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let brand = profile.camera_brand.trim();
        let model = profile.camera_model.trim();
        let lens = profile.lens_model.trim();

        if brand.is_empty() {
            errors.push("Camera brand is required for lens profile submission.".to_string());
        }
        if model.is_empty() {
            errors.push("Camera model is required for lens profile submission.".to_string());
        }

        if !allow_other_manual && !brand.is_empty() && !model.is_empty() {
            let known = self.cameras.iter().any(|c| {
                c.brand.eq_ignore_ascii_case(brand) && c.model.eq_ignore_ascii_case(model)
            });
            if !known {
                errors.push(format!(
                    "Camera '{} {}' is not in the standardized catalog (enable 'Other' to submit a custom model).",
                    brand, model
                ));
            }
        }

        let (_, is_ilc) = Self::infer_mount_and_interchangeable(brand, model);
        if is_ilc && lens.is_empty() {
            errors.push(
                "Interchangeable-lens camera requires explicit lens model parameters.".to_string(),
            );
        }

        let (_, _, is_zoom) = Self::parse_focal_range_mm(lens);
        if is_zoom && profile.focal_length.filter(|&f| f > 0.0).is_none() {
            errors.push(
                "Zoom lens profile must specify the exact calibrated focal length in mm.".to_string(),
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Interactive review session allowing maintainers/users to toggle between all submitted
/// lens profiles for a given `(camera_brand, camera_model, lens_model)` setup and reject bad ones (#742).
#[derive(Clone, Debug)]
pub struct ProfileReviewSession {
    pub camera_brand: String,
    pub camera_model: String,
    pub lens_model: String,
    pub candidate_ids: Vec<String>,
    pub current_index: usize,
    pub rejected_ids: BTreeSet<String>,
}

impl ProfileReviewSession {
    pub fn new(
        db: &LensProfileDatabase,
        camera_brand: &str,
        camera_model: &str,
        lens_model: &str,
    ) -> Self {
        let mut candidate_ids: Vec<String> = db
            .get_all()
            .iter()
            .filter(|(_, p)| {
                p.camera_brand.eq_ignore_ascii_case(camera_brand.trim())
                    && p.camera_model.eq_ignore_ascii_case(camera_model.trim())
                    && (lens_model.trim().is_empty()
                        || p.lens_model.eq_ignore_ascii_case(lens_model.trim()))
            })
            .map(|(k, _)| k.clone())
            .collect();
        candidate_ids.sort();
        Self {
            camera_brand: camera_brand.to_string(),
            camera_model: camera_model.to_string(),
            lens_model: lens_model.to_string(),
            candidate_ids,
            current_index: 0,
            rejected_ids: BTreeSet::new(),
        }
    }

    pub fn current_profile_id(&self) -> Option<&str> {
        self.candidate_ids.get(self.current_index).map(|s| s.as_str())
    }

    pub fn next_profile(&mut self) -> Option<&str> {
        if self.candidate_ids.is_empty() {
            return None;
        }
        self.current_index = (self.current_index + 1) % self.candidate_ids.len();
        self.current_profile_id()
    }

    pub fn prev_profile(&mut self) -> Option<&str> {
        if self.candidate_ids.is_empty() {
            return None;
        }
        self.current_index = if self.current_index == 0 {
            self.candidate_ids.len() - 1
        } else {
            self.current_index - 1
        };
        self.current_profile_id()
    }

    pub fn mark_current_rejected(&mut self) -> bool {
        if let Some(id) = self.current_profile_id().map(|s| s.to_string()) {
            self.rejected_ids.insert(id)
        } else {
            false
        }
    }

    pub fn approved_profile_ids(&self) -> Vec<String> {
        self.candidate_ids
            .iter()
            .filter(|id| !self.rejected_ids.contains(*id))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_mirrorless_and_zoom_lens_submission_rules() {
        let mut catalog = CameraLensCatalog::with_builtin_mounts();
        catalog.cameras.push(CameraModelSpec {
            brand: "Sony".to_string(),
            model: "ILCE-7SM3".to_string(),
            mount: "sony_e".to_string(),
            sensor_width_mm: 36.0,
            sensor_height_mm: 24.0,
            crop_factor: 1.0,
            is_interchangeable_lens: true,
        });
        catalog.cameras.push(CameraModelSpec {
            brand: "Sony".to_string(),
            model: "ILME-FX3".to_string(),
            mount: "sony_e".to_string(),
            sensor_width_mm: 36.0,
            sensor_height_mm: 24.0,
            crop_factor: 1.0,
            is_interchangeable_lens: true,
        });

        let compat = catalog.find_compatible_cameras("Sony", "ILCE-7SM3");
        assert_eq!(compat.len(), 2);

        let mut bad_profile = LensProfile {
            camera_brand: "Sony".to_string(),
            camera_model: "ILCE-7SM3".to_string(),
            lens_model: "".to_string(),
            ..Default::default()
        };
        assert!(catalog.validate_profile_for_submission(&bad_profile, false).is_err());

        bad_profile.lens_model = "FE 24-70mm F2.8 GM II".to_string();
        bad_profile.focal_length = None;
        assert!(catalog.validate_profile_for_submission(&bad_profile, false).is_err());

        bad_profile.focal_length = Some(24.0);
        assert!(catalog.validate_profile_for_submission(&bad_profile, false).is_ok());
    }
}
