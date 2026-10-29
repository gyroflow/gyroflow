// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::StabilizationManager;
use super::distortion_models::DistortionModel;
use crate::stabilization_params::ReadoutDirection;
use crate::GyroSource;
use crate::keyframes::KeyframeManager;
use crate::lens_profile::LensProfile;
use std::sync::Arc;
use parking_lot::RwLock;

#[derive(Default, Clone)]
pub struct ComputeParams {
    pub gyro: Arc<RwLock<GyroSource>>,
    pub fovs: Vec<f64>,
    pub minimal_fovs: Vec<f64>,
    pub keyframes: KeyframeManager,
    pub lens: LensProfile,
    pub camera_diagonal_fovs: Vec<f64>,

    pub frame_count: usize,
    pub fov_scale: f64,
    pub fov_overview: bool,
    pub show_safe_area: bool,
    pub width: usize,
    pub height: usize,
    pub output_width: usize,
    pub output_height: usize,
    pub video_rotation: f64,
    pub lens_correction_amount: f64,
    /// [`crate::stabilization::projection::OutputProjection`] as `i32`: the one picked in the Lens profile
    /// section, with "Automatic" already resolved for the clip's lens, so never `projection::AUTOMATIC` here
    pub output_projection: i32,
    pub light_refraction_coefficient: f64,
    pub video_speed: f64,
    pub video_speed_affects_smoothing: bool,
    pub video_speed_affects_zooming: bool,
    pub video_speed_affects_zooming_limit: bool,
    pub background: nalgebra::Vector4<f32>,
    pub background_mode: crate::stabilization_params::BackgroundMode,
    pub background_margin: f64,
    pub background_margin_feather: f64,
    pub frame_readout_time: f64,
    pub frame_readout_direction: ReadoutDirection,
    pub trim_ranges: Vec<(f64, f64)>,
    pub scaled_fps: f64,
    pub scaled_duration_ms: f64,
    /// The video's frame times and the frame rate scale (`StabilizationParams::frame_times`, `fps_scale`), which
    /// `frame_at_timestamp` and `timestamp_at_frame` map frames by
    pub frame_times: Option<Arc<crate::frame_times::FrameTimes>>,
    pub fps_scale: f64,
    pub adaptive_zoom_window: f64,
    pub adaptive_zoom_center_offset: (f64, f64),
    pub adaptive_zoom_method: i32,
    pub additional_rotation: (f64, f64, f64),
    pub additional_translation: (f64, f64, f64),
    pub framebuffer_inverted: bool,
    pub suppress_rotation: bool,
    pub fov_algorithm_margin: f32,
    pub smoothing_fov_limit_per_frame: Vec<f64>,
    /// The lens per lens state, for the crop constraint of the optimal smoothing, see `calculate_camera_fovs`
    pub smoothing_crop_lenses: Option<Arc<crate::smoothing::optimal::crop::CropLenses>>,
    /// Per frame, the time (ms) the render reads its orientations at, in the motion's own timeline, see `calculate_camera_fovs`
    pub smoothing_frame_times: Option<Arc<Vec<f64>>>,
    /// The frames being smoothed, when they're a segment of a video cut from recordings, smoothed on its own
    /// (`GyroSource::recompute_smoothness`); all of them when `None`
    pub smoothing_frames: Option<std::ops::Range<usize>>,
    pub max_zoom: Option<f64>,
    pub max_zoom_iterations: usize,
    pub fixed_zoom: bool,
    pub zoom_limit_whole_clip: bool,

    pub zooming_debug_points: bool,

    pub distortion_model: DistortionModel,
    pub digital_lens: Option<DistortionModel>,
    pub digital_lens_params: Option<Vec<f64>>,

