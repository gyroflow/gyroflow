// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>, Aphobius

// 1. Calculate velocity for each quaternion
// 2. Smooth the velocities
// 3. Get max velocity and convert all velocities to ratio from 0.0 to 1.0, where 1.0 is max velocity
// 4. Perform plain 3D smoothing with varying alpha, where each alpha is between `Smoothness` and `Smoothness at high velocity`, according to velocity ratio
// 5. This way, low velocities are smoothed using `Smoothness`, but high velocities are smoothed using `Smoothness at high velocity`

use std::collections::BTreeMap;

use super::*;
use nalgebra::*;
use crate::gyro_source::TimeQuat;
use crate::Quat64;

#[derive(Clone)]
pub struct VelocityDampenedAxis {
    pub smoothness_pitch: f64,
    pub smoothness_yaw: f64,
    pub smoothness_roll: f64,
    pub horizonlockpercent: f64,
    pub horizonroll: f64
}

impl Default for VelocityDampenedAxis {
    fn default() -> Self { Self {
        smoothness_pitch: 0.2,
        smoothness_yaw: 0.2,
        smoothness_roll: 0.2,
        horizonlockpercent: 0.0,
        horizonroll: 0.0
    } }
}

impl SmoothingAlgorithm for VelocityDampenedAxis {
    fn get_name(&self) -> String { "Velocity dampened per axis".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "smoothness_pitch" => self.smoothness_pitch = val,
            "smoothness_yaw" => self.smoothness_yaw = val,
            "smoothness_roll" => self.smoothness_roll = val,
            "horizonroll" => self.horizonroll = val,
            "horizonlockpercent" => self.horizonlockpercent = val,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameters_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "name": "smoothness_pitch",
                "description": "Pitch smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness_pitch,
                "default": 0.2,
                "unit": "",
                "precision": 3
            },
            {
                "name": "smoothness_yaw",
                "description": "Yaw smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness_yaw,
                "default": 0.2,
                "unit": "",
                "precision": 3
            },
            {
                "name": "smoothness_roll",
                "description": "Roll smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness_roll,
                "default": 0.2,
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
        hasher.write_u64(self.smoothness_pitch.to_bits());
        hasher.write_u64(self.smoothness_yaw.to_bits());
        hasher.write_u64(self.smoothness_roll.to_bits());
        hasher.write_u64(self.horizonroll.to_bits());
        hasher.write_u64(self.horizonlockpercent.to_bits());
        hasher.finish()
    }

    fn smooth(&mut self, quats: &TimeQuat, duration: f64, _params: &crate::BasicParams) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        const MAX_VELOCITY: f64 = 500.0;
        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let alpha = 1.0 - (-(1.0 / sample_rate) / 1.0).exp();
        let high_alpha = 1.0 - (-(1.0 / sample_rate) / 0.1).exp();

        let mut velocity = BTreeMap::<i64, Vector3<f64>>::new();

        let first_quat = quats.iter().next().unwrap(); // First quat
        velocity.insert(*first_quat.0, Vector3::from_element(0.0));

        // Calculate velocity
        let rad_to_deg_per_sec: f64 = sample_rate * 180.0 / std::f64::consts::PI;
        let mut prev_quat = *quats.iter().next().unwrap().1; // First quat
        for (timestamp, quat) in quats.iter().skip(1) {
            let dist = prev_quat.inverse() * quat;
            let euler = dist.euler_angles();
            velocity.insert(*timestamp, Vector3::new(
                euler.0.abs() * rad_to_deg_per_sec,
                euler.1.abs() * rad_to_deg_per_sec,
                euler.2.abs() * rad_to_deg_per_sec
            ));
            prev_quat = *quat;
        }

        // Smooth velocity
        let mut prev_velocity = *velocity.iter().next().unwrap().1; // First velocity
        for (_ts, vel) in velocity.iter_mut().skip(1) {
            *vel = prev_velocity * (1.0 - high_alpha) + *vel * high_alpha;
            prev_velocity = *vel;
        }
        for (_ts, vel) in velocity.iter_mut().rev().skip(1) {
            *vel = prev_velocity * (1.0 - high_alpha) + *vel * high_alpha;
            prev_velocity = *vel;
        }

        // Calculate max velocity
        let mut max_velocity = Vector3::from_element(MAX_VELOCITY);
        max_velocity[0] *= self.smoothness_pitch;
        max_velocity[1] *= self.smoothness_yaw;
        max_velocity[2] *= self.smoothness_roll;

        // Normalize velocity
        for (_ts, vec) in velocity.iter_mut() {
            vec[0] /= max_velocity[0];
            vec[1] /= max_velocity[1];
            vec[2] /= max_velocity[2];
        }

        // Plain 3D smoothing with varying alpha
        let mut q = *quats.iter().next().unwrap().1;
        let smoothed1: TimeQuat = quats.iter().map(|(ts, x)| {
            let ratio = velocity[ts];
            let pitch_factor = alpha * (1.0 - ratio[0]) + high_alpha * ratio[0];
            let yaw_factor = alpha * (1.0 - ratio[1]) + high_alpha * ratio[1];
            let roll_factor = alpha * (1.0 - ratio[2]) + high_alpha * ratio[2];

            let euler_rot = (q.inverse() * x).euler_angles();

            let quat_rot = Quat64::from_euler_angles(
                euler_rot.0 * pitch_factor.min(1.0),
                euler_rot.1 * yaw_factor.min(1.0),
                euler_rot.2 * roll_factor.min(1.0),
            );
            q *= quat_rot;
            (*ts, q)
        }).collect();

        // Reverse pass
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        let smoothed2: TimeQuat = smoothed1.iter().rev().map(|(ts, x)| {
            let ratio = velocity[ts];
            let pitch_factor = alpha * (1.0 - ratio[0]) + high_alpha * ratio[0];
            let yaw_factor = alpha * (1.0 - ratio[1]) + high_alpha * ratio[1];
            let roll_factor = alpha * (1.0 - ratio[2]) + high_alpha * ratio[2];

            let euler_rot = (q.inverse() * x).euler_angles();

            let quat_rot = Quat64::from_euler_angles(
                euler_rot.0 * pitch_factor.min(1.0),
                euler_rot.1 * yaw_factor.min(1.0),
                euler_rot.2 * roll_factor.min(1.0),
            );
            q *= quat_rot;
            (*ts, q)
        }).collect();

        // level horizon
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
