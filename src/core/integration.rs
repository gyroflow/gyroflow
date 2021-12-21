
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
        if imu_data.is_empty() { return BTreeMap::new(); }

        let mut quats = BTreeMap::new();
        let init_pos = UnitQuaternion::from_euler_angles(0.0, std::f64::consts::FRAC_PI_2, 0.0);
        let sample_time_s = duration_ms / 1000.0 / imu_data.len() as f64;
    
        let mut ahrs = Madgwick::new_with_quat(sample_time_s, 0.01, init_pos);
        let mut prev_time = imu_data[0].timestamp_ms - sample_time_s;
        for v in imu_data {
            if let Some(g) = v.gyro.as_ref() {
                let gyro = Vector3::new(-g[1], g[0], g[2]) * (std::f64::consts::PI / 180.0);
                let a = v.accl.unwrap_or([0.000001, 0.0, 0.0]);
                let accl = Vector3::new(-a[1], a[0], a[2]);

                *ahrs.sample_period_mut() = (v.timestamp_ms - prev_time) / 1000.0;

                if let Some(m) = v.magn.as_ref() {
                    let magn = Vector3::new(-m[1], m[0], m[2]);

                    match ahrs.update(&gyro, &accl, &magn) {
                        Ok(quat) => { quats.insert((v.timestamp_ms * 1000.0) as i64, *quat); },
                        Err(e) => log::warn!("Invalid data! {} Gyro: [{}, {}, {}] Accel: [{}, {}, {}] Magn: [{}, {}, {}]", e, gyro[0], gyro[1], gyro[2], accl[0], accl[1], accl[2], magn[0], magn[1], magn[2])
                    }
                } else {
                    match ahrs.update_imu(&gyro, &accl) {
                        Ok(quat) => { quats.insert((v.timestamp_ms * 1000.0) as i64, *quat); },
                        Err(e) => log::warn!("Invalid data! {} Gyro: [{}, {}, {}] Accel: [{}, {}, {}]", e, gyro[0], gyro[1], gyro[2], accl[0], accl[1], accl[2])
                    }
                }
                prev_time = v.timestamp_ms;
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
        if imu_data.is_empty() { return BTreeMap::new(); }

        let mut quats = BTreeMap::new();
        let init_pos = UnitQuaternion::from_euler_angles(0.0, std::f64::consts::FRAC_PI_2, 0.0);
        let sample_time_s = duration_ms / 1000.0 / imu_data.len() as f64;
    
        let mut ahrs = Mahony::new_with_quat(sample_time_s, 0.5, 0.0, init_pos);
        let mut prev_time = imu_data[0].timestamp_ms - sample_time_s;
        for v in imu_data {
            if let Some(g) = v.gyro.as_ref() {
                let gyro = Vector3::new(-g[1], g[0], g[2]) * (std::f64::consts::PI / 180.0);
                let a = v.accl.unwrap_or([0.000001, 0.0, 0.0]);
                let accl = Vector3::new(-a[1], a[0], a[2]);

                *ahrs.sample_period_mut() = (v.timestamp_ms - prev_time) / 1000.0;

                if let Some(m) = v.magn.as_ref() {
                    let magn = Vector3::new(-m[1], m[0], m[2]);

                    match ahrs.update(&gyro, &accl, &magn) {
                        Ok(quat) => { quats.insert((v.timestamp_ms * 1000.0) as i64, *quat); },
                        Err(e) => log::warn!("Invalid data! {} Gyro: [{}, {}, {}] Accel: [{}, {}, {}] Magn: [{}, {}, {}]", e, gyro[0], gyro[1], gyro[2], accl[0], accl[1], accl[2], magn[0], magn[1], magn[2])
                    }
                } else {
                    match ahrs.update_imu(&gyro, &accl) {
                        Ok(quat) => { quats.insert((v.timestamp_ms * 1000.0) as i64, *quat); },
                        Err(e) => log::warn!("Invalid data! {} Gyro: [{}, {}, {}] Accel: [{}, {}, {}]", e, gyro[0], gyro[1], gyro[2], accl[0], accl[1], accl[2])
                    }
                }
                prev_time = v.timestamp_ms;
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
        if imu_data.is_empty() { return BTreeMap::new(); }
        let mut quats = BTreeMap::new();
        // let gyro_sample_rate = imu_data.len() as f64 / (duration_ms / 1000.0);
        let sample_time_s = duration_ms / 1000.0 / imu_data.len() as f64;
        let mut orientation = UnitQuaternion::from_euler_angles(0.0, std::f64::consts::FRAC_PI_2, 0.0);

        // let start_time_s = imu_data[0].timestamp / 1000.0;

        // let mut i: i32 = 0;

        for v in imu_data {
            if let Some(g) = v.gyro.as_ref() {
                let omega = Vector3::new(-g[1], g[0], g[2]) * (std::f64::consts::PI / 180.0);
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

                quats.insert((v.timestamp_ms * 1000.0) as i64, orientation);

            }
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
        if imu_data.is_empty() { return BTreeMap::new(); }
        let mut quats = BTreeMap::new();
        let sample_time_s = duration_ms / 1000.0 / imu_data.len() as f64;
        let init_pos = UnitQuaternion::from_euler_angles(0.0, std::f64::consts::FRAC_PI_2, 0.0);

        let mut f = ComplementaryFilter::default();
        f.do_adaptive_gain = true;
        let init_pos_q = init_pos.quaternion();
        f.set_orientation(init_pos_q.scalar(), init_pos_q.vector()[0], init_pos_q.vector()[1], init_pos_q.vector()[2]);
        
        const DEG2RAD: f64 = std::f64::consts::PI / 180.0;
        let mut prev_time = imu_data[0].timestamp_ms - sample_time_s;
        for v in imu_data {
            if let Some(g) = v.gyro.as_ref() {
                let a = v.accl.unwrap_or([0.000001, 0.0, 0.0]);

                if let Some(acc) = Vector3::new(-a[1], a[0], a[2]).try_normalize(0.0) {
                    if let Some(m) = v.magn.as_ref() {
                        if let Some(magn) = Vector3::new(-m[1], m[0], m[2]).try_normalize(0.0) {
                            f.update_mag(acc[0], acc[1], acc[2],
                                -g[1] * DEG2RAD, g[0] * DEG2RAD, g[2] * DEG2RAD, 
                                magn[0], magn[1], magn[2],
                                (v.timestamp_ms - prev_time) / 1000.0);
                        }
                    } else {
                        f.update(acc[0], acc[1], acc[2],
                            -g[1] * DEG2RAD, g[0] * DEG2RAD, g[2] * DEG2RAD, 
                            (v.timestamp_ms - prev_time) / 1000.0);
                    }
                    let x = f.get_orientation();
                    quats.insert((v.timestamp_ms * 1000.0) as i64, Quat64::from_quaternion(Quaternion::from_parts(x.0, Vector3::new(x.1, x.2, x.3))));
                }
                prev_time = v.timestamp_ms;
            }
        }

        quats
    }
}
