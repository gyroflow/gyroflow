// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::{ HashSet, BTreeMap };
use itertools::Itertools;

use serde::{ Serialize, Deserialize };

use crate::stabilization::distortion_models::DistortionModel;
use crate::stabilization_params::ReadoutDirection;

// Import the specific distortion models for coefficient rescaling
use crate::stabilization::distortion_models::{poly3::Poly3, poly5::Poly5, ptlens::PtLens};

#[cfg(feature = "opencv")]
use super::LensCalibrator;

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
pub struct Dimensions { pub w: usize, pub h: usize }

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
#[allow(non_snake_case)]
pub struct CameraParams { pub RMS_error: f64, pub camera_matrix: Vec<[f64; 3]>, pub distortion_coeffs: Vec<f64>, pub radial_distortion_limit: Option<f64> }

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct LensProfile {
    pub name: String,
    pub note: String,
    pub calibrated_by: String,
    pub camera_brand: String,
    pub camera_model: String,
    pub lens_model: String,
    pub camera_setting: String,

    pub calib_dimension: Dimensions,
    pub orig_dimension: Dimensions,

    pub output_dimension: Option<Dimensions>,

    pub frame_readout_time: Option<f64>,
    pub frame_readout_direction: Option<ReadoutDirection>,
    pub gyro_lpf: Option<f64>,

    pub input_horizontal_stretch: f64,
    pub input_vertical_stretch: f64,
    pub num_images: usize,

    pub fps: f64,

    pub crop: Option<f64>,

    pub official: bool,

    pub asymmetrical: bool,

    pub fisheye_params: CameraParams,

    pub identifier: String,

    pub calibrator_version: String,
    pub date: String,

    pub compatible_settings: Vec<serde_json::Value>,

    pub sync_settings: Option<serde_json::Value>,

    pub rig: Option<serde_json::Value>,

    pub distortion_model: Option<String>,
    pub digital_lens: Option<String>,
    pub digital_lens_params: Option<Vec<f64>>,

    pub interpolations: Option<serde_json::Value>,

    pub focal_length: Option<f64>,
    pub crop_factor: Option<f64>,
    pub global_shutter: bool,

    // Skip these fields, make sure to update in `get_json_value`
    pub path_to_file: String,
    pub optimal_fov: Option<f64>,
    pub is_copy: bool,
    pub rating: Option<f64>,
    pub checksum: Option<String>,
    parsed_interpolations: BTreeMap<i64, LensProfile>,
}

impl LensProfile {
    pub fn init(&mut self) {
        if !self.fisheye_params.distortion_coeffs.is_empty() && !self.distortion_model.as_deref().is_some_and(|x| x == "gopro") {
            let distortion_model = DistortionModel::from_name(self.distortion_model.as_deref().unwrap_or("opencv_fisheye"));
            self.fisheye_params.radial_distortion_limit = distortion_model.radial_distortion_limit(&self.get_distortion_coeffs());
        }

        // ----- New code: rescale coefficients for LensFun models -----
        // LensFun stores coefficients assuming a 35 mm‑equivalent focal length. When a profile
        // provides a real focal length (or a crop factor) we need to convert the coefficients to
        // the scale used by Gyroflow. The logic mirrors the comment in `poly3.rs`.
        if let Some(model) = &self.distortion_model {
            // Only the three physical models need rescaling.
            match model.as_str() {
                "poly3" | "poly5" | "ptlens" => {
                    // Real focal length in mm – if not supplied we cannot rescale.
                    if let Some(real_focal) = self.focal_length {
                        let crop = self.crop_factor.unwrap_or(1.0);
                        // Aspect ratio of the calibration image.
                        let aspect = if self.calib_dimension.h != 0 {
                            self.calib_dimension.w as f64 / self.calib_dimension.h as f64
                        } else {
                            1.0
                        };
                        // Hugin scaling factor (see LensFun implementation).
                        let hugin_scale_mm = (36.0_f64.hypot(24.0)) / crop / (aspect.hypot(1.0)) / 2.0;
                        let hugin_scaling = real_focal / hugin_scale_mm;

                        match model.as_str() {
                            "poly3" => Poly3::rescale_coeffs(&mut self.fisheye_params.distortion_coeffs, hugin_scaling),
                            "poly5" => Poly5::rescale_coeffs(&mut self.fisheye_params.distortion_coeffs, hugin_scaling),
                            "ptlens" => PtLens::rescale_coeffs(&mut self.fisheye_params.distortion_coeffs, hugin_scaling),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        // ------------------------------------------------------------
    }

    pub fn from_value(json: serde_json::Value) -> Result<Self, serde_json::Error> {
        let mut lens: Result<Self, serde_json::Error> = serde_json::from_value(json);
        if let Ok(ref mut lens) = lens { lens.init(); }
        lens
    }
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let mut lens: Result<Self, serde_json::Error> = serde_json::from_str(json);
        if let Ok(ref mut lens) = lens { lens.init(); }
        lens
    }

    // ... rest of the file unchanged ...
    // (the rest of the original content is retained unchanged)
}
