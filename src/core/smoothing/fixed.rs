// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Elvin Chen

use super::*;
use nalgebra::*;
use crate::gyro_source::{TimeQuat, Quat64};
use crate::keyframes::*;

#[derive(Default, Clone)]
pub struct Fixed {
    pub roll: f64,
    pub pitch: f64,
    pub yaw: f64,
}

impl SmoothingAlgorithm for Fixed {
    fn get_name(&self) -> String { "Fixed camera".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "roll"  => self.roll  = val,
            "pitch" => self.pitch = val,
            "yaw"   => self.yaw   = val,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameter(&self, name: &str) -> f64 {
        match name {
            "roll"  => self.roll,
            "pitch" => self.pitch,
            "yaw"   => self.yaw,
            _ => 0.0
        }
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
                "unit": "°",
                "keyframe": "SmoothingParamRoll"
            },
            {
                "name": "pitch",
                "description": "Pitch angle",
                "type": "SliderWithField",
                "from": -90,
                "to": 90,
                "value": self.pitch,
                "default": 0,
                "unit": "°",
                "keyframe": "SmoothingParamPitch"
            },
            {
                "name": "yaw",
                "description": "Yaw angle",
                "type": "SliderWithField",
                "from": -180,
                "to": 180,
                "value": self.yaw,
                "default": 0,
                "unit": "°",
                "keyframe": "SmoothingParamYaw"
            }
        ])
    }
    fn get_status_json(&self) -> serde_json::Value { serde_json::json!([]) }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.roll.to_bits());
        hasher.write_u64(self.pitch.to_bits());
        hasher.write_u64(self.yaw.to_bits());
        hasher.finish()
    }

    fn smooth(&self, quats: &TimeQuat, duration: f64, _stabilization_params: &StabilizationParams, keyframes: &KeyframeManager) -> TimeQuat {
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        fn quat_for_rpy(roll: f64, pitch: f64, yaw: f64) -> Quat64 {
            const DEG2RAD: f64 = std::f64::consts::PI / 180.0;
            let x_axis = nalgebra::Vector3::<f64>::x_axis();
            let y_axis = nalgebra::Vector3::<f64>::y_axis();
            let z_axis = nalgebra::Vector3::<f64>::z_axis();

            let rot_x = Rotation3::from_axis_angle(&x_axis, pitch * DEG2RAD);
            let rot_y = Rotation3::from_axis_angle(&y_axis, (roll + 90.0) * DEG2RAD);
            let rot_z = Rotation3::from_axis_angle(&z_axis, yaw * DEG2RAD);

            let correction = Rotation3::from_axis_angle(&z_axis, 90.0 * DEG2RAD) * Rotation3::from_axis_angle(&y_axis, 90.0 * DEG2RAD);

            // Z rotation corresponds to body-centric roll, so placed last
            // using x as second rotation corresponds gives the usual pan/tilt combination
            let combined_rot = rot_z * rot_x * rot_y * correction;

            UnitQuaternion::from_rotation_matrix(&combined_rot)
        }

        let fixed_quat = quat_for_rpy(self.roll, self.pitch, self.yaw);

        let is_keyframed = keyframes.is_keyframed(&KeyframeType::SmoothingParamRoll)
                             || keyframes.is_keyframed(&KeyframeType::SmoothingParamPitch)
                             || keyframes.is_keyframed(&KeyframeType::SmoothingParamYaw);

        quats.iter().map(|x| {
            if is_keyframed {
                let timestamp_ms = *x.0 as f64 / 1000.0;
                let r = keyframes.value_at_gyro_timestamp(&KeyframeType::SmoothingParamRoll, timestamp_ms).unwrap_or(self.roll);
                let p = keyframes.value_at_gyro_timestamp(&KeyframeType::SmoothingParamPitch, timestamp_ms).unwrap_or(self.pitch);
                let y = keyframes.value_at_gyro_timestamp(&KeyframeType::SmoothingParamYaw, timestamp_ms).unwrap_or(self.yaw);
                (*x.0, quat_for_rpy(r, p, y))
            } else {
                (*x.0, fixed_quat)
            }
        }).collect()
        // No need to reverse the BTreeMap, because it's sorted by definition
    }
}
