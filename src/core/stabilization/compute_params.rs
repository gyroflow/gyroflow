// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::StabilizationManager;
use super::distortion_models::DistortionModel;
use crate::GyroSource;
use crate::keyframes::KeyframeManager;
use crate::lens_profile::LensProfile;
use std::sync::Arc;
use parking_lot::RwLock;

#[derive(Default, Clone)]
pub struct ComputeParams {
    pub gyro: Arc<RwLock<GyroSource>>,
    pub fovs: Vec<f64>,
    pub minimal_fovs: Vec<f64>,
    pub keyframes: KeyframeManager,
    pub lens: LensProfile,
    pub lens_is_dynamic: bool,

    pub frame_count: usize,
    pub fov_scale: f64,
    pub fov_overview: bool,
    pub show_safe_area: bool,
    pub width: usize,
    pub height: usize,
    pub output_width: usize,
    pub output_height: usize,
    pub video_output_width: usize,
    pub video_output_height: usize,
    pub video_width: usize,
    pub video_height: usize,
    pub video_rotation: f64,
    pub lens_correction_amount: f64,
    pub light_refraction_coefficient: f64,
    pub video_speed: f64,
    pub video_speed_affects_smoothing: bool,
    pub video_speed_affects_zooming: bool,
    pub background: nalgebra::Vector4<f32>,
    pub background_mode: crate::stabilization_params::BackgroundMode,
    pub background_margin: f64,
    pub background_margin_feather: f64,
    pub frame_readout_time: f64,
    pub trim_ranges: Vec<(f64, f64)>,
    pub scaled_fps: f64,
    pub scaled_duration_ms: f64,
    pub plane_scale: (f64, f64),
    pub adaptive_zoom_window: f64,
    pub adaptive_zoom_center_offset: (f64, f64),
    pub adaptive_zoom_method: i32,
    pub additional_rotation: (f64, f64, f64),
    pub additional_translation: (f64, f64, f64),
    pub framebuffer_inverted: bool,
    pub horizontal_rs: bool,
    pub smoothness_limiter: f64,

    pub zooming_debug_points: bool,

    pub distortion_model: DistortionModel,
    pub digital_lens: Option<DistortionModel>,
    pub digital_lens_params: Option<Vec<f64>>
}
impl ComputeParams {
    pub fn from_manager(mgr: &StabilizationManager) -> Self {
        let params = mgr.params.read();
        let lens_is_dynamic = mgr.gyro.read().file_metadata.lens_params.len() > 1;

        let lens = mgr.lens.read().clone();

        let distortion_model = DistortionModel::from_name(lens.distortion_model.as_deref().unwrap_or("opencv_fisheye"));
        let digital_lens = lens.digital_lens.as_ref().map(|x| DistortionModel::from_name(&x));

        let digital_lens_params = lens.digital_lens_params.clone();

        Self {
            gyro: mgr.gyro.clone(),
            lens,
            lens_is_dynamic,

            frame_count: params.frame_count,
            fov_scale: params.fov,
            fov_overview: params.fov_overview,
            show_safe_area: params.show_safe_area,
            fovs: params.fovs.clone(),
            minimal_fovs: params.minimal_fovs.clone(),
            width: params.size.0.max(1),
            height: params.size.1.max(1),
            video_width: params.video_size.0.max(1),
            video_height: params.video_size.1.max(1),
            video_output_width: params.video_output_size.0.max(1),
            video_output_height: params.video_output_size.1.max(1),
            output_width: params.output_size.0.max(1),
            output_height: params.output_size.1.max(1),
            plane_scale: params.plane_scale.unwrap_or((1.0, 1.0)),
            video_rotation: params.video_rotation,
            background: params.background,
            background_mode: params.background_mode,
            background_margin: params.background_margin,
            background_margin_feather: params.background_margin_feather,
            lens_correction_amount: params.lens_correction_amount,
            light_refraction_coefficient: params.light_refraction_coefficient,
            framebuffer_inverted: params.framebuffer_inverted,
            horizontal_rs: params.horizontal_rs,
            frame_readout_time: params.frame_readout_time,
            trim_ranges: params.trim_ranges.clone(),
            scaled_fps: params.get_scaled_fps(),
            scaled_duration_ms: params.get_scaled_duration_ms(),
            adaptive_zoom_window: params.adaptive_zoom_window,
            adaptive_zoom_center_offset: params.adaptive_zoom_center_offset,
            additional_rotation: params.additional_rotation,
            additional_translation: params.additional_translation,
            adaptive_zoom_method: params.adaptive_zoom_method,
            video_speed: params.video_speed,
            video_speed_affects_smoothing: params.video_speed_affects_smoothing,
            video_speed_affects_zooming: params.video_speed_affects_zooming,

            distortion_model,
            digital_lens,
            digital_lens_params,
            smoothness_limiter: params.smoothness_limiter,

            keyframes: mgr.keyframes.read().clone(),

            zooming_debug_points: false
        }
    }
}

impl std::fmt::Debug for ComputeParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let gyro = self.gyro.read();
        f.debug_struct("ComputeParams")
         .field("gyro.imu_orientation", &gyro.imu_orientation)
         .field("gyro.imu_rotation", &gyro.imu_rotation_angles)
         .field("gyro.acc_rotation", &gyro.acc_rotation_angles)
         .field("gyro.duration_ms", &gyro.duration_ms)
         .field("gyro.imu_lpf", &gyro.imu_lpf)
         .field("gyro.gyro_bias", &gyro.gyro_bias)
         .field("gyro.integration_method", &gyro.integration_method)
         .field("fovs.len", &self.fovs.len())
         .field("keyframed", &self.keyframes.get_all_keys())

         .field("frame_count",          &self.frame_count)
         .field("fov_scale",            &self.fov_scale)
         .field("fov_overview",         &self.fov_overview)
         .field("width",                &self.width)
         .field("height",               &self.height)
         .field("output_width",         &self.output_width)
         .field("output_height",        &self.output_height)
         .field("video_output_width",   &self.video_output_width)
         .field("video_output_height",  &self.video_output_height)
         .field("video_width",          &self.video_width)
         .field("video_height",         &self.video_height)
         .field("video_rotation",       &self.video_rotation)
         .field("lens_correction_amount",    &self.lens_correction_amount)
         .field("light_refraction_coefficient", &self.light_refraction_coefficient)
         .field("background_mode",           &self.background_mode)
         .field("background_margin",         &self.background_margin)
         .field("background_margin_feather", &self.background_margin_feather)
         .field("frame_readout_time",        &self.frame_readout_time)
         .field("trim_ranges",               &self.trim_ranges)
         .field("scaled_fps",                &self.scaled_fps)
         .field("adaptive_zoom_window",      &self.adaptive_zoom_window)
         .field("adaptive_zoom_center_offset", &self.adaptive_zoom_center_offset)
         .field("additional_rotation",       &self.additional_rotation)
         .field("additional_translation",    &self.additional_translation)
         .field("adaptive_zoom_method",      &self.adaptive_zoom_method)
         .field("framebuffer_inverted",      &self.framebuffer_inverted)
         .field("zooming_debug_points",      &self.zooming_debug_points)
         .field("distortion_model",          &self.distortion_model.id())
         .field("digital_lens",              &self.digital_lens.as_ref().map(|x| x.id()).unwrap_or("None"))
         .finish()
    }
}
