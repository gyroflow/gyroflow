// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::{ HashSet, BTreeMap };
use itertools::Itertools;

use serde::{ Serialize, Deserialize };

use crate::stabilization::distortion_models::DistortionModel;

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
    pub fn from_value(json: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(json)
    }
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn load_from_data(&mut self, data: &str) -> std::result::Result<(), crate::GyroflowCoreError> {
        *self = Self::from_json(&data)?;

        // Trust lens profiles loaded from file
        self.official = true;

        if self.calibrator_version.is_empty() || self.fisheye_params.camera_matrix.is_empty() || self.calib_dimension.w <= 0 || self.calib_dimension.h <= 0 {
            return Err(crate::GyroflowCoreError::InvalidData);
        }

        Ok(())
    }

    pub fn load_from_file(&mut self, url: &str) -> std::result::Result<(), crate::GyroflowCoreError> {
        self.load_from_data(&crate::filesystem::read_to_string(url)?)
    }

    pub fn load_from_json_value(&mut self, v: &serde_json::Value) -> Option<()> {
        *self = <Self as Deserialize>::deserialize(v).ok()?;
        Some(())
    }

    #[cfg(feature = "opencv")]
    pub fn set_from_calibrator(&mut self, cal: &LensCalibrator) {
        if self.input_horizontal_stretch <= 0.01 { self.input_horizontal_stretch = 1.0; }
        if self.input_vertical_stretch   <= 0.01 { self.input_vertical_stretch   = 1.0; }

        self.calib_dimension = Dimensions { w: cal.width, h: cal.height };
        self.orig_dimension  = Dimensions { w: cal.width, h: cal.height };
        self.num_images = cal.used_points.len();
        self.digital_lens = cal.digital_lens.clone();
        self.optimal_fov = None;

        self.asymmetrical = cal.asymmetrical;

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
        self.date = time::OffsetDateTime::now_local().map(|v| v.date().to_string()).unwrap_or_default();
        self.name = self.get_name();
    }

    pub fn get_json_value(&self) -> Result<serde_json::Value, serde_json::error::Error> {
        let mut v = serde_json::to_value(&self)?;
        if let Some(obj) = v.as_object_mut() {
            obj.remove("filename");
            obj.remove("path_to_file");
            obj.remove("optimal_fov");
            obj.remove("is_copy");
            obj.remove("rating");
            obj.remove("checksum");
            obj.remove("parsed_interpolations");
        }
        Ok(v)
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
            (8.0/7.0, "8:7"), (7.0/8.0, "7:8"),
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
        else if self.calib_dimension.w == 1920 && self.calib_dimension.h == 1440 { "1440p" }
        else if self.calib_dimension.w >= 1920 { "1080p" }
        else if self.calib_dimension.w >= 1280 { "720p" }
        else if self.calib_dimension.w >= 640  { "480p" }
        else { "" }
    }

    pub fn save_to_file(&mut self, url: &str) -> std::result::Result<String, crate::GyroflowCoreError> {
        let json = self.get_json()?;

        crate::filesystem::write(url, json.as_bytes())?;

        Ok(json)
    }

    pub fn swapped(&self) -> LensProfile {
        let mut ret = self.clone();
        std::mem::swap(&mut ret.orig_dimension.w, &mut ret.orig_dimension.h);
        std::mem::swap(&mut ret.calib_dimension.w, &mut ret.calib_dimension.h);
        if let Some(ref mut out) = ret.output_dimension {
            std::mem::swap(&mut out.w, &mut out.h);
        }
        std::mem::swap(&mut ret.input_horizontal_stretch, &mut ret.input_vertical_stretch);

        if ret.fisheye_params.camera_matrix.len() == 3 {
            let mut mtrx0 = ret.fisheye_params.camera_matrix[0];
            let mut mtrx1 = ret.fisheye_params.camera_matrix[1];
            std::mem::swap(&mut mtrx0[0], &mut mtrx1[1]);
            std::mem::swap(&mut mtrx0[2], &mut mtrx1[2]);
            ret.fisheye_params.camera_matrix[0] = mtrx0;
            ret.fisheye_params.camera_matrix[1] = mtrx1;
        }

        // Swap compatible settings
        for x in ret.compatible_settings.iter_mut() {
            if let Some(x) = x.as_object_mut() {
                match (x.get("width").and_then(|x| x.as_u64()), x.get("height").and_then(|x| x.as_u64())) {
                    (Some(w), Some(h)) => {
                        x["width"] = h.into();
                        x["height"] = w.into();
                    }
                    _ => { }
                }
            }
        }

        // Swap interpolations
        for (_, x) in ret.parsed_interpolations.iter_mut() {
            *x = x.swapped();
        }

        ret
    }

    fn get_camera_matrix_internal(&self) -> Option<nalgebra::Matrix3<f64>> {
        if self.fisheye_params.camera_matrix.len() == 3 {
            let mut mat = nalgebra::Matrix3::from_rows(&[
                self.fisheye_params.camera_matrix[0].into(),
                self.fisheye_params.camera_matrix[1].into(),
                self.fisheye_params.camera_matrix[2].into()
            ]);
            if !self.asymmetrical {
                mat[(0, 2)] = self.calib_dimension.w as f64 / 2.0;
                mat[(1, 2)] = self.calib_dimension.h as f64 / 2.0;
            }
            if let Some(crop) = self.crop {
                mat[(0, 0)] /= crop;
                mat[(1, 1)] /= crop;
            }
            Some(mat)
        } else {
            None
        }
    }
    pub fn get_camera_matrix(&self, _size: (usize, usize), video_size: (usize, usize)) -> nalgebra::Matrix3<f64> {
        if self.fisheye_params.camera_matrix.len() == 3 {
            let mat = self.get_camera_matrix_internal().unwrap();

            // TODO: this didn't really work, try to figure it out and re-enable
            // if self.optimal_fov.is_none() && self.num_images > 3 {
            //     self.optimal_fov = Some(self.calculate_optimal_fov(video_size));
            //     log::debug!("Optimal lens FOV: {:?} ({:?})", self.optimal_fov, video_size);
            // }

            mat
        } else {
            // Default camera matrix
            let mut mat = nalgebra::Matrix3::<f64>::identity();
            mat[(0, 0)] = video_size.0 as f64 * 0.8;
            mat[(1, 1)] = video_size.0 as f64 * 0.8;
            mat[(0, 2)] = video_size.0 as f64 / 2.0;
            mat[(1, 2)] = video_size.1 as f64 / 2.0;
            mat
        }
    }
    pub fn get_distortion_coeffs(&self) -> [f64; 12] {
        let mut ret = [0.0; 12];
        for (i, x) in self.fisheye_params.distortion_coeffs.iter().enumerate() {
            if i < 12 {
                ret[i] = *x;
            }
        }
        ret
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
                        let mut ratiow = new_w as f64 / cpy.calib_dimension.w as f64;
                        let ratioh = new_h as f64 / cpy.calib_dimension.h as f64;
                        match x.get("digital_lens").and_then(|x| x.as_str()) {
                            Some("gopro_superview") => { ratiow /= 1.33333333333; },
                            Some("gopro_hyperview") => { ratiow /= 1.55555555555; },
                            _ => { }
                        }
                        fn scale(val: &mut usize, ratio: f64, pad: bool) {
                            *val = (*val as f64 * ratio).round() as usize;
                            if pad && *val % 2 != 0 { *val -= 1; }
                        }
                        scale(&mut cpy.calib_dimension.w, ratiow, true);
                        scale(&mut cpy.calib_dimension.h, ratioh, true);
                        scale(&mut cpy.orig_dimension.w, ratiow, true);
                        scale(&mut cpy.orig_dimension.h, ratioh, true);
                        if cpy.fisheye_params.camera_matrix.len() > 1 {
                            // If aspect ratio is different, then we treat it as a sensor crop.
                            // In this case, we don't want to scale the camera matrix
                            // Otherwise, it's not a crop, but sub- or super-sampling so we simply "zoom" the entire video
                            if (ratiow - ratioh).abs() < 0.001 { // if x and y aspect ratios are the same
                                cpy.fisheye_params.camera_matrix[0][0] *= ratiow;
                                cpy.fisheye_params.camera_matrix[0][2] *= ratiow;
                                cpy.fisheye_params.camera_matrix[1][1] *= ratioh;
                                cpy.fisheye_params.camera_matrix[1][2] *= ratioh;
                            }
                        }
                        if let Some(ref mut odim) = cpy.output_dimension {
                            scale(&mut odim.w, ratiow, true);
                            scale(&mut odim.h, ratioh, true);
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
                if x.contains_key("crop") { cpy.crop = x["crop"].as_f64(); }
                if x.contains_key("interpolations") { cpy.interpolations = x.get("interpolations").cloned(); }
                if x.contains_key("digital_lens")   { cpy.digital_lens   = x.get("digital_lens").and_then(|x| x.as_str().map(|x| x.to_owned())); }
                if x.contains_key("focal_length")   { cpy.focal_length   = x.get("focal_length").and_then(|x| x.as_f64()); }
                if x.contains_key("crop_factor")    { cpy.crop_factor    = x.get("crop_factor").and_then(|x| x.as_f64()); }

                if x.contains_key("sync_settings") {
                    if let Some(obj) = x.get("sync_settings") {
                        if let Some(ref mut ss) = cpy.sync_settings {
                            if obj.get("custom_sync_pattern").is_some() && ss.get("custom_sync_pattern").is_some() {
                                ss.as_object_mut().unwrap().remove("custom_sync_pattern");
                            }
                            crate::util::merge_json(ss, obj);
                        } else {
                            cpy.sync_settings = Some(obj.clone());
                        }
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

    pub fn calculate_optimal_fov(&self, _output_size: (usize, usize)) -> f64 {
        /*if output_size.0 <= 0 || output_size.1 <= 0 { return 1.0; }
        let mut params = crate::stabilization::ComputeParams::default();
        params.frame_count = 1;
        params.fov_scale = 1.0;
        params.adaptive_zoom_window = -1.0; // Static crop
        params.width              = self.calib_dimension.w;  params.height              = self.calib_dimension.h;
        params.output_width       = output_size.0;           params.output_height       = output_size.1;
        params.video_output_width = params.output_width;     params.video_output_height = params.output_height;
        params.video_width        = params.width;            params.video_height        = params.height;
        params.camera_matrix = self.get_camera_matrix_internal().unwrap_or_else(|| nalgebra::Matrix3::identity());
        params.distortion_coeffs = self.get_distortion_coeffs();

        let zoom = super::zooming::from_compute_params(params);
        zoom.compute(&[0.0], &crate::keyframes::KeyframeManager::new()).first().map(|x| x.0).unwrap_or(1.0)*/
        1.0
    }

    pub fn get_interpolated_lens_at(&self, val: f64) -> LensProfile {
        let mut cpy = self.clone();

        if !self.parsed_interpolations.is_empty() {
            let key = (val * 1000000.0).round() as i64;

            if let Some(v) = self.parsed_interpolations.get(&key) { return v.clone(); }

            if let Some(&first) = self.parsed_interpolations.keys().next() {
                if let Some(&last) = self.parsed_interpolations.keys().next_back() {
                    let lookup = (key).min(last-1).max(first+1);
                    if let Some(p1) = self.parsed_interpolations.range(..=lookup).next_back() {
                        if *p1.0 == lookup {
                            return p1.1.clone();
                        }
                        if let Some(p2) = self.parsed_interpolations.range(lookup..).next() {
                            let time_delta = (p2.0 - p1.0) as f64;
                            let fract = (key - p1.0) as f64 / time_delta;

                            let l1 = p1.1;
                            let l2 = p2.1;
                            // println!("interpolated at {:.4}, fract: {:.4}", val, fract);

                            cpy.fisheye_params.camera_matrix[0][0] = l1.fisheye_params.camera_matrix[0][0] * (1.0 - fract) + (l2.fisheye_params.camera_matrix[0][0] * fract);
                            cpy.fisheye_params.camera_matrix[1][1] = l1.fisheye_params.camera_matrix[1][1] * (1.0 - fract) + (l2.fisheye_params.camera_matrix[1][1] * fract);
                            cpy.fisheye_params.camera_matrix[0][2] = l1.fisheye_params.camera_matrix[0][2] * (1.0 - fract) + (l2.fisheye_params.camera_matrix[0][2] * fract);
                            cpy.fisheye_params.camera_matrix[1][2] = l1.fisheye_params.camera_matrix[1][2] * (1.0 - fract) + (l2.fisheye_params.camera_matrix[1][2] * fract);

                            if cpy.fisheye_params.distortion_coeffs.len() == l1.fisheye_params.distortion_coeffs.len() && l1.fisheye_params.distortion_coeffs.len() == l2.fisheye_params.distortion_coeffs.len() {
                                for i in 0..l1.fisheye_params.distortion_coeffs.len() {
                                    cpy.fisheye_params.distortion_coeffs[i] = l1.fisheye_params.distortion_coeffs[i] * (1.0 - fract) + (l2.fisheye_params.distortion_coeffs[i] * fract);
                                }
                            }
                            cpy.crop = Some(l1.crop.unwrap_or(1.0) * (1.0 - fract) + (l2.crop.unwrap_or(1.0) * fract));

                            match (l1.focal_length, l2.focal_length) {
                                (Some(fl1), Some(fl2)) => { cpy.focal_length = Some(fl1 * (1.0 - fract) + (fl2 * fract))},
                                _ => { }
                            }

                            cpy.calib_dimension.w = (l1.calib_dimension.w as f64 * (1.0 - fract) + (l2.calib_dimension.w as f64 * fract)).round() as usize;
                            cpy.calib_dimension.h = (l1.calib_dimension.h as f64 * (1.0 - fract) + (l2.calib_dimension.h as f64 * fract)).round() as usize;

                            cpy.input_horizontal_stretch = l1.input_horizontal_stretch * (1.0 - fract) + (l2.input_horizontal_stretch * fract);
                            cpy.input_vertical_stretch   = l1.input_vertical_stretch   * (1.0 - fract) + (l2.input_vertical_stretch   * fract);

                            // TODO: digital lens interpolation?
                        }
                    }
                }
            }
        }

        cpy
    }

    pub fn resolve_interpolations(&mut self, db: &crate::lens_profile_database::LensProfileDatabase) {
        if !self.parsed_interpolations.is_empty() {
            return; // Already resolved
        }

        if let Some(digital) = self.digital_lens.as_ref() {
            let model = DistortionModel::from_name(&digital);
            model.adjust_lens_profile(self);
        }

        if let Some(serde_json::Value::Object(map)) = &self.interpolations {
            let mut interpolations = BTreeMap::new();
            for (k, v) in map {
                if let serde_json::Value::Object(v) = v {
                    if let Ok(key) = k.parse::<f64>() {
                        let key = (key * 1000000.0).round() as i64;
                        let mut new_profile = self.clone();
                        if let Some(id) = v.get("identifier").and_then(|x| x.as_str()) {
                            if let Some(profile) = db.get_by_id(id) {
                                new_profile = profile.clone();
                            }
                        }
                        new_profile.interpolations = None;
                        if let Some(row) = v.get("camera_matrix").and_then(|x| x.as_array()) {
                            for (i, r) in row.iter().enumerate() {
                                if let Some(col) = r.as_array() {
                                    for (j, c) in col.iter().enumerate() {
                                        if let Some(v) = c.as_f64() {
                                            new_profile.fisheye_params.camera_matrix[i][j] = v;
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(row) = v.get("distortion_coeffs").and_then(|x| x.as_array()) {
                            for (i, v) in row.iter().enumerate() {
                                if let Some(v) = v.as_f64() {
                                    new_profile.fisheye_params.distortion_coeffs[i] = v;
                                }
                            }
                        }
                        if let Some(fl) = v.get("focal_length").and_then(|x| x.as_f64()) {
                            new_profile.focal_length = Some(fl);
                        }
                        interpolations.insert(key, new_profile);
                    }
                }
            }
            self.parsed_interpolations = interpolations;
        }
    }

    pub fn for_light_refraction(&self, ior_ratio: f64, tir_margin: f64) -> LensProfile {
        if ior_ratio == 1.0 {
            return self.clone();
        }
        if self.fisheye_params.distortion_coeffs.len() < 4 {
            log::warn!("Not enough distortion coefficients! {}", self.fisheye_params.distortion_coeffs.len());
            return self.clone();
        }
        use argmin::{ core::{ Executor, Jacobian, Operator, State }, solver::gaussnewton::GaussNewton };
        use nalgebra::{ DMatrix, DVector };

        struct Problem {
            ior_ratio: f64,
            max_ray_angle: f64,
            params_orig: [f64; 5],
            model: DistortionModel,
        }

        let params_orig = [
            1.0,
            self.fisheye_params.distortion_coeffs[0],
            self.fisheye_params.distortion_coeffs[1],
            self.fisheye_params.distortion_coeffs[2],
            self.fisheye_params.distortion_coeffs[3],
        ];

        impl Problem {
            const STEP: f64 = 0.01;
        }

        impl Jacobian for Problem {
            type Param = DVector<f64>;
            type Jacobian = DMatrix<f64>;

            fn jacobian(&self, param: &Self::Param) -> Result<Self::Jacobian, argmin::core::Error> {
                let n_pts = (self.max_ray_angle / Problem::STEP) as i32;
                let jac = DMatrix::from_row_iterator(
                    n_pts as usize,
                    5,
                    (0..n_pts).flat_map(|i| {
                        let theta = (i as f64) * Problem::STEP;
                        self.model.undistort_for_light_refraction_gradient(param.as_slice(), theta).into_iter()
                    }),
                );

                Ok(jac)
            }
        }

        impl Operator for Problem {
            type Param = DVector<f64>;
            type Output = DVector<f64>;

            fn apply(&self, param: &Self::Param) -> Result<Self::Output, argmin::core::Error> {
                let n_pts = (self.max_ray_angle / Problem::STEP) as i32;

                let residue = DVector::from_iterator(
                    n_pts as usize,
                    (0..n_pts).map(|i| {
                        let theta = (i as f64) * Problem::STEP;
                        let theta_new = (theta.sin() * self.ior_ratio).asin();
                        (self.model.distort_for_light_refraction(param.as_slice(), theta)
                            - self.model.distort_for_light_refraction(self.params_orig.as_slice(), theta_new))
                            as f64
                    }),
                );

                Ok(residue)
            }
        }

        let max_ray_angle = if ior_ratio > 1.0 {
            (1.0 / ior_ratio).asin() - tir_margin
        } else {
            std::f64::consts::PI / 2.0
        };
        let cost = Problem {
            ior_ratio,
            max_ray_angle,
            params_orig,
            model: DistortionModel::from_name(self.distortion_model.as_deref().unwrap_or("opencv_fisheye"))
        };

        let init_param = DVector::from_row_slice(&cost.params_orig);

        let solver: GaussNewton<f64> = GaussNewton::new();

        let res = Executor::new(cost, solver)
            .configure(|state| state.param(init_param).max_iters(10))
            .run();

        let mut clone = self.clone();
        match res {
            Ok(x) => {
                if let Some(best_param) = x.state().get_best_param() {
                    clone.focal_length = self.focal_length.map(|x| x * best_param[0]);
                    clone.fisheye_params.distortion_coeffs = vec![
                        best_param[1],
                        best_param[2],
                        best_param[3],
                        best_param[4],
                    ];
                    clone.fisheye_params.camera_matrix[0][0] *= best_param[0];
                    clone.fisheye_params.camera_matrix[1][1] *= best_param[0];
                }
            }
            Err(e) => {
                log::warn!("Failed to optimize distortion coefficients for underwater correction: {e:?}");
            },
        }
        clone
    }
}
