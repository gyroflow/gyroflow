use super::*;
use nalgebra::*;
use crate::gyro_source::TimeQuat;


#[derive(Clone)]
pub struct HorizonLock { pub time_constant: f64,  pub roll: f64, pub pitch: f64, pub yaw: f64 }

impl Default for HorizonLock {
    fn default() -> Self { Self { time_constant: 0.2, roll: 0.0, pitch: 0.0, yaw: 0.0 } }
}

// Alternative implementation adapted from https://en.wikipedia.org/wiki/Conversion_between_quaternions_and_Euler_angles for the "standard" order
/*
fn from_euler_angles(roll: f64, pitch: f64, yaw: f64) -> UnitQuaternion<f64> {
    let (sr, cr) = (roll * 0.5f64).simd_sin_cos();
    let (sp, cp) = (pitch * 0.5f64).simd_sin_cos();
    let (sy, cy) = (yaw * 0.5f64).simd_sin_cos();
    

    let q = Quaternion::<f64>::new(
        cr.clone() * cp.clone() * cy.clone() + sr.clone() * sp.clone() * sy.clone(),
        sr.clone() * cp.clone() * cy.clone() - cr.clone() * sp.clone() * sy.clone(),
        cr.clone() * sp.clone() * cy.clone() + sr.clone() * cp.clone() * sy.clone(),
        cr * cp * sy - sr * sp * cy,
    );

    UnitQuaternion::<f64>::from_quaternion(q)
}

fn to_euler_angles(q: UnitQuaternion<f64>) -> (f64, f64, f64) {
    // roll (x-axis rotation)
    let sinr_cosp = 2. * (q.w * q.i + q.j * q.k);
    let cosr_cosp = 1. - 2. * (q.i * q.i + q.j * q.j);
    let roll = sinr_cosp.simd_atan2(cosr_cosp);

    // pitch (y-axis rotation)
    let sinp = 2. * (q.w * q.j - q.k * q.i);
    let pitch = if sinp.abs() >= 1. {
        std::f64::consts::FRAC_PI_2.simd_copysign(sinp) // use 90 degrees if out of range
    } else {
        sinp.asin()
    };

    // yaw (z-axis rotation)
    let siny_cosp = 2. * (q.w * q.k + q.i * q.j);
    let cosy_cosp = 1. - 2. * (q.j * q.j + q.k * q.k);
    let yaw = siny_cosp.simd_atan2(cosy_cosp);

    (roll, pitch, yaw)
}
*/

// "correct" euler angle order
fn from_euler_yxz(x: f64, y: f64, z: f64) -> UnitQuaternion<f64> {
    let x_axis = nalgebra::Vector3::<f64>::x_axis();
    let y_axis = nalgebra::Vector3::<f64>::y_axis();
    let z_axis = nalgebra::Vector3::<f64>::z_axis();
    
    let rot_x = Rotation3::from_axis_angle(&x_axis, x);
    let rot_y = Rotation3::from_axis_angle(&y_axis, y);
    let rot_z = Rotation3::from_axis_angle(&z_axis, z);

    // Z rotation corresponds to body-centric roll, so placed last
    // using x as second rotation corresponds gives the usual pan/tilt combination
    let combined_rotation = rot_y * rot_x * rot_z;
    UnitQuaternion::from_rotation_matrix(&combined_rotation)
}

fn lock_horizon_angle(q: UnitQuaternion<f64>, roll_correction: f64) -> UnitQuaternion<f64> {
    // z axis points in view direction, use as reference
    let z_axis = nalgebra::Vector3::<f64>::z_axis();
    
    // since this coincides with roll axis, the roll is neglected when transformed back
    let z_transformed = q.transform_vector(&z_axis);

    let pitch = (-z_transformed.y).asin();
    let yaw = z_transformed.x.simd_atan2(z_transformed.z);

    from_euler_yxz(pitch, yaw,roll_correction)
}


impl SmoothingAlgorithm for HorizonLock {
    fn get_name(&self) -> String { "Lock horizon".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "time_constant" => self.time_constant = val,
            "roll" => self.roll = val,
            "pitch" => self.pitch = val,
            "yaw" => self.yaw = val,
            _ => eprintln!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameters_json(&self) -> simd_json::owned::Value {
        simd_json::json!([
            {
                "name": "time_constant",
                "description": "Smoothness",
                "type": "SliderWithField",
                "from": 0.01,
                "to": 10.0,
                "value": 0.25,
                "unit": "s"
            },
            {
                "name": "roll",
                "description": "Roll angle correction",
                "type": "SliderWithField",
                "from": -180,
                "to": 180,
                "value": 0,
                "unit": "deg"
            },
            {
                "name": "pitch",
                "description": "Pitch angle correction (todo)",
                "type": "SliderWithField",
                "from": -90,
                "to": 90,
                "value": 0,
                "unit": "deg"
            },
            {
                "name": "yaw",
                "description": "Yaw angle correction (todo)",
                "type": "SliderWithField",
                "from": -180,
                "to": 180,
                "value": 0,
                "unit": "deg"
            },
        ])
    }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.time_constant.to_bits());
        hasher.write_u64(self.roll.to_bits());
        hasher.write_u64(self.pitch.to_bits());
        hasher.write_u64(self.yaw.to_bits());
        hasher.finish()
    }

    fn smooth(&self, quats: &TimeQuat, duration: f64) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let mut alpha = 1.0;
        if self.time_constant > 0.0 {
            alpha = 1.0 - (-(1.0 / sample_rate) / self.time_constant).exp();
        }
        const DEG2RAD: f64 = std::f64::consts::PI / 180.0;

        // TODO: This correction should be applied to the raw orientations as well. Alternatively the transform needs to be cancelled out
        //let correction_quat = from_euler_yxz(self.pitch * DEG2RAD, self.yaw * DEG2RAD, self.roll * DEG2RAD);
        let mut q = *quats.iter().next().unwrap().1;
        let smoothed1: TimeQuat = quats.iter().map(|x| {
            q = q.slerp(x.1, alpha);
            (*x.0, q)
        }).collect();

        // Reverse pass, while leveling horizon
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        smoothed1.iter().rev().map(|x| {
            q = lock_horizon_angle(q.slerp(x.1, alpha), self.roll * DEG2RAD);
            (*x.0, q)
        }).collect()
        // No need to reverse the BTreeMap, because it's sorted by definition
    }
}