    // Focal length stabilization, see smoothing::focal_length. Per frame, in output pixels
    pub focal_lengths: Vec<Option<f64>>,          // effective focal length the renderer projects with (dequantized)
    pub smoothed_focal_lengths: Vec<Option<f64>>, // target of the output zoom (smoothing envelope, breathing compensation), empty: no zoom (`focal_length::compensation`)
    pub focal_length_smoothing_enabled: bool,
    pub focal_length_max_zoom_rate: f64,          // d ln(f) / dt in 1/s, 0.5 is roughly 50% of magnification per second
    pub lens_metadata_delay_frames: i32,          // every per-frame lens lookup is shifted by this many frames, see synchronization::lens_delay

    pub lens_breathing_enabled: bool,             // breathing compensation: the target holds the narrowest field of view (the breathing itself is in `focal_lengths` either way)
}
impl ComputeParams {
    /// Time (µs) the lens metadata of the picture at `timestamp_ms` is looked up at: the frame time shifted by
    /// `lens_metadata_delay_frames`, the delay the lens reports its state with (see `synchronization::lens_delay`). The
    /// one place the shift is spelled out; every per-frame lens lookup, the projection's and the focal length curves',
    /// goes through it, so they can't disagree about which metadata entry a frame gets
    pub fn lens_timestamp_us(&self, timestamp_ms: f64) -> i64 {
        Self::lens_timestamp_us_with_delay(timestamp_ms, self.lens_metadata_delay_frames, self.scaled_fps)
    }
    /// `lens_timestamp_us` for a given delay
    pub fn lens_timestamp_us_with_delay(timestamp_ms: f64, delay_frames: i32, fps: f64) -> i64 {
        ((timestamp_ms + crate::timestamp_at_frame(delay_frames, fps.max(1e-9))) * 1000.0).round() as i64
    }

    /// The frame (its ordinal) presented at the video's time `timestamp_ms`, `StabilizationParams::frame_at_timestamp`
    pub fn frame_at_timestamp(&self, timestamp_ms: f64) -> i32 {
        match &self.frame_times {
            Some(times) => times.frame_at_timestamp(timestamp_ms, if self.fps_scale > 0.0 { self.fps_scale } else { 1.0 }),
            None => crate::frame_at_timestamp(timestamp_ms, self.scaled_fps),
        }
    }
    /// The video's time (ms) frame `frame` is presented at, `StabilizationParams::timestamp_at_frame`
    pub fn timestamp_at_frame(&self, frame: i32) -> f64 {
        match &self.frame_times {
            Some(times) => times.timestamp_at_frame(frame, if self.fps_scale > 0.0 { self.fps_scale } else { 1.0 }),
            None => crate::timestamp_at_frame(frame, self.scaled_fps),
        }
    }

