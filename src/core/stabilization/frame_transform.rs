// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::Matrix3;
use super::{ ComputeParams, KernelParams };
use rayon::iter::{ ParallelIterator, IntoParallelIterator };
use crate::keyframes::KeyframeType;

#[derive(Default, Clone)]
pub struct FrameTransform {
    pub matrices: Vec<[f32; 9]>,
    pub kernel_params: super::KernelParams,
    pub fov: f64,
    pub minimal_fov: f64,
    pub focal_length: Option<f64>,
}

impl FrameTransform {
    fn get_frame_readout_time(params: &ComputeParams, can_invert: bool) -> f64 {
        let mut frame_readout_time = params.frame_readout_time;
        if can_invert && params.framebuffer_inverted {
            frame_readout_time *= -1.0;
        }
        frame_readout_time
    }
    fn get_new_k(params: &ComputeParams, camera_matrix: &Matrix3<f64>, fov: f64) -> Matrix3<f64> {
        let horizontal_ratio = if params.lens.input_horizontal_stretch > 0.01 { params.lens.input_horizontal_stretch } else { 1.0 };

        let img_dim_ratio = Self::get_ratio(params) / horizontal_ratio;

        let out_dim = (params.output_width as f64, params.output_height as f64);
        //let focal_center = (params.video_width as f64 / 2.0, params.video_height as f64 / 2.0);

        let mut new_k = *camera_matrix;
        new_k[(0, 0)] = new_k[(0, 0)] * img_dim_ratio / fov;
        new_k[(1, 1)] = new_k[(1, 1)] * img_dim_ratio / fov;
        new_k[(0, 2)] = /*(params.video_width  as f64 / 2.0 - new_k[(0, 2)]) * img_dim_ratio / fov + */out_dim.0 / 2.0;
        new_k[(1, 2)] = /*(params.video_height as f64 / 2.0 - new_k[(1, 2)]) * img_dim_ratio / fov + */out_dim.1 / 2.0;
        new_k
    }
    pub fn get_ratio(params: &ComputeParams) -> f64 {
        params.width as f64 / params.video_width.max(1) as f64
    }
    fn get_fov(params: &ComputeParams, frame: usize, use_fovs: bool, timestamp_ms: f64, for_ui: bool) -> f64 {
        let mut fov_scale = params.keyframes.value_at_video_timestamp(&KeyframeType::Fov, timestamp_ms).unwrap_or(params.fov_scale);
        fov_scale += if params.fov_overview && use_fovs && !for_ui { 1.0 } else { 0.0 };
        let mut fov = if use_fovs { params.fovs.get(frame).unwrap_or(&1.0) * fov_scale } else { 1.0 }.max(0.001);
        if !for_ui {
            //fov *= params.video_width as f64 / params.video_output_width.max(1) as f64;
            fov *= params.width as f64 / params.output_width.max(1) as f64;
        }
        fov
    }

    pub fn get_lens_data_at_timestamp(params: &ComputeParams, timestamp_ms: f64) -> (Matrix3<f64>, [f64; 12], f64, f64, f64, Option<f64>) {
        let mut interpolated_lens = None;
        if let Some(lens_positions) = params.gyro.lens_positions.as_ref() {
            use crate::util::MapClosest;
            if let Some(val) = lens_positions.get_closest(&((timestamp_ms * 1000.0).round() as i64), 100000) { // closest within 100ms
                interpolated_lens = Some(params.lens.get_interpolated_lens_at(*val));
            }
        }

        let lens = interpolated_lens.as_ref().unwrap_or(&params.lens);

        let mut camera_matrix = lens.get_camera_matrix((params.width, params.height), (params.video_width, params.video_height));
        let distortion_coeffs = lens.get_distortion_coeffs();
        let radial_distortion_limit = lens.fisheye_params.radial_distortion_limit.unwrap_or_default();

        let (calib_width, calib_height) = if lens.calib_dimension.w > 0 && lens.calib_dimension.h > 0 {
            (lens.calib_dimension.w as f64, lens.calib_dimension.h as f64)
        } else {
            (params.video_width.max(1) as f64, params.video_height.max(1) as f64)
        };

        let input_horizontal_stretch = if lens.input_horizontal_stretch > 0.01 { lens.input_horizontal_stretch } else { 1.0 };
        let input_vertical_stretch = if lens.input_vertical_stretch > 0.01 { lens.input_vertical_stretch } else { 1.0 };

        let lens_ratiox = (params.video_width as f64 / calib_width) * input_horizontal_stretch;
        let lens_ratioy = (params.video_height as f64 / calib_height) * input_vertical_stretch;
        camera_matrix[(0, 0)] *= lens_ratiox;
        camera_matrix[(1, 1)] *= lens_ratioy;
        camera_matrix[(0, 2)] *= lens_ratiox;
        camera_matrix[(1, 2)] *= lens_ratioy;
        (camera_matrix, distortion_coeffs, radial_distortion_limit, input_horizontal_stretch, input_vertical_stretch, lens.focal_length)
    }

