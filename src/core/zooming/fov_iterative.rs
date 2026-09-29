// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Maik <myco at gmx>

use super::*;
use crate::stabilization::undistort_points_with_rolling_shutter;
use crate::keyframes::*;
use parking_lot::RwLock;
use rayon::iter::{ ParallelIterator, IntoParallelIterator };
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

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
    /// The output frame's real width in pixels, which is what a resample stretch is counted in
    org_output_width: usize,
    output_inv_aspect: f32,
    compute_params: &'a ComputeParams,
    debug_points: RwLock<BTreeMap<i64, Vec<(f64, f64)>>>,
    /// `zooming::max_zoom_out_fov` by the lens data it was computed from, see [`FovIterative::max_fov_at`]
    zoom_limits: RwLock<HashMap<u64, f64>>,
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
            org_output_width: org_output_size.0,
            output_inv_aspect,
            compute_params,
            debug_points: RwLock::new(BTreeMap::new()),
            zoom_limits: RwLock::new(HashMap::new())
        }
    }

    /// How far the zoom may open at all at this frame (`zooming::max_zoom_out_fov`), cached on the lens data
    /// it is computed from. The search is a 200-sample sweep plus two bisections - about 1200 evaluations of
    /// the lens model, each a Newton solve on some of them - and it depends on the frame only through that
    /// lens data, which on a lens that doesn't move is the same for every frame of the clip. Everything else
    /// it reads (the frame size, the distortion model, the output projection and width) is fixed for the
    /// whole pass, so the key is the lens data and the correction strength alone
    fn max_fov_at(&self, ts: f64, amount: f64) -> f64 {
        let p = self.compute_params;
        let lens = crate::stabilization::FrameTransform::get_lens_data_at_timestamp(p, ts, p.framebuffer_inverted);
        let mut hasher = DefaultHasher::new();
        for v in [lens.0[(0, 0)], lens.0[(1, 1)], lens.0[(0, 2)], lens.0[(1, 2)], lens.2, lens.3, lens.4, amount] {
            hasher.write_u64(v.to_bits());
        }
        for v in lens.1.iter() { hasher.write_u64(v.to_bits()); }
        let key = hasher.finish();
        if let Some(v) = self.zoom_limits.read().get(&key) { return *v; }

        let v = super::max_zoom_out_fov_for_lens(p, amount, self.org_output_width, &lens);
        self.zoom_limits.write().insert(key, v);
        v
    }

    fn find_fov(&self, rect: &[(f32, f32)], ts: f64, frame: usize, center: &Point2D, keyframe_values: &(f64, f64, f64)) -> f64 {
        let ts_us = (ts * 1000.0).round() as i64;

        let adaptive_zoom_center_x = keyframe_values.0;
        let adaptive_zoom_center_y = keyframe_values.1;
        let lens_correction_amount = keyframe_values.2;

        // How far the zoom may open at all (`zooming::max_zoom_out_fov`). It is also where a border point that
        // has no position stands in: a ray past what the output projection can hold lands on its infinity,
        // the picture is then unbounded in that direction, and the zoom's own limit is the honest edge there
        let max_fov = self.max_fov_at(ts, lens_correction_amount);
        // The stand-in goes on the *boundary of the rectangle* that limit stands for, not on a circle through
        // it. `nearest_edge` inscribes an aspect-matched rectangle through every point it keeps, so a point
        // placed on the circle through that rectangle would, in a corner direction, fit a rectangle a factor of
        // cos(φ) smaller - 0.87·max_fov on 16:9, 0.71·max_fov on a square frame, ie. a picture cropped
        // tighter than the bound it is meant to express, and on an X4/X5 at high correction *every* border
        // point is a stand-in. On the rectangle each direction gives back exactly `max_fov`, which is also
        // what the render then draws, so the debug overlay drawn from these points stays honest
        let half = ((max_fov * self.output_dim.0 as f64 / 2.0) as f32, (max_fov * self.output_dim.1 as f64 / 2.0) as f32);
        let stand_in = |polygon: &mut Vec<(f32, f32)>| {
            if !(half.0 > 0.0) || !(half.1 > 0.0) { return; }
            for p in polygon.iter_mut() {
                if !crate::stabilization::is_valid_point(*p) { continue; }
                let d = (p.0 - center.0, p.1 - center.1);
                if d.0.abs().max(d.1.abs()) <= 1.0e6 { continue; }
                // The point where the ray from the center leaves that rectangle
                let m = (d.0.abs() / half.0).max(d.1.abs() / half.1);
                if m > 0.0 && m.is_finite() {
                    *p = (center.0 + d.0 / m, center.1 + d.1 / m);
                }
            }
        };

        let mut polygon = undistort_points_with_rolling_shutter(&rect, ts, Some(frame), &self.compute_params, lens_correction_amount, false, true);
        for (x, y) in polygon.iter_mut() {
            *x -= adaptive_zoom_center_x as f32 * self.input_dim.0;
            *y -= adaptive_zoom_center_y as f32 * self.input_dim.1;
        }
        stand_in(&mut polygon);
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
                polygon = undistort_points_with_rolling_shutter(&distorted, ts, Some(frame), &self.compute_params, lens_correction_amount, false, true);
                for (x, y) in polygon.iter_mut() {
                    *x -= adaptive_zoom_center_x as f32 * self.input_dim.0;
                    *y -= adaptive_zoom_center_y as f32 * self.input_dim.1;
                }
                stand_in(&mut polygon);
                nearest = self.nearest_edge(&polygon, center, nearest.1);
            } else {
                break;
            }
        }

        let fov = (nearest.1.0 * 2.0 / self.output_dim.0) as f64;
        // ... and however far the polygon reaches, the zoom stops where the picture stops being worth showing
        if max_fov > 0.0 { fov.min(max_fov) } else { fov }
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
    // No safety inset here on purpose: this polygon is the *measurement* of where the frame actually
    // ends, and `minimal_fovs` derived from it is what the FOV warning and the safe-area overlay
    // compare against 1.0. Insetting biased that measurement (~2.5px of reported FOV for a 2px inset),
    // which silently swallowed genuinely-uncovered frames. The safety pad is applied to the *applied*
    // zoom instead - see `fov_algorithm_margin` in zooming::calculate_fovs.
    pub fn points_around_rect(&self, w: f32, h: f32, w_div: usize, h_div: usize) -> Vec<(f32, f32)> {
        let (wcnt, hcnt) = (w_div.max(2) - 1, h_div.max(2) - 1);
        let (wstep, hstep) = (w / wcnt as f32, h / hcnt as f32);

        // ordered!
        let mut distorted_points: Vec<(f32, f32)> = Vec::with_capacity((wcnt + hcnt) * 2);
        for i in 0..wcnt { distorted_points.push((i as f32 * wstep,          0.0)); }
        for i in 0..hcnt { distorted_points.push((w,                         i as f32 * hstep)); }
        for i in 0..wcnt { distorted_points.push(((wcnt - i) as f32 * wstep, h)); }
        for i in 0..hcnt { distorted_points.push((0.0,                       (hcnt - i) as f32 * hstep)); }

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