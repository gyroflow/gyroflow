// SPDX-License-Identifier: GPL-3.0-or-later

//! Canonical camera/lens metadata derived from the loaded Gyroflow profile set.
//!
//! This is deliberately independent from the QML layer so the same catalogue can
//! drive the main profile selector, calibrator validation and future LensFun
//! enrichment without duplicating normalization logic in the UI.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::lens_profile::LensProfile;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraCatalogue {
    pub cameras: Vec<CameraRecord>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraRecord {
    pub brand: String,
    pub model: String,
    pub lenses: Vec<String>,
    pub crop_factor: Option<f64>,
    pub has_official_profile: bool,
    #[serde(default)]
    pub mount: String,
    #[serde(default)]
    pub metadata_sources: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct LensfunCatalogue {
    source: String,
    source_revision: String,
    license: String,
    cameras: Vec<LensfunCamera>,
    lenses: Vec<LensfunLens>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct LensfunCamera {
    brand: String,
    model: String,
    mount: String,
    crop_factor: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct LensfunLens {
    brand: String,
    model: String,
    mounts: Vec<String>,
}

impl CameraCatalogue {
    pub fn from_profiles<'a, I>(profiles: I) -> Self
    where
        I: IntoIterator<Item = &'a LensProfile>,
    {
        let mut cameras: BTreeMap<(String, String), CameraRecord> = BTreeMap::new();

        for profile in profiles {
            if profile.is_copy {
                continue;
            }
            let brand = profile.camera_brand.trim();
            let model = profile.camera_model.trim();
            if brand.is_empty() || model.is_empty() {
                continue;
            }

            let key = (normalize(brand), normalize(model));
            let record = cameras.entry(key).or_insert_with(|| CameraRecord {
                brand: brand.to_string(),
                model: model.to_string(),
                metadata_sources: vec!["Gyroflow profiles".to_owned()],
                ..Default::default()
            });

            let lens = profile.lens_model.trim();
            if !lens.is_empty()
                && !record
                    .lenses
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(lens))
            {
                record.lenses.push(lens.to_string());
            }

            if record.crop_factor.is_none() {
                record.crop_factor = profile
                    .crop_factor
                    .filter(|value| value.is_finite() && *value > 0.0);
            }
            record.has_official_profile |= profile.official;
        }

        let mut cameras = cameras.into_values().collect::<Vec<_>>();
        for camera in &mut cameras {
            camera.lenses.sort_by_key(|lens| normalize(lens));
        }
        cameras.sort_by(|a, b| {
            normalize(&a.brand)
                .cmp(&normalize(&b.brand))
                .then_with(|| normalize(&a.model).cmp(&normalize(&b.model)))
        });

        Self { cameras }
    }

    pub fn enrich_from_lensfun_json(&mut self, json: &str) -> Result<(), serde_json::Error> {
        let external: LensfunCatalogue = serde_json::from_str(json)?;
        if external.source != "Lensfun" || external.license != "CC BY-SA 3.0" {
            return Ok(());
        }

        let mut lenses_by_mount: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for lens in external.lenses {
            let display = if lens.brand.trim().is_empty() {
                lens.model
            } else if normalize(&lens.model).starts_with(&normalize(&lens.brand)) {
                lens.model
            } else {
                format!("{} {}", lens.brand.trim(), lens.model.trim())
            };
            for mount in lens.mounts {
                lenses_by_mount
                    .entry(normalize(&mount))
                    .or_default()
                    .push(display.clone());
            }
        }
        for values in lenses_by_mount.values_mut() {
            values.sort_by_key(|value| normalize(value));
            values.dedup_by(|a, b| normalize(a) == normalize(b));
        }

        for camera in external.cameras {
            if camera.brand.trim().is_empty() || camera.model.trim().is_empty() {
                continue;
            }
            let key_brand = normalize(&camera.brand);
            let key_model = normalize(&camera.model);
            let record = if let Some(existing) = self.cameras.iter_mut().find(|item| {
                normalize(&item.brand) == key_brand && normalize(&item.model) == key_model
            }) {
                existing
            } else {
                self.cameras.push(CameraRecord {
                    brand: camera.brand.clone(),
                    model: camera.model.clone(),
                    ..Default::default()
                });
                self.cameras.last_mut().expect("camera was just inserted")
            };
            if record.crop_factor.is_none() {
                record.crop_factor = camera.crop_factor.filter(|v| v.is_finite() && *v > 0.0);
            }
            if record.mount.is_empty() {
                record.mount = camera.mount.clone();
            }
            if !record
                .metadata_sources
                .iter()
                .any(|value| value == "Lensfun CC BY-SA 3.0")
            {
                record
                    .metadata_sources
                    .push("Lensfun CC BY-SA 3.0".to_owned());
            }
            if let Some(lenses) = lenses_by_mount.get(&normalize(&camera.mount)) {
                for lens in lenses {
                    if !record
                        .lenses
                        .iter()
                        .any(|known| normalize(known) == normalize(lens))
                    {
                        record.lenses.push(lens.clone());
                    }
                }
                record.lenses.sort_by_key(|lens| normalize(lens));
            }
        }
        self.cameras.sort_by(|a, b| {
            normalize(&a.brand)
                .cmp(&normalize(&b.brand))
                .then_with(|| normalize(&a.model).cmp(&normalize(&b.model)))
        });
        Ok(())
    }

    pub fn brands(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut brands = Vec::new();
        for camera in &self.cameras {
            let key = normalize(&camera.brand);
            if seen.insert(key) {
                brands.push(camera.brand.clone());
            }
        }
        brands
    }

    pub fn models(&self, brand: &str) -> Vec<String> {
        let brand = normalize(brand);
        self.cameras
            .iter()
            .filter(|camera| normalize(&camera.brand) == brand)
            .map(|camera| camera.model.clone())
            .collect()
    }

    pub fn lenses(&self, brand: &str, model: &str) -> Vec<String> {
        self.find(brand, model)
            .map(|camera| camera.lenses.clone())
            .unwrap_or_default()
    }

    pub fn find(&self, brand: &str, model: &str) -> Option<&CameraRecord> {
        let brand = normalize(brand);
        let model = normalize(model);
        self.cameras
            .iter()
            .find(|camera| normalize(&camera.brand) == brand && normalize(&camera.model) == model)
    }

    pub fn contains_lens(&self, brand: &str, model: &str, lens: &str) -> bool {
        let lens = normalize(lens);
        self.find(brand, model)
            .is_some_and(|camera| camera.lenses.iter().any(|known| normalize(known) == lens))
    }

    pub fn compatible_cameras(&self, brand: &str, model: &str) -> Vec<&CameraRecord> {
        let Some(source) = self.find(brand, model) else {
            return Vec::new();
        };
        let Some(source_crop) = source.crop_factor.filter(|v| v.is_finite() && *v > 0.0) else {
            return Vec::new();
        };
        let source_brand = normalize(&source.brand);
        let source_model = normalize(&source.model);

        self.cameras
            .iter()
            .filter(|camera| {
                if normalize(&camera.brand) != source_brand
                    || normalize(&camera.model) == source_model
                {
                    return false;
                }
                camera.crop_factor.is_some_and(|crop| {
                    crop.is_finite()
                        && crop > 0.0
                        && ((crop - source_crop).abs() / source_crop) <= 0.01
                })
            })
            .collect()
    }

    pub fn submission_errors(&self, profile: &LensProfile) -> Vec<String> {
        let mut errors = Vec::new();
        if profile.camera_brand.trim().is_empty() {
            errors.push("Camera brand is required.".to_owned());
        }
        if profile.camera_model.trim().is_empty() {
            errors.push("Camera model is required.".to_owned());
        }
        if profile.lens_model.trim().is_empty() {
            errors.push("Lens model or built-in lens mode is required.".to_owned());
        }

        let lens = profile.lens_model.trim();
        let looks_like_zoom = lens.contains('-') && lens.chars().any(|c| c.is_ascii_digit());
        if looks_like_zoom
            && !profile
                .focal_length
                .is_some_and(|v| v.is_finite() && v > 0.0)
        {
            errors.push("Zoom-lens profiles require a specific focal length.".to_owned());
        }
        errors
    }
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(
        brand: &str,
        model: &str,
        lens: &str,
        crop: Option<f64>,
        official: bool,
    ) -> LensProfile {
        let mut profile = LensProfile::default();
        profile.camera_brand = brand.into();
        profile.camera_model = model.into();
        profile.lens_model = lens.into();
        profile.crop_factor = crop;
        profile.official = official;
        profile
    }

    #[test]
    fn aggregates_case_insensitive_camera_and_lens_names() {
        let profiles = vec![
            profile("Sony", "A7 IV", "FE 24-70mm F2.8", Some(1.0), true),
            profile("sony", "a7 iv", "FE 24-70MM F2.8", None, false),
            profile("Sony", "A7 IV", "FE 35mm F1.8", None, false),
        ];
        let catalogue = CameraCatalogue::from_profiles(&profiles);

        assert_eq!(catalogue.cameras.len(), 1);
        let camera = &catalogue.cameras[0];
        assert_eq!(camera.brand, "Sony");
        assert_eq!(camera.model, "A7 IV");
        assert_eq!(camera.crop_factor, Some(1.0));
        assert!(camera.has_official_profile);
        assert_eq!(camera.lenses.len(), 2);
    }

    #[test]
    fn skips_copies_and_incomplete_profiles() {
        let mut copy = profile("Sony", "A7 IV", "FE 35mm F1.8", Some(1.0), true);
        copy.is_copy = true;
        let profiles = vec![
            copy,
            profile("", "A7 IV", "FE 35mm F1.8", None, false),
            profile("Sony", "", "FE 35mm F1.8", None, false),
        ];

        assert!(CameraCatalogue::from_profiles(&profiles).cameras.is_empty());
    }

    #[test]
    fn enriches_from_separately_licensed_lensfun_metadata() {
        let profiles = vec![profile("Sony", "A7 IV", "FE 35mm F1.8", Some(1.0), true)];
        let mut catalogue = CameraCatalogue::from_profiles(&profiles);
        let data = r#"{"source":"Lensfun","source_revision":"abc","license":"CC BY-SA 3.0","cameras":[{"brand":"Sony","model":"A7 IV","mount":"Sony E","crop_factor":1.0},{"brand":"Sony","model":"A7S III","mount":"Sony E","crop_factor":1.0}],"lenses":[{"brand":"Sigma","model":"24-70mm F2.8","mounts":["Sony E"]}]}"#;
        catalogue.enrich_from_lensfun_json(data).unwrap();
        let a7 = catalogue.find("sony", "a7 iv").unwrap();
        assert_eq!(a7.mount, "Sony E");
        assert!(a7.metadata_sources.iter().any(|x| x.contains("Lensfun")));
        assert!(a7.lenses.iter().any(|x| x == "Sigma 24-70mm F2.8"));
        assert!(catalogue.find("Sony", "A7S III").is_some());
    }

    #[test]
    fn ignores_unlabelled_external_metadata() {
        let mut catalogue = CameraCatalogue::default();
        catalogue.enrich_from_lensfun_json(r#"{"source":"Unknown","source_revision":"abc","license":"Proprietary","cameras":[],"lenses":[]}"#).unwrap();
        assert!(catalogue.cameras.is_empty());
    }

    #[test]
    fn recommends_same_brand_cameras_with_matching_crop_factor() {
        let profiles = vec![
            profile("Sony", "A7 IV", "FE 35mm F1.8", Some(1.0), true),
            profile("Sony", "A7S III", "FE 24mm F1.4", Some(1.005), true),
            profile("Sony", "A6700", "E 16-50mm", Some(1.5), true),
            profile("Canon", "R5", "RF 24-105mm", Some(1.0), true),
        ];
        let catalogue = CameraCatalogue::from_profiles(&profiles);
        let compatible = catalogue.compatible_cameras("sony", "a7 iv");
        assert_eq!(compatible.len(), 1);
        assert_eq!(compatible[0].model, "A7S III");
    }

    #[test]
    fn rejects_ambiguous_submission_metadata() {
        let catalogue = CameraCatalogue::default();
        let mut incomplete = LensProfile::default();
        incomplete.lens_model = "24-70mm".into();
        let errors = catalogue.submission_errors(&incomplete);
        assert!(errors.iter().any(|x| x.contains("Camera brand")));
        assert!(errors.iter().any(|x| x.contains("Camera model")));
        assert!(errors.iter().any(|x| x.contains("focal length")));

        incomplete.camera_brand = "Sony".into();
        incomplete.camera_model = "A7 IV".into();
        incomplete.focal_length = Some(35.0);
        assert!(catalogue.submission_errors(&incomplete).is_empty());
    }

    #[test]
    fn exposes_selector_lists_case_insensitively() {
        let profiles = vec![
            profile("Sony", "A7 IV", "FE 35mm F1.8", Some(1.0), true),
            profile("Sony", "A6700", "E 16-50mm", Some(1.5), true),
            profile("Canon", "R5", "RF 24-105mm", Some(1.0), true),
        ];
        let catalogue = CameraCatalogue::from_profiles(&profiles);

        assert_eq!(catalogue.brands(), vec!["Canon", "Sony"]);
        assert_eq!(catalogue.models("SONY"), vec!["A6700", "A7 IV"]);
        assert!(catalogue.contains_lens("sony", "a7 iv", "fe 35MM f1.8"));
        assert!(!catalogue.contains_lens("sony", "a7 iv", "RF 24-105mm"));
    }
}
