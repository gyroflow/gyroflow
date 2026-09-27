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

#[derive(Clone, Copy, Debug)]
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

/// Output pixels one source pixel may be smeared over at the edge of the frame before the zoom stops
/// opening up. 1.0 is the untouched frame; a normal lens at fov 1 sits right around there and never reaches
/// this, so this only ever bites a lens whose corrected image runs away faster than its pixels do.
const MAX_EDGE_STRETCH: f64 = 4.0;

/// The largest zoom-out the fov search may return (`0`: no limit), in its own units - the fitted rectangle's
/// width as a fraction of the source width.
///
/// A rectilinear image of a fisheye has no edge: `tan θ` simply runs away, so the polygon the zoom fits the
/// frame into can reach as far as the lens's field allows and the zoom follows it, turning the periphery
/// into smear (an X5 at 100% correction opened to ±89.5° and 199 output pixels per source pixel before this
/// existed). The zoom therefore needs a statement about picture, not about geometry.
///
/// It is where the picture stops being picture: with the frame fitted so its edge sits at the ray of angle
/// `θ`, one source pixel there is worth `(W/2)·(dρ/dθ) / (ρ·f·dR/dθ)` output pixels, and the largest `θ`
/// still under [`MAX_EDGE_STRETCH`] wins (if the lens never gets under it at all - a very wide one at full
/// correction - the least bad angle does). The sweep runs in ray angles rather than output radii because
/// that is the side the blend is a closed form on, and the side that stays finite: on an X4/X5 every border
/// point of the frame is past 90°, has no rectilinear position at all, and lands on `P`'s infinity - so the
/// polygon the zoom fits has no edge to find and this bound is the only thing that answers. It is bounded by
/// where the source runs out anyway - the frame's own corner and the lens's field - so on an ordinary lens
/// it lands on the frame corner and never fires.
///
/// It bounds the *fov*, not the polygon: the polygon is the picture's own outline, and clamping that would
/// make the zoom overlay - and the STMap, and the sync - disagree with what the render actually draws.
pub fn max_zoom_out_fov(params: &ComputeParams, amount: f64, output_width: usize, timestamp_ms: f64) -> f64 {
    let lens = crate::stabilization::FrameTransform::get_lens_data_at_timestamp(params, timestamp_ms, params.framebuffer_inverted);
    max_zoom_out_fov_for_lens(params, amount, output_width, &lens)
}

/// The lens data [`max_zoom_out_fov`] reads, exactly as `FrameTransform::get_lens_data_at_timestamp` hands
/// it out: camera matrix, coefficients, field limit, the two anamorphic stretches, and the focal length
/// bookkeeping this doesn't use
pub type LensData = (nalgebra::Matrix3<f64>, [f64; 24], f64, f64, f64, Option<f64>, bool);

