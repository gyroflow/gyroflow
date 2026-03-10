// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use crate::camera_database::{CameraMetadata, LensMetadata, CameraLensList};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LensFunCamera {
    pub maker: String,
    pub model: String,
    pub mount: Option<String>,
    pub cropfactor: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LensFunLens {
    pub maker: String,
    pub model: String,
    pub mount: Option<String>,
    pub cropfactor: Option<f64>,
    pub focal: Option<Vec<f64>>, // [min, max] in mm
    pub aperture: Option<Vec<f64>>, // [min, max]
}

/// Parse LensFun XML database and convert to Gyroflow format
/// This is a basic implementation that can be extended to parse actual LensFun XML files
pub struct LensFunParser;

impl LensFunParser {
    /// Parse LensFun XML file (placeholder - would need XML parsing library)
    /// For now, returns empty data structure
    pub fn parse_xml(_xml_path: &str) -> Result<(Vec<LensFunCamera>, Vec<LensFunLens>), Box<dyn std::error::Error>> {
        // TODO: Implement actual XML parsing
        // This would require adding an XML parsing dependency like quick-xml or xml-rs
        // For now, return empty vectors
        Ok((Vec::new(), Vec::new()))
    }

    /// Convert LensFun cameras to Gyroflow CameraMetadata
    pub fn convert_cameras(lensfun_cameras: Vec<LensFunCamera>) -> Vec<CameraMetadata> {
        lensfun_cameras.into_iter().map(|cam| {
            CameraMetadata {
                brand: cam.maker,
                model: cam.model,
                mount: cam.mount,
                sensor_width: None, // Would need to calculate from crop factor
                sensor_height: None,
                crop_factor: cam.cropfactor,
                full_frame: cam.cropfactor.map(|cf| (cf - 1.0).abs() < 0.1).unwrap_or(false),
            }
        }).collect()
    }

    /// Convert LensFun lenses to Gyroflow LensMetadata
    pub fn convert_lenses(lensfun_lenses: Vec<LensFunLens>) -> Vec<LensMetadata> {
        lensfun_lenses.into_iter().map(|lens| {
            let (min_fl, max_fl) = lens.focal
                .map(|f| {
                    if f.len() >= 2 {
                        (Some(f[0]), Some(f[1]))
                    } else if f.len() == 1 {
                        (Some(f[0]), Some(f[0]))
                    } else {
                        (None, None)
                    }
                })
                .unwrap_or((None, None));

            let is_zoom = min_fl.zip(max_fl)
                .map(|(min, max)| (max - min).abs() > 0.1)
                .unwrap_or(false);

            LensMetadata {
                brand: lens.maker,
                model: lens.model,
                mount: lens.mount,
                min_focal_length: min_fl,
                max_focal_length: max_fl,
                is_zoom,
            }
        }).collect()
    }

    /// Merge LensFun data into existing CameraLensList
    pub fn merge_into_list(
        mut list: CameraLensList,
        lensfun_cameras: Vec<LensFunCamera>,
        lensfun_lenses: Vec<LensFunLens>
    ) -> CameraLensList {
        // Convert and add cameras
        let cameras = Self::convert_cameras(lensfun_cameras);
        for cam in cameras {
            if !list.brands.contains(&cam.brand) {
                list.brands.push(cam.brand.clone());
            }
            let brand_entry = list.cameras.entry(cam.brand.clone()).or_insert_with(Vec::new);
            if !brand_entry.contains(&cam.model) {
                brand_entry.push(cam.model);
            }
        }

        // Convert and add lenses
        let lenses = Self::convert_lenses(lensfun_lenses);
        for lens in lenses {
            if !list.brands.contains(&lens.brand) {
                list.brands.push(lens.brand.clone());
            }
            let brand_entry = list.lenses.entry(lens.brand.clone()).or_insert_with(Vec::new);
            
            // Check if lens already exists
            if !brand_entry.iter().any(|l| l.model == lens.model) {
                brand_entry.push(lens);
            }
        }

        // Sort brands
        list.brands.sort();

        list
    }
}

/// Try to find LensFun database path on the system
pub fn find_lensfun_database() -> Option<std::path::PathBuf> {
    // Common LensFun database locations
    let candidates = [
        #[cfg(target_os = "linux")]
        std::path::PathBuf::from("/usr/share/lensfun/version_1"),
        #[cfg(target_os = "linux")]
        std::path::PathBuf::from("/usr/local/share/lensfun/version_1"),
        #[cfg(target_os = "macos")]
        std::path::PathBuf::from("/opt/homebrew/share/lensfun/version_1"),
        #[cfg(target_os = "macos")]
        std::path::PathBuf::from("/usr/local/share/lensfun/version_1"),
        #[cfg(target_os = "windows")]
        std::path::PathBuf::from("C:/Program Files/Lensfun/version_1"),
    ];

    for path in &candidates {
        if path.exists() {
            return Some(path.clone());
        }
    }

    None
}
