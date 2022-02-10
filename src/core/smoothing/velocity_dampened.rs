// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>, Aphobius

// 1. Calculate velocity for each quaternion
// 2. Smooth the velocities
// 3. Multiply max velocity (500 deg/s) with slider value
// 4. Perform plain 3D smoothing with varying alpha, where each alpha is interpolated between 1s smoothness at 0 velocity, 0.1s smoothness at max velocity and extrapolated above that
// 5. This way, low velocities are smoothed using 1s smoothness, but high velocities are smoothed using 0.1s smoothness at max velocity (500 deg/s multiplied by slider) and gradually lower smoothness above that

use std::collections::BTreeMap;

use super::*;
use crate::gyro_source::TimeQuat;

#[derive(Clone)]
pub struct VelocityDampened {
    pub smoothness: f64,
    pub horizonlockpercent: f64,
    pub horizonroll: f64
}

impl Default for VelocityDampened {
    fn default() -> Self { Self {
        smoothness: 0.3,
        horizonlockpercent: 0.0,
        horizonroll: 0.0
    } }
}

impl SmoothingAlgorithm for VelocityDampened {
    fn get_name(&self) -> String { "Velocity dampened".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "smoothness" => self.smoothness = val,
            "horizonroll" => self.horizonroll = val,
            "horizonlockpercent" => self.horizonlockpercent = val,
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
                "default": 0.5,
                "unit": "",
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
        hasher.write_u64(self.horizonroll.to_bits());
        hasher.write_u64(self.horizonlockpercent.to_bits());
        hasher.finish()
    }

    fn smooth(&mut self, quats: &TimeQuat, duration: f64, _params: &ProcessingParams) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        const MAX_VELOCITY: f64 = 500.0;
        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let alpha = 1.0 - (-(1.0 / sample_rate) / 1.0).exp();
        let high_alpha = 1.0 - (-(1.0 / sample_rate) / 0.1).exp();

        let mut velocity = BTreeMap::<i64, f64>::new();

        let first_quat = quats.iter().next().unwrap(); // First quat
        velocity.insert(*first_quat.0, 0.0);

        // Calculate velocity
        let rad_to_deg_per_sec: f64 = sample_rate * 180.0 / std::f64::consts::PI;
        let mut prev_quat = *quats.iter().next().unwrap().1; // First quat
        for (timestamp, quat) in quats.iter().skip(1) {
            let dist = (prev_quat.inverse() * quat).angle();
            velocity.insert(*timestamp, dist.abs() * rad_to_deg_per_sec);
            prev_quat = *quat;
        }

        // Smooth velocity
        let mut prev_velocity = *velocity.iter().next().unwrap().1; // First velocity
        for (_timestamp, vel) in velocity.iter_mut().skip(1) {
            *vel = prev_velocity * (1.0 - high_alpha) + *vel * high_alpha;
            prev_velocity = *vel;
        }
        for (_timestamp, vel) in velocity.iter_mut().rev().skip(1) {
            *vel = prev_velocity * (1.0 - high_alpha) + *vel * high_alpha;
            prev_velocity = *vel;
        }

        // Calculate max velocity
        let max_velocity = MAX_VELOCITY * self.smoothness;

        // Calculate ratios
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
        let smoothed2: TimeQuat = smoothed1.iter().rev().map(|(ts, x)| {
            let ratio = ratios[ts];
            let val = alpha * (1.0 - ratio) + high_alpha * ratio;
            q = q.slerp(x, val.min(1.0));
            (*ts, q)
        }).collect();

        // Level horizon
        const DEG2RAD: f64 = std::f64::consts::PI / 180.0;

        if self.horizonlockpercent == 0.0 {
            smoothed2
        } else {
            smoothed2.iter().map(|x| {
                (*x.0,  lock_horizon_angle(*x.1, self.horizonroll * DEG2RAD).slerp(x.1, 1.0-self.horizonlockpercent/100.0))
            }).collect()
        }
    }
}
