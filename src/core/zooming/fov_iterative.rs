// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Maik <myco at gmx>

use super::*;
use crate::stabilization::undistort_points_with_rolling_shutter;
use crate::keyframes::*;
use std::collections::BTreeMap;
use parking_lot::RwLock;
use rayon::iter::{ ParallelIterator, IntoParallelIterator };

/*
Iterative FOV calculation:
    - gets polygon points around the outline of the undistorted image
    - draws a symetric rectangle around center
    - if a polygon point happens to be inside the rectangle, it becomes the nearest point and the rectangle shrinks, repeat for all points
    - interpolate between the points around the nearest polygon point
    - repeat shrinking the rectangle
*/

pub struct FovIterative<'a> {
    input_dim: (f32, f32),
    output_dim: (f32, f32),
    output_inv_aspect: f32,
    compute_params: &'a ComputeParams,
    debug_points: RwLock<BTreeMap<i64, Vec<(f64, f64)>>>,
}
impl FieldOfViewAlgorithm for FovIterative<'_> {
    fn get_debug_points(&self) -> BTreeMap<i64, Vec<(f64, f64)>> {
        self.debug_points.read().clone()
    }

    fn compute(&self, timestamps: &[(usize, f64)], ranges: &[(f64, f64)]) -> Vec<f64> {
        if timestamps.is_empty() {
            return Vec::new();
        }
        let l = (timestamps.len() - 1) as f64;
        let keyframes = &self.compute_params.keyframes;

        let rect = self.points_around_rect(self.input_dim.0, self.input_dim.1, 31, 31);

        let cp = Point2D(self.input_dim.0 / 2.0, self.input_dim.1 / 2.0);
        let mut fov_values: Vec<f64> = if keyframes.is_keyframed(&KeyframeType::ZoomingCenterX) || keyframes.is_keyframed(&KeyframeType::ZoomingCenterY) || keyframes.is_keyframed(&KeyframeType::LensCorrectionStrength) {
            timestamps.into_par_iter()
                .map(|&(frame, ts)| {
                    let adaptive_zoom_center_x = self.compute_params.keyframes.value_at_video_timestamp(&KeyframeType::ZoomingCenterX, ts).unwrap_or(self.compute_params.adaptive_zoom_center_offset.0);
                    let adaptive_zoom_center_y = self.compute_params.keyframes.value_at_video_timestamp(&KeyframeType::ZoomingCenterY, ts).unwrap_or(self.compute_params.adaptive_zoom_center_offset.1);
                    let lens_correction_amount = self.compute_params.keyframes.value_at_video_timestamp(&KeyframeType::LensCorrectionStrength, ts).unwrap_or(self.compute_params.lens_correction_amount);

                    let kv = (adaptive_zoom_center_x, adaptive_zoom_center_y, lens_correction_amount);
                    self.find_fov(&rect, ts, frame, &cp, &kv)
                })
                .collect()
        } else {
            let kv = (self.compute_params.adaptive_zoom_center_offset.0, self.compute_params.adaptive_zoom_center_offset.1, self.compute_params.lens_correction_amount);
            timestamps.into_par_iter()
                .map(|&(frame, ts)| self.find_fov(&rect, ts, frame, &cp, &kv))
                .collect()
        };

        if !ranges.is_empty() {
            // Only within render range.
            if let Some(max_fov) = fov_values.iter().copied().reduce(f64::max) {
                for (i, v) in fov_values.iter_mut().enumerate() {
                    let within_range = ranges.iter().any(|r| i >= (l*r.0).floor() as usize && i <= (l*r.1).ceil() as usize);
                    if !within_range {
                        *v = max_fov;
                    }
                }
            }
        }

        fov_values
    }
}

impl<'a>  FovIterative<'a> {
    pub fn new(compute_params: &'a ComputeParams, org_output_size: (usize, usize)) -> Self {
        let ratio = compute_params.width as f32 / org_output_size.0.max(1) as f32;
        let input_dim = (compute_params.width as f32, compute_params.height as f32);
        let output_dim = (org_output_size.0 as f32 * ratio, org_output_size.1 as f32 * ratio);
        let output_inv_aspect = output_dim.1 / output_dim.0;

        Self {
            input_dim,
            output_dim,
            output_inv_aspect,
            compute_params,
            debug_points: RwLock::new(BTreeMap::new())
        }
    }

