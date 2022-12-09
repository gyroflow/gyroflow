// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::StabilizationManager;
use super::PixelType;
use super::distortion_models::DistortionModel;
use crate::GyroSource;
use crate::keyframes::KeyframeManager;
use crate::lens_profile::LensProfile;

#[derive(Default, Clone)]
pub struct ComputeParams {
    pub gyro: GyroSource,
    pub fovs: Vec<f64>,
    pub keyframes: KeyframeManager,
    pub lens: LensProfile,

    pub frame_count: usize,
    pub fov_scale: f64,
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
    pub adaptive_zoom_window: f64,
    pub adaptive_zoom_center_offset: (f64, f64),
    pub adaptive_zoom_method: i32,
    pub framebuffer_inverted: bool,

    pub zooming_debug_points: bool,

    pub distortion_model: DistortionModel,
    pub digital_lens: Option<DistortionModel>,
    pub digital_lens_params: Option<Vec<f64>>
}
impl ComputeParams {
    pub fn from_manager<T: PixelType>(mgr: &StabilizationManager<T>, full_gyro: bool) -> Self {
        let params = mgr.params.read();

        let lens = mgr.lens.read();

        let distortion_model = DistortionModel::from_name(lens.distortion_model.as_deref().unwrap_or("opencv_fisheye"));
        let digital_lens = lens.digital_lens.as_ref().map(|x| DistortionModel::from_name(&x));

        Self {
            gyro: if full_gyro { mgr.gyro.read().clone() } else { mgr.gyro.read().clone_quaternions() },
            lens: lens.clone(),

            frame_count: params.frame_count,
            fov_scale: params.fov,
            fovs: params.fovs.clone(),
            width: params.size.0.max(1),
            height: params.size.1.max(1),
            video_width: params.video_size.0.max(1),
            video_height: params.video_size.1.max(1),
            video_output_width: params.video_output_size.0.max(1),
            video_output_height: params.video_output_size.1.max(1),
            output_width: params.output_size.0.max(1),
            output_height: params.output_size.1.max(1),
            video_rotation: params.video_rotation,
            background_mode: params.background_mode,
            background_margin: params.background_margin,
            background_margin_feather: params.background_margin_feather,
            lens_correction_amount: params.lens_correction_amount,
            framebuffer_inverted: params.framebuffer_inverted,
            frame_readout_time: params.frame_readout_time,
            trim_start: params.trim_start,
            trim_end: params.trim_end,
            scaled_fps: params.get_scaled_fps(),
            adaptive_zoom_window: params.adaptive_zoom_window,
            adaptive_zoom_center_offset: params.adaptive_zoom_center_offset,
            adaptive_zoom_method: params.adaptive_zoom_method,
            video_speed: params.video_speed,
            video_speed_affects_smoothing: params.video_speed_affects_smoothing,
            video_speed_affects_zooming: params.video_speed_affects_zooming,

            distortion_model,
            digital_lens,
            digital_lens_params: lens.digital_lens_params.clone(),

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
         .field("background_mode",           &self.background_mode)
         .field("background_margin",         &self.background_margin)
         .field("background_margin_feather", &self.background_margin_feather)
         .field("frame_readout_time",        &self.frame_readout_time)
         .field("trim_start",                &self.trim_start)
         .field("trim_end",                  &self.trim_end)
         .field("scaled_fps",                &self.scaled_fps)
         .field("adaptive_zoom_window",      &self.adaptive_zoom_window)
         .field("adaptive_zoom_center_offset", &self.adaptive_zoom_center_offset)
         .field("adaptive_zoom_method",      &self.adaptive_zoom_method)
         .field("framebuffer_inverted",      &self.framebuffer_inverted)
         .field("zooming_debug_points",      &self.zooming_debug_points)
         .field("distortion_model",          &self.distortion_model.id())
         .field("digital_lens",              &self.digital_lens.as_ref().map(|x| x.id()).unwrap_or("None"))
         .finish()
    }
}
