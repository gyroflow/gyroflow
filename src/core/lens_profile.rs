// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::HashSet;
use itertools::Itertools;

use serde::{ Serialize, Deserialize };
use super::zooming;

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
    pub gyro_lpf: Option<f64>,

    pub input_horizontal_stretch: f64,
    pub num_images: usize,

    pub fps: f64,

    pub official: bool,

    pub use_opencv_fisheye: bool,
    pub fisheye_params: CameraParams,

    pub use_opencv_standard: bool,
    pub calib_params: CameraParams,

    pub identifier: String,
    
    pub calibrator_version: String,
    pub date: String,

    pub compatible_settings: Vec<serde_json::Value>,

    #[serde(skip)]
    pub filename: String,

    #[serde(skip)]
    pub optimal_fov: Option<f64>,

    #[serde(skip)]
    pub is_copy: bool
}

impl LensProfile {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn load_from_file(&mut self, path: &str) -> Result<(), serde_json::Error> {
        let data = std::fs::read_to_string(path).map_err(|e| serde_json::Error::io(e))?;
        *self = Self::from_json(&data)?;

        // Trust lens profiles loaded from file
        self.official = true;

        if self.calibrator_version.is_empty() || self.fisheye_params.camera_matrix.is_empty() || self.calib_dimension.w <= 0 || self.calib_dimension.h <= 0 {
            return Err(serde_json::Error::io(std::io::ErrorKind::InvalidData.into()));
        }
        
        Ok(())
    }

    #[cfg(feature = "opencv")]
    pub fn set_from_calibrator(&mut self, cal: &LensCalibrator) {
        self.input_horizontal_stretch = 1.0;
        self.use_opencv_fisheye = true;
        self.calib_dimension = Dimensions { w: cal.width, h: cal.height };
        self.orig_dimension  = Dimensions { w: cal.width, h: cal.height };
        self.num_images = cal.used_points.len();

        self.fisheye_params = CameraParams {
            RMS_error: cal.rms,
            camera_matrix: cal.k.row_iter().map(|x| [x[0], x[1], x[2]]).collect(),
            distortion_coeffs: cal.d.as_slice().to_vec(),
            radial_distortion_limit: if cal.r_limit > 0.0 { Some(cal.r_limit) } else { None }
        };

        self.init();
    }

    pub fn init(&mut self) {
        self.calibrator_version = env!("CARGO_PKG_VERSION").to_string();
        self.date = chrono::Local::today().naive_local().to_string();
        self.name = self.get_name();
    }

    pub fn get_json_value(&self) -> Result<serde_json::Value, serde_json::error::Error> {
        Ok(serde_json::to_value(&self)?)
    }
    pub fn get_json(&self) -> Result<String, serde_json::error::Error> {
        Ok(serde_json::to_string_pretty(&self.get_json_value()?)?)
    }

    pub fn get_name(&self) -> String {
        let setting = if self.camera_setting.is_empty() { &self.note } else { &self.camera_setting };
        format!("{}_{}_{}_{}_{}_{}_{}x{}-{:.2}fps", self.camera_brand, self.camera_model, self.lens_model, setting, self.get_size_str(), self.get_aspect_ratio().replace(':', "by"), self.calib_dimension.w, self.calib_dimension.h, self.fps)
    }

