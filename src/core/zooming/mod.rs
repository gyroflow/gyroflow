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
