// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Elvin Chen

use super::*;
use nalgebra::*;
use crate::gyro_source::TimeQuat;

#[derive(Default, Clone)]
pub struct Fixed {
    pub roll: f64,
    pub pitch: f64,
    pub yaw: f64,
    pub horizonlock: horizon::HorizonLock
}

impl SmoothingAlgorithm for Fixed {
    fn get_name(&self) -> String { "Fixed camera".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "roll" => self.roll = val,
            "pitch" => self.pitch = val,
            "yaw" => self.yaw = val,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }

    fn set_horizon_lock(&mut self, lock_percent: f64, roll: f64) {
        self.horizonlock.set_horizon(lock_percent, roll);
    }

    fn get_parameters_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "name": "roll",
                "description": "Roll angle",
                "type": "SliderWithField",
                "from": -180,
                "to": 180,
                "value": self.roll,
                "default": 0,
                "unit": "°"
            },
            {
                "name": "pitch",
                "description": "Pitch angle",
                "type": "SliderWithField",
                "from": -90,
                "to": 90,
                "value": self.pitch,
                "default": 0,
                "unit": "°"
            },
            {
                "name": "yaw",
                "description": "Yaw angle",
                "type": "SliderWithField",
                "from": -180,
                "to": 180,
                "value": self.yaw,
                "default": 0,
                "unit": "°"
            }
        ])
    }
    fn get_status_json(&self) -> serde_json::Value { serde_json::json!([]) }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.roll.to_bits());
        hasher.write_u64(self.pitch.to_bits());
        hasher.write_u64(self.yaw.to_bits());
        hasher.write_u64(self.horizonlock.get_checksum());
        hasher.finish()
    }

    fn smooth(&mut self, quats: &TimeQuat, duration: f64, _stabilization_params: &StabilizationParams) -> TimeQuat {
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        const DEG2RAD: f64 = std::f64::consts::PI / 180.0;
        let x_axis = nalgebra::Vector3::<f64>::x_axis();
        let y_axis = nalgebra::Vector3::<f64>::y_axis();
        let z_axis = nalgebra::Vector3::<f64>::z_axis();
        
        let rot_x = Rotation3::from_axis_angle(&x_axis, self.pitch * DEG2RAD);
        let rot_y = Rotation3::from_axis_angle(&y_axis, (self.roll + 90.0) * DEG2RAD);
        let rot_z = Rotation3::from_axis_angle(&z_axis, self.yaw * DEG2RAD);

        let correction = Rotation3::from_axis_angle(&z_axis, 90.0 * DEG2RAD) * Rotation3::from_axis_angle(&y_axis, 90.0 * DEG2RAD);

        // Z rotation corresponds to body-centric roll, so placed last
        // using x as second rotation corresponds gives the usual pan/tilt combination
        let combined_rot = rot_z * rot_x * rot_y * correction;

        // only one computation
        let fixed_quat = self.horizonlock.lockquat(UnitQuaternion::from_rotation_matrix(&combined_rot));

        quats.iter().map(|x| {
            (*x.0, fixed_quat)
        }).collect()
        // No need to reverse the BTreeMap, because it's sorted by definition
    }
}