    fn find_fov(&self, rect: &[(f32, f32)], ts: f64, frame: usize, center: &Point2D, keyframe_values: &(f64, f64, f64)) -> f64 {
        let ts_us = (ts * 1000.0).round() as i64;

        let adaptive_zoom_center_x = keyframe_values.0;
        let adaptive_zoom_center_y = keyframe_values.1;
        let lens_correction_amount = keyframe_values.2;

        let mut polygon = undistort_points_with_rolling_shutter(&rect, ts, Some(frame), &self.compute_params, lens_correction_amount, false);
        for (x, y) in polygon.iter_mut() {
            *x -= adaptive_zoom_center_x as f32 * self.input_dim.0;
            *y -= adaptive_zoom_center_y as f32 * self.input_dim.1;
        }
        if self.compute_params.zooming_debug_points {
            self.debug_points.write().insert(ts_us, polygon.iter().map(|(x, y)| ((x / self.input_dim.0) as f64, (y / self.input_dim.1) as f64)).collect());
        }

        let initial = (1000000.0, 1000000.0 * self.output_inv_aspect);
        let mut nearest = (None, initial);

        for _ in 1..5 {
            nearest = self.nearest_edge(&polygon, center, nearest.1);
            if let Some(idx) = nearest.0 {
                let len = rect.len();
                if len == 0 { continue; }
                let relevant = [
                    rect[idx.overflowing_sub(1).0 % len],
                    rect[idx],
                    rect[idx.overflowing_add(1).0 % len]
                ];

                let distorted = interpolate_points(&relevant, 30);
                polygon = undistort_points_with_rolling_shutter(&distorted, ts, Some(frame), &self.compute_params, lens_correction_amount, false);
                for (x, y) in polygon.iter_mut() {
                    *x -= adaptive_zoom_center_x as f32 * self.input_dim.0;
                    *y -= adaptive_zoom_center_y as f32 * self.input_dim.1;
                }
                nearest = self.nearest_edge(&polygon, center, nearest.1);
            } else {
                break;
            }
        }

        (nearest.1.0 * 2.0 / self.output_dim.0) as f64
    }

    fn nearest_edge(&self, polygon: &[(f32, f32)], center: &Point2D, initial: (f32, f32)) -> (Option<usize>, (f32, f32)) {
        polygon
            .iter()
            .enumerate()
            .fold((None, initial), |mp, (i, (x,y))| {
                let ap = ((x - center.0).abs(), (y - center.1).abs());
                if ap.0 < mp.1.0 && ap.1 < mp.1.1 {
                    if ap.1 > ap.0 * self.output_inv_aspect {
                        return (Some(i), (ap.1 / self.output_inv_aspect, ap.1));
                    } else {
                        return (Some(i), (ap.0, ap.0 * self.output_inv_aspect));
                    }
                }
                mp
            })
    }

    // Returns points placed around a rectangle in a continous order
    pub fn points_around_rect(&self, mut w: f32, mut h: f32, w_div: usize, h_div: usize) -> Vec<(f32, f32)> {
        w -= self.compute_params.fov_algorithm_margin * 2.0;
        h -= self.compute_params.fov_algorithm_margin * 2.0;

        let (wcnt, hcnt) = (w_div.max(2) - 1, h_div.max(2) - 1);
        let (wstep, hstep) = (w / wcnt as f32, h / hcnt as f32);

        // ordered!
        let mut distorted_points: Vec<(f32, f32)> = Vec::with_capacity((wcnt + hcnt) * 2);
        for i in 0..wcnt { distorted_points.push((i as f32 * wstep,          0.0)); }
        for i in 0..hcnt { distorted_points.push((w,                         i as f32 * hstep)); }
        for i in 0..wcnt { distorted_points.push(((wcnt - i) as f32 * wstep, h)); }
        for i in 0..hcnt { distorted_points.push((0.0,                       (hcnt - i) as f32 * hstep)); }

        // Add margin
        for (x, y) in distorted_points.iter_mut() {
            *x += self.compute_params.fov_algorithm_margin;
            *y += self.compute_params.fov_algorithm_margin;
        }

        distorted_points
    }

}

// linear interpolates steps between points in array
fn interpolate_points(pts: &[(f32, f32)], steps: usize) -> Vec<(f32,f32)> {
    let d = steps+1;
    let new_len = d * pts.len() - steps;
    (0..new_len).map(|i| {
        let idx1 = i / d;
        let idx2 = (idx1+1).min(pts.len()-1);
        let f = ((i % d) as f32) / (d as f32);
        (pts[idx1].0 + f * (pts[idx2].0 - pts[idx1].0), pts[idx1].1 + f * (pts[idx2].1 - pts[idx1].1))
    }).collect()
}