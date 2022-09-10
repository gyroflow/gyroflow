// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::StabilizationManager;
use super::PixelType;
use super::distortion_models::DistortionModel;
use crate::GyroSource;
use crate::keyframes::KeyframeManager;
use nalgebra::Matrix3;

#[derive(Default, Clone)]
pub struct ComputeParams {
    pub gyro: GyroSource,
    pub fovs: Vec<f64>,
    pub keyframes: KeyframeManager,

    pub frame_count: usize,
    pub fov_scale: f64,
    pub lens_fov_adjustment: f64,
    pub width: usize,
    pub height: usize,
    pub output_width: usize,
    pub output_height: usize,
    pub video_output_width: usize,
    pub video_output_height: usize,
    pub video_width: usize,
    pub video_height: usize,
    pub video_rotation: f64,
    pub camera_matrix: Matrix3<f64>,
    pub distortion_coeffs: [f64; 12],
    pub radial_distortion_limit: f64,
    pub lens_correction_amount: f64,
    pub video_speed: f64,
    pub video_speed_affects_smoothing: bool,
    pub video_speed_affects_zooming: bool,
    pub background_mode: crate::stabilization_params::BackgroundMode,
    pub background_margin: f64,
    pub background_margin_feather: f64,
    pub frame_readout_time: f64,
    pub trim_start: f64,
    pub trim_end: f64,
    pub scaled_fps: f64,
    pub input_horizontal_stretch: f64,
    pub input_vertical_stretch: f64,
    pub adaptive_zoom_window: f64,
    pub adaptive_zoom_center_offset: (f64, f64),
    pub is_superview: bool,
    pub framebuffer_inverted: bool,

    pub zooming_debug_points: bool,

    pub distortion_model: DistortionModel
}
impl ComputeParams {
    pub fn from_manager<T: PixelType>(mgr: &StabilizationManager<T>, full_gyro: bool) -> Self {
        let params = mgr.params.read();

        let lens = mgr.lens.read();
        let mut camera_matrix = lens.get_camera_matrix(params.size, params.video_size);
        let distortion_coeffs = lens.get_distortion_coeffs();
        let radial_distortion_limit = lens.fisheye_params.radial_distortion_limit.unwrap_or_default();

        let (calib_width, calib_height) = if lens.calib_dimension.w > 0 && lens.calib_dimension.h > 0 {
            (lens.calib_dimension.w as f64, lens.calib_dimension.h as f64)
        } else {
            (params.size.0.max(1) as f64, params.size.1.max(1) as f64)
        };

        let input_horizontal_stretch = if lens.input_horizontal_stretch > 0.01 { lens.input_horizontal_stretch } else { 1.0 };
        let input_vertical_stretch = if lens.input_vertical_stretch > 0.01 { lens.input_vertical_stretch } else { 1.0 };

        let lens_ratiox = (params.video_size.0 as f64 / calib_width) * input_horizontal_stretch;
        let lens_ratioy = (params.video_size.1 as f64 / calib_height) * input_vertical_stretch;
        camera_matrix[(0, 0)] *= lens_ratiox;
        camera_matrix[(1, 1)] *= lens_ratioy;
        camera_matrix[(0, 2)] *= lens_ratiox;
        camera_matrix[(1, 2)] *= lens_ratioy;

        let distortion_model = DistortionModel::from_id(lens.distortion_model_id);

        Self {
            gyro: if full_gyro { mgr.gyro.read().clone() } else { mgr.gyro.read().clone_quaternions() },

            frame_count: params.frame_count,
            fov_scale: params.fov,
            lens_fov_adjustment: lens.optimal_fov.unwrap_or(1.0),
            fovs: params.fovs.clone(),
            width: params.size.0.max(1),
            height: params.size.1.max(1),
            video_width: params.video_size.0.max(1),
            video_height: params.video_size.1.max(1),
            video_output_width: params.video_output_size.0.max(1),
            video_output_height: params.video_output_size.1.max(1),
            output_width: params.output_size.0.max(1),
            output_height: params.output_size.1.max(1),
            camera_matrix,
            video_rotation: params.video_rotation,
            distortion_coeffs,
            radial_distortion_limit,
            background_mode: params.background_mode,
            background_margin: params.background_margin,
            background_margin_feather: params.background_margin_feather,
            lens_correction_amount: params.lens_correction_amount,
            framebuffer_inverted: params.framebuffer_inverted,
            frame_readout_time: params.frame_readout_time,
            trim_start: params.trim_start,
            trim_end: params.trim_end,
            input_horizontal_stretch,
            input_vertical_stretch,
            scaled_fps: params.get_scaled_fps(),
            adaptive_zoom_window: params.adaptive_zoom_window,
            adaptive_zoom_center_offset: params.adaptive_zoom_center_offset,
            video_speed: params.video_speed,
            video_speed_affects_smoothing: params.video_speed_affects_smoothing,
            video_speed_affects_zooming: params.video_speed_affects_zooming,
            is_superview: lens.is_superview,

            distortion_model,

            keyframes: mgr.keyframes.read().clone(),

            zooming_debug_points: false
        }
    }
}

impl std::fmt::Debug for ComputeParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComputeParams")
         .field("gyro.imu_orientation", &self.gyro.imu_orientation)
         .field("gyro.imu_rotation", &self.gyro.imu_rotation_angles)
         .field("gyro.acc_rotation", &self.gyro.acc_rotation_angles)
         .field("gyro.duration_ms", &self.gyro.duration_ms)
         .field("gyro.fps", &self.gyro.fps)
         .field("gyro.imu_lpf", &self.gyro.imu_lpf)
         .field("gyro.gyro_bias", &self.gyro.gyro_bias)
         .field("gyro.integration_method", &self.gyro.integration_method)
         .field("fovs.len", &self.fovs.len())
         .field("keyframed", &self.keyframes.get_all_keys())

         .field("frame_count",          &self.frame_count)
         .field("fov_scale",            &self.fov_scale)
         .field("lens_fov_adjustment",  &self.lens_fov_adjustment)
         .field("width",                &self.width)
         .field("height",               &self.height)
         .field("output_width",         &self.output_width)
         .field("output_height",        &self.output_height)
         .field("video_output_width",   &self.video_output_width)
         .field("video_output_height",  &self.video_output_height)
         .field("video_width",          &self.video_width)
         .field("video_height",         &self.video_height)
         .field("video_rotation",       &self.video_rotation)
         .field("camera_matrix",        &self.camera_matrix)
         .field("distortion_coeffs",    &self.distortion_coeffs)
         .field("radial_distortion_limit",   &self.radial_distortion_limit)
         .field("lens_correction_amount",    &self.lens_correction_amount)
         .field("background_mode",           &self.background_mode)
         .field("background_margin",         &self.background_margin)
         .field("background_margin_feather", &self.background_margin_feather)
         .field("frame_readout_time",        &self.frame_readout_time)
         .field("trim_start",                &self.trim_start)
         .field("trim_end",                  &self.trim_end)
         .field("scaled_fps",                &self.scaled_fps)
         .field("input_horizontal_stretch",  &self.input_horizontal_stretch)
         .field("input_vertical_stretch",    &self.input_vertical_stretch)
         .field("adaptive_zoom_window",      &self.adaptive_zoom_window)
         .field("adaptive_zoom_center_offset", &self.adaptive_zoom_center_offset)
         .field("is_superview",              &self.is_superview)
         .field("framebuffer_inverted",      &self.framebuffer_inverted)
         .field("zooming_debug_points",      &self.zooming_debug_points)
         .field("distortion_model",          &self.distortion_model.id())
         .finish()
    }
}
