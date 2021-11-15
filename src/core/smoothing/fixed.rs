use super::*;
use nalgebra::*;
use crate::core::gyro_source::TimeQuat;

pub struct Fixed { pub roll: f64, pub pitch: f64, pub yaw: f64}

impl Default for Fixed {
    fn default() -> Self { Self { roll: 0.0, pitch: 0.0, yaw: 0.0 } }
}

impl SmoothingAlgorithm for Fixed {
    fn get_name(&self) -> String { "Fixed camera".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "roll" => self.roll = val,
            "pitch" => self.pitch = val,
            "yaw" => self.yaw = val,
            _ => eprintln!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameters_json(&self) -> simd_json::owned::Value {
        simd_json::json!([
            {
                "name": "roll",
                "description": "Roll angle",
                "type": "Slider",
                "from": -180,
                "to": 180,
                "value": 0,
                "unit": "deg"
            },
            {
                "name": "pitch",
                "description": "Pitch angle",
                "type": "Slider",
                "from": -180,
                "to": 180,
                "value": 0,
                "unit": "deg"
            },
            {
                "name": "yaw",
                "description": "Yaw angle",
                "type": "Slider",
                "from": -180,
                "to": 180,
                "value": 0,
                "unit": "deg"
            }
        ])
    }

    fn smooth(&self, quats: &TimeQuat, duration: f64) -> TimeQuat { // TODO Result<>?

        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }
        let fixedQuat = UnitQuaternion::from_euler_angles(self.roll * std::f64::consts::PI / 180.0,self.pitch * std::f64::consts::PI / 180.0,self.yaw * std::f64::consts::PI / 180.0);
        let mut q = *quats.iter().next().unwrap().1;
        quats.iter().map(|x| {
            q = fixedQuat;
            (*x.0, q)
        }).collect()
        // No need to reverse the BTreeMap, because it's sorted by definition
    }
}
