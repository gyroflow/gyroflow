// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Aphobius

// 1. Calculate velocity for each quaternion
// 2. Smooth the velocities
// 3. Multiply max velocity (500 deg/s) with slider value
// 4. Perform plain 3D smoothing with varying alpha, where each alpha is interpolated between 1s smoothness at 0 velocity, 0.1s smoothness at max velocity and extrapolated above that
// 5. This way, low velocities are smoothed using 1s smoothness, but high velocities are smoothed using 0.1s smoothness at max velocity (500 deg/s multiplied by slider) and gradually lower smoothness above that

use std::collections::BTreeMap;

use super::*;
use crate::gyro_source::TimeQuat;
use nalgebra::*;
use crate::Quat64;

#[derive(Clone)]
pub struct DefaultAlgo {
    pub smoothness: f64,
    pub smoothness_pitch: f64,
    pub smoothness_yaw: f64,
    pub smoothness_roll: f64,
    pub per_axis: bool,
    pub max_smoothness: f64,
    pub horizonlock: horizon::HorizonLock
}

impl Default for DefaultAlgo {
    fn default() -> Self { Self {
        smoothness: 0.5,
        smoothness_pitch: 0.5,
        smoothness_yaw: 0.5,
        smoothness_roll: 0.5,
        per_axis: false,
        max_smoothness: 1.0,
        horizonlock: Default::default()
    } }
}

impl SmoothingAlgorithm for DefaultAlgo {
    fn get_name(&self) -> String { "Default".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "smoothness" => self.smoothness = val,
            "smoothness_pitch" => self.smoothness_pitch = val,
            "smoothness_yaw" => self.smoothness_yaw = val,
            "smoothness_roll" => self.smoothness_roll = val,
            "per_axis" => self.per_axis = val > 0.1,
            "max_smoothness" => self.max_smoothness = val,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }

    fn set_horizon_lock(&mut self, lock_percent: f64, roll: f64) {
        self.horizonlock.set_horizon(lock_percent, roll);
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
            },
            {
                "name": "smoothness_pitch",
                "description": "Pitch smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness_pitch,
                "default": 0.5,
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
                "default": 0.5,
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
                "default": 0.5,
                "unit": "",
                "precision": 3
            },
            {
                "name": "per_axis",
                "description": "Per axis",
                "advanced": true,
                "type": "CheckBox",
                "default": self.per_axis,
                "custom_qml": "Connections { function onCheckedChanged() {
                    const checked = root.getParamElement('per_axis').checked;
                    root.getParamElement('smoothness-label').visible = !checked;
                    root.getParamElement('smoothness_pitch-label').visible = checked;
                    root.getParamElement('smoothness_yaw-label').visible = checked;
                    root.getParamElement('smoothness_roll-label').visible = checked;
                }}"
            },
            {
                "name": "max_smoothness",
                "description": "Max smoothness",
                "advanced": true,
                "type": "SliderWithField",
                "from": 0.1,
                "to": 5.0,
                "value": self.max_smoothness,
                "default": 1.0,
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
        hasher.write_u8(if self.per_axis { 1 } else { 0 });
        hasher.write_u64(self.horizonlock.get_checksum());
        hasher.finish()
    }

    fn smooth(&mut self, quats: &TimeQuat, duration: f64, _stabilization_params: &StabilizationParams) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        const MAX_VELOCITY: f64 = 500.0;
        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let alpha = 1.0 - (-(1.0 / sample_rate) / self.max_smoothness).exp();
        let high_alpha = 1.0 - (-(1.0 / sample_rate) / 0.1).exp();

        let mut velocity = BTreeMap::<i64, Vector3<f64>>::new();

        let first_quat = quats.iter().next().unwrap(); // First quat
        velocity.insert(*first_quat.0, Vector3::from_element(0.0));

        // Calculate velocity
        let rad_to_deg_per_sec: f64 = sample_rate * 180.0 / std::f64::consts::PI;
        let mut prev_quat = *quats.iter().next().unwrap().1; // First quat
        for (timestamp, quat) in quats.iter().skip(1) {
            let dist = prev_quat.inverse() * quat;
            if self.per_axis {
                let euler = dist.euler_angles();
                velocity.insert(*timestamp, Vector3::new(
                    euler.0.abs() * rad_to_deg_per_sec,
                    euler.1.abs() * rad_to_deg_per_sec,
                    euler.2.abs() * rad_to_deg_per_sec
                ));
            } else {
                velocity.insert(*timestamp, Vector3::from_element(dist.angle().abs() * rad_to_deg_per_sec));
            }
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
        let mut max_velocity = Vector3::from_element(MAX_VELOCITY);
        if self.per_axis {
            max_velocity[0] *= self.smoothness_pitch;
            max_velocity[1] *= self.smoothness_yaw;
            max_velocity[2] *= self.smoothness_roll;
        } else {
            max_velocity[0] *= self.smoothness;
        }

        // Normalize velocity
        for (_ts, vel) in velocity.iter_mut() {
            vel[0] /= max_velocity[0];
            if self.per_axis {
                vel[1] /= max_velocity[1];
                vel[2] /= max_velocity[2];
            }
        }

        // Plain 3D smoothing with varying alpha
        let mut q = *quats.iter().next().unwrap().1;
        let smoothed1: TimeQuat = quats.iter().map(|(ts, x)| {
            let ratio = velocity[ts];
            if self.per_axis {
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
            } else {
                let val = alpha * (1.0 - ratio[0]) + high_alpha * ratio[0];
                q = q.slerp(x, val.min(1.0));
            }
            (*ts, q)
        }).collect();

        // Reverse pass
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        let smoothed2: TimeQuat = smoothed1.iter().rev().map(|(ts, x)| {
            let ratio = velocity[ts];
            if self.per_axis {
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
            } else {
                let val = alpha * (1.0 - ratio[0]) + high_alpha * ratio[0];
                q = q.slerp(x, val.min(1.0));
            }
            (*ts, q)
        }).collect();

        self.horizonlock.lock(&smoothed2)
    }
}
