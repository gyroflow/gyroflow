// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Gyroflow contributors

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LensProfileSubmission {
    pub camera_brand: String,
    pub camera_model: String,
    pub lens_brand: String,
    pub lens_model: String,
    pub focal_length: Option<f64>,
    pub is_zoom_lens: bool,
    pub sensor_format: Option<String>,
    pub input_resolution: (u32, u32),
    pub distortion_coefficients: Vec<f64>,
    pub optical_center: (f64, f64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub struct ProfileValidator;

impl ProfileValidator {
    pub fn validate(sub: &LensProfileSubmission) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // 1. Camera brand & model validation
        let trimmed_brand = sub.camera_brand.trim();
        let trimmed_model = sub.camera_model.trim();

        if trimmed_brand.is_empty() {
            errors.push("Camera brand is required and cannot be empty.".to_string());
        }
        if trimmed_model.is_empty() {
            errors.push("Camera model is required and cannot be empty.".to_string());
        }
        if trimmed_brand.eq_ignore_ascii_case("other") || trimmed_model.eq_ignore_ascii_case("other") {
            errors.push("Specific camera brand and model must be specified instead of 'Other'.".to_string());
        }

        // 2. Lens validation
        let trimmed_lens_model = sub.lens_model.trim();
        let is_integrated_or_action = trimmed_brand.eq_ignore_ascii_case("gopro")
            || trimmed_brand.eq_ignore_ascii_case("dji")
            || trimmed_brand.eq_ignore_ascii_case("insta360");

        if !is_integrated_or_action && trimmed_lens_model.is_empty() {
            errors.push("Interchangeable lens cameras must have a specific lens model defined.".to_string());
        }

        // 3. Zoom lens check: must have explicit focal length
        if sub.is_zoom_lens {
            match sub.focal_length {
                None => {
                    errors.push("For zoom lenses, the exact calibrated focal length (in mm) must be specified.".to_string());
                }
                Some(fl) if fl <= 0.0 => {
                    errors.push("Calibrated focal length must be a positive number.".to_string());
                }
                _ => {}
            }
        }

        // 4. Optical parameters sanity check
        if sub.input_resolution.0 == 0 || sub.input_resolution.1 == 0 {
            errors.push("Input resolution width and height must be non-zero.".to_string());
        }

        if sub.distortion_coefficients.is_empty() {
            errors.push("Distortion coefficients are missing.".to_string());
        }

        if sub.optical_center.0 <= 0.0 || sub.optical_center.1 <= 0.0 {
            warnings.push("Optical center appears uninitialized or out of bounds.".to_string());
        }

        ValidationResult {
            is_valid: errors.is_empty(),
            errors,
            warnings,
        }
    }
}
