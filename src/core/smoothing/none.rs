// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Elvin Chen

use super::*;
use crate::gyro_source::TimeQuat;


#[derive(Default, Clone)]
pub struct None {
    pub horizonlock: horizon::HorizonLock
}

impl SmoothingAlgorithm for None {
    fn get_name(&self) -> String { "No smoothing".to_owned() }

    fn get_parameters_json(&self) -> serde_json::Value { serde_json::json!([]) }
    fn get_status_json(&self) -> serde_json::Value { serde_json::json!([]) }
    fn set_parameter(&mut self, _name: &str, _val: f64) { }
    
    fn set_horizon_lock(&mut self, lock_percent: f64, roll: f64) {
        self.horizonlock.set_horizon(lock_percent, roll);
    }
    
    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.horizonlock.get_checksum());
        hasher.finish()
    }
    fn smooth(&mut self, quats: &TimeQuat, duration: f64, _stabilization_params: &StabilizationParams) -> TimeQuat {
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        self.horizonlock.lock(&quats)
    }
}