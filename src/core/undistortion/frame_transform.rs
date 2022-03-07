// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::Matrix3;
use super::ComputeParams;
use rayon::iter::{ ParallelIterator, IntoParallelIterator };

#[derive(Default, Clone)]
pub struct FrameTransform {
    pub params: Vec<[f32; 9]>,
    pub fov: f64,
}

impl FrameTransform {
    fn get_frame_readout_time(params: &ComputeParams, can_invert: bool) -> f64 {
        let mut frame_readout_time = params.frame_readout_time;
        if can_invert && params.framebuffer_inverted {
            frame_readout_time *= -1.0;
        }
        frame_readout_time / 2.0
    }
    fn get_new_k(params: &ComputeParams, fov: f64) -> Matrix3<f64> {
        let img_dim_ratio = Self::get_ratio(params);
        
        let out_dim = (params.output_width as f64, params.output_height as f64);
        //let focal_center = (params.video_width as f64 / 2.0, params.video_height as f64 / 2.0);

        let mut new_k = params.camera_matrix;
        new_k[(0, 0)] = new_k[(0, 0)] * img_dim_ratio / fov;
        new_k[(1, 1)] = new_k[(1, 1)] * img_dim_ratio / fov;
        new_k[(0, 2)] = /*(params.video_width  as f64 / 2.0 - focal_center.0) * img_dim_ratio / fov + */out_dim.0 / 2.0;
        new_k[(1, 2)] = /*(params.video_height as f64 / 2.0 - focal_center.1) * img_dim_ratio / fov + */out_dim.1 / 2.0;
        new_k
    }
    fn get_ratio(params: &ComputeParams) -> f64 {
        params.width as f64 / params.video_width.max(1) as f64
    }
    fn get_fov(params: &ComputeParams, frame: usize, use_fovs: bool) -> f64 {
        let mut fov = if use_fovs && params.fovs.len() > frame { params.fovs[frame] * params.fov_scale } else { params.fov_scale }.max(0.001);
        //fov *= params.video_width as f64 / params.video_output_width.max(1) as f64;
        fov *= params.width as f64 / params.output_width.max(1) as f64;
        fov
    }

