
use std::collections::BTreeMap;

use nalgebra::*;
use super::gyro_source::{TimeIMU, Quat64, TimeQuat};
use ahrs::{Ahrs, Madgwick, Mahony};

pub trait GyroIntegrator {
    fn integrate(imu_data: &[TimeIMU], duration_ms: f64) -> TimeQuat;
}

pub struct MadgwickIntegrator { }
pub struct GyroOnlyIntegrator { }
pub struct MahonyIntegrator { }
pub struct ComplementaryIntegrator { }

impl GyroIntegrator for MadgwickIntegrator {
    fn integrate(imu_data: &[TimeIMU], duration_ms: f64) -> TimeQuat {
        let mut quats = BTreeMap::new();
        let sample_time_s = duration_ms / 1000.0 / imu_data.len() as f64;
        let init_pos = UnitQuaternion::from_euler_angles(0.0, std::f64::consts::FRAC_PI_2, 0.0);
    
        let mut ahrs = Madgwick::new_with_quat(sample_time_s, 0.0, init_pos);
        for v in imu_data {
            let gyro = Vector3::new(-v.gyro[1], v.gyro[0], v.gyro[2]) * (std::f64::consts::PI / 180.0);
            let accl = Vector3::new(-v.accl[1], v.accl[0], v.accl[2]);
            //let magn = Vector3::new(v.magn[0], v.magn[1], v.magn[2]);
            match ahrs.update_imu(&gyro, &accl) {
                Ok(quat) => { quats.insert((v.timestamp * 1000.0) as i64, *quat); },
                Err(e) => eprintln!("Invalid data! {} Gyro: [{}, {}, {}] Accel: [{}, {}, {}]", e, gyro[0], gyro[1], gyro[2], accl[0], accl[1], accl[2])
            }
        }

        quats
    }
}

///////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////

impl GyroIntegrator for MahonyIntegrator {
    fn integrate(imu_data: &[TimeIMU], duration_ms: f64) -> TimeQuat {
        let mut quats = BTreeMap::new();
        let sample_time_s = duration_ms / 1000.0 / imu_data.len() as f64;
        let init_pos = UnitQuaternion::from_euler_angles(0.0, std::f64::consts::FRAC_PI_2, 0.0);
    
        let mut ahrs = Mahony::new_with_quat(sample_time_s, 0.5, 0.0, init_pos);
        for v in imu_data {
            let gyro = Vector3::new(-v.gyro[1], v.gyro[0], v.gyro[2]) * (std::f64::consts::PI / 180.0);
            let accl = Vector3::new(-v.accl[1], v.accl[0], v.accl[2]);
            //let magn = Vector3::new(v.magn[0], v.magn[1], v.magn[2]);
            match ahrs.update_imu(&gyro, &accl) {
                Ok(quat) => { quats.insert((v.timestamp * 1000.0) as i64, *quat); },
                Err(e) => eprintln!("Invalid data! {} Gyro: [{}, {}, {}] Accel: [{}, {}, {}]", e, gyro[0], gyro[1], gyro[2], accl[0], accl[1], accl[2])
            }
        }

        quats
    }
}

///////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////

impl GyroIntegrator for GyroOnlyIntegrator {
    fn integrate(imu_data: &[TimeIMU], duration_ms: f64) -> TimeQuat {
        let mut quats = BTreeMap::new();
        // let gyro_sample_rate = imu_data.len() as f64 / (duration_ms / 1000.0);
        let sample_time_s = duration_ms / 1000.0 / imu_data.len() as f64;
        let mut orientation = UnitQuaternion::from_euler_angles(0.0, std::f64::consts::FRAC_PI_2, 0.0);

        // let start_time_s = imu_data[0].timestamp / 1000.0;

        // let mut i: i32 = 0;

        for v in imu_data {
            let omega = Vector3::new(-v.gyro[1], v.gyro[0], v.gyro[2]) * (std::f64::consts::PI / 180.0);
            // let accl  = Vector3::new(-v.accl[1], v.accl[0], v.accl[2]).normalize();

            // let last_time = imu_data[(i-1).max(0) as usize].timestamp / 1000.0;
            // let this_time = imu_data[i as usize].timestamp / 1000.0;
            // let next_time = imu_data[(i+1).min(imu_data.len() as i32-1) as usize].timestamp / 1000.0;

            // let delta_time = (next_time - last_time) / 2.0;

            fn rate_to_quat(omega: Vector3<f64>, dt: f64) -> Quaternion<f64> {
                // https://stackoverflow.com/questions/24197182/efficient-quaternion-angular-velocity/24201879#24201879
                // no idea how it fully works, but it does
                let mut ha = omega * dt * 0.5;
                let l = ha.dot(&ha).sqrt();
        
                if l > 1.0e-12 {
                    ha *= l.sin() / l;
                    Quaternion::from_parts(l.cos(), ha).normalize()
                } else {
                    Quaternion::from_parts(1.0, Vector3::from_element(0.0))
                }
            }

            // calculate rotation quaternion
            let delta_q = rate_to_quat(omega, sample_time_s);

            // rotate orientation by this quaternion
            orientation = Quat64::from_quaternion(orientation.quaternion() * delta_q);

            quats.insert((v.timestamp * 1000.0) as i64, orientation);

            // i += 1;
        }

        quats
    }
}

///////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////

use super::integration_complementary::ComplementaryFilter;

impl GyroIntegrator for ComplementaryIntegrator {
    fn integrate(imu_data: &[TimeIMU], duration_ms: f64) -> TimeQuat {
        let mut quats = BTreeMap::new();
        let sample_time_s = duration_ms / 1000.0 / imu_data.len() as f64;
        let init_pos = UnitQuaternion::from_euler_angles(0.0, std::f64::consts::FRAC_PI_2, 0.0);

        let mut f = ComplementaryFilter::default();
        f.do_adaptive_gain = true;
        let init_pos_q = init_pos.quaternion();
        f.set_orientation(init_pos_q.scalar(), init_pos_q.vector()[0], init_pos_q.vector()[1], init_pos_q.vector()[2]);
        
        let deg2rad = std::f64::consts::PI / 180.0;
        for v in imu_data {
            f.update(-v.accl[1],           v.accl[0],           v.accl[2],
                     -v.gyro[1] * deg2rad, v.gyro[0] * deg2rad, v.gyro[2] * deg2rad, 
                     sample_time_s);
            let x = f.get_orientation();
            quats.insert((v.timestamp * 1000.0) as i64, Quat64::from_quaternion(Quaternion::from_parts(x.0, Vector3::new(x.1, x.2, x.3))));
        }

        quats
    }
}
