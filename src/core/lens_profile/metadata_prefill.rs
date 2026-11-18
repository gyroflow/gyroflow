// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Gyroflow contributors

use super::camera_db::CAMERA_DB;

#[derive(Debug, Clone, Default)]
pub struct ExtractedVideoMetadata {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens_model: Option<String>,
    pub focal_length: Option<f64>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default)]
pub struct PrefillRecommendation {
    pub camera_brand: String,
    pub camera_model: String,
    pub lens_brand: String,
    pub lens_model: String,
    pub focal_length: f64,
    pub is_matched: bool,
    pub compatible_models: Vec<String>,
}

pub struct MetadataPrefillResolver;

impl MetadataPrefillResolver {
    pub fn resolve(meta: &ExtractedVideoMetadata) -> PrefillRecommendation {
        let db = CAMERA_DB.read().unwrap();
        let mut rec = PrefillRecommendation::default();

        let brand_query = meta.make.as_deref().unwrap_or("").trim();
        let model_query = meta.model.as_deref().unwrap_or("").trim();

        // Match camera
        let matched_cam = if !brand_query.is_empty() && !model_query.is_empty() {
            db.find_camera(brand_query, model_query)
        } else if !model_query.is_empty() {
            db.match_camera_by_name(model_query)
        } else {
            None
        };

        if let Some(cam) = matched_cam {
            rec.camera_brand = cam.brand.clone();
            rec.camera_model = cam.model.clone();
            rec.is_matched = true;

            // Fetch compatible cameras (e.g. Sony A7S3 -> FX3, ZV-E1)
            let comp = db.get_compatible_cameras(cam);
            rec.compatible_models = comp.into_iter()
                .filter(|c| c.model != cam.model)
                .map(|c| format!("{} {}", c.brand, c.model))
                .collect();
        } else {
            rec.camera_brand = brand_query.to_string();
            rec.camera_model = model_query.to_string();
        }

        // Lens matching
        if let Some(lens_str) = &meta.lens_model {
            let lens_trimmed = lens_str.trim();
            rec.lens_model = lens_trimmed.to_string();

            // Attempt to derive lens brand
            for b in &["Sony", "Canon", "Nikon", "Sigma", "Tamron", "Fujifilm", "Panasonic", "Laowa", "Zeiss", "Samyang", "Rokinon", "DJI", "GoPro"] {
                if lens_trimmed.to_lowercase().starts_with(&b.to_lowercase()) {
                    rec.lens_brand = b.to_string();
                    break;
                }
            }
            if rec.lens_brand.is_empty() && !rec.camera_brand.is_empty() {
                rec.lens_brand = rec.camera_brand.clone();
            }
        }

        if let Some(fl) = meta.focal_length {
            rec.focal_length = fl;
        }

        rec
    }
}
