// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::*;

use crate::gyro_source::TimeQuat;

#[derive(Clone)]
pub struct Plain {
    pub time_constant: f64,
    pub horizonlockpercent: f64,
    pub horizonroll: f64
}

impl Default for Plain {
    fn default() -> Self { Self {
        time_constant: 0.25,
        horizonlockpercent: 0.0,
        horizonroll: 0.0
    } }
}

impl SmoothingAlgorithm for Plain {
    fn get_name(&self) -> String { "Plain 3D".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "time_constant" => self.time_constant = val,
            "horizonroll" => self.horizonroll = val,
            "horizonlockpercent" => self.horizonlockpercent = val,
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
                "unit": "s"
            }
        ])
    }
    fn get_status_json(&self) -> serde_json::Value {
        serde_json::json!([])
    }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.time_constant.to_bits());
        hasher.write_u64(self.horizonroll.to_bits());
        hasher.write_u64(self.horizonlockpercent.to_bits());
        hasher.finish()
    }

    fn smooth(&mut self, quats: &TimeQuat, duration: f64, _params: &crate::BasicParams) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let mut alpha = 1.0;
        if self.time_constant > 0.0 {
            alpha = 1.0 - (-(1.0 / sample_rate) / self.time_constant).exp();
        }
        const DEG2RAD: f64 = std::f64::consts::PI / 180.0;

        let mut q = *quats.iter().next().unwrap().1;
        let smoothed1: TimeQuat = quats.iter().map(|x| {
            q = q.slerp(x.1, alpha);
            (*x.0, q)
        }).collect();

        // Reverse pass, while leveling horizon
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        let smoothed2: TimeQuat = smoothed1.iter().rev().map(|x| {
            q = q.slerp(x.1, alpha);
            (*x.0, q)
        }).collect();

        // level horizon
        if self.horizonlockpercent == 0.0 {
            smoothed2
        } else {
            smoothed2.iter().map(|x| {
                (*x.0,  lock_horizon_angle(*x.1, self.horizonroll * DEG2RAD).slerp(x.1, 1.0-self.horizonlockpercent/100.0))
            }).collect()
        }
    }
}
