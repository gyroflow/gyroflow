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
    pub width: usize,
    pub height: usize, 
    pub output_width: usize,
    pub output_height: usize,
    pub calib_width: f64,
    pub calib_height: f64,
    pub video_rotation: f64,
    pub camera_matrix: Matrix3<f64>,
    pub distortion_coeffs: [f64; 4],
    pub radial_distortion_limit: f64,
    pub frame_readout_time: f64,
    pub trim_start_frame: usize,
    pub trim_end_frame: usize,
    pub framebuffer_inverted: bool,
}
impl ComputeParams {
    pub fn from_manager<T: PixelType>(mgr: &StabilizationManager<T>) -> Self {
        let params = mgr.params.read();
        let lens = mgr.lens.read();

        let camera_matrix = lens.get_camera_matrix(params.size);
        let distortion_coeffs = lens.get_distortion_coeffs();
        let distortion_coeffs = [distortion_coeffs[0], distortion_coeffs[1], distortion_coeffs[2], distortion_coeffs[3]];
        let radial_distortion_limit = lens.fisheye_params.radial_distortion_limit.unwrap_or_default();

        let (calib_width, calib_height) = if lens.calib_dimension.w > 0 && lens.calib_dimension.h > 0 {
            (lens.calib_dimension.w as f64, lens.calib_dimension.h as f64)
        } else {
            (params.size.0.max(1) as f64, params.size.1.max(1) as f64)
        };

        Self {
            gyro: mgr.gyro.read().clone(), // TODO: maybe not clone?

            frame_count: params.frame_count,
            fov_scale: params.fov / (params.size.0.max(1) as f64 / calib_width),
            fovs: params.fovs.clone(),
            width: params.size.0.max(1),
            height: params.size.1.max(1),
            output_width: params.output_size.0.max(1),
            output_height: params.output_size.1.max(1),
            calib_width,
            calib_height,
            camera_matrix,
            video_rotation: params.video_rotation,
            distortion_coeffs,
            radial_distortion_limit,
            framebuffer_inverted: params.framebuffer_inverted,
            frame_readout_time: params.frame_readout_time,
            trim_start_frame: (params.trim_start * params.frame_count as f64).floor() as usize,
            trim_end_frame: (params.trim_end * params.frame_count as f64).ceil() as usize,
        }
    }
}
