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
pub struct VelocityDampenedAdvanced {
    pub time_constant: f64,
    pub time_constant2: f64,
    pub velocity_factor: f64,
    pub horizonlock: horizon::HorizonLock
}

impl Default for VelocityDampenedAdvanced {
    fn default() -> Self { Self {
        time_constant: 0.6,
        time_constant2: 0.1,
        velocity_factor: 0.9,
        horizonlock: Default::default()
    } }
}

impl SmoothingAlgorithm for VelocityDampenedAdvanced {
    fn get_name(&self) -> String { "Velocity dampened (advanced)".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "time_constant"   => self.time_constant   = val,
            "time_constant2"  => self.time_constant2  = val,
            "velocity_factor" => self.velocity_factor = val,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }

    fn set_horizon_lock(&mut self, lock_percent: f64, roll: f64) {
        self.horizonlock.set_horizon(lock_percent, roll);
    }

    fn get_parameters_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "name": "time_constant",
                "description": "Smoothness",
                "type": "SliderWithField",
                "from": 0.01,
                "to": 3.0,
                "value": self.time_constant,
                "default": 1.0,
                "unit": "s"
            },
            {
                "name": "time_constant2",
                "description": "Smoothness at high velocity",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 0.3,
                "value": self.time_constant2,
                "default": 0.1,
                "unit": "s",
                "precision": 3
            },
            {
                "name": "velocity_factor",
                "description": "Velocity factor",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 5.0,
                "value": self.velocity_factor,
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
        hasher.write_u64(self.time_constant.to_bits());
        hasher.write_u64(self.time_constant2.to_bits());
        hasher.write_u64(self.velocity_factor.to_bits());
        hasher.write_u64(self.horizonlock.get_checksum());
        hasher.finish()
    }

    fn smooth(&mut self, quats: &TimeQuat, duration: f64, _stabilization_params: &StabilizationParams) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        const MAX_VELOCITY: f64 = 500.0;
        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let mut alpha = 1.0;
        let mut high_alpha = 1.0;
        if self.time_constant > 0.0 {
            alpha = 1.0 - (-(1.0 / sample_rate) / self.time_constant).exp();
            high_alpha = 1.0 - (-(1.0 / sample_rate) / self.time_constant2).exp();
        }

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
        let max_velocity = MAX_VELOCITY * self.velocity_factor;

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

        self.horizonlock.lock(&smoothed2)
    }
}