/// [`max_zoom_out_fov`] on lens data the caller already holds. The answer depends on the timestamp only
/// through this, so the per-frame zoom sweep keys a cache on it (`FovIterative::max_fov_at`) rather than
/// running the ~1200 lens evaluations below for every frame of a clip whose lens never moves.
pub fn max_zoom_out_fov_for_lens(params: &ComputeParams, amount: f64, output_width: usize, lens: &LensData) -> f64 {
    use crate::stabilization::{ KernelParams, projection::OutputProjection };
    if output_width == 0 { return 0.0; }
    let (camera_matrix, coeffs, field_limit) = (&lens.0, &lens.1, lens.2);
    // The camera matrix lives in the plane the calibration was made in, where a stored pixel of an anamorphic
    // source is `sh` (`sv`) wide - the render puts the squeeze back on at the very end, and
    // `FrameTransform::get_new_k` builds the output plane with `f/sh`. So the frame's own corner is measured
    // with the stretch on, and the pixel counts and the fov below are in the plane the zoom polygon lives in,
    // with it off; read with the raw `fx` they were out by the stretch factor (33% on a 1.33x anamorphic)
    let sh = if lens.3 > 0.01 { lens.3 } else { 1.0 };
    let sv = if lens.4 > 0.01 { lens.4 } else { 1.0 };
    let (fx, fy) = (camera_matrix[(0, 0)], camera_matrix[(1, 1)]);
    if !(fx > 0.0) || !(fy > 0.0) { return 0.0; }

    let mut kp = KernelParams::default();
    for (i, v) in coeffs.iter().enumerate().take(24) { kp.k[i] = *v as f32; }
    kp.field_limit = field_limit as f32;
    kp.output_projection = params.output_projection;
    let model = &params.distortion_model;
    let proj = OutputProjection::from_i32(params.output_projection);
    let a = amount.clamp(0.0, 1.0);

    // The lens, forward: the normalized image radius it images a ray angle at. The whole sweep below is in
    // ray angles, because that is the side the blend is a closed form on - and the side that stays finite:
    // a ray past 90° has no rectilinear position at all, so an X4/X5's every frame border point lands on
    // `P`'s infinity and the polygon the zoom fits has no edge to find
    let radius_at = |t: f64| -> f64 { let p = model.distort_point(t.sin() as f32, 0.0, t.cos() as f32, &kp); (p.0 as f64).hypot(p.1 as f64) };
    // Where a ray of that angle lands on the output plane, and how fast each of the two moves
    let rho_at = |t: f64| -> f64 { (1.0 - a) * radius_at(t) + a * proj.radius_of_theta(t) };

    // How far the sweep may go: the lens's own field, and the frame's own corner - there is no picture past
    // either, whatever the projection would still show. The corner furthest from the principal point, the
    // way `OutputProjection::for_lens` measures it: the point is the largest ray the frame really holds, and
    // a body that records a principal point of its own per frame (Sony) does not put it in the middle
    let (cx, cy) = (camera_matrix[(0, 2)], camera_matrix[(1, 2)]);
    let (frame_w, frame_h) = (params.width as f64, params.height as f64);
    let r_corner = [(0.0, 0.0), (frame_w, 0.0), (0.0, frame_h), (frame_w, frame_h)].iter()
        .map(|&(x, y)| ((x * sh - cx) / fx).hypot((y * sv - cy) / fy))
        .fold(0.0f64, f64::max);
    let mut max_theta = if field_limit > 0.0 { field_limit } else { std::f64::consts::PI };
    // Any correction at all puts the output projection in the answer, and it has an end of its own
    if a > 0.0 { max_theta = max_theta.min(proj.max_theta()); }
    {
        let (mut lo, mut hi) = (0.0f64, max_theta);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if radius_at(mid) < r_corner { lo = mid; } else { hi = mid; }
        }
        max_theta = max_theta.min(0.5 * (lo + hi));
    }

    let half_w = output_width as f64 / 2.0;
    // Source pixels per unit of normalized radius, counted in the pixels the file actually stores
    let f = (fx / sh + fy / sv) / 2.0;
    // Output pixels per source pixel at the frame's edge, with the frame fitted so its edge sits at this
    // ray: `(W/2)/rho` output pixels per unit of output radius, against `f·dR/dθ` source pixels per radian
    let stretch = |t: f64| -> Option<f64> {
        let h = (t * 1e-3).max(1e-7);
        let (rl, rh) = (rho_at((t - h).max(0.0)), rho_at(t + h));
        let (s_lo, s_hi) = (radius_at((t - h).max(0.0)), radius_at(t + h));
        let rho = rho_at(t);
        let (drho, ds) = ((rh - rl) / (2.0 * h), (s_hi - s_lo) / (2.0 * h) * f);
        if !(ds > 0.0) || !(drho > 0.0) || !(rho > 0.0) || !rho.is_finite() { return None; }
        Some(half_w * drho / (rho * ds))
    };

    // Sweep the angle out to where the picture stops, and bisect the crossing. `least_bad` is the angle that
    // stretches least of all, for the case where nothing gets under the cap - it has to be tracked next to
    // `best` rather than folded into it, or the first sample wins the fallback and every better one after it
    // is ignored, which collapses the zoom to a two-hundredth of the field
    let (mut best, mut best_stretch, mut least_bad, mut last_ok) = (0.0, f64::MAX, 0.0, 0.0);
    let steps = 200;
    for i in 1..=steps {
        let t = i as f64 / steps as f64 * max_theta;
        let Some(s) = stretch(t) else { break };
        last_ok = t;
        if s <= MAX_EDGE_STRETCH { best = t; }
        if s < best_stretch { best_stretch = s; least_bad = t; }
    }
    if best > 0.0 {
        if best < last_ok {
            // Between the last angle under the cap and the first one over it
            let (mut lo, mut hi) = (best, last_ok);
            for _ in 0..24 {
                let mid = 0.5 * (lo + hi);
                match stretch(mid) { Some(s) if s <= MAX_EDGE_STRETCH => lo = mid, _ => hi = mid }
            }
            best = lo;
        }
    } else {
        // Nothing got under the cap: a very wide lens at full correction, or an output so much larger than
        // the source that every fit magnifies. There is no crossing to bisect, so the least bad angle stands
        // - which on an ordinary lens is the frame's own corner, ie. no bound at all
        best = least_bad;
    }
    let best = rho_at(best);
    if !best.is_finite() || best <= 0.0 { return 0.0; }
    // -> the fov the search speaks in: the fitted rectangle's half-width is `best · fx/sh` in the plane the
    // polygon is measured in (`FrameTransform::get_new_k` at fov 1), and `find_fov` divides by the source width
    2.0 * best * (fx / sh) / params.width.max(1) as f64
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
    let measured = fov_estimator.compute(timestamps, &compute_params.trim_ranges);
    let debug_points = fov_estimator.get_debug_points();

    // Focal length stabilization: the renderer multiplies `fov` by `comp = raw / target <= 1`
    // (crop-only, see smoothing::focal_length). `coverage` is the largest zoom fov that still keeps the
    // compensated view inside the source, so the zoom may use the pixels that crop hides anyway, but it
    // must never zoom out past the target further than it would without compensation (`max(P, 1)`),
    // otherwise it would fit the frame around the crop and undo the smoothing.
    let mut fov_values = measured.clone();
    let mut coverage = measured;
    if compute_params.focal_length_smoothing_enabled && !compute_params.smoothed_focal_lengths.is_empty() {
        for (i, &(frame, _)) in timestamps.iter().enumerate() {
            let comp = crate::smoothing::focal_length::compensation_at(&compute_params, frame);
            let p = coverage[i];
            coverage[i] = p / comp;
            fov_values[i] = p.max(1.0).min(p / comp);
        }
    }

    let zoom_enabled = compute_params.adaptive_zoom_window < -0.9 || compute_params.adaptive_zoom_window > 0.0001;

    // `final_fovs_minimal` is the honest measurement the FOV warning and the safe-area overlay compare
    // against 1.0; with focal length smoothing that's `coverage`, the frame relative to the compensated view
    let final_fovs_minimal = coverage;
    let mut final_fovs = if compute_params.adaptive_zoom_window < -0.9 {
        // Static zoom
        if let Some(max_f) = fov_values.iter().copied().reduce(f64::min) {
            fov_values.iter_mut().for_each(|v| *v = max_f);
        }
        fov_values
    } else if compute_params.adaptive_zoom_window > 0.0001 {
        // Dynamic zoom
        zoom_dynamic::compute(&compute_params, fov_values, timestamps, method).0
    } else {
        // Disabled zoom
        vec![1.0; fov_values.len()]
    };

    // Safety pad so the applied zoom never samples the outermost `fov_algorithm_margin` pixels of
    // the source. `fov` is the fitted rectangle's width as a fraction of `width`, and the rectangle
    // keeps the output aspect, so its height is `fov * out_h` (`out_h` is the output height in the
    // same source-width units - FovIterative's `output_dim.1`). Subtracting `2 * margin / dim` shrinks
    // that axis by exactly `margin` px per side no matter how zoomed-in `fov` already is (a
    // multiplicative pad would only inset `fov * margin`); using the smaller dimension makes the
    // inset >= `margin` on both axes.
    // Applied to the *applied* fovs only - never to `final_fovs_minimal`, which must stay an honest
    // measurement (the FOV warning and the safe-area overlay compare it against 1.0). Not applied
    // when zooming is off: the user asked for no crop.
    if zoom_enabled && compute_params.fov_algorithm_margin > 0.0 {
        let out_h = org_output_size.1 as f64 * compute_params.width as f64 / org_output_size.0.max(1) as f64;
        let min_dim = (compute_params.width as f64).min(out_h);
        if min_dim > 0.0 {
            let inset = 2.0 * compute_params.fov_algorithm_margin as f64 / min_dim;
            final_fovs.iter_mut().for_each(|v| *v = (*v - inset).max(0.001)); // same floor as FrameTransform::get_fov
        }
    }

    (final_fovs, final_fovs_minimal, debug_points)
}