    pub fn get_aspect_ratio(&self) -> String {
        if self.calib_dimension.w == 0 || self.calib_dimension.h == 0 {
            return String::new();
        }

        let ratios = [
            (1.0, "1:1"), 
            (3.0/2.0, "3:2"), (2.0/3.0, "2:3"), 
            (4.0/3.0, "4:3"), (3.0/4.0, "3:4"), 
            (16.0/9.0, "16:9"), (9.0/16.0, "9:16")
        ];
        let ratio = self.calib_dimension.w as f64 / self.calib_dimension.h as f64;
        let mut diffs = ratios.into_iter().map(|x| ((x.0 - ratio).abs(), x.1)).collect::<Vec<_>>();
        diffs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Less));
        let (lowest_diff, ratio_str) = *diffs.first().unwrap();
        if lowest_diff < 0.05 {
            return ratio_str.to_string();
        }

        let gcd = num::integer::gcd(self.calib_dimension.w, self.calib_dimension.h);

        let ratio1 = self.calib_dimension.w / gcd;
        let ratio2 = self.calib_dimension.h / gcd;
        
        if ratio1 >= 20 || ratio2 >= 20 {
            format!("{:.2}:1", ratio)
        } else {
            format!("{}:{}", ratio1, ratio2)
        }
    }
    pub fn get_size_str(&self) -> &'static str {
             if self.calib_dimension.w >= 8000 { "8k" }
        else if self.calib_dimension.w >= 6000 { "6k" }
        else if self.calib_dimension.w >= 5000 { "5k" }
        else if self.calib_dimension.w >  4000 { "C4k" }
        else if self.calib_dimension.w >= 3840 { "4k" }
        else if self.calib_dimension.w >= 2700 { "2.7k" }
        else if self.calib_dimension.w >= 2500 { "2.5k" }
        else if self.calib_dimension.w >= 2000 { "2k" }
        else if self.calib_dimension.w >= 1920 { "1080p" }
        else if self.calib_dimension.w >= 1280 { "720p" }
        else if self.calib_dimension.w >= 640  { "480p" }
        else { "" }
    }

    pub fn save_to_file(&mut self, path: &str) -> std::io::Result<String> {
        let json = self.get_json()?;

        std::fs::write(path, &json)?;

        Ok(json)
    }

    fn get_camera_matrix_internal(&self) -> Option<nalgebra::Matrix3<f64>> {
        if self.fisheye_params.camera_matrix.len() == 3 {
            let mut mat = nalgebra::Matrix3::from_rows(&[
                self.fisheye_params.camera_matrix[0].into(), 
                self.fisheye_params.camera_matrix[1].into(), 
                self.fisheye_params.camera_matrix[2].into()
            ]);
            mat[(0, 2)] = self.calib_dimension.w as f64 / 2.0;
            mat[(1, 2)] = self.calib_dimension.h as f64 / 2.0;
            Some(mat)
        } else {
            None
        }
    }
    pub fn get_camera_matrix(&mut self, size: (usize, usize), video_size: (usize, usize)) -> nalgebra::Matrix3<f64> {
        if self.fisheye_params.camera_matrix.len() == 3 {
            let mat = self.get_camera_matrix_internal().unwrap();

            if self.optimal_fov.is_none() {
                self.optimal_fov = Some(self.calculate_optimal_fov(video_size));
            }
            
            mat
        } else {
            // Default camera matrix
            let mut mat = nalgebra::Matrix3::<f64>::identity();
            mat[(0, 0)] = size.0 as f64 * 0.8;
            mat[(1, 1)] = size.0 as f64 * 0.8;
            mat[(0, 2)] = size.0 as f64 / 2.0;
            mat[(1, 2)] = size.1 as f64 / 2.0;
            mat
        }
    }
    pub fn get_distortion_coeffs(&self) -> nalgebra::Vector4<f64> {
        if self.fisheye_params.distortion_coeffs.len() != 4 {
            // Default coefficients
            return nalgebra::Vector4::new(0.25, 0.05, 0.5, -0.5);
        }
        nalgebra::Vector4::from_row_slice(&self.fisheye_params.distortion_coeffs)
    }

    pub fn load_from_json_value(&mut self, v: &serde_json::Value) -> Option<()> {
        *self = <Self as Deserialize>::deserialize(v).ok()?;
        Some(())
    }

    pub fn get_all_matching_profiles(&self) -> Vec<LensProfile> {
        let mut ret = Vec::with_capacity(self.compatible_settings.len() + 1);
        ret.push(self.clone());
        for x in &self.compatible_settings {
            let mut cpy = self.clone();
            cpy.compatible_settings.clear();
            if let Some(x) = x.as_object() {
                if x.contains_key("width") && x.contains_key("height") {
                    let (new_w, new_h) = (x["width"].as_u64().unwrap_or_default(), x["height"].as_u64().unwrap_or_default());
                    if new_w > 0 && new_h > 0 {
                        let ratio = new_w as f64 / cpy.calib_dimension.w as f64;
                        let scale = |val: &mut usize, pad: bool| {
                            *val = (*val as f64 * ratio).round() as usize;
                            if pad && *val % 2 != 0 { *val -= 1; }
                        };
                        scale(&mut cpy.calib_dimension.w, false);
                        scale(&mut cpy.calib_dimension.h, false);
                        scale(&mut cpy.orig_dimension.w, false);
                        scale(&mut cpy.orig_dimension.h, false);
                        if cpy.fisheye_params.camera_matrix.len() > 1 {
                            cpy.fisheye_params.camera_matrix[0][0] *= ratio;
                            cpy.fisheye_params.camera_matrix[0][2] *= ratio;
                            cpy.fisheye_params.camera_matrix[1][1] *= ratio;
                            cpy.fisheye_params.camera_matrix[1][2] *= ratio;
                        }
                        if let Some(ref mut odim) = cpy.output_dimension {
                            scale(&mut odim.w, true);
                            scale(&mut odim.h, true);
                        }
                    }
                }
                if x.contains_key("frame_readout_time") {
                    cpy.frame_readout_time = x["frame_readout_time"].as_f64();
                }
                if x.contains_key("fps") {
                    if let Some(fps) = x["fps"].as_f64() {
                        cpy.fps = fps;
                    }
                }
                if x.contains_key("identifier") {
                    cpy.identifier = x["identifier"].as_str().unwrap_or_default().to_string();
                }
                cpy.is_copy = true;
                ret.push(cpy);
            }
        }
        ret
    }

    pub fn get_display_name(&self) -> String {
        let mut all_sizes = HashSet::new();
        let mut all_fps = HashSet::new();
        all_sizes.insert(self.calib_dimension.w * 10000 + self.calib_dimension.h);
        if self.fps > 0.0 { all_fps.insert((self.fps * 10000.0) as usize); }
        for x in &self.compatible_settings {
            if let Some(x) = x.as_object() {
                match (x.get("width").and_then(|v| v.as_u64()), x.get("height").and_then(|v| v.as_u64())) {
                    (Some(w), Some(h)) => { all_sizes.insert(w as usize * 10000 + h as usize); }
                    _ => { }
                }
                match x.get("fps").and_then(|v| v.as_f64()) {
                    Some(fps) => { all_fps.insert((fps * 10000.0).round() as usize); }
                    _ => { }
                }
            }
        }
            
        let include_size = all_sizes.len() <= 1;
        let include_fps = all_fps.len() <= 1 || (all_fps.len() == 2 && all_fps.into_iter().next().unwrap() >= 200_0000);

        let mut final_name = vec![&self.camera_brand, &self.camera_model].into_iter().filter(|x| !x.is_empty()).join(" ");
        if include_size {
            final_name.push(' ');
            final_name.push_str(&self.get_size_str());
        }
        final_name.push(' ');
        final_name.push_str(&self.get_aspect_ratio());

        final_name.push(' ');
        final_name.push_str(&Self::cleanup_name(vec![&self.lens_model, &self.camera_setting, &self.note].into_iter().filter(|x| !x.is_empty()).join(" ")));

        if include_size {
            final_name.push_str(&format!(" {}x{}", self.calib_dimension.w, self.calib_dimension.h));
        }
        if include_fps && self.fps > 0.0 {
            final_name.push_str(&format!(" {:.2}fps", self.fps));
        }
        final_name
    }
    pub fn cleanup_name(name: String) -> String {
        name.replace(".json", "")
            .replace("4_3", "")
            .replace("4:3", "")
            .replace("4by3", "")
            .replace("16:9", "")
            .replace("169", "")
            .replace("16_9", "")
            .replace("16*9", "")
            .replace("16/9", "")
            .replace("16by9", "")
            .replace("2_7K", "")
            .replace("2,7K", "")
            .replace("2.7K", "")
            .replace("4K", "")
            .replace("5K", "")
            .replace('_', " ")
    }

    pub fn calculate_optimal_fov(&self, output_size: (usize, usize)) -> f64 {
        if output_size.0 <= 0 || output_size.1 <= 0 { return 1.0; }
        let mut params = crate::undistortion::ComputeParams::default();
        params.frame_count = 1;
        params.fov_scale = 1.0;
        params.adaptive_zoom_window = -1.0; // Static crop
        params.width              = self.calib_dimension.w;  params.height              = self.calib_dimension.h;
        params.output_width       = output_size.0;           params.output_height       = output_size.1;
        params.video_output_width = params.output_width;     params.video_output_height = params.output_height;
        params.video_width        = params.width;            params.video_height        = params.height;
        params.camera_matrix = self.get_camera_matrix_internal().unwrap_or_else(|| nalgebra::Matrix3::identity());
        let distortion_coeffs = self.get_distortion_coeffs();
        params.distortion_coeffs = [distortion_coeffs[0], distortion_coeffs[1], distortion_coeffs[2], distortion_coeffs[3]];

        let zoom = zooming::from_compute_params(params);
        zoom.compute(&[0.0]).first().map(|x| x.0).unwrap_or(1.0)
    }
}
