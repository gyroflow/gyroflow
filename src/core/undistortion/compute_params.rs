// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::StabilizationManager;
use super::PixelType;
use crate::GyroSource;
use nalgebra::Matrix3;

#[derive(Default, Clone)]
pub struct ComputeParams {
    pub gyro: GyroSource,
    pub fovs: Vec<f64>,

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
    pub distortion_coeffs: [f64; 4],
    pub radial_distortion_limit: f64,
    pub frame_readout_time: f64,
    pub trim_start: f64,
    pub trim_end: f64,
    pub scaled_fps: f64,
    pub adaptive_zoom_window: f64,
    pub framebuffer_inverted: bool,
}
impl ComputeParams {
    pub fn from_manager<T: PixelType>(mgr: &StabilizationManager<T>) -> Self {
        let params = mgr.params.read();

        let mut camera_matrix = mgr.lens.write().get_camera_matrix(params.size, params.video_size);
        let lens = mgr.lens.read();
        let distortion_coeffs = lens.get_distortion_coeffs();
        let distortion_coeffs = [distortion_coeffs[0], distortion_coeffs[1], distortion_coeffs[2], distortion_coeffs[3]];
        let radial_distortion_limit = lens.fisheye_params.radial_distortion_limit.unwrap_or_default();

        let (calib_width, calib_height) = if lens.calib_dimension.w > 0 && lens.calib_dimension.h > 0 {
            (lens.calib_dimension.w as f64, lens.calib_dimension.h as f64)
        } else {
            (params.size.0.max(1) as f64, params.size.1.max(1) as f64)
        };
        
        let lens_ratiox = params.video_size.0 as f64 / calib_width;
        let lens_ratioy = params.video_size.1 as f64 / calib_height;
        camera_matrix[(0, 0)] *= lens_ratiox;
        camera_matrix[(1, 1)] *= lens_ratioy;
        camera_matrix[(0, 2)] *= lens_ratiox;
        camera_matrix[(1, 2)] *= lens_ratioy;

        Self {
            gyro: mgr.gyro.read().clone_quaternions(),

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
            framebuffer_inverted: params.framebuffer_inverted,
            frame_readout_time: params.frame_readout_time,
            trim_start: params.trim_start,
            trim_end: params.trim_end,
            scaled_fps: params.get_scaled_fps(),
            adaptive_zoom_window: params.adaptive_zoom_window
        }
    }
}