    pub fn from_manager(mgr: &StabilizationManager) -> Self {
        let params = mgr.params.read();

        let lens = mgr.lens.read().clone();

        let distortion_model = DistortionModel::from_name(lens.distortion_model.as_deref().unwrap_or("opencv_fisheye"));
        let digital_lens = lens.digital_lens.as_ref().map(|x| DistortionModel::from_name(&x));

        let digital_lens_params = lens.digital_lens_params.clone();

        let mut ret = Self {
            gyro: mgr.gyro.clone(),
            lens,
            camera_diagonal_fovs: Vec::new(),

            smoothing_fov_limit_per_frame: Vec::new(),
            smoothing_crop_lenses: None,
            smoothing_frame_times: None,
            smoothing_frames: None,
            max_zoom: params.max_zoom.clone(),
            max_zoom_iterations: params.max_zoom_iterations,
            fixed_zoom: params.fixed_zoom,
            zoom_limit_whole_clip: params.zoom_limit_whole_clip,

            frame_count: params.frame_count,
            fov_scale: params.fov,
            fov_overview: params.fov_overview,
            show_safe_area: params.show_safe_area,
            fovs: params.fovs.clone(),
            minimal_fovs: params.minimal_fovs.clone(),
            width: params.size.0.max(1),
            height: params.size.1.max(1),
            output_width: params.output_size.0.max(1),
            output_height: params.output_size.1.max(1),
            video_rotation: params.video_rotation,
            background: params.background,
            background_mode: params.background_mode,
            background_margin: params.background_margin,
            background_margin_feather: params.background_margin_feather,
            lens_correction_amount: params.lens_correction_amount,
            output_projection: params.output_projection,
            light_refraction_coefficient: params.light_refraction_coefficient,
            framebuffer_inverted: params.framebuffer_inverted,
            frame_readout_time: params.frame_readout_time,
            frame_readout_direction: params.frame_readout_direction,
            trim_ranges: params.trim_ranges.clone(),
            scaled_fps: params.get_scaled_fps(),
            scaled_duration_ms: params.get_scaled_duration_ms(),
            frame_times: params.frame_times.clone(),
            fps_scale: params.get_fps_scale(),
            adaptive_zoom_window: params.adaptive_zoom_window,
            adaptive_zoom_center_offset: params.adaptive_zoom_center_offset,
            additional_rotation: params.additional_rotation,
            additional_translation: params.additional_translation,
            adaptive_zoom_method: params.adaptive_zoom_method,
            video_speed: params.video_speed,
            video_speed_affects_smoothing: params.video_speed_affects_smoothing,
            video_speed_affects_zooming: params.video_speed_affects_zooming,
            video_speed_affects_zooming_limit: params.video_speed_affects_zooming_limit,

            distortion_model,
            digital_lens,
            digital_lens_params,
            suppress_rotation: false,
            fov_algorithm_margin: 3.0,

            keyframes: mgr.keyframes.read().clone(),

            zooming_debug_points: false,

            focal_lengths: params.focal_lengths.clone(),
            smoothed_focal_lengths: params.smoothed_focal_lengths.clone(),
            focal_length_smoothing_enabled: params.focal_length_smoothing_enabled,
            focal_length_max_zoom_rate: params.focal_length_max_zoom_rate,
            lens_metadata_delay_frames: params.lens_metadata_delay_frames,

            lens_breathing_enabled: params.lens_breathing_enabled,
        };
        // Everything above is a read of `mgr.params`, and nothing below needs it. "Automatic" measures the
        // projection on the lens data, which takes the `gyro` and `file_metadata` locks (`get_lens_data_at_timestamp`),
        // and this runs under `stabilization.write()` at some call sites - so the guard is dropped first
        // rather than held across two more locks it has no reason to be nested with
        drop(params);

        // "Automatic": rectilinear for every lens rectilinear can hold, stereographic for the ones it can't (a
        // 200° body) - measured on the lens data the render will use, so the choice and the picture agree
        if ret.output_projection == crate::stabilization::projection::AUTOMATIC {
            ret.output_projection = crate::stabilization::projection::OutputProjection::for_lens(&ret) as i32;
        }
        ret
    }

    pub fn calculate_camera_fovs(&mut self) {
        // Per frame only when the focal length changes over the clip: the metadata's, in whichever unit the camera reports
        // it (see `FrameTransform::get_lens_data_at_timestamp`), or the lens's own through its breathing (the crop lenses
        // are scaled to it, `FrameTransform::effective_camera_matrix`); a fixed lens has one FOV
        let frame_count = if self.gyro.read().file_metadata.read().has_varying_focal_length() {
            self.frame_count
        } else {
            1 // FOV is constant (ie. lens is fixed focal length)
        };
        self.camera_diagonal_fovs = Vec::with_capacity(frame_count);
        // The smoothing doesn't look the lens up itself (it may run where the gyro lock is held, by a caller of
        // `GyroSource::recompute_smoothness`), so the crop constraint of the optimal smoothing takes it from here, the
        // same lookups
        let mut crop_lenses = crate::smoothing::optimal::crop::CropLenses::default();
        for f in 0..frame_count as i32 {
            let timestamp = self.timestamp_at_frame(f);
            let lens = crate::stabilization::FrameTransform::get_lens_data_at_timestamp(&self, timestamp, false);
            let camera_matrix = lens.0;
            let diag_length = ((self.width.pow(2) + self.height.pow(2)) as f64).sqrt();
            // let diag_pixel_focal_length = (camera_matrix[(0, 0)].powi(2) + camera_matrix[(1, 1)].powi(2)).sqrt();
            let d_fov = 2.0 * ((diag_length / (2.0 * camera_matrix[(1, 1)])).atan()) * 180.0 / std::f64::consts::PI;
            self.camera_diagonal_fovs.push(d_fov);
            crop_lenses.push(self, f as usize, &lens);
        }
        crop_lenses.finish();
        self.smoothing_crop_lenses = Some(Arc::new(crop_lenses));

        // ... and the time the render reads each frame's orientations at, in the motion's own timeline: the frame's
        // time with its per-frame offset, less the sync offset there (`GyroSource::quat_at_timestamp`)
        let times = {
            let gyro = self.gyro.read();
            let file_metadata = gyro.file_metadata.read();
            (0..self.frame_count).map(|f| {
                let t = self.timestamp_at_frame(f as i32) + file_metadata.frame_time_offset(f);
                t - gyro.offset_at_frame(f, t)
            }).collect()
        };
        self.smoothing_frame_times = Some(Arc::new(times));
    }
}

