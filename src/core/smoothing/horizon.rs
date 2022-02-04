// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Elvin Chen

use super::*;
use nalgebra::*;
use crate::gyro_source::TimeQuat;

#[derive(Clone)]
pub struct HorizonLock {
    pub time_constant: f64,
    pub roll: f64
}

impl Default for HorizonLock {
    fn default() -> Self { Self {
        time_constant: 0.25,
        roll: 0.0
    } }
}

fn from_euler_yxz(x: f64, y: f64, z: f64) -> UnitQuaternion<f64> {

    let x_axis = nalgebra::Vector3::<f64>::x_axis();
    let y_axis = nalgebra::Vector3::<f64>::y_axis();
    let z_axis = nalgebra::Vector3::<f64>::z_axis();
    
    let rot_x = Rotation3::from_axis_angle(&x_axis, x);
    let rot_y = Rotation3::from_axis_angle(&y_axis, y + std::f64::consts::FRAC_PI_2);
    let rot_z = Rotation3::from_axis_angle(&z_axis, z);

    let correction = Rotation3::from_axis_angle(&z_axis, std::f64::consts::FRAC_PI_2) * Rotation3::from_axis_angle(&y_axis, std::f64::consts::FRAC_PI_2);

    let combined_rot = rot_z * rot_x * rot_y * correction;
    UnitQuaternion::from_rotation_matrix(&combined_rot)
}

fn lock_horizon_angle(q: UnitQuaternion<f64>, roll_correction: f64) -> UnitQuaternion<f64> {
    // z axis points in view direction, use as reference
    let axis = nalgebra::Vector3::<f64>::y_axis();

    // let x_axis = nalgebra::Vector3::<f64>::x_axis();
    let y_axis = nalgebra::Vector3::<f64>::y_axis();
    let z_axis = nalgebra::Vector3::<f64>::z_axis();

    let corrected_transform = q.to_rotation_matrix() * Rotation3::from_axis_angle(&y_axis, -std::f64::consts::FRAC_PI_2) * Rotation3::from_axis_angle(&z_axis, -std::f64::consts::FRAC_PI_2);
    // since this coincides with roll axis, the roll is neglected when transformed back
    let axis_transformed = corrected_transform * axis;

    let pitch = (axis_transformed.z).asin();
    let yaw = axis_transformed.y.simd_atan2(axis_transformed.x) - std::f64::consts::FRAC_PI_2;
    
    from_euler_yxz(pitch, roll_correction, yaw)
}


impl SmoothingAlgorithm for HorizonLock {
    fn get_name(&self) -> String { "Lock horizon".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "time_constant" => self.time_constant = val,
            "roll" => self.roll = val,
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
            },
            {
                "name": "roll",
                "description": "Roll angle correction",
                "type": "SliderWithField",
                "from": -180,
                "to": 180,
                "value": self.roll,
                "default": 0,
                "unit": "°"
            }
        ])
    }
    fn get_status_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "name": "label",
                "text": "Requires accurate orientation determination. Try with Complementary, Mahony, or Madgwick integration method.",
                "text_args": [],
                "type": "Label"
            }
        ])
    }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.time_constant.to_bits());
        hasher.write_u64(self.roll.to_bits());
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
        smoothed2.iter().map(|x| {
            (*x.0, lock_horizon_angle(*x.1, self.roll * DEG2RAD))
        }).collect()

        // No need to reverse the BTreeMap, because it's sorted by definition
    }
}
