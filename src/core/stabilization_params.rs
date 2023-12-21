// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::BTreeMap;

use nalgebra::Vector4;

use crate::keyframes::*;

#[derive(Default, Clone, Copy, Debug)]
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

#[derive(Clone, Debug)]
pub struct StabilizationParams {
    pub size: (usize, usize), // Processing input size
    pub output_size: (usize, usize), // Processing output size
    pub video_size: (usize, usize), // Full resolution input size
    pub video_output_size: (usize, usize), // Full resoution output size

    pub background: Vector4<f32>,

    pub frame_readout_time: f64,
    pub adaptive_zoom_window: f64,
    pub adaptive_zoom_center_offset: (f64, f64),
    pub adaptive_zoom_method: i32,
    pub fov: f64,
    pub fov_overview: bool,
    pub show_safe_area: bool,
    pub fovs: Vec<f64>,
    pub minimal_fovs: Vec<f64>,
    pub min_fov: f64,
    pub fps: f64,
    pub fps_scale: Option<f64>,
    pub plane_scale: Option<(f64, f64)>,
    pub video_speed: f64,
    pub video_speed_affects_smoothing: bool,
    pub video_speed_affects_zooming: bool,
    pub speed_ramped_timestamps: Option<BTreeMap<i64, i64>>,
    pub frame_count: usize,
    pub duration_ms: f64,

    pub trim_start: f64,
    pub trim_end: f64,

    pub video_rotation: f64,

    pub lens_correction_amount: f64,
    pub background_mode: BackgroundMode,
    pub background_margin: f64,
    pub background_margin_feather: f64,

    pub horizontal_rs: bool,
    pub framebuffer_inverted: bool,
    pub is_calibrator: bool,

    pub stab_enabled: bool,
    pub show_detected_features: bool,
    pub show_optical_flow: bool,

    pub of_method: u32,
    pub current_device: i32,

    pub zooming_debug_points: std::collections::BTreeMap<i64, Vec<(f64, f64)>>
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
            adaptive_zoom_window: 4.0,
            adaptive_zoom_center_offset: (0.0, 0.0),
            adaptive_zoom_method: 0,

            size: (0, 0),
            output_size: (0, 0),
            video_size: (0, 0),
            video_output_size: (0, 0),

            video_rotation: 0.0,

            lens_correction_amount: 1.0,
            background_mode: BackgroundMode::SolidColor,
            background_margin: 0.0,
            background_margin_feather: 0.0,

            horizontal_rs: false,
            framebuffer_inverted: false,
            is_calibrator: false,

            trim_start: 0.0,
            trim_end: 1.0,

            zooming_debug_points: BTreeMap::new(),

            background: Vector4::new(0.0, 0.0, 0.0, 0.0),

            of_method: 2,

            current_device: 0,

            fps: 0.0,
            fps_scale: None,
            plane_scale: None,
            video_speed: 1.0,
            video_speed_affects_smoothing: true,
            video_speed_affects_zooming: true,
            speed_ramped_timestamps: None,
            frame_count: 0,
            duration_ms: 0.0,
        }
    }
}

impl StabilizationParams {
    pub fn get_scaled_duration_ms(&self) -> f64 {
        match self.fps_scale {
            Some(scale) => self.duration_ms / scale,
            None            => self.duration_ms
        }
    }
    pub fn get_scaled_fps(&self) -> f64 {
        match self.fps_scale {
            Some(scale) => self.fps * scale,
            None            => self.fps
        }
    }

    pub fn set_fovs(&mut self, fovs: Vec<f64>, mut lens_fov_adjustment: f64) {
        if let Some(mut min_fov) = fovs.iter().copied().reduce(f64::min) {
            min_fov *= self.video_size.0 as f64 / self.video_output_size.0.max(1) as f64;
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
            background_mode:           self.background_mode,
            background_margin:         self.background_margin,
            background_margin_feather: self.background_margin_feather,
            of_method:                 self.of_method,
            current_device:            self.current_device,
            adaptive_zoom_method:      self.adaptive_zoom_method,
            fov_overview:              self.fov_overview,
            show_safe_area:            self.show_safe_area,
            ..Default::default()
        };
    }
}
