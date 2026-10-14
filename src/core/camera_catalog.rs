// SPDX-License-Identifier: GPL-3.0-or-later
//! Strongly-typed camera catalog database and upload validation models.
//!
//! Lensfun and camera catalog entries serve as standardized metadata for
//! UI selectors and calibration validation; they deliberately do not implement
//! `LensProfile` and cannot be loaded as geometry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;

static ZOOM_NAME: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*[-–—]\s*(\d+(?:\.\d+)?)\s*mm\b").unwrap()
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CatalogSources {
    #[serde(default)]
    pub gyroflow_revision: String,
    #[serde(default)]
    pub lensfun_revision: String,
    #[serde(default)]
    pub input_sha256: String,
    #[serde(default)]
    pub profile_count: usize,
    #[serde(default)]
    pub lensfun_xml_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CameraEntry {
    pub brand: String,
    pub model: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub mounts: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop_factor: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equivalent_sensor_diagonal_mm: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LensEntry {
    pub brand: String,
    pub model: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub mounts: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop_factor: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_focal_length: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_focal_length: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sampled_focal_lengths: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProfileUiEntry {
    pub id: String,
    pub name: String,
    pub brand: String,
    pub model: String,
    pub lens: String,
    pub checksum: String,
    pub official: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    pub author: String,
    pub width: usize,
    pub height: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focal_length: Option<f64>,
    pub fps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CameraCatalog {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<CatalogSources>,
    #[serde(default)]
    pub cameras: Vec<CameraEntry>,
    #[serde(default)]
    pub lenses: Vec<LensEntry>,
    #[serde(default)]
    pub mounts: HashMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<ProfileUiEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubmissionInfo {
    #[serde(default)]
    pub camera_brand: Option<String>,
    #[serde(default)]
    pub camera_model: Option<String>,
    #[serde(default)]
    pub lens_model: Option<String>,
    #[serde(default)]
    pub focal_length: Option<f64>,
    #[serde(default)]
    pub lens_is_zoom: Option<bool>,
}

fn is_meaningful(val: Option<&str>) -> bool {
    if let Some(s) = val {
        let s = s.trim().to_lowercase();
        !s.is_empty() && !["---", "?", "unknown", "other", "n/a", "none"].contains(&s.as_str())
    } else {
        false
    }
}

fn parse_named_zoom_range(lens: &str) -> Option<(f64, f64)> {
    let c = ZOOM_NAME.captures(lens)?;
    let min = c.get(1)?.as_str().parse::<f64>().ok()?;
    let max = c.get(2)?.as_str().parse::<f64>().ok()?;
    if min.is_finite() && max.is_finite() && min > 0.0 && max > min {
        Some((min, max))
    } else {
        None
    }
}

impl CameraCatalog {
    pub fn is_empty(&self) -> bool {
        self.cameras.is_empty() && self.lenses.is_empty()
    }

    /// A schema-valid but empty auto-update is not a usable camera/lens database.
    /// Keep parsing permissive for tooling, but never promote empty metadata over
    /// the bundled catalogue that drives user-visible camera/lens selectors.
    pub fn has_deliverable_metadata(&self) -> bool {
        !self.cameras.is_empty() || !self.lenses.is_empty()
    }

    pub fn submission_errors(&self, info: &SubmissionInfo) -> Vec<String> {
        let mut errors = Vec::new();
        if !is_meaningful(info.camera_brand.as_deref()) {
            errors.push("Enter a specific camera brand before uploading.".into());
        }
        if !is_meaningful(info.camera_model.as_deref()) {
            errors.push("Enter a specific camera model before uploading.".into());
        }
        if !is_meaningful(info.lens_model.as_deref()) {
            errors.push("Enter a specific lens model (or an explicit built-in lens description) before uploading.".into());
        }

        let lens = info.lens_model.as_deref().unwrap_or("").trim();
        let named_range = parse_named_zoom_range(lens);
        let mut catalog_zoom = false;
        let mut catalog_ranges = Vec::new();

        for entry in &self.lenses {
            let name_matches = entry.model.trim().eq_ignore_ascii_case(lens)
                || entry.aliases.iter().any(|alias| alias.trim().eq_ignore_ascii_case(lens));
            if !name_matches {
                continue;
            }
            if let (Some(min), Some(max)) = (entry.min_focal_length, entry.max_focal_length) {
                if min.is_finite() && max.is_finite() && min > 0.0 && max > min {
                    catalog_ranges.push((min, max));
                    catalog_zoom = true;
                }
            }
            if !entry.sampled_focal_lengths.is_empty() {
                let positive_samples: Vec<f64> = entry.sampled_focal_lengths.iter().copied()
                    .filter(|n| n.is_finite() && *n > 0.0).collect();
                if let Some(&first) = positive_samples.first() {
                    if positive_samples.iter().any(|&v| (v - first).abs() > 0.01) {
                        catalog_zoom = true;
                    }
                }
            }
        }

        let focal = info.focal_length.filter(|v| v.is_finite() && *v > 0.0);
        let is_zoom = named_range.is_some() || catalog_zoom || info.lens_is_zoom == Some(true);

        if is_zoom && focal.is_none() {
            errors.push("Specify the actual focal length used to calibrate this zoom lens before uploading.".into());
        }

        if let Some(focal) = focal {
            let allowed = if let Some((min, max)) = named_range {
                Some(focal >= min && focal <= max)
            } else if !catalog_ranges.is_empty() {
                Some(catalog_ranges.iter().any(|&(min, max)| focal >= min && focal <= max))
            } else {
                None
            };
            if allowed == Some(false) {
                errors.push("The focal length is outside the selected zoom lens's documented range.".into());
            }
        }

        errors
    }
}

pub fn parse_catalog(text: &str) -> Result<CameraCatalog, String> {
    if text.len() as u64 > MAX_CATALOG_BYTES {
        return Err("Camera catalogue is too large".into());
    }
    let catalog: CameraCatalog = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if catalog.schema_version != 1 {
        return Err("Unsupported camera catalogue version".into());
    }
    for cam in &catalog.cameras {
        if cam.brand.trim().is_empty() || cam.model.trim().is_empty() {
            return Err("Camera catalogue entry is missing its identity".into());
        }
    }
    for lens in &catalog.lenses {
        if lens.brand.trim().is_empty() || lens.model.trim().is_empty() {
            return Err("Camera catalogue entry is missing its identity".into());
        }
    }
    Ok(catalog)
}

pub fn has_deliverable_metadata(catalog: &CameraCatalog) -> bool {
    catalog.has_deliverable_metadata()
}

pub fn load_catalog(bundled: &str, cached: &Path) -> CameraCatalog {
    if std::fs::metadata(cached).is_ok_and(|m| m.len() <= MAX_CATALOG_BYTES) {
        if let Ok(text) = std::fs::read_to_string(cached) {
            if let Ok(catalog) = parse_catalog(&text) {
                if catalog.has_deliverable_metadata() {
                    return catalog;
                }
            }
        }
    }
    parse_catalog(bundled).unwrap_or_else(|_| CameraCatalog {
        schema_version: 1,
        ..Default::default()
    })
}

pub fn submission_errors(info: &SubmissionInfo, catalog: &CameraCatalog) -> Vec<String> {
    catalog.submission_errors(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_catalog_is_not_an_empty_success() {
        assert!(parse_catalog(r#"{"schema_version":2,"cameras":[],"lenses":[]}"#).is_err());
        assert!(parse_catalog(r#"{"schema_version":1,"cameras":[{}],"lenses":[]}"#).is_err());
        assert!(parse_catalog(r#"{"schema_version":1,"cameras":[{"brand":"Sony","model":""}],"lenses":[]}"#).is_err());
        assert!(parse_catalog(r#"{"schema_version":1,"cameras":[],"lenses":[]}"#).is_ok());
    }

    #[test]
    fn valid_but_empty_cached_catalog_cannot_replace_bundled_metadata() {
        let empty = r#"{"schema_version":1,"cameras":[],"lenses":[]}"#;
        let bundled = r#"{"schema_version":1,"cameras":[{"brand":"Sony","model":"Test Camera"}],"lenses":[]}"#;
        let empty_catalog = parse_catalog(empty).unwrap();
        assert!(!empty_catalog.has_deliverable_metadata());

        let tmp = std::env::temp_dir().join(format!(
            "gyroflow-empty-camera-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::write(&tmp, empty).unwrap();
        let loaded = load_catalog(bundled, &tmp);
        std::fs::remove_file(&tmp).unwrap();
        assert_eq!(loaded.cameras[0].model, "Test Camera");

        std::fs::write(&tmp, bundled).unwrap();
        let loaded_cache = load_catalog(empty, &tmp);
        std::fs::remove_file(&tmp).unwrap();
        assert_eq!(loaded_cache.cameras[0].model, "Test Camera");
    }

    #[test]
    fn upload_requires_an_identifiable_setup_and_zoom_focal_length() {
        let catalog = CameraCatalog { schema_version: 1, ..Default::default() };
        let mut info = SubmissionInfo {
            camera_brand: Some("Sony".into()),
            camera_model: Some("ILCE-7M4".into()),
            lens_model: Some("FE 24–70mm F2.8".into()),
            ..Default::default()
        };
        assert_eq!(catalog.submission_errors(&info).len(), 1);
        info.focal_length = Some(35.0);
        assert!(catalog.submission_errors(&info).is_empty());
        info.camera_model = Some("  --- ".into());
        assert_eq!(catalog.submission_errors(&info).len(), 1);
    }

    #[test]
    fn catalog_detects_zoom_without_a_range_in_the_name() {
        let mut catalog = CameraCatalog { schema_version: 1, ..Default::default() };
        catalog.lenses.push(LensEntry {
            brand: "Sony".into(),
            model: "Power Zoom".into(),
            min_focal_length: Some(10.0),
            max_focal_length: Some(30.0),
            ..Default::default()
        });
        let mut info = SubmissionInfo {
            camera_brand: Some("Example".into()),
            camera_model: Some("Body".into()),
            lens_model: Some("Power Zoom".into()),
            focal_length: Some(0.0),
            ..Default::default()
        };
        assert_eq!(catalog.submission_errors(&info).len(), 1);
        info.focal_length = Some(20.0);
        assert!(catalog.submission_errors(&info).is_empty());
    }

    #[test]
    fn upload_rejects_named_zoom_out_of_range_but_accepts_endpoints() {
        let catalog = CameraCatalog { schema_version: 1, ..Default::default() };
        let mut info = SubmissionInfo {
            camera_brand: Some("Sony".into()),
            camera_model: Some("A7".into()),
            lens_model: Some("24–70mm Zoom".into()),
            focal_length: Some(200.0),
            ..Default::default()
        };
        assert_eq!(catalog.submission_errors(&info).len(), 1);
        for valid in [24.0, 70.0] {
            info.focal_length = Some(valid);
            assert!(catalog.submission_errors(&info).is_empty());
        }
        info.lens_model = Some("Unknown manual lens".into());
        info.focal_length = Some(200.0);
        assert!(catalog.submission_errors(&info).is_empty());
    }

    #[test]
    fn upload_rejects_catalog_alias_out_of_range_and_allows_other_matching_ranges() {
        let mut catalog = CameraCatalog { schema_version: 1, ..Default::default() };
        catalog.lenses.push(LensEntry {
            brand: "Example".into(),
            model: "PZ One".into(),
            aliases: vec!["My Zoom".into()],
            min_focal_length: Some(10.0),
            max_focal_length: Some(30.0),
            ..Default::default()
        });
        catalog.lenses.push(LensEntry {
            brand: "Example".into(),
            model: "PZ Two".into(),
            aliases: vec!["My Zoom".into()],
            min_focal_length: Some(20.0),
            max_focal_length: Some(80.0),
            ..Default::default()
        });
        catalog.lenses.push(LensEntry {
            brand: "Example".into(),
            model: "Unrelated".into(),
            min_focal_length: Some(100.0),
            max_focal_length: Some(400.0),
            ..Default::default()
        });

        let mut info = SubmissionInfo {
            camera_brand: Some("Example".into()),
            camera_model: Some("Body".into()),
            lens_model: Some("My Zoom".into()),
            focal_length: Some(90.0),
            ..Default::default()
        };
        assert_eq!(catalog.submission_errors(&info).len(), 1);
        info.focal_length = Some(80.0);
        assert!(catalog.submission_errors(&info).is_empty());
        info.focal_length = Some(10.0);
        assert!(catalog.submission_errors(&info).is_empty());
    }
}
