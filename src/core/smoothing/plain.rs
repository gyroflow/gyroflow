// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::*;

use crate::gyro_source::TimeQuat;
use crate::keyframes::*;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct Plain {
    pub time_constant: f64,
    pub trim_range_only: bool,
}

impl Default for Plain {
    fn default() -> Self { Self {
        time_constant: 0.25,
        trim_range_only: false,
    } }
}

impl SmoothingAlgorithm for Plain {
    fn get_name(&self) -> String { "Plain 3D".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "time_constant" => self.time_constant = val,
            "trim_range_only" => self.trim_range_only = val > 0.1,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameter(&self, name: &str) -> f64 {
        match name {
            "time_constant" => self.time_constant,
            "trim_range_only" => if self.trim_range_only { 1.0 } else { 0.0 },
            _ => 0.0
        }
    }

    fn get_parameters_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "name": "time_constant",
                "description": "Smoothness",
                "type": "SliderWithField",
                "from": 0.01,
                "to": 10.0,
                "value": self.time_constant,
                "default": 0.25,
                "unit": "s",
                "keyframe": "SmoothingParamTimeConstant"
            },
            {
                "name": "trim_range_only",
                "description": "Only within trim range",
                "advanced": true,
                "type": "CheckBox",
                "default": self.trim_range_only,
                "value": if self.trim_range_only { 1.0 } else { 0.0 },
            },
        ])
    }
    fn get_status_json(&self) -> serde_json::Value {
        serde_json::json!([])
    }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.time_constant.to_bits());
        hasher.finish()
    }

    fn smooth(&self, quats: &TimeQuat, duration: f64, compute_params: &ComputeParams) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        let keyframes = &compute_params.keyframes;

        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let get_alpha = |time_constant: f64| {
            1.0 - (-(1.0 / sample_rate) / time_constant).exp()
        };
        let mut alpha = 1.0;
        if self.time_constant > 0.0 {
            alpha = get_alpha(self.time_constant);
        }

        let quats = Smoothing::get_trimmed_quats(quats, duration, self.trim_range_only, &compute_params.trim_ranges);
        let quats = quats.as_ref();

        let mut alpha_per_timestamp = BTreeMap::<i64, f64>::new();
        if keyframes.is_keyframed(&KeyframeType::SmoothingParamTimeConstant)
          || (compute_params.video_speed_affects_smoothing && (compute_params.video_speed != 1.0 || keyframes.is_keyframed(&KeyframeType::VideoSpeed)))
          || (keyframes.is_keyframed(&KeyframeType::SmoothnessLimiter) || compute_params.smoothness_limiter != 1.0) {
            alpha_per_timestamp = quats.iter().map(|(ts, _)| {
                let timestamp_ms = *ts as f64 / 1000.0;

                let mut val = keyframes.value_at_gyro_timestamp(&KeyframeType::SmoothingParamTimeConstant, timestamp_ms).unwrap_or(self.time_constant);

                val *= keyframes.value_at_gyro_timestamp(&KeyframeType::SmoothnessLimiter, timestamp_ms).unwrap_or(1.0) * compute_params.smoothness_limiter;

                if compute_params.video_speed_affects_smoothing {
                    let vid_speed = keyframes.value_at_gyro_timestamp(&KeyframeType::VideoSpeed, timestamp_ms).unwrap_or(compute_params.video_speed);
                    val *= vid_speed;
                }

                (*ts, get_alpha(val))
            }).collect();
        }

        let mut q = *quats.iter().next().unwrap().1;
        let smoothed1: TimeQuat = quats.iter().map(|x| {
            q = q.slerp(x.1, *alpha_per_timestamp.get(x.0).unwrap_or(&alpha));
            (*x.0, q)
        }).collect();

        // Reverse pass, while leveling horizon
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        smoothed1.iter().rev().map(|x| {
            q = q.slerp(x.1, *alpha_per_timestamp.get(x.0).unwrap_or(&alpha));
            (*x.0, q)
        }).collect()
    }
}
