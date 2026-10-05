// SPDX-License-Identifier: GPL-3.0-or-later
//! Metadata shared by the selectors and the upload boundary. Lensfun entries
//! deliberately do not implement `LensProfile` and cannot be loaded as geometry.

use serde_json::{json, Value};
use std::path::Path;
use std::sync::LazyLock;

const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;
static ZOOM_NAME: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*[-–—]\s*(\d+(?:\.\d+)?)\s*mm\b").unwrap()
});

pub fn parse_catalog(text: &str) -> Result<Value, String> {
    if text.len() as u64 > MAX_CATALOG_BYTES {
        return Err("Camera catalogue is too large".into());
    }
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if value["schema_version"].as_u64() != Some(1) {
        return Err("Unsupported camera catalogue version".into());
    }
    for collection in ["cameras", "lenses"] {
        let rows = value[collection].as_array().ok_or("Invalid camera catalogue collection")?;
        for row in rows {
            for field in ["brand", "model"] {
                if row[field].as_str().is_none_or(|s| s.trim().is_empty()) {
                    return Err("Camera catalogue entry is missing its identity".into());
                }
            }
            for field in ["mounts", "aliases"] {
                if let Some(v) = row.get(field) {
                    if !v.as_array().is_some_and(|a| a.iter().all(Value::is_string)) {
                        return Err("Invalid camera catalogue aliases or mounts".into());
                    }
                }
            }
        }
    }
    Ok(value)
}

pub fn load_catalog(bundled: &str, cached: &Path) -> Value {
    // Failed/incompatible updates retain the bundled catalogue; never turn a
    // network/parse failure into an empty list of known cameras.
    if std::fs::metadata(cached).is_ok_and(|m| m.len() <= MAX_CATALOG_BYTES) {
        if let Ok(text) = std::fs::read_to_string(cached) {
            if let Ok(value) = parse_catalog(&text) {
                return value;
            }
        }
    }
    parse_catalog(bundled).unwrap_or_else(|_| json!({"schema_version":1,"cameras":[],"lenses":[],"mounts":{}}))
}

fn meaningful(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        let s = s.trim().to_lowercase();
        !s.is_empty() && !["---", "?", "unknown", "other", "n/a", "none"].contains(&s.as_str())
    })
}

fn positive(value: &Value) -> bool {
    value.as_f64().is_some_and(|v| v.is_finite() && v > 0.0)
}

pub fn submission_errors(info: &Value, catalog: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    for (key, label) in [("camera_brand", "camera brand"), ("camera_model", "camera model"), ("lens_model", "lens model (or an explicit built-in lens description)")] {
        if !meaningful(&info[key]) {
            errors.push(format!("Enter a specific {label} before uploading."));
        }
    }
    let lens = info["lens_model"].as_str().unwrap_or("");
    let named_zoom = ZOOM_NAME.captures(lens).is_some_and(|c| {
        matches!((c[1].parse::<f64>(), c[2].parse::<f64>()), (Ok(a), Ok(b)) if a > 0.0 && b > a)
    });
    let catalog_zoom = catalog["lenses"].as_array().is_some_and(|lenses| lenses.iter().any(|entry| {
        let name_matches = entry["model"].as_str().is_some_and(|name| name.trim().eq_ignore_ascii_case(lens.trim()))
            || entry["aliases"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str().is_some_and(|s| s.trim().eq_ignore_ascii_case(lens.trim()))));
        let range_zoom = matches!((entry["min_focal_length"].as_f64(), entry["max_focal_length"].as_f64()), (Some(a), Some(b)) if a > 0.0 && b > a);
        let sampled_zoom = entry["sampled_focal_lengths"].as_array().is_some_and(|a| {
            let positive: Vec<_> = a.iter().filter_map(Value::as_f64).filter(|n| n.is_finite() && *n > 0.0).collect();
            positive.first().is_some_and(|first| positive.iter().any(|v| (*v - *first).abs() > 0.01))
        });
        name_matches && (range_zoom || sampled_zoom)
    }));
    if (named_zoom || catalog_zoom || info["lens_is_zoom"].as_bool() == Some(true)) && !positive(&info["focal_length"]) {
        errors.push("Specify the actual focal length used to calibrate this zoom lens before uploading.".into());
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_catalog_is_not_an_empty_success() {
        assert!(parse_catalog(r#"{"schema_version":2,"cameras":[],"lenses":[]}"#).is_err());
        assert!(parse_catalog(r#"{"schema_version":1,"cameras":[{}],"lenses":[]}"#).is_err());
        assert!(parse_catalog(r#"{"schema_version":1,"cameras":[],"lenses":[]}"#).is_ok());
    }
    #[test]
    fn upload_requires_an_identifiable_setup_and_zoom_focal_length() {
        let catalog = json!({"lenses":[]});
        let mut info = json!({"camera_brand":"Sony","camera_model":"ILCE-7M4","lens_model":"FE 24–70mm F2.8"});
        assert_eq!(submission_errors(&info, &catalog).len(), 1);
        info["focal_length"] = json!(35.0);
        assert!(submission_errors(&info, &catalog).is_empty());
        info["camera_model"] = json!("  --- ");
        assert_eq!(submission_errors(&info, &catalog).len(), 1);
    }
    #[test]
    fn catalog_detects_zoom_without_a_range_in_the_name() {
        let catalog = json!({"lenses":[{"model":"Power Zoom","min_focal_length":10.0,"max_focal_length":30.0}]});
        let mut info = json!({"camera_brand":"Example","camera_model":"Body","lens_model":"Power Zoom","focal_length":0});
        assert_eq!(submission_errors(&info, &catalog).len(), 1);
        info["focal_length"] = json!(20.0);
        assert!(submission_errors(&info, &catalog).is_empty());
    }
}
