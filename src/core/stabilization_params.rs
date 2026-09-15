// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::BTreeMap;

use nalgebra::Vector4;

use crate::keyframes::*;

#[derive(Default, Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum BackgroundMode {
    #[default]
    SolidColor = 0,
    RepeatPixels = 1,
    MirrorPixels = 2,
    MarginWithFeather = 3,
}
impl From<i32> for BackgroundMode {
    fn from(v: i32) -> Self {
        match v {
            1 => Self::RepeatPixels,
            2 => Self::MirrorPixels,
            3 => Self::MarginWithFeather,
            _ => Self::SolidColor
        }
    }
}
#[derive(Default, Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum ReadoutDirection {
    #[default]
    TopToBottom = 0,
    BottomToTop = 1,
    LeftToRight = 2,
    RightToLeft = 3,
}
impl From<i32> for ReadoutDirection {
    fn from(v: i32) -> Self {
        match v {
            1 => Self::BottomToTop,
            2 => Self::LeftToRight,
            3 => Self::RightToLeft,
            _ => Self::TopToBottom
        }
    }
}
/// How Auto sync uses optical-flow camera motion relative to gyro/IMU data
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpticalMotionMode {
    /// Existing behavior: optical flow becomes camera motion only when the file has no gyro/IMU
    #[default]
    Auto = 0,
    /// Auto sync always writes the optical-flow estimate as camera motion, even if gyro exists
    OpticalOnly = 1,
    /// Never replace gyro/IMU with optical flow
    GyroOnly = 2,
}
impl From<i32> for OpticalMotionMode {
    fn from(v: i32) -> Self {
        match v {
            1 => Self::OpticalOnly,
            2 => Self::GyroOnly,
            _ => Self::Auto,
        }
    }
}
impl OpticalMotionMode {
    /// Whether a finished Auto sync should copy estimated optical-flow rates into the gyro source
    pub fn uses_optical_as_motion(self, file_has_motion: bool) -> bool {
        match self {
            Self::Auto => !file_has_motion,
            Self::OpticalOnly => true,
            Self::GyroOnly => false,
        }
    }
}

/// 2D similarity `p' = scale * R(rot) * p + (tx, ty)` in pixels / radians
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Similarity2D {
    pub tx: f64,
    pub ty: f64,
    pub rot: f64,
    pub scale: f64,
}
impl Default for Similarity2D {
    fn default() -> Self { Self::identity() }
}
impl Similarity2D {
    pub fn identity() -> Self {
        Self { tx: 0.0, ty: 0.0, rot: 0.0, scale: 1.0 }
    }
    pub fn is_finite(&self) -> bool {
        self.tx.is_finite() && self.ty.is_finite() && self.rot.is_finite() && self.scale.is_finite() && self.scale > 0.0
    }
    pub fn is_near_identity(&self) -> bool {
        self.tx.abs() < 1e-9 && self.ty.abs() < 1e-9 && self.rot.abs() < 1e-9 && (self.scale - 1.0).abs() < 1e-9
    }
    /// `self` then `other` (other is applied after self)
    pub fn compose(self, other: Self) -> Self {
        let (c, s) = (other.rot.cos(), other.rot.sin());
        let sc = other.scale;
        Self {
            tx: sc * (c * self.tx - s * self.ty) + other.tx,
            ty: sc * (s * self.tx + c * self.ty) + other.ty,
            rot: self.rot + other.rot,
            scale: self.scale * other.scale,
        }
    }
    pub fn inverse(self) -> Self {
        let (c, s) = (self.rot.cos(), self.rot.sin());
        let inv_s = if self.scale.abs() > 1e-12 { 1.0 / self.scale } else { 1.0 };
        Self {
            tx: -inv_s * ( c * self.tx + s * self.ty),
            ty: -inv_s * (-s * self.tx + c * self.ty),
            rot: -self.rot,
            scale: inv_s,
        }
    }
    pub fn scale_strength(self, strength: f64) -> Self {
        let t = strength.clamp(0.0, 1.0);
        Self {
            tx: self.tx * t,
            ty: self.ty * t,
            rot: self.rot * t,
            scale: 1.0 + (self.scale - 1.0) * t,
        }
    }
    pub fn apply(self, p: (f64, f64)) -> (f64, f64) {
        let (c, s) = (self.rot.cos(), self.rot.sin());
        let x = self.scale * (c * p.0 - s * p.1) + self.tx;
        let y = self.scale * (s * p.0 + c * p.1) + self.ty;
        (x, y)
    }
    /// Homography about `(cx, cy)`: rotate/scale around the centre, then translate
    pub fn as_matrix_about(self, cx: f64, cy: f64) -> nalgebra::Matrix3<f64> {
        let (c, s) = ((self.scale * self.rot.cos()), (self.scale * self.rot.sin()));
        nalgebra::Matrix3::new(
            c, -s, self.tx + cx - c * cx + s * cy,
            s,  c, self.ty + cy - s * cx - c * cy,
            0.0, 0.0, 1.0,
        )
    }
}

