// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::BTreeMap;

use nalgebra::*;
use super::gyro_source::{TimeIMU, Quat64, TimeQuat};
use ahrs::{Ahrs, Madgwick, Mahony};

pub trait GyroIntegrator {
    fn integrate(imu_data: &[TimeIMU], duration_ms: f64) -> TimeQuat;
}

pub struct QuaternionConverter { }
pub struct MadgwickIntegrator { }
pub struct GyroOnlyIntegrator { }
pub struct MahonyIntegrator { }
pub struct ComplementaryIntegrator { }

// const RAD2DEG: f64 = 180.0 / std::f64::consts::PI;
const DEG2RAD: f64 = std::f64::consts::PI / 180.0;

impl QuaternionConverter {
    pub fn convert(org_quaternions: &TimeQuat, imu_data: &[TimeIMU], _duration_ms: f64) -> TimeQuat {

        let x_axis = nalgebra::Vector3::<f64>::x_axis();
        let y_axis = nalgebra::Vector3::<f64>::y_axis();
        let z_axis = nalgebra::Vector3::<f64>::z_axis();

        let initial_quat = UnitQuaternion::from_axis_angle(&y_axis, std::f64::consts::FRAC_PI_2)
                         * UnitQuaternion::from_axis_angle(&z_axis, std::f64::consts::FRAC_PI_2);

        let pitch_offset = if imu_data.is_empty() {
                UnitQuaternion::identity()
            } else {
                let first_imu = imu_data.first().unwrap();
                let a = first_imu.accl.unwrap_or_default();
                let p = -a[2].atan2(a[0]);

                UnitQuaternion::from_axis_angle(&x_axis, p)
            };

        let correction = initial_quat * pitch_offset;

        org_quaternions.iter().map(|(&ts, &org_q)| {
            (ts, correction * org_q)
        }).collect()
    }
}

///////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////

impl GyroIntegrator for MadgwickIntegrator {
    fn integrate(imu_data: &[TimeIMU], duration_ms: f64) -> TimeQuat {
        if imu_data.is_empty() { return BTreeMap::new(); }

        let mut quats = BTreeMap::new();
        let init_pos = UnitQuaternion::from_euler_angles(std::f64::consts::FRAC_PI_2, 0.0, 0.0);
        let sample_time_s = duration_ms / 1000.0 / imu_data.len() as f64;

        let mut ahrs = Madgwick::new_with_quat(sample_time_s, 0.02, init_pos);
        let mut prev_time = imu_data[0].timestamp_ms - sample_time_s;
        for v in imu_data {
            if let Some(g) = v.gyro.as_ref() {
                let gyro = Vector3::new(-g[1], g[0], g[2]) * (std::f64::consts::PI / 180.0);
                let mut a = v.accl.unwrap_or_default();
                if a[0].abs() == 0.0 && a[1].abs() == 0.0 && a[2].abs() == 0.0 { a[0] += 0.0000001; }
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
        let init_pos = UnitQuaternion::from_euler_angles(std::f64::consts::FRAC_PI_2, 0.0, 0.0);
        let sample_time_s = duration_ms / 1000.0 / imu_data.len() as f64;

        let mut ahrs = Mahony::new_with_quat(sample_time_s, 0.5, 0.0, init_pos);
        let mut prev_time = imu_data[0].timestamp_ms - sample_time_s;
        for v in imu_data {
            if let Some(g) = v.gyro.as_ref() {
                let gyro = Vector3::new(-g[1], g[0], g[2]) * (std::f64::consts::PI / 180.0);
                let mut a = v.accl.unwrap_or_default();
                if a[0].abs() == 0.0 && a[1].abs() == 0.0 && a[2].abs() == 0.0 { a[0] += 0.0000001; }
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
        let mut orientation = UnitQuaternion::from_euler_angles(std::f64::consts::FRAC_PI_2, 0.0, 0.0);

        let sample_time_ms = duration_ms / 1000.0 / imu_data.len() as f64;
        let mut prev_time = imu_data[0].timestamp_ms - sample_time_ms;

        for v in imu_data {
            if let Some(g) = v.gyro.as_ref() {
                let omega = Vector3::new(-g[1], g[0], g[2]) * (std::f64::consts::PI / 180.0);

                // calculate rotation quaternion
                let dt = (v.timestamp_ms - prev_time) / 1000.0;
                let delta_q = UnitQuaternion::from_scaled_axis(omega * dt);

                // rotate orientation by this quaternion
                orientation = Quat64::from_quaternion(orientation.quaternion() * delta_q.quaternion());

                quats.insert((v.timestamp_ms * 1000.0) as i64, orientation);

                prev_time = v.timestamp_ms;
            }
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
        let sample_time_ms = duration_ms / imu_data.len() as f64;
        let init_pos = UnitQuaternion::from_euler_angles(std::f64::consts::FRAC_PI_2, 0.0, 0.0);

        let mut f = ComplementaryFilter::default();
        let init_pos_q = init_pos.quaternion();
        f.set_orientation(init_pos_q.scalar(), -init_pos_q.vector()[0], -init_pos_q.vector()[1], -init_pos_q.vector()[2]);

        let mut prev_time = imu_data[0].timestamp_ms - sample_time_ms;
        for v in imu_data {
            if let Some(g) = v.gyro.as_ref() {
                let mut a = v.accl.unwrap_or_default();
                if a[0].abs() == 0.0 && a[1].abs() == 0.0 && a[2].abs() == 0.0 { a[0] += 0.0000001; }

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