/// Key of everything the zoom pass reads, so `recompute_threaded` can tell whether the stored fovs are stale.
/// `smoothing_checksum` is the smoothing state key (`Smoothing::get_state_checksum`): the zoom fits the output
/// frame around the smoothed quaternions, so it has to follow them as well
pub fn get_checksum(compute_params: &ComputeParams, smoothing_checksum: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write_u64(smoothing_checksum);

    // Lens: the profile's projection, which also covers `distortion_model`, `digital_lens` and `digital_lens_params`
    // (`ComputeParams::from_manager` derives them from the profile and nothing else writes them)
    hasher.write_u64(compute_params.lens.get_checksum());
    hasher.write_u64(compute_params.lens_correction_amount.to_bits());
    hasher.write_u64(compute_params.light_refraction_coefficient.to_bits());

    // Video geometry, timing and rolling shutter
    hasher.write_usize(compute_params.width);
    hasher.write_usize(compute_params.height);
    hasher.write_usize(compute_params.output_width);
    hasher.write_usize(compute_params.output_height);
    hasher.write_u64(compute_params.scaled_fps.to_bits());
    hasher.write_u64(compute_params.frame_readout_time.to_bits());
    hasher.write_i32(compute_params.frame_readout_direction as i32);
    for x in compute_params.trim_ranges.iter() {
        hasher.write_u64(x.0.to_bits());
        hasher.write_u64(x.1.to_bits());
    }
    hasher.write_u64(compute_params.video_rotation.to_bits());
    hasher.write_u64(compute_params.video_speed.to_bits());
    hasher.write_u8(compute_params.video_speed_affects_zooming as u8);
    hasher.write_u8(compute_params.video_speed_affects_zooming_limit as u8);

    // Zoom settings
    hasher.write_u64(compute_params.adaptive_zoom_window.to_bits());
    hasher.write_i32(compute_params.adaptive_zoom_method);
    hasher.write_u64(compute_params.adaptive_zoom_center_offset.0.to_bits());
    hasher.write_u64(compute_params.adaptive_zoom_center_offset.1.to_bits());
    hasher.write_u64(compute_params.additional_translation.0.to_bits());
    hasher.write_u64(compute_params.additional_translation.1.to_bits());
    hasher.write_u64(compute_params.additional_translation.2.to_bits());
    hasher.write_u64(compute_params.max_zoom.unwrap_or_default().to_bits());
    hasher.write_usize(compute_params.max_zoom_iterations);
    hasher.write_u32(compute_params.fov_algorithm_margin.to_bits());

    // Focal length stabilization: the zoom accounts for the compensation, so it has to follow the curves themselves
    hasher.write_u8(compute_params.focal_length_smoothing_enabled as u8);
    hasher.write_u64(compute_params.focal_length_max_zoom_rate.to_bits());
    hasher.write_i32(compute_params.lens_metadata_delay_frames);
    hasher.write_u8(compute_params.lens_breathing_enabled as u8);
    for x in compute_params.focal_lengths.iter().chain(compute_params.smoothed_focal_lengths.iter()) {
        hasher.write_u64(x.unwrap_or_default().to_bits());
    }

    // Keyframes the zoom evaluates per frame (the additional rotation ones act through the smoothing)
    use crate::keyframes::KeyframeType::*;
    hasher.write_u64(compute_params.keyframes.get_checksum_for(&[
        VideoRotation, ZoomingSpeed, ZoomingCenterX, ZoomingCenterY, MaxZoom,
        AdditionalTranslationX, AdditionalTranslationY, AdditionalTranslationZ,
        LensCorrectionStrength, LightRefractionCoeff, VideoSpeed,
    ]));

    hasher.finish()
}