    pub fn at_timestamp(params: &ComputeParams, timestamp_ms: f64, frame: usize) -> Self {
        // ----------- Keyframes -----------
        let video_rotation = params.keyframes.value_at_video_timestamp(&KeyframeType::VideoRotation, timestamp_ms).unwrap_or(params.video_rotation);
        let background_margin = params.keyframes.value_at_video_timestamp(&KeyframeType::BackgroundMargin, timestamp_ms).unwrap_or(params.background_margin);
        let background_feather = params.keyframes.value_at_video_timestamp(&KeyframeType::BackgroundFeather, timestamp_ms).unwrap_or(params.background_margin_feather);
        let lens_correction_amount = params.keyframes.value_at_video_timestamp(&KeyframeType::LensCorrectionStrength, timestamp_ms).unwrap_or(params.lens_correction_amount);
        let adaptive_zoom_center_x = params.keyframes.value_at_video_timestamp(&KeyframeType::ZoomingCenterX, timestamp_ms).unwrap_or(params.adaptive_zoom_center_offset.0);
        let adaptive_zoom_center_y = params.keyframes.value_at_video_timestamp(&KeyframeType::ZoomingCenterY, timestamp_ms).unwrap_or(params.adaptive_zoom_center_offset.1);
        // ----------- Keyframes -----------

        // ----------- Lens -----------
        let (camera_matrix,
            distortion_coeffs,
            radial_distortion_limit,
            input_horizontal_stretch,
            input_vertical_stretch,
            focal_length) = Self::get_lens_data_at_timestamp(params, timestamp_ms);
        // ----------- Lens -----------

        let img_dim_ratio = Self::get_ratio(params);
        let mut fov = Self::get_fov(params, frame, true, timestamp_ms, false);
        let mut ui_fov = Self::get_fov(params, frame, true, timestamp_ms, true);
        if let Some(adj) = params.lens.optimal_fov {
            if params.fovs.is_empty() {
                fov *= adj;
            } else {
                ui_fov /= adj;
            }
        }

        let scaled_k = camera_matrix * img_dim_ratio;
        let new_k = Self::get_new_k(&params, &camera_matrix, fov);

        // ----------- Rolling shutter correction -----------
        let frame_readout_time = Self::get_frame_readout_time(&params, true);

        let row_readout_time = frame_readout_time / params.height as f64;
        let start_ts = timestamp_ms - (frame_readout_time / 2.0);
        // ----------- Rolling shutter correction -----------

        let image_rotation = Matrix3::new_rotation(video_rotation * (std::f64::consts::PI / 180.0));

        let quat1 = params.gyro.org_quat_at_timestamp(timestamp_ms).inverse();
        let smoothed_quat1 = params.gyro.smoothed_quat_at_timestamp(timestamp_ms);

        // Only compute 1 matrix if not using rolling shutter correction
        let rows = if frame_readout_time.abs() > 0.0 { params.height } else { 1 };

        let matrices = (0..rows).into_par_iter().map(|y| {
            let quat_time = if frame_readout_time.abs() > 0.0 && timestamp_ms > 0.0 {
                start_ts + row_readout_time * y as f64
            } else {
                timestamp_ms
            };
            let quat = smoothed_quat1
                     * quat1
                     * params.gyro.org_quat_at_timestamp(quat_time);

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

        let mut digital_lens_params = [0f32; 4];
        if let Some(p) = &params.digital_lens_params {
            for (i, v) in p.iter().enumerate() {
                digital_lens_params[i] = *v as f32;
            }
        }

        let kernel_params = KernelParams {
            matrix_count:  matrices.len() as i32,
            f:             [scaled_k[(0, 0)] as f32, scaled_k[(1, 1)] as f32],
            c:             [scaled_k[(0, 2)] as f32, scaled_k[(1, 2)] as f32],
            k:             distortion_coeffs.iter().map(|x| *x as f32).collect::<Vec<f32>>().try_into().unwrap(),
            fov:           fov as f32,
            r_limit:       radial_distortion_limit as f32,
            lens_correction_amount:   lens_correction_amount as f32,
            input_vertical_stretch:   input_vertical_stretch as f32,
            input_horizontal_stretch: input_horizontal_stretch as f32,
            background_mode:          params.background_mode as i32,
            background_margin:        background_margin as f32,
            background_margin_feather:background_feather as f32,
            translation2d: [(adaptive_zoom_center_x * params.width as f64 / fov) as f32, (adaptive_zoom_center_y * params.height as f64 / fov) as f32],
            translation3d: [0.0, 0.0, 0.0, 0.0], // currently unused
            digital_lens_params,
            ..Default::default()
        };

        Self {
            matrices,
            kernel_params,
            fov: ui_fov,
            minimal_fov: *params.minimal_fovs.get(frame).unwrap_or(&1.0),
            focal_length
        }
    }

    pub fn at_timestamp_for_points(params: &ComputeParams, points: &[(f32, f32)], timestamp_ms: f64) -> (Matrix3<f64>, [f64; 12], Matrix3<f64>, Vec<Matrix3<f64>>) { // camera_matrix, dist_coeffs, p, rotations_per_point
        // ----------- Keyframes -----------
        let video_rotation = params.keyframes.value_at_video_timestamp(&KeyframeType::VideoRotation, timestamp_ms).unwrap_or(params.video_rotation);
        // ----------- Keyframes -----------

        let (camera_matrix, distortion_coeffs, _, _, _, _) = Self::get_lens_data_at_timestamp(params, timestamp_ms);

        let img_dim_ratio = Self::get_ratio(params);
        let fov = Self::get_fov(params, 0, false, timestamp_ms, false);

        let scaled_k = camera_matrix * img_dim_ratio;
        let new_k = Self::get_new_k(params, &camera_matrix, fov);

        // ----------- Rolling shutter correction -----------
        let frame_readout_time = Self::get_frame_readout_time(params, false);

        let row_readout_time = frame_readout_time / params.height as f64;
        let start_ts = timestamp_ms - (frame_readout_time / 2.0);
        // ----------- Rolling shutter correction -----------

        let image_rotation = Matrix3::new_rotation(video_rotation * (std::f64::consts::PI / 180.0));

        let quat1 = params.gyro.org_quat_at_timestamp(timestamp_ms).inverse();
        let smoothed_quat1 = params.gyro.smoothed_quat_at_timestamp(timestamp_ms);

        // Only compute 1 matrix if not using rolling shutter correction
        let points_iter = if frame_readout_time.abs() > 0.0 { points } else { &[(0.0, 0.0)] };

        let rotations: Vec<Matrix3<f64>> = points_iter.iter().map(|&(_, y)| {
            let quat_time = if frame_readout_time.abs() > 0.0 && timestamp_ms > 0.0 {
                start_ts + row_readout_time * y as f64
            } else {
                timestamp_ms
            };
            let quat = smoothed_quat1
                     * quat1
                     * params.gyro.org_quat_at_timestamp(quat_time);

            let mut r = image_rotation * *quat.to_rotation_matrix().matrix();
            r[(0, 1)] *= -1.0; r[(0, 2)] *= -1.0;
            r[(1, 0)] *= -1.0; r[(2, 0)] *= -1.0;

            new_k * r
        }).collect();

        (scaled_k, distortion_coeffs, new_k, rotations)
    }
}