impl From<&str> for ReadoutDirection {
    fn from(v: &str) -> Self {
        match v {
            "BottomToTop" => Self::BottomToTop,
            "LeftToRight" => Self::LeftToRight,
            "RightToLeft" => Self::RightToLeft,
            _ => Self::TopToBottom
        }
    }
}
impl ReadoutDirection {
    pub fn is_horizontal(&self) -> bool {
        matches!(self, Self::LeftToRight | Self::RightToLeft)
    }
    pub fn is_inverted(&self) -> bool {
        matches!(self, Self::BottomToTop | Self::RightToLeft)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct StabilizationParams {
    pub size: (usize, usize), // Full resolution input size
    pub output_size: (usize, usize), // Full resoution output size

    pub background: Vector4<f32>,

    pub frame_readout_time: f64,
    pub frame_readout_direction: ReadoutDirection,
    pub adaptive_zoom_window: f64,
    pub adaptive_zoom_center_offset: (f64, f64),
    pub adaptive_zoom_method: i32,
    pub additional_rotation: (f64, f64, f64),
    pub additional_translation: (f64, f64, f64),
    pub fov: f64,
    pub fov_overview: bool,
    pub max_zoom: Option<f64>,
    pub max_zoom_iterations: usize,
    pub show_safe_area: bool,
    pub fovs: Vec<f64>,
    pub minimal_fovs: Vec<f64>,
    pub min_fov: f64,
    pub fps: f64,
    pub fps_scale: Option<f64>,
    pub video_speed: f64,
    pub video_speed_affects_smoothing: bool,
    pub video_speed_affects_zooming: bool,
    pub video_speed_affects_zooming_limit: bool,
    pub speed_ramped_timestamps: Option<BTreeMap<i64, i64>>,
    pub frame_count: usize,
    pub duration_ms: f64,
    pub video_created_at: Option<u64>,

    pub trim_ranges: Vec<(f64, f64)>,

    pub video_rotation: f64,

    pub lens_correction_amount: f64,
    pub light_refraction_coefficient: f64,
    pub background_mode: BackgroundMode,
    pub background_margin: f64,
    pub background_margin_feather: f64,

    pub framebuffer_inverted: bool,
    pub is_calibrator: bool,

    pub stab_enabled: bool,
    pub show_detected_features: bool,
    pub show_optical_flow: bool,

    pub frame_offset: i32,

    pub of_method: u32,
    pub current_device: i32,

    /// Auto / optical-only / gyro-only: whether Auto sync writes optical-flow rates as camera motion
    pub optical_motion_mode: OpticalMotionMode,
    /// Second-pass global 2D residual after camera-motion stabilization
    pub optical_residual_enabled: bool,
    /// 0–1 blend of the residual correction
    pub optical_residual_strength: f64,
    /// Box-filter window (seconds) used to separate intentional motion from leftover jitter
    pub optical_residual_smooth_window: f64,

    pub zooming_debug_points: std::collections::BTreeMap<i64, Vec<(f64, f64)>>,

    // Focal length smoothing
    pub focal_lengths: Vec<Option<f64>>,
    pub smoothed_focal_lengths: Vec<Option<f64>>,
    /// The delay-free dequantized curve the two above are derived from (`smoothing::focal_length::compute_base_curve`),
    /// with a hash of everything it depends on: the file's lens metadata, the lens profile and the video geometry, no
    /// setting. A recompute reuses it and only re-derives the curves, so a slider tick never repeats the per-frame sweep
    pub focal_length_base: Vec<f64>,
    pub focal_length_base_key: u64,
    pub focal_length_smoothing_enabled: bool,
    pub focal_length_max_zoom_rate: f64,
    pub lens_metadata_delay_frames: i32, // how many frames the lens metadata lags the picture

    pub lens_breathing_enabled: bool, // Sony lens breathing compensation, when the file carries the lens tables
}
impl Default for StabilizationParams {
    fn default() -> Self {
        Self {
            fov: 1.0,
            fov_overview: false,
            show_safe_area: false,
            min_fov: 1.0,
            fovs: vec![],
            minimal_fovs: vec![],
            stab_enabled: true,
            show_detected_features: true,
            show_optical_flow: true,
            frame_readout_time: 0.0,
            frame_readout_direction: ReadoutDirection::TopToBottom,
            adaptive_zoom_window: 4.0,
            adaptive_zoom_center_offset: (0.0, 0.0),
            adaptive_zoom_method: 1,

            additional_rotation: (0.0, 0.0, 0.0),
            additional_translation: (0.0, 0.0, 0.0),

            size: (0, 0),
            output_size: (0, 0),

            video_rotation: 0.0,

            max_zoom: Some(130.0),
            max_zoom_iterations: 5,

            lens_correction_amount: 1.0,
            light_refraction_coefficient: 1.0,
            background_mode: BackgroundMode::SolidColor,
            background_margin: 0.0,
            background_margin_feather: 0.0,

            framebuffer_inverted: false,
            is_calibrator: false,

            frame_offset: 0,

            trim_ranges: Vec::new(),

            zooming_debug_points: BTreeMap::new(),

            background: Vector4::new(0.0, 0.0, 0.0, 0.0),

            of_method: 2,
            optical_motion_mode: OpticalMotionMode::Auto,
            optical_residual_enabled: false,
            optical_residual_strength: 1.0,
            optical_residual_smooth_window: 0.5,

            current_device: 0,

            fps: 0.0,
            fps_scale: None,
            video_speed: 1.0,
            video_speed_affects_smoothing: true,
            video_speed_affects_zooming: true,
            video_speed_affects_zooming_limit: true,
            speed_ramped_timestamps: None,
            frame_count: 0,
            duration_ms: 0.0,
            video_created_at: None,

            focal_lengths: vec![],
            smoothed_focal_lengths: vec![],
            focal_length_base: vec![],
            focal_length_base_key: 0,
            focal_length_smoothing_enabled: false,
            focal_length_max_zoom_rate: 0.5,
            lens_metadata_delay_frames: 0,

            lens_breathing_enabled: true,
        }
    }
}

impl StabilizationParams {
    pub fn get_trim_ratio(&self) -> f64 {
        if self.trim_ranges.is_empty() {
            1.0
        } else {
            self.trim_ranges.iter().fold(0.0, |acc, &x| acc + (x.1 - x.0))
        }
    }
    /// `fps_scale` is a multiplier for `fps`, so only a finite, positive value makes any sense.
    pub fn is_valid_fps_scale(scale: f64) -> bool { scale.is_finite() && scale > 0.0 }
    pub fn set_fps_scale(&mut self, scale: Option<f64>) {
        self.fps_scale = match scale {
            Some(v) if !Self::is_valid_fps_scale(v) => {
                log::warn!("Ignoring invalid fps scale: {v}");
                None
            },
            v => v
        };
    }

    pub fn get_scaled_duration_ms(&self) -> f64 {
        match self.fps_scale {
            Some(scale) if Self::is_valid_fps_scale(scale) => self.duration_ms / scale,
            _ => self.duration_ms
        }
    }
    pub fn get_scaled_fps(&self) -> f64 {
        match self.fps_scale {
            Some(scale) if Self::is_valid_fps_scale(scale) => self.fps * scale,
            _ => self.fps
        }
    }

    pub fn set_fovs(&mut self, fovs: Vec<f64>, mut lens_fov_adjustment: f64) {
        if let Some(mut min_fov) = fovs.iter().copied().reduce(f64::min) {
            min_fov *= self.size.0 as f64 / self.output_size.0.max(1) as f64;
            if lens_fov_adjustment <= 0.0001 { lens_fov_adjustment = 1.0 };
            self.min_fov = min_fov / lens_fov_adjustment;
        }
        if fovs.is_empty() {
            self.min_fov = 1.0;
        }
        self.fovs = fovs;
    }

    pub fn calculate_ramped_timestamps(&mut self, keyframes: &KeyframeManager, speed_inverse: bool, map_inverse: bool) {
        if keyframes.is_keyframed(&KeyframeType::VideoSpeed) || self.video_speed != 1.0 {
            let fps = self.fps; // get_scaled_fps();
            let mut ramped_ts = 0.0;
            let mut prev_real_ts = 0.0;
            let mut map = BTreeMap::new();
            for i in 0..self.frame_count {
                let ts = crate::timestamp_at_frame(i as i32, fps);
                let vid_speed = keyframes.value_at_video_timestamp(&KeyframeType::VideoSpeed, ts).unwrap_or(self.video_speed);
                let vid_speed = if speed_inverse {
                    1.0 / vid_speed
                } else {
                    vid_speed
                };
                let current_interval = ((ts - prev_real_ts) as f64) / vid_speed;
                ramped_ts += current_interval;
                prev_real_ts = ts;
                if map_inverse {
                    map.insert((ts * 1000.0).round() as i64, (ramped_ts * 1000.0).round() as i64);
                } else {
                    map.insert((ramped_ts * 1000.0).round() as i64, (ts * 1000.0).round() as i64);
                }
            }

            self.speed_ramped_timestamps = Some(map);
        }
    }
    pub fn get_source_timestamp_at_ramped_timestamp(&self, timestamp_us: i64) -> i64 {
        if let Some(map) = &self.speed_ramped_timestamps {
            match map.len() {
                0 => { return timestamp_us; },
                1 => { return *map.values().next().unwrap(); },
                _ => {
                    if let Some(&first_ts) = map.keys().next() {
                        if let Some(&last_ts) = map.keys().next_back() {
                            let lookup_ts = timestamp_us.min(last_ts-1).max(first_ts+1);
                            if let Some(v1) = map.range(..=lookup_ts).next_back() {
                                if *v1.0 == lookup_ts {
                                    return *v1.1;
                                }
                                if let Some(v2) = map.range(lookup_ts..).next() {
                                    let time_delta = (v2.0 - v1.0) as f64;
                                    let fract = (timestamp_us - v1.0) as f64 / time_delta;
                                    return (*v1.1 as f64 + (*v2.1 as f64 - *v1.1 as f64) * fract).round() as i64;
                                }
                            }
                        }
                    }
                    return timestamp_us;
                }
            }
        }
        timestamp_us
    }

    pub fn clear(&mut self) {
        *self = StabilizationParams {
            stab_enabled:              self.stab_enabled,
            show_detected_features:    self.show_detected_features,
            show_optical_flow:         self.show_optical_flow,
            background:                self.background,
            adaptive_zoom_window:      self.adaptive_zoom_window,
            framebuffer_inverted:      self.framebuffer_inverted,
            lens_correction_amount:    self.lens_correction_amount,
            video_speed:               self.video_speed,
            video_speed_affects_smoothing: self.video_speed_affects_smoothing,
            video_speed_affects_zooming:   self.video_speed_affects_zooming,
            video_speed_affects_zooming_limit: self.video_speed_affects_zooming_limit,
            light_refraction_coefficient:  self.light_refraction_coefficient,
            background_mode:           self.background_mode,
            background_margin:         self.background_margin,
            background_margin_feather: self.background_margin_feather,
            of_method:                 self.of_method,
            optical_motion_mode:       self.optical_motion_mode,
            optical_residual_enabled:  self.optical_residual_enabled,
            optical_residual_strength: self.optical_residual_strength,
            optical_residual_smooth_window: self.optical_residual_smooth_window,
            current_device:            self.current_device,
            adaptive_zoom_method:      self.adaptive_zoom_method,
            fov_overview:              self.fov_overview,
            show_safe_area:            self.show_safe_area,
            max_zoom:                  self.max_zoom,
            max_zoom_iterations:       self.max_zoom_iterations,
            focal_length_smoothing_enabled: self.focal_length_smoothing_enabled,
            focal_length_max_zoom_rate: self.focal_length_max_zoom_rate,
            ..Self::default()
        };
    }
}
