// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::*;

use crate::gyro_source::TimeQuat;
use crate::keyframes::*;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct Plain {
    pub time_constant: f64,
}

impl Default for Plain {
    fn default() -> Self { Self {
        time_constant: 0.25,
    } }
}

impl SmoothingAlgorithm for Plain {
    fn get_name(&self) -> String { "Plain 3D".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "time_constant" => self.time_constant = val,
            _ => log::error!("Invalid parameter name: {}", name)
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
            }
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

    fn smooth(&self, quats: &TimeQuat, duration: f64, _stabilization_params: &StabilizationParams, keyframes: &KeyframeManager) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let get_alpha = |time_constant: f64| {
            1.0 - (-(1.0 / sample_rate) / time_constant).exp()
        };
        let mut alpha = 1.0;
        if self.time_constant > 0.0 {
            alpha = get_alpha(self.time_constant);
        }

        let mut alpha_per_timestamp = BTreeMap::<i64, f64>::new();
        if keyframes.is_keyframed(&KeyframeType::SmoothingParamTimeConstant) {
            alpha_per_timestamp = quats.iter().map(|(ts, _)| {
                let timestamp_ms = *ts as f64 / 1000.0;
                (*ts, get_alpha(keyframes.value_at_gyro_timestamp(&KeyframeType::SmoothingParamTimeConstant, timestamp_ms).unwrap_or(self.time_constant)))
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
