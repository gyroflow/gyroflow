// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>, Aphobius

// 1. Calculate velocity for each quaternion
// 2. Smooth the velocities
// 3. Get max velocity and convert all velocities to ratio from 0.0 to 1.0, where 1.0 is max velocity
// 4. Perform plain 3D smoothing with varying alpha, where each alpha is between `Smoothness` and `Smoothness at high velocity`, according to velocity ratio
// 5. This way, low velocities are smoothed using `Smoothness`, but high velocities are smoothed using `Smoothness at high velocity`

use std::collections::BTreeMap;

use super::*;
use crate::gyro_source::TimeQuat;

#[derive(Clone)]
pub struct VelocityDampened {
    pub smoothness: f64
}

impl Default for VelocityDampened {
    fn default() -> Self { Self {
        smoothness: 0.2
    } }
}

impl SmoothingAlgorithm for VelocityDampened {
    fn get_name(&self) -> String { "Velocity dampened".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "smoothness" => self.smoothness = val,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameters_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "name": "smoothness",
                "description": "Smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness,
                "unit": "s",
                "precision": 3
            }
        ])
    }
    fn get_status_json(&self) -> serde_json::Value {
        serde_json::json!([])
    }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.smoothness.to_bits());
        hasher.finish()
    }

    fn smooth(&mut self, quats: &TimeQuat, duration: f64, params: &crate::BasicParams) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        let start_ts = (params.trim_start * params.get_scaled_duration_ms() * 1000.0) as i64;
        let end_ts = (params.trim_end * params.get_scaled_duration_ms() * 1000.0) as i64;

        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let alpha = 1.0 - (-(1.0 / sample_rate) / 1.0).exp();
        let high_alpha = 1.0 - (-(1.0 / sample_rate) / 0.1).exp();

        let mut velocity = BTreeMap::<i64, f64>::new();

        let first_quat = quats.iter().next().unwrap(); // First quat
        velocity.insert(*first_quat.0, 0.0);

        // Calculate velocity
        let mut prev_quat = *quats.iter().next().unwrap().1; // First quat
        for (timestamp, quat) in quats.iter().skip(1) {
            let dist = (prev_quat.inverse() * quat).angle();
            velocity.insert(*timestamp, dist);
            prev_quat = *quat;
        }

        // Smooth velocity
        let mut max_velocity = 0.0001;
        let mut prev_velocity = *velocity.iter().next().unwrap().1; // First velocity
        for (_timestamp, vel) in velocity.iter_mut().skip(1) {
            *vel = prev_velocity * (1.0 - high_alpha) + *vel * high_alpha;
            prev_velocity = *vel;
        }
        for (timestamp, vel) in velocity.iter_mut().rev().skip(1) {
            *vel = prev_velocity * (1.0 - high_alpha) + *vel * high_alpha;
            prev_velocity = *vel;

            if timestamp >= &start_ts && timestamp <= &end_ts {
                if *vel > max_velocity { max_velocity = *vel; }
            }
        }

        if self.smoothness > 0.0 {
            max_velocity *= self.smoothness;
        }

        let ratios: BTreeMap<i64, f64> = velocity.iter().map(|(k, vel)| {
            (*k, vel / max_velocity)
        }).collect();

        // Plain 3D smoothing with varying alpha
        let mut q = *quats.iter().next().unwrap().1;
        let smoothed1: TimeQuat = quats.iter().map(|(ts, x)| {
            let ratio = ratios[ts];
            let val = alpha * (1.0 - ratio) + high_alpha * ratio;
            q = q.slerp(x, val.min(1.0));
            (*ts, q)
        }).collect();

        // Reverse pass
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        smoothed1.iter().rev().map(|(ts, x)| {
            let ratio = ratios[ts];
            let val = alpha * (1.0 - ratio) + high_alpha * ratio;
            q = q.slerp(x, val.min(1.0));
            (*ts, q)
        }).collect()
    }
}
