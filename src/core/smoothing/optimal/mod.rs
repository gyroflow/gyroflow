// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! "Optimal path": the virtual camera's path solved as an optimization over the whole clip (see `path`), smoothed as
//! far as the crop at the zoom limit allows at every frame (see `crop`).

pub mod banded;
pub mod path;
pub mod crop;

use std::collections::BTreeMap;
use super::*;
use crate::keyframes::*;

#[derive(Clone)]
pub struct Optimal {
    pub smoothness: f64,
    pub smoothness_pitch: f64,
    pub smoothness_yaw: f64,
    pub smoothness_roll: f64,
    pub per_axis: bool,
    pub trim_range_only: bool,
    /// Time constant (s) at smoothness 1
    pub max_time_constant: f64,
    /// Keep the output frame within the source at the zoom limit
    pub crop_constraint: bool,
    pub settings: path::Settings,
}

impl Default for Optimal {
    fn default() -> Self { Self {
        smoothness: 0.5,
        smoothness_pitch: 0.5,
        smoothness_yaw: 0.5,
        smoothness_roll: 0.5,
        per_axis: false,
        trim_range_only: true,
        max_time_constant: 2.0,
        crop_constraint: true,
        settings: path::Settings::default(),
    } }
}

/// The smoothing time constant (s) is this many times the node spacing at most: the normal equations' condition number
/// grows with its 6th power, and past this the path gets sampled more sparsely than the frames instead
const MAX_TAU_PER_NODE: f64 = 80.0;

impl Optimal {
    fn time_constant(&self, smoothness: f64) -> f64 {
        self.max_time_constant * smoothness.max(0.0).powi(2)
    }
}

impl SmoothingAlgorithm for Optimal {
    fn get_name(&self) -> String { "Optimal path".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "smoothness"        => self.smoothness = val,
            "smoothness_pitch"  => self.smoothness_pitch = val,
            "smoothness_yaw"    => self.smoothness_yaw = val,
            "smoothness_roll"   => self.smoothness_roll = val,
            "per_axis"          => self.per_axis = val > 0.1,
            "trim_range_only"   => self.trim_range_only = val > 0.1,
            "max_time_constant" => self.max_time_constant = val,
            "crop_constraint"   => self.crop_constraint = val > 0.1,
            "order_1"           => self.settings.order_weights[0] = val,
            "order_2"           => self.settings.order_weights[1] = val,
            "order_3"           => self.settings.order_weights[2] = val,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameter(&self, name: &str) -> f64 {
        match name {
            "smoothness"        => self.smoothness,
            "smoothness_pitch"  => self.smoothness_pitch,
            "smoothness_yaw"    => self.smoothness_yaw,
            "smoothness_roll"   => self.smoothness_roll,
            "per_axis"          => if self.per_axis { 1.0 } else { 0.0 },
            "trim_range_only"   => if self.trim_range_only { 1.0 } else { 0.0 },
            "max_time_constant" => self.max_time_constant,
            "crop_constraint"   => if self.crop_constraint { 1.0 } else { 0.0 },
            "order_1"           => self.settings.order_weights[0],
            "order_2"           => self.settings.order_weights[1],
            "order_3"           => self.settings.order_weights[2],
            _ => 0.0
        }
    }

    fn get_parameters_json(&self) -> serde_json::Value {
        let slider = |name: &str, description: &str, value: f64, keyframe: &str| serde_json::json!({
            "name": name,
            "description": description,
            "type": "SliderWithField",
            "from": 0.001,
            "to": 1.0,
            "value": value,
            "default": 0.5,
            "unit": "",
            "precision": 3,
            "keyframe": keyframe
        });
        serde_json::json!([
            slider("smoothness",       "Smoothness",       self.smoothness,       "SmoothingParamSmoothness"),
            slider("smoothness_pitch", "Pitch smoothness", self.smoothness_pitch, "SmoothingParamPitch"),
            slider("smoothness_yaw",   "Yaw smoothness",   self.smoothness_yaw,   "SmoothingParamYaw"),
            slider("smoothness_roll",  "Roll smoothness",  self.smoothness_roll,  "SmoothingParamRoll"),
            {
                "name": "per_axis",
                "description": "Per axis",
                "advanced": true,
                "type": "CheckBox",
                "default": self.per_axis,
                "value": if self.per_axis { 1.0 } else { 0.0 },
                "custom_qml": "Connections { function onCheckedChanged() {
                    const checked = root.getParamElement('per_axis').checked;
                    root.getParamElement('smoothness-label').visible = !checked;
                    root.getParamElement('smoothness_pitch-label').visible = checked;
                    root.getParamElement('smoothness_yaw-label').visible = checked;
                    root.getParamElement('smoothness_roll-label').visible = checked;
                }}"
            },
            {
                "name": "crop_constraint",
                "description": "Keep within zoom limit",
                "advanced": true,
                "type": "CheckBox",
                "default": self.crop_constraint,
                "value": if self.crop_constraint { 1.0 } else { 0.0 },
            },
            {
                "name": "trim_range_only",
                "description": "Only within trim range",
                "advanced": true,
                "type": "CheckBox",
                "default": self.trim_range_only,
                "value": if self.trim_range_only { 1.0 } else { 0.0 },
            },
            {
                "name": "max_time_constant",
                "description": "Max smoothness",
                "advanced": true,
                "type": "SliderWithField",
                "from": 0.1,
                "to": 10.0,
                "value": self.max_time_constant,
                "default": 2.0,
                "precision": 2,
                "unit": "s"
            }
        ])
    }

    fn get_status_json(&self) -> serde_json::Value {
        serde_json::json!([])
    }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.smoothness.to_bits());
        hasher.write_u64(self.smoothness_pitch.to_bits());
        hasher.write_u64(self.smoothness_yaw.to_bits());
        hasher.write_u64(self.smoothness_roll.to_bits());
        hasher.write_u8(self.per_axis as u8);
        hasher.write_u8(self.trim_range_only as u8);
        hasher.write_u64(self.max_time_constant.to_bits());
        hasher.write_u8(self.crop_constraint as u8);
        for w in self.settings.order_weights { hasher.write_u64(w.to_bits()); }
        hasher.finish()
    }

    fn get_compute_checksum(&self, p: &ComputeParams) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        // The node grid and the time constants
        hasher.write_u64(p.scaled_fps.to_bits());
        hasher.write_usize(p.frame_count);
        hasher.write_u64(p.scaled_duration_ms.to_bits());
        if let Some(f) = &p.smoothing_frames { hasher.write_usize(f.start); hasher.write_usize(f.end); }
        for r in &p.trim_ranges { hasher.write_u64(r.0.to_bits()); hasher.write_u64(r.1.to_bits()); }
        hasher.write_u8(p.video_speed_affects_smoothing as u8);
        hasher.write_u64(p.video_speed.to_bits());
        let crop = self.crop_constraint && zooms(p);
        hasher.write_u8(crop as u8);
        if crop {
            // Everything the crop model reads
            if let Some(l) = &p.smoothing_crop_lenses { hasher.write_u64(l.checksum()); }
            for t in p.smoothing_frame_times.iter().flat_map(|x| x.iter()) { hasher.write_u64(t.to_bits()); }
            for v in [p.lens_correction_amount, p.adaptive_zoom_center_offset.0, p.adaptive_zoom_center_offset.1, p.adaptive_zoom_window,
                      p.video_rotation, p.frame_readout_time, p.max_zoom.unwrap_or_default()] {
                hasher.write_u64(v.to_bits());
            }
            hasher.write_i32(p.frame_readout_direction as i32);
            hasher.write_i32(p.output_projection);
            hasher.write_u32(p.fov_algorithm_margin.to_bits());
            hasher.write_u8(p.fixed_zoom as u8);
            hasher.write_u8(p.video_speed_affects_zooming_limit as u8);
            hasher.write_usize(p.width);
            hasher.write_usize(p.height);
            hasher.write_usize(p.output_width);
            hasher.write_usize(p.output_height);
            hasher.write(p.distortion_model.id().as_bytes());
            if let Some(d) = &p.digital_lens { hasher.write(d.id().as_bytes()); }
            use crate::keyframes::KeyframeType::*;
            hasher.write_u64(p.keyframes.get_checksum_for(&[LensCorrectionStrength, ZoomingCenterX, ZoomingCenterY, VideoRotation, MaxZoom, VideoSpeed]));
        }
        hasher.finish()
    }

    fn smooth(&self, quats: &TimeQuat, duration_ms: f64, compute_params: &ComputeParams) -> TimeQuat {
        self.smooth_with_original(quats, quats, duration_ms, compute_params)
    }

    fn smooth_with_original(&self, quats: &TimeQuat, original: &TimeQuat, duration_ms: f64, compute_params: &ComputeParams) -> TimeQuat {
        if quats.len() < 2 || duration_ms <= 0.0 { return quats.clone(); }
        let started = std::time::Instant::now();

        let trim = self.trim_range_only && !compute_params.trim_ranges.is_empty();
        let quats = Smoothing::get_trimmed_quats(quats, compute_params.scaled_duration_ms, self.trim_range_only, &compute_params.trim_ranges);
        let quats = quats.as_ref();
        let keyframes = &compute_params.keyframes;

        // The time constant per axis at a timestamp (ms), with the keyframes and the video speed
        let speed_affects = compute_params.video_speed_affects_smoothing && (compute_params.video_speed != 1.0 || keyframes.is_keyframed(&KeyframeType::VideoSpeed));
        let tau_at = |t_ms: f64| -> [f64; 3] {
            let kf = |typ: KeyframeType, def: f64| keyframes.value_at_gyro_timestamp(&typ, t_ms).unwrap_or(def);
            let speed = if speed_affects { kf(KeyframeType::VideoSpeed, compute_params.video_speed).abs() } else { 1.0 };
            let s = if self.per_axis {
                [kf(KeyframeType::SmoothingParamPitch, self.smoothness_pitch), kf(KeyframeType::SmoothingParamYaw, self.smoothness_yaw), kf(KeyframeType::SmoothingParamRoll, self.smoothness_roll)]
            } else {
                [kf(KeyframeType::SmoothingParamSmoothness, self.smoothness); 3]
            };
            // Played `speed` times faster, the motion's frequencies are `speed` times higher: the time constant in the
            // clip's own time scales with it, so the smoothing looks the same
            s.map(|s| self.time_constant(s) * speed)
        };
        let (first_ms, last_ms) = (*quats.keys().next().unwrap() as f64 / 1000.0, *quats.keys().next_back().unwrap() as f64 / 1000.0);
        let max_tau = {
            let mut m = tau_at(first_ms).into_iter().fold(0.0, f64::max);
            for (ts, _) in quats.iter().step_by((quats.len() / 200).max(1)) { m = tau_at(*ts as f64 / 1000.0).into_iter().fold(m, f64::max); }
            m
        };

        // The frames, each at the time the render reads it at; with "Only within trim range" those within the ranges, as
        // the others get the smoothing of the whole clip (see `GyroSource::recompute_smoothness`)
        let fps = compute_params.scaled_fps;
        let frames: Vec<(usize, f64)> = if fps > 0.0 && fps.is_finite() {
            let times = compute_params.smoothing_frame_times.as_deref();
            let duration = compute_params.scaled_duration_ms;
            // The frames of the segment being smoothed, of a video cut from recordings (`ComputeParams::smoothing_frames`)
            let frames = compute_params.smoothing_frames.clone().unwrap_or(0..compute_params.frame_count);
            (frames.start..frames.end.min(compute_params.frame_count)).filter_map(|f| {
                let video_ms = compute_params.timestamp_at_frame(f as i32);
                if trim && !compute_params.trim_ranges.iter().any(|r| video_ms >= r.0 * duration && video_ms <= r.1 * duration) { return None; }
                Some((f, times.and_then(|t| t.get(f)).copied().unwrap_or(video_ms)))
            }).collect()
        } else {
            Vec::new()
        };

        // Nodes, equally spaced: a frame apart, or every `step`-th frame's spacing when the time constant is long compared
        // to a frame. Over the frames and a few time constants beyond them, not a whole external log around a short clip -
        // or over the motion, when there are no frames to go by
        let step = if fps > 0.0 && fps.is_finite() { (max_tau * fps / MAX_TAU_PER_NODE).ceil().max(1.0) } else { 0.0 };
        let h = if step > 0.0 { step / fps } else { (max_tau / MAX_TAU_PER_NODE).max(0.01) };
        let (mut lo, mut hi) = (first_ms, last_ms);
        if let (Some(a), Some(b)) = (frames.iter().map(|f| f.1).reduce(f64::min), frames.iter().map(|f| f.1).reduce(f64::max)) {
            let margin = 3000.0 * max_tau + 500.0;
            (lo, hi) = ((a - margin).max(first_ms), (b + margin).min(last_ms));
            if !(hi > lo) { (lo, hi) = (first_ms, last_ms); }
        }
        let n = ((hi - lo) / (h * 1000.0)).ceil() as usize + 1;
        if n < 4 { return quats.clone(); }
        let node_times: Vec<f64> = (0..n).map(|k| lo + k as f64 * h * 1000.0).collect();

        // The motion at each node, averaged over the node's interval: point samples a few frames apart would fold
        // vibration near a multiple of the node rate down into the path, and a box average has its nulls right there
        let raw: Vec<Quat64> = node_times.iter().map(|t| {
            let Some(c) = crop::quat_at(quats, (t * 1000.0).round() as i64) else { return Quat64::identity(); };
            let ci = c.inverse();
            let (from, to) = (((t - h * 500.0) * 1000.0).round() as i64, ((t + h * 500.0) * 1000.0).round() as i64);
            let (sum, count) = quats.range(from..to).fold((Vector3::zeros(), 0usize), |(s, n), (_, q)| (s + (ci * q).scaled_axis(), n + 1));
            if count > 0 { c * Quat64::from_scaled_axis(sum / count as f64) } else { c }
        }).collect();
        let tau: Vec<[f64; 3]> = node_times.iter().map(|t| tau_at(*t)).collect();

        // With "No zooming" no crop is applied, so there's no limit to keep the frame within
        let model = if self.crop_constraint && zooms(compute_params) { crop::CropModel::new(compute_params, &frames, quats, original) } else { None };
        let settings = path::Settings { h, t0_ms: node_times[0], ..self.settings.clone() };
        let (smoothed, stats) = path::solve(&raw, &tau, &settings, model.as_ref().map(|m| m as &dyn path::Constraints));
        log::debug!("Optimal path: {n} nodes ({h:.4} s apart), {} frames, {stats:?}, {:.1} ms", frames.len(), started.elapsed().as_secs_f64() * 1000.0);

        let a = path::intervals(&smoothed);
        quats.keys().map(|&ts| (ts, path::sample(&smoothed, &a, (ts as f64 / 1000.0 - node_times[0]) / (h * 1000.0)))).collect::<BTreeMap<i64, Quat64>>()
    }
}

/// Whether the zoom applies a crop at all (not "No zooming")
fn zooms(p: &ComputeParams) -> bool {
    p.adaptive_zoom_window < -0.9 || p.adaptive_zoom_window > 0.0001
}
