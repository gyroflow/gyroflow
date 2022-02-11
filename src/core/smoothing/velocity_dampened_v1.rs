// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Aphobius

use std::collections::BTreeMap;

use super::*;
use nalgebra::*;
use crate::gyro_source::TimeQuat;
use crate::Quat64;

#[derive(Clone)]
pub struct VelocityDampened {
    pub time_constant: f64,
    pub pitch_vel_damp: f64,
    pub yaw_vel_damp: f64,
    pub roll_vel_damp: f64,
    pub horizonlock: horizon::HorizonLock
}

impl Default for VelocityDampened {
    fn default() -> Self { Self {
        time_constant: 0.4,
        pitch_vel_damp: 2.0,
        yaw_vel_damp: 2.0,
        roll_vel_damp: 2.0,
        horizonlock: Default::default()
    } }
}

impl SmoothingAlgorithm for VelocityDampened {
    fn get_name(&self) -> String { "Velocity dampened smoothing".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "time_constant"  => self.time_constant  = val,
            "pitch_vel_damp" => self.pitch_vel_damp = val,
            "yaw_vel_damp"   => self.yaw_vel_damp   = val,
            "roll_vel_damp"  => self.roll_vel_damp  = val,
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
                "to": 1.0,
                "value": 0.4,
                "default": 0.4,
                "unit": "s"
            },
            {
                "name": "pitch_vel_damp",
                "description": "Pitch velocity dampening",
                "type": "SliderWithField",
                "from": 0.0,
                "to": 100.0,
                "value": self.pitch_vel_damp,
                "default": 2.0,
                "unit": "",
                "precision": 1
            },
            {
                "name": "yaw_vel_damp",
                "description": "Yaw velocity dampening",
                "type": "SliderWithField",
                "from": 0.0,
                "to": 100.0,
                "value": self.yaw_vel_damp,
                "default": 2.0,
                "unit": "",
                "precision": 1
            },
            {
                "name": "roll_vel_damp",
                "description": "Roll velocity dampening",
                "type": "SliderWithField",
                "from": 0.0,
                "to": 100.0,
                "value": self.roll_vel_damp,
                "default": 2.0,
                "unit": "",
                "precision": 1
            }
        ])
    }
    fn get_status_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "name": "label",
                "text": "Modify dampening settings until you get the desired values (recommended around 6 on all axes).",
                "text_args": [],
                "type": "Label"
            }
        ])
    }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.time_constant.to_bits());
        hasher.write_u64(self.pitch_vel_damp.to_bits());
        hasher.write_u64(self.yaw_vel_damp.to_bits());
        hasher.write_u64(self.roll_vel_damp.to_bits());
        hasher.write_u64(self.horizonlock.get_checksum());
        hasher.finish()
    }

    fn smooth(&mut self, quats: &TimeQuat, duration: f64, _stabilization_params: &crate::StabilizationParams) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let mut alpha = 1.0;
        let mut high_alpha = 1.0;
        if self.time_constant > 0.0 {
            alpha = 1.0 - (-(1.0 / sample_rate) / self.time_constant).exp();
            high_alpha = 1.0 - (-(1.0 / sample_rate * 10.0) / self.time_constant).exp();
        }

        let mut velocity = BTreeMap::<i64, Vector3<f64>>::new();

        let first_quat = quats.iter().next().unwrap(); // First quat
        velocity.insert(*first_quat.0, Vector3::from_element(0.0));

        // Calculate low smooth
        // Forward pass

        let mut prev_quat = *first_quat.1;
        let mut low_smooth: TimeQuat = quats.iter().skip(1).map(|(timestamp, quat)| {
            prev_quat = prev_quat.slerp(quat, high_alpha);
            (*timestamp, prev_quat)
        }).collect();
        low_smooth.insert(*first_quat.0, *first_quat.1);

        // Reverse pass
        for (_timestamp, quat) in low_smooth.iter_mut().rev().skip(1) {
            *quat = prev_quat.slerp(quat, high_alpha);
            prev_quat = *quat;
        }

        // Calculate velocity
        let mut prev_quat = *low_smooth.iter().next().unwrap().1; // First quat
        for (timestamp, quat) in low_smooth.iter().skip(1) {
            let dist = prev_quat.inverse() * quat;
            let euler = dist.euler_angles();

            velocity.insert(*timestamp, Vector3::new(
                euler.0.abs() * sample_rate, // Roll
                euler.1.abs() * sample_rate, // Pitch
                euler.2.abs() * sample_rate  // Yaw
            ));
            prev_quat = *quat;
        }

        // Smooth velocity
        let mut prev_velocity = *velocity.iter().next().unwrap().1; // First velocity
        for (_timestamp, vec) in velocity.iter_mut().skip(1) {
            *vec = prev_velocity * (1.0 - high_alpha) + *vec * high_alpha;
            prev_velocity = *vec;
        }
        for (_timestamp, vec) in velocity.iter_mut().rev().skip(1) {
            *vec = prev_velocity * (1.0 - high_alpha) + *vec * high_alpha;
            prev_velocity = *vec;
        }
        
        // Calculate velocity corrected smooth
        for (_timestamp, vec) in velocity.iter_mut() {
            vec[0] = (vec[0] * self.pitch_vel_damp) + 1.0;
            vec[1] = (vec[1] * self.yaw_vel_damp) + 1.0;
            vec[2] = (vec[2] * self.roll_vel_damp) + 1.0;
        }

        let mut prev_quat = *quats.iter().next().unwrap().1; // Get first quaternion
        let mut vel_corr_smooth: TimeQuat = quats.iter().skip(1).map(|(timestamp, quat)| {
            let rot = (prev_quat.inverse() * quat).euler_angles();
            let vel_vector = velocity[timestamp];

            let vel_quat = Quat64::from_euler_angles(
                rot.0 * (alpha * vel_vector[0]).min(1.0),
                rot.1 * (alpha * vel_vector[1]).min(1.0),
                rot.2 * (alpha * vel_vector[2]).min(1.0),
            );
            prev_quat *= vel_quat;
            (*timestamp, prev_quat)
        }).collect();

        for (timestamp, quat) in vel_corr_smooth.iter_mut().rev().skip(1) {
            let rot = (prev_quat.inverse() * *quat).euler_angles();
            let vel_vector = velocity[timestamp];

            let vel_quat = Quat64::from_euler_angles(
                rot.0 * (alpha * vel_vector[0]).min(1.0),
                rot.1 * (alpha * vel_vector[1]).min(1.0),
                rot.2 * (alpha * vel_vector[2]).min(1.0),
            );
            *quat = prev_quat * vel_quat;
            prev_quat = *quat;
        }
        
        self.horizonlock.lock(&vel_corr_smooth)
    }
}
