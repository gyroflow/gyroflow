use super::*;
use nalgebra::*;
use crate::gyro_source::TimeQuat;


#[derive(Clone)]
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
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameters_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "name": "roll",
                "description": "Roll angle",
                "type": "Slider",
                "from": -180,
                "to": 180,
                "value": 0,
                "unit": "°"
            },
            {
                "name": "pitch",
                "description": "Pitch angle",
                "type": "Slider",
                "from": -90,
                "to": 90,
                "value": 0,
                "unit": "°"
            },
            {
                "name": "yaw",
                "description": "Yaw angle",
                "type": "Slider",
                "from": -180,
                "to": 180,
                "value": 0,
                "unit": "°"
            }
        ])
    }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.roll.to_bits());
        hasher.write_u64(self.pitch.to_bits());
        hasher.write_u64(self.yaw.to_bits());
        hasher.finish()
    }

    fn smooth(&self, quats: &TimeQuat, duration: f64) -> TimeQuat {

        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }
        const DEG2RAD: f64 = std::f64::consts::PI / 180.0;
        let x_axis = nalgebra::Vector3::<f64>::x_axis();
        let y_axis = nalgebra::Vector3::<f64>::y_axis();
        let z_axis = nalgebra::Vector3::<f64>::z_axis();
        
        let rot_x = Rotation3::from_axis_angle(&x_axis, self.pitch * DEG2RAD);
        let rot_y = Rotation3::from_axis_angle(&y_axis, self.yaw * DEG2RAD);
        let rot_z = Rotation3::from_axis_angle(&z_axis, self.roll * DEG2RAD);

        // Z rotation corresponds to body-centric roll, so placed last
        // using x as second rotation corresponds gives the usual pan/tilt combination
        let combined_rot = rot_y * rot_x * rot_z;
        let fixed_quat = UnitQuaternion::from_rotation_matrix(&combined_rot);
        
        //let fixed_quat = UnitQuaternion::from_euler_angles(self.yaw * DEG2RAD,self.roll * DEG2RAD,self.pitch * DEG2RAD);
        
        quats.iter().map(|x| {
            (*x.0, fixed_quat)
        }).collect()
        // No need to reverse the BTreeMap, because it's sorted by definition
    }
}
