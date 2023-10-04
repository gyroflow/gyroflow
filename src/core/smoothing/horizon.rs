// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Elvin Chen

use super::*;
use nalgebra::*;
use crate::{ gyro_source::TimeQuat, keyframes::* };


pub fn lock_horizon_angle(q: &UnitQuaternion<f64>, roll_correction: f64) -> UnitQuaternion<f64> {
    // z axis points in view direction, use as reference

    let x_axis = nalgebra::Vector3::<f64>::x_axis();
    let y_axis = nalgebra::Vector3::<f64>::y_axis();
    let z_axis = nalgebra::Vector3::<f64>::z_axis();

    let test_vec = q * nalgebra::Vector3::<f64>::z_axis();
    let pitch    = (-test_vec.z).asin();
    let yaw      = test_vec.y.simd_atan2(test_vec.x);

    let rot_yaw   = UnitQuaternion::from_axis_angle(&y_axis, yaw);
    let rot_pitch = UnitQuaternion::from_axis_angle(&x_axis, pitch);
    let rot_roll  = UnitQuaternion::from_axis_angle(&z_axis, roll_correction);

    let initial_quat = UnitQuaternion::from_axis_angle(&y_axis, std::f64::consts::FRAC_PI_2) * UnitQuaternion::from_axis_angle(&z_axis, std::f64::consts::FRAC_PI_2);

    initial_quat * rot_yaw * rot_pitch * rot_roll
}

#[derive(Clone)]
pub struct HorizonLock {
    pub lock_enabled: bool,
    pub horizonlockpercent: f64,
    pub horizonroll: f64,
}

impl Default for HorizonLock {
    fn default() -> Self { Self {
        lock_enabled: false,
        horizonlockpercent: 100.0,
        horizonroll: 0.0,
    } }
}

impl HorizonLock {
    pub fn set_horizon(&mut self, lock_percent: f64, roll: f64) {
        self.horizonroll = roll;
        self.horizonlockpercent = lock_percent;
        self.lock_enabled = self.horizonlockpercent > 1e-6;
    }
    pub fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.horizonlockpercent.to_bits());
        hasher.write_u64(self.horizonroll.to_bits());
        hasher.finish()
    }

    pub fn lock(&self, quats: &mut TimeQuat, org_quats: &TimeQuat, grav: &Option<crate::gyro_source::TimeVec>, use_grav: bool, _int_method: usize, keyframes: &KeyframeManager, params: &StabilizationParams) {
        if self.lock_enabled || keyframes.is_keyframed(&KeyframeType::LockHorizonAmount) {
            if let Some(gvec) = grav {
                if !gvec.is_empty() && use_grav {
                    let z_axis = nalgebra::Vector3::<f64>::z_axis();
                    let y_axis = nalgebra::Vector3::<f64>::y_axis();
                    // let corr = Rotation3::from_axis_angle(&z_axis, std::f64::consts::PI);

                    for (ts, smoothed_ori) in quats.iter_mut() {
                        let gv = Self::interpolate_gravity_vector(&gvec, *ts).unwrap_or(*y_axis);
                        let ori = org_quats.get(ts).unwrap_or(&smoothed_ori).to_rotation_matrix();

                        // Correct for angle difference between original and smoothed orientation
                        let correction = ori.inverse() * smoothed_ori.to_rotation_matrix();
                        let angle_corr = (-correction[(0, 1)]).simd_atan2(correction[(0, 0)]);

                        let timestamp_ms = *ts as f64 / 1000.0;
                        let video_rotation = keyframes.value_at_gyro_timestamp(&KeyframeType::VideoRotation, timestamp_ms).unwrap_or(params.video_rotation);
                        let horizonroll = keyframes.value_at_gyro_timestamp(&KeyframeType::LockHorizonRoll, timestamp_ms).unwrap_or(self.horizonroll) + video_rotation;
                        let horizonlockpercent = keyframes.value_at_gyro_timestamp(&KeyframeType::LockHorizonAmount, timestamp_ms).unwrap_or(self.horizonlockpercent);

                        // let gv_corrected = corr.inverse() * correction * corr * gv; // Alternative matrix approach
                        // let locked_ori = smoothed_ori.to_rotation_matrix() * Rotation3::from_axis_angle(&z_axis, gv_corrected[0].simd_atan2(gv_corrected[1]) + horizonroll * std::f64::consts::PI / 180.0);
                        let locked_ori = smoothed_ori.to_rotation_matrix() * Rotation3::from_axis_angle(&z_axis, -angle_corr + gv[0].simd_atan2(gv[1]) + horizonroll * std::f64::consts::PI / 180.0);
                        *smoothed_ori = UnitQuaternion::from_rotation_matrix(&locked_ori).slerp(&smoothed_ori, 1.0 - horizonlockpercent / 100.0)
                    }
                    return;
                }
            }

            for (ts, smoothed_ori) in quats.iter_mut() {
                let timestamp_ms = *ts as f64 / 1000.0;
                let video_rotation = keyframes.value_at_gyro_timestamp(&KeyframeType::VideoRotation, timestamp_ms).unwrap_or(params.video_rotation);
                let horizonroll = keyframes.value_at_gyro_timestamp(&KeyframeType::LockHorizonRoll, timestamp_ms).unwrap_or(self.horizonroll) + video_rotation;
                let horizonlockpercent = keyframes.value_at_gyro_timestamp(&KeyframeType::LockHorizonAmount, timestamp_ms).unwrap_or(self.horizonlockpercent);

                *smoothed_ori = lock_horizon_angle(smoothed_ori, horizonroll * std::f64::consts::PI / 180.0).slerp(&smoothed_ori, 1.0 - horizonlockpercent / 100.0);
            }
        }
    }

    pub fn interpolate_gravity_vector(gravs: &crate::gyro_source::TimeVec, timestamp_us: i64) -> Option<Vector3<f64>> {
        match gravs.len() {
            0 => None,
            1 => gravs.values().next().cloned(),
            _ => {
                if let Some(&first_ts) = gravs.keys().next() {
                    if let Some(&last_ts) = gravs.keys().next_back() {
                        let lookup_ts = timestamp_us.min(last_ts).max(first_ts);
                        if let Some(offs1) = gravs.range(..=lookup_ts).next_back() {
                            if *offs1.0 == lookup_ts {
                                return Some(*offs1.1);
                            }
                            if let Some(offs2) = gravs.range(lookup_ts..).next() {
                                let time_delta = (offs2.0 - offs1.0) as f64;
                                let fract = (timestamp_us - offs1.0) as f64 / time_delta;
                                return Some(offs1.1 + (offs2.1 - offs1.1) * fract);
                            }
                        }
                    }
                }

                None
            }
        }
    }

}
