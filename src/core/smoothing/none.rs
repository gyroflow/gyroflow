// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Elvin Chen

use super::*;
use crate::gyro_source::TimeQuat;

#[derive(Clone)]
pub struct None {
    pub horizonlockpercent: f64,
    pub horizonroll: f64,
}

impl Default for None {
    fn default() -> Self { Self {
        horizonlockpercent: 0.0,
        horizonroll: 0.0,
    } }
}

impl SmoothingAlgorithm for None {
    fn get_name(&self) -> String { "No smoothing".to_owned() }

    fn get_parameters_json(&self) -> serde_json::Value { serde_json::json!([]) }
    fn get_status_json(&self) -> serde_json::Value { serde_json::json!([]) }
    fn set_parameter(&mut self, name: &str, val: f64) {
        print!("{}", val);
        match name {
            "horizonroll" => self.horizonroll = val,
            "horizonlockpercent" => self.horizonlockpercent = val,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }
    
    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.horizonlockpercent.to_bits());
        hasher.write_u64(self.horizonroll.to_bits());
        hasher.finish()
    }
    fn smooth(&mut self, quats: &TimeQuat, duration: f64, _params: &crate::BasicParams) -> TimeQuat {
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        if self.horizonlockpercent == 0.0 {
            quats.clone()
        } else {
            const DEG2RAD: f64 = std::f64::consts::PI / 180.0;
            quats.iter().map(|x| {
                (*x.0,  lock_horizon_angle(*x.1, self.horizonroll * DEG2RAD).slerp(x.1, 1.0-self.horizonlockpercent/100.0))
            }).collect()
        }
    }
}