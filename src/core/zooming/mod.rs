// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Maik <myco at gmx>

pub mod fov_iterative;
pub mod zoom_dynamic;

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::collections::BTreeMap;

use crate::stabilization::ComputeParams;

#[derive(Default, Clone, Copy, Debug)]
pub struct Point2D(f32, f32);

pub enum ZoomMethod {
    GaussianFilter,
    EnvelopeFollower,
}
impl From<i32> for ZoomMethod {
    fn from(v: i32) -> Self {
        match v {
            0 => Self::GaussianFilter,
            1 => Self::EnvelopeFollower,
            _ => { log::error!("Invalid zooming method: {v}"); Self::GaussianFilter }
        }
    }
}

pub trait FieldOfViewAlgorithm {
    fn compute(&self, timestamps: &[(usize, f64)], range: &[(f64, f64)]) -> Vec<f64>;
    fn get_debug_points(&self) -> BTreeMap<i64, Vec<(f64, f64)>>;
}

pub fn calculate_fovs(compute_params: &ComputeParams, timestamps: &[(usize, f64)], method: ZoomMethod) -> (Vec<f64>, Vec<f64>, BTreeMap<i64, Vec<(f64, f64)>>)  {
    if timestamps.is_empty() {
        return Default::default();
    }

    let mut compute_params = compute_params.clone();
    compute_params.fov_scale = 1.0;
    compute_params.fovs.clear();
    compute_params.minimal_fovs.clear();

    // Use original video dimensions, because this is used to undistort points, and we need to find original image bounding box
    // Then we can use real `output_dim` to fit the fov
    let org_output_size = (compute_params.output_width, compute_params.output_height);
    compute_params.output_width = compute_params.width;
    compute_params.output_height = compute_params.height;

    let fov_estimator = fov_iterative::FovIterative::new(&compute_params, org_output_size);
    let mut fov_values = fov_estimator.compute(timestamps, &compute_params.trim_ranges);
    let debug_points = fov_estimator.get_debug_points();
    
    let (final_fovs, final_fovs_minimal) = if compute_params.adaptive_zoom_window < -0.9 {
        // Static zoom
        let fov_minimal = fov_values.clone();
        if let Some(max_f) = fov_values.iter().copied().reduce(f64::min) {
            fov_values.iter_mut().for_each(|v| *v = max_f);
        }
        (fov_values, fov_minimal)
    } else if compute_params.adaptive_zoom_window > 0.0001 {
        // Dynamic zoom
        zoom_dynamic::compute(&compute_params, fov_values, timestamps, method)
    } else {
        // Disabled zoom
        (vec![1.0; fov_values.len()], fov_values)
    };
    (final_fovs, final_fovs_minimal, debug_points)
}

pub fn get_checksum(compute_params: &ComputeParams) -> u64 {
    let mut hasher = DefaultHasher::new();
    for x in &compute_params.lens.get_distortion_coeffs() {
        hasher.write_u64(x.to_bits());
    }

    hasher.write_usize(compute_params.width);
    hasher.write_usize(compute_params.height);
    hasher.write_usize(compute_params.output_width);
    hasher.write_usize(compute_params.output_height);
    hasher.write_u64(compute_params.scaled_fps.to_bits());
    hasher.write_u64(compute_params.max_zoom.unwrap_or_default().to_bits());
    hasher.write_usize(compute_params.max_zoom_iterations);
    for x in compute_params.trim_ranges.iter() {
        hasher.write_u64(x.0.to_bits());
        hasher.write_u64(x.1.to_bits());
    }
    hasher.write_u64(compute_params.video_rotation.to_bits());
    hasher.write_u64(compute_params.adaptive_zoom_window.to_bits());
    hasher.write_u8(if compute_params.focal_length_smoothing_enabled { 1 } else { 0 });
    hasher.write_u64(compute_params.focal_length_smoothing_strength.to_bits());
    hasher.write_u64(compute_params.focal_length_time_window.to_bits());
    // Include focal length data arrays to ensure recalculation when they change
    for fl in &compute_params.focal_lengths {
        if let Some(val) = fl {
            hasher.write_u64(val.to_bits());
        } else {
            hasher.write_u64(0);
        }
    }
    for fl in &compute_params.smoothed_focal_lengths {
        if let Some(val) = fl {
            hasher.write_u64(val.to_bits());
        } else {
            hasher.write_u64(0);
        }
    }

    hasher.finish()
}
