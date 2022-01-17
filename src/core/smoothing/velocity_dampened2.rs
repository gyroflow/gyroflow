// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

// 1. Calculate velocity for each quaternion
// 2. Smooth the velocities
// 3. Get max velocity and convert all velocities to ratio from 0.0 to 1.0, where 1.0 is max velocity
// 4. Perform plain 3D smoothing with varying alpha, where each alpha is between `Smoothness` and `Smoothness at high velocity`, according to velocity ratio
// 5. This way, low velocities are smoothed using `Smoothness`, but high velocities are smoothed using `Smoothness at high velocity`

use std::collections::BTreeMap;

use super::*;
use nalgebra::*;
use crate::gyro_source::TimeQuat;

#[derive(Clone)]
pub struct VelocityDampened2 {
    pub time_constant: f64,
    pub time_constant2: f64,
    pub velocity_factor: f64,
    pub label_arguments: [String; 3],
}

impl Default for VelocityDampened2 {
    fn default() -> Self { Self {
        time_constant: 0.6,
        time_constant2: 0.1,
        velocity_factor: 0.9,
        label_arguments: ["<b>0.0°</b>".into(), "<b>0.0°</b>".into(), "<b>0.0°</b>".into()]
    } }
}

impl SmoothingAlgorithm for VelocityDampened2 {
    fn get_name(&self) -> String { "Velocity dampened smoothing 2".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "time_constant"   => self.time_constant   = val,
            "time_constant2"  => self.time_constant2  = val,
            "velocity_factor" => self.velocity_factor = val,
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
                "to": 1.0,
                "value": self.time_constant,
                "unit": "s"
            },
            {
                "name": "time_constant2",
                "description": "Smoothness at high velocity",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 0.1,
                "value": self.time_constant2,
                "unit": "s"
            },
            {
                "name": "velocity_factor",
                "description": "Velocity factor",
                "type": "SliderWithField",
                "from": 0.01,
                "to": 10.0,
                "value": self.velocity_factor,
                "unit": ""
            }
        ])
    }
    fn get_status_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "name": "label",
                "text": "Max rotation:\nPitch: %1, Yaw: %2, Roll: %3.\nModify velocity factor until you get the desired values (recommended less than 20).",
                "text_args": self.label_arguments,
                "type": "Label"
            }
        ])
    }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.time_constant.to_bits());
        hasher.write_u64(self.velocity_factor.to_bits());
        hasher.finish()
    }

    fn smooth(&mut self, quats: &TimeQuat, duration: f64, params: &crate::BasicParams) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        let start_ts = (params.trim_start * params.get_scaled_duration_ms() * 1000.0) as i64;
        let end_ts = (params.trim_end * params.get_scaled_duration_ms() * 1000.0) as i64;

        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let mut alpha = 1.0;
        let mut high_alpha = 1.0;
        if self.time_constant > 0.0 {
            alpha = 1.0 - (-(1.0 / sample_rate) / self.time_constant).exp();
            high_alpha = 1.0 - (-(1.0 / sample_rate) / self.time_constant2).exp();
        }

        let mut velocity = BTreeMap::<i64, Vector3<f64>>::new();

        let first_quat = quats.iter().next().unwrap(); // First quat
        velocity.insert(*first_quat.0, Vector3::from_element(0.0));

        // Calculate velocity
        let mut prev_quat = *quats.iter().next().unwrap().1; // First quat
        for (timestamp, quat) in quats.iter().skip(1) {
            let dist = prev_quat.inverse() * quat;
            let euler = dist.scaled_axis();

            let v = Vector3::new(
                euler[0].abs() * sample_rate, // Roll
                euler[1].abs() * sample_rate, // Pitch
                euler[2].abs() * sample_rate  // Yaw
            );

            velocity.insert(*timestamp, v);
            prev_quat = *quat;
        }

        // Smooth velocity
        let mut max_velocity = 0.0001;
        let mut prev_velocity = *velocity.iter().next().unwrap().1; // First velocity
        for (_timestamp, vec) in velocity.iter_mut().skip(1) {
            *vec = prev_velocity * (1.0 - high_alpha) + *vec * high_alpha;
            prev_velocity = *vec;
        }
        for (timestamp, vec) in velocity.iter_mut().rev().skip(1) {
            *vec = prev_velocity * (1.0 - high_alpha) + *vec * high_alpha;
            prev_velocity = *vec;

            if timestamp >= &start_ts && timestamp <= &end_ts {
                let max = vec[0].max(vec[1]).max(vec[2]);
                if max > max_velocity { max_velocity = max; }
            }
        }

        if self.velocity_factor > 0.0 {
            max_velocity *= self.velocity_factor;
        }

        let ratios: BTreeMap<i64, f64> = velocity.iter().map(|(k, v)| {
            let max = v[0].max(v[1]).max(v[2]);
            (*k, max / max_velocity)
        }).collect();

        // Plain 3D smoothing with varying alpha
        let mut q = *quats.iter().next().unwrap().1;
        let smoothed1: TimeQuat = quats.iter().map(|(ts, x)| {
            let ratio = ratios[ts];
            let val = alpha * (1.0 - ratio) + high_alpha * ratio;
            q = q.slerp(x, val);
            (*ts, q)
        }).collect();

        // Reverse pass
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        let vel_corr_smooth: TimeQuat = smoothed1.iter().rev().map(|(ts, x)| {
            let ratio = ratios[ts];
            let val = alpha * (1.0 - ratio) + high_alpha * ratio;
            q = q.slerp(x, val);
            (*ts, q)
        }).collect();

        // Calculate max distance
        let mut max_pitch = 0.0;
        let mut max_yaw = 0.0;
        let mut max_roll = 0.0;

        for (timestamp, quat) in vel_corr_smooth.iter() {
            if timestamp >= &start_ts && timestamp <= &end_ts {
                let dist = quat.inverse() * quats[timestamp];
                let euler_dist = dist.euler_angles();
                if euler_dist.2.abs() > max_roll  { max_roll  = euler_dist.2.abs(); }
                if euler_dist.0.abs() > max_pitch { max_pitch = euler_dist.0.abs(); }
                if euler_dist.1.abs() > max_yaw   { max_yaw   = euler_dist.1.abs(); }
            }
        }
        
        const RAD2DEG: f64 = 180.0 / std::f64::consts::PI;
        self.label_arguments[0] = format!("<b>{:.2}°</b>", max_pitch * RAD2DEG);
        self.label_arguments[1] = format!("<b>{:.2}°</b>", max_yaw * RAD2DEG);
        self.label_arguments[2] = format!("<b>{:.2}°</b>", max_roll * RAD2DEG);

        vel_corr_smooth
    }
}