    pub fn at_timestamp(params: &ComputeParams, timestamp_ms: f64, frame: usize) -> Self {
        let img_dim_ratio = Self::get_ratio(params);
        let mut fov = Self::get_fov(params, frame, true);
        let mut ui_fov = fov / (params.width as f64 / params.output_width.max(1) as f64);
        if params.lens_fov_adjustment > 0.0001 {
            if params.fovs.is_empty() {
                fov *= params.lens_fov_adjustment;
            } else {
                ui_fov /= params.lens_fov_adjustment;
            }
        }
    
        let scaled_k = params.camera_matrix * img_dim_ratio;
        let new_k = Self::get_new_k(params, fov);
        
        // ----------- Rolling shutter correction -----------
        let frame_readout_time = Self::get_frame_readout_time(params, true);

        let row_readout_time = frame_readout_time / params.height as f64;
        let start_ts = timestamp_ms - (frame_readout_time / 2.0);
        // ----------- Rolling shutter correction -----------

        let image_rotation = Matrix3::new_rotation(params.video_rotation * (std::f64::consts::PI / 180.0));

        let quat1 = params.gyro.org_quat_at_timestamp(timestamp_ms).inverse();

        // Only compute 1 matrix if not using rolling shutter correction
        let rows = if frame_readout_time.abs() > 0.0 { params.height } else { 1 };

        let mut transform_params = (0..rows).into_par_iter().map(|y| {
            let quat_time = if frame_readout_time.abs() > 0.0 && timestamp_ms > 0.0 {
                start_ts + row_readout_time * y as f64
            } else {
                timestamp_ms
            };
            let quat = quat1
                     * params.gyro.org_quat_at_timestamp(quat_time)
                     * params.gyro.smoothed_quat_at_timestamp(quat_time);

            let mut r = image_rotation * *quat.to_rotation_matrix().matrix();
            if params.framebuffer_inverted {
                r[(0, 2)] *= -1.0; r[(1, 2)] *= -1.0;
                r[(2, 0)] *= -1.0; r[(2, 1)] *= -1.0;
            } else {
                r[(0, 1)] *= -1.0; r[(0, 2)] *= -1.0;
                r[(1, 0)] *= -1.0; r[(2, 0)] *= -1.0;
            }
            
            let i_r = (new_k * r).pseudo_inverse(0.000001);
            if let Err(err) = i_r {
                log::error!("Failed to multiply matrices: {:?} * {:?}: {}", new_k, r, err);
            }
            let i_r: Matrix3<f32> = nalgebra::convert(i_r.unwrap_or_default());
            [
                i_r[(0, 0)], i_r[(0, 1)], i_r[(0, 2)], 
                i_r[(1, 0)], i_r[(1, 1)], i_r[(1, 2)], 
                i_r[(2, 0)], i_r[(2, 1)], i_r[(2, 2)],
            ]
        }).collect::<Vec<[f32; 9]>>();

        // Prepend lens params at the beginning
        transform_params.insert(0, [
            scaled_k[(0, 0)] as f32, scaled_k[(1, 1)] as f32, // 1, 2 - f
            scaled_k[(0, 2)] as f32, scaled_k[(1, 2)] as f32, // 3, 4 - c
    
            params.distortion_coeffs[0] as f32, // 5
            params.distortion_coeffs[1] as f32, // 6
            params.distortion_coeffs[2] as f32, // 7
            params.distortion_coeffs[3] as f32, // 8
            params.radial_distortion_limit as f32
        ]);

        // Add additional params after lens params
        transform_params.insert(1, [
            params.lens_correction_amount as f32,
            params.background_mode as i32 as f32, 
            fov as f32, 
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0 // unused
        ]);

        Self {
            params: transform_params,
            fov: ui_fov
        }
    }

    pub fn at_timestamp_for_points(params: &ComputeParams, points: &[(f64, f64)], timestamp_ms: f64) -> (Matrix3<f64>, [f64; 4], Matrix3<f64>, Vec<Matrix3<f64>>) { // camera_matrix, dist_coeffs, p, rotations_per_point
        let img_dim_ratio = Self::get_ratio(params);
        let fov = Self::get_fov(params, 0, false);

        let scaled_k = params.camera_matrix * img_dim_ratio;
        let new_k = Self::get_new_k(params, fov);

        // ----------- Rolling shutter correction -----------
        let frame_readout_time = Self::get_frame_readout_time(params, false);

        let row_readout_time = frame_readout_time / params.height as f64;
        let start_ts = timestamp_ms - (frame_readout_time / 2.0);
        // ----------- Rolling shutter correction -----------

        let image_rotation = Matrix3::new_rotation(params.video_rotation * (std::f64::consts::PI / 180.0));

        let quat1 = params.gyro.org_quat_at_timestamp(timestamp_ms).inverse();

        // Only compute 1 matrix if not using rolling shutter correction
        let points_iter = if frame_readout_time.abs() > 0.0 { points } else { &[(0.0, 0.0)] };

        let rotations: Vec<Matrix3<f64>> = points_iter.iter().map(|&(_, y)| {
            let quat_time = if frame_readout_time.abs() > 0.0 && timestamp_ms > 0.0 {
                start_ts + row_readout_time * y as f64
            } else {
                timestamp_ms
            };
            let quat = quat1
                     * params.gyro.org_quat_at_timestamp(quat_time)
                     * params.gyro.smoothed_quat_at_timestamp(quat_time);

            let mut r = image_rotation * *quat.to_rotation_matrix().matrix();
            r[(0, 1)] *= -1.0; r[(0, 2)] *= -1.0;
            r[(1, 0)] *= -1.0; r[(2, 0)] *= -1.0;
            
            new_k * r
        }).collect();

        (scaled_k, params.distortion_coeffs, new_k, rotations)
    }
}
