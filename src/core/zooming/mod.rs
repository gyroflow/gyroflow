// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Maik <myco at gmx>

// pub mod fov_default;
// pub mod fov_direct;
pub mod fov_iterative;

pub mod zoom_disabled;
pub mod zoom_static;
pub mod zoom_dynamic;

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use enterpolation::Merge;
use std::collections::BTreeMap;

use crate::stabilization::ComputeParams;
use crate::keyframes::*;

#[derive(PartialEq, Clone)]
pub enum Mode {
    Disabled,
    Dynamic(f64), // f64 - smoothing focus window in seconds
    Static
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Point2D(f32, f32);
impl Merge<f32> for Point2D {
    fn merge(self, other: Self, factor: f32) -> Self {
        Point2D(
            self.0 * (1.0 - factor) + other.0 * factor,
            self.1 * (1.0 - factor) + other.1 * factor
        )
    }
}

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

pub trait ZoomingAlgorithm {
    fn compute(&self, timestamps: &[f64], keyframes: &KeyframeManager, method: ZoomMethod) -> Vec<((f64, f64), Point2D)>;
    fn compute_params(&self) -> &ComputeParams;
    fn get_debug_points(&self) -> BTreeMap<i64, Vec<(f64, f64)>>;
    fn hash(&self, hasher: &mut dyn Hasher);
}

pub trait FieldOfViewAlgorithm {
    fn compute(&self, timestamps: &[f64], range: (f64, f64)) -> (Vec<f64>, Vec<Point2D>);
    fn get_debug_points(&self) -> BTreeMap<i64, Vec<(f64, f64)>>;
}

pub fn from_compute_params(mut compute_params: ComputeParams) -> Box<dyn ZoomingAlgorithm> {
    compute_params.fov_scale = 1.0;
    compute_params.fovs.clear();
    compute_params.minimal_fovs.clear();

    // Use original video dimensions, because this is used to undistort points, and we need to find original image bounding box
    // Then we can use real `output_dim` to fit the fov
    compute_params.width = compute_params.video_width;
    compute_params.height = compute_params.video_height;
    compute_params.output_width = compute_params.video_width;
    compute_params.output_height = compute_params.video_height;

    let mode = if compute_params.adaptive_zoom_window < -0.9 {
        Mode::Static
    } else if compute_params.adaptive_zoom_window > 0.0001 {
        Mode::Dynamic(compute_params.adaptive_zoom_window)
    } else {
        Mode::Disabled
    };

    let fov_estimator = Box::new(fov_iterative::FovIterative::new(compute_params.clone()));
    // let fov_estimator = Box::new(fov_direct::FovDirect::new(compute_params.clone()));
    // let fov_estimator = Box::new(fov_default::FovDefault::new(compute_params.clone()));
    match mode {
        Mode::Disabled            => Box::new(zoom_disabled::ZoomDisabled::new(fov_estimator, compute_params)),
        Mode::Static              => Box::new(zoom_static::ZoomStatic::new(fov_estimator, compute_params)),
        Mode::Dynamic(window) => Box::new(zoom_dynamic::ZoomDynamic::new(window, fov_estimator, compute_params)),
    }
}

pub fn get_checksum(zoom: &Box<dyn ZoomingAlgorithm>) -> u64 {
    let compute_params = zoom.compute_params();

    let mut hasher = DefaultHasher::new();
    for x in &compute_params.lens.get_distortion_coeffs() {
        hasher.write_u64(x.to_bits());
    }

    hasher.write_usize(compute_params.video_width);
    hasher.write_usize(compute_params.video_height);
    hasher.write_usize(compute_params.video_output_width);
    hasher.write_usize(compute_params.video_output_height);
    hasher.write_u64(compute_params.scaled_fps.to_bits());
    hasher.write_u64(compute_params.trim_start.to_bits());
    hasher.write_u64(compute_params.trim_end.to_bits());
    hasher.write_u64(compute_params.video_rotation.to_bits());

    zoom.hash(&mut hasher);

    hasher.finish()
}