impl std::fmt::Debug for ComputeParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let gyro = self.gyro.read();
        f.debug_struct("ComputeParams")
         .field("gyro.imu_orientation", &gyro.imu_transforms.imu_orientation)
         .field("gyro.imu_rotation", &gyro.imu_transforms.imu_rotation_angles)
         .field("gyro.acc_rotation", &gyro.imu_transforms.acc_rotation_angles)
         .field("gyro.duration_ms", &gyro.duration_ms)
         .field("gyro.imu_lpf", &gyro.imu_transforms.imu_lpf)
         .field("gyro.imu_mf", &gyro.imu_transforms.imu_mf)
         .field("gyro.gyro_bias", &gyro.imu_transforms.gyro_bias)
         .field("gyro.integration_method", &gyro.integration_method)
         .field("fovs.len", &self.fovs.len())
         .field("keyframed", &self.keyframes.get_all_keys())

         .field("frame_count",          &self.frame_count)
         .field("fov_scale",            &self.fov_scale)
         .field("fov_overview",         &self.fov_overview)
         .field("width",                &self.width)
         .field("height",               &self.height)
         .field("output_width",         &self.output_width)
         .field("output_height",        &self.output_height)
         .field("video_rotation",       &self.video_rotation)
         .field("lens_correction_amount",    &self.lens_correction_amount)
         .field("light_refraction_coefficient", &self.light_refraction_coefficient)
         .field("background_mode",           &self.background_mode)
         .field("background_margin",         &self.background_margin)
         .field("background_margin_feather", &self.background_margin_feather)
         .field("frame_readout_time",        &self.frame_readout_time)
         .field("frame_readout_direction",   &self.frame_readout_direction)
         .field("trim_ranges",               &self.trim_ranges)
         .field("scaled_fps",                &self.scaled_fps)
         .field("adaptive_zoom_window",      &self.adaptive_zoom_window)
         .field("adaptive_zoom_center_offset", &self.adaptive_zoom_center_offset)
         .field("additional_rotation",       &self.additional_rotation)
         .field("additional_translation",    &self.additional_translation)
         .field("adaptive_zoom_method",      &self.adaptive_zoom_method)
         .field("fixed_zoom",                &self.fixed_zoom)
         .field("zoom_limit_whole_clip",     &self.zoom_limit_whole_clip)
         .field("framebuffer_inverted",      &self.framebuffer_inverted)
         .field("zooming_debug_points",      &self.zooming_debug_points)
         .field("distortion_model",          &self.distortion_model.id())
         .field("digital_lens",              &self.digital_lens.as_ref().map(|x| x.id()).unwrap_or("None"))
         .field("focal_length_smoothing_enabled", &self.focal_length_smoothing_enabled)
         .field("focal_length_max_zoom_rate",     &self.focal_length_max_zoom_rate)
         .field("lens_metadata_delay_frames",     &self.lens_metadata_delay_frames)
         .field("focal_lengths.len",         &self.focal_lengths.len())
         .field("lens_breathing_enabled",    &self.lens_breathing_enabled)
         .finish()
    }
}
