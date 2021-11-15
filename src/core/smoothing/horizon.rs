use super::*;
use nalgebra::*;
use crate::core::gyro_source::TimeQuat;


// Alternative implementation, TODO: figure out horizon lock math for the used coordinate system
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
// https://en.wikipedia.org/wiki/Conversion_between_quaternions_and_Euler_angles
fn to_euler_angles(q: UnitQuaternion<f64>) -> (f64, f64, f64) {
    // roll (x-axis rotation)
    let sinr_cosp = 2. * (q.w * q.i + q.j * q.k);
    let cosr_cosp = 1. - 2. * (q.i * q.i + q.j * q.j);
    let roll = sinr_cosp.simd_atan2(cosr_cosp);

    // pitch (y-axis rotation)
    let sinp = 2. * (q.w * q.j - q.k * q.i);
    let mut pitch;
    if (sinp.abs() >= 1.) {
        pitch = std::f64::consts::FRAC_PI_2.simd_copysign(sinp); // use 90 degrees if out of range
    }
    else {
        pitch = sinp.asin();
    }

    // yaw (z-axis rotation)
    let siny_cosp = 2. * (q.w * q.k + q.i * q.j);
    let cosy_cosp = 1. - 2. * (q.j * q.j + q.k * q.k);
    let yaw = siny_cosp.simd_atan2(cosy_cosp);

    (roll, pitch, yaw)
}

fn lock_horizon_angle(q: UnitQuaternion<f64>) -> UnitQuaternion<f64> {
    let euler = to_euler_angles(q);
    println!("{:?}", euler);
    from_euler_angles(euler.0, euler.1, euler.2)
}

pub struct HorizonLock { pub time_constant: f64 }

impl Default for HorizonLock {
    fn default() -> Self { Self { time_constant: 0.2 } }
}

impl SmoothingAlgorithm for HorizonLock {
    fn get_name(&self) -> String { "Lock horizon".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "time_constant" => self.time_constant = val,
            _ => eprintln!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameters_json(&self) -> simd_json::owned::Value {
        simd_json::json!([
            {
                "name": "time_constant",
                "description": "Time constant",
                "type": "Slider",
                "from": 0.01,
                "to": 10.0,
                "value": 0.25,
                "unit": "s"
            }
        ])
    }

    fn smooth(&self, quats: &TimeQuat, duration: f64) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);

        let mut alpha = 1.0;
        if self.time_constant > 0.0 {
            alpha = 1.0 - (-(1.0 / sample_rate) / self.time_constant).exp();
        }
        
        let mut q = *quats.iter().next().unwrap().1;
        let smoothed1: TimeQuat = quats.iter().map(|x| {
            q = q.slerp(x.1, alpha);
            (*x.0, q)
        }).collect();

        // Reverse pass
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        smoothed1.iter().rev().map(|x| {
            q = q.slerp(x.1, alpha);
            q = lock_horizon_angle(q);
            (*x.0, q)
        }).collect()
        // No need to reverse the BTreeMap, because it's sorted by definition
    }
}
