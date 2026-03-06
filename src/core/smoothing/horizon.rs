// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Elvin Chen

use super::*;
use nalgebra::*;
use crate::keyframes::*;

pub fn lock_horizon_angle(q: &UnitQuaternion<f64>, roll_correction: f64, lock_pitch: bool, pitch_correction: f64) -> UnitQuaternion<f64> {
    let x_axis = nalgebra::Vector3::<f64>::x_axis();
    let y_axis = nalgebra::Vector3::<f64>::y_axis();
    let z_axis = nalgebra::Vector3::<f64>::z_axis();

    let test_vec = q * nalgebra::Vector3::<f64>::z_axis();
    let pitch    = if lock_pitch { pitch_correction } else { (-test_vec.z).asin() };
    let yaw      = test_vec.y.simd_atan2(test_vec.x);

    let rot_yaw   = UnitQuaternion::from_axis_angle(&y_axis, yaw);
    let rot_pitch = UnitQuaternion::from_axis_angle(&x_axis, pitch);
    let rot_roll  = UnitQuaternion::from_axis_angle(&z_axis, roll_correction);

    let initial_quat = UnitQuaternion::from_axis_angle(&y_axis, std::f64::consts::FRAC_PI_2) * UnitQuaternion::from_axis_angle(&z_axis, std::f64::consts::FRAC_PI_2);

    initial_quat * rot_yaw * rot_pitch * rot_roll
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct HorizonLock {
    pub lock_enabled: bool,
    pub horizonlockpercent: f64,
    pub horizonroll: f64,
    pub lock_pitch: bool,
    pub horizonpitch: f64,
    pub automatic_lock: bool,
    pub turn_threshold: f64,
    pub turn_smoothing_ms: f64,
    pub turn_multiplier: f64,
    pub tilt_accel_limit: f64,
}

impl Default for HorizonLock {
    fn default() -> Self { Self {
        lock_enabled: false,
        horizonlockpercent: 100.0,
        horizonroll: 0.0,
        lock_pitch: false,
        horizonpitch: 0.0,
        automatic_lock: false,
        turn_threshold: 5.0,
        turn_smoothing_ms: 500.0,
        turn_multiplier: 1.0,
        tilt_accel_limit: f64::INFINITY,
    } }
}

impl HorizonLock {
    pub fn set_horizon(&mut self, lock_percent: f64, roll: f64, lock_pitch: bool, pitch: f64, automatic_lock: bool, turn_threshold: f64, turn_smoothing_ms: f64, turn_multiplier: f64, tilt_accel_limit: f64) {
        self.horizonroll = roll;
        self.horizonlockpercent = lock_percent;
        self.lock_enabled = self.horizonlockpercent > 1e-6;
        self.horizonpitch = pitch;
        self.lock_pitch = lock_pitch;
        self.automatic_lock = automatic_lock;
        self.turn_threshold = turn_threshold;
        self.turn_smoothing_ms = turn_smoothing_ms;
        self.turn_multiplier = turn_multiplier;
        self.tilt_accel_limit = tilt_accel_limit;
    }

    pub fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.horizonlockpercent.to_bits());
        hasher.write_u64(self.horizonroll.to_bits());
        hasher.write_u8(self.lock_pitch as u8);
        hasher.write_u64(self.horizonpitch.to_bits());
        hasher.write_u64(self.turn_threshold.to_bits());
        hasher.write_u64(self.turn_smoothing_ms.to_bits());
        hasher.write_u64(self.turn_multiplier.to_bits());
        hasher.write_u64(self.tilt_accel_limit.to_bits());
        hasher.finish()
    }

    pub fn lock(&self, quats: &mut TimeQuat, org_quats: &TimeQuat, grav: &Option<crate::gyro_source::TimeVec>, use_grav: bool, _int_method: usize, compute_params: &ComputeParams) {
        let keyframes = &compute_params.keyframes;
        if self.lock_enabled || keyframes.is_keyframed(&KeyframeType::LockHorizonAmount) {
            let mut roll_rates: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
            let mut prev_roll: Option<f64> = None;
            let mut prev_ts: Option<i64> = None;
            let mut prev_smoothed: Option<f64> = None;
            let tau_s: f64 = self.turn_smoothing_ms / 1000.0;
            for (ts, org_quat) in org_quats.iter() {
                let current_euler = org_quat.euler_angles();
                let current_roll: f64 = current_euler.2;
                if let (Some(pr), Some(pt)) = (prev_roll, prev_ts) {
                    let dt = (*ts as f64 - pt as f64) / 1_000_000.0;
                    if dt > 0.0 && dt < 1.0 {
                        let mut diff_deg: f64 = (current_roll - pr).to_degrees();
                        while diff_deg > 180.0 { diff_deg -= 360.0; }
                        while diff_deg < -180.0 { diff_deg += 360.0; }
                        let rate = diff_deg / dt;

                        let alpha = if tau_s <= 0.0 { 1.0 } else { dt / (tau_s + dt) };
                        let smoothed = if let Some(prev) = prev_smoothed {
                            prev * (1.0 - alpha) + rate * alpha
                        } else {
                            rate
                        };
                        prev_smoothed = Some(smoothed);
                        roll_rates.insert(*ts, smoothed);
                    }
                }
                prev_roll = Some(current_roll);
                prev_ts = Some(*ts);
            }

            // Prepare tilt-smoothing state for gravity branch
            let mut prev_tilt_smoothed_grav: Option<f64> = None;
            let mut prev_tilt_ts_grav: Option<i64> = None;

            if let Some(gvec) = grav {
                if !gvec.is_empty() && use_grav {
                    let z_axis = nalgebra::Vector3::<f64>::z_axis();
                    let y_axis = nalgebra::Vector3::<f64>::y_axis();

                    for (ts, smoothed_ori) in quats.iter_mut() {
                        let gv = Self::interpolate_gravity_vector(&gvec, *ts).unwrap_or(*y_axis);
                        let ori = org_quats.get(ts).unwrap_or(&smoothed_ori).to_rotation_matrix();

                        let correction = ori.inverse() * smoothed_ori.to_rotation_matrix();
                        let angle_corr = (-correction[(0, 1)]).simd_atan2(correction[(0, 0)]);

                        let timestamp_ms = *ts as f64 / 1000.0;
                        let video_rotation = keyframes.value_at_gyro_timestamp(&KeyframeType::VideoRotation, timestamp_ms).unwrap_or(compute_params.video_rotation);
                        let horizonroll = keyframes.value_at_gyro_timestamp(&KeyframeType::LockHorizonRoll, timestamp_ms).unwrap_or(self.horizonroll) + video_rotation;
                        let horizonlockpercent = keyframes.value_at_gyro_timestamp(&KeyframeType::LockHorizonAmount, timestamp_ms).unwrap_or(self.horizonlockpercent);

                        // Smooth ramping for dynamic tilt so threshold crossing isn't instantaneous
                        let mut dynamic_tilt_deg: f64 = 0.0;
                        if self.automatic_lock {
                            // Target tilt (deg): proportional to roll rate if above threshold, otherwise 0
                            let target = if let Some(&roll_rate) = roll_rates.get(ts) {
                                if roll_rate.abs() > self.turn_threshold { roll_rate * self.turn_multiplier } else { 0.0 }
                            } else { 0.0 };

                            // Compute smoothing alpha based on time since previous tilt sample
                            let alpha = if let Some(prev_ts_val) = prev_tilt_ts_grav {
                                let dt = (*ts as f64 - prev_ts_val as f64) / 1_000_000.0;
                                if tau_s <= 0.0 { 1.0 } else { (dt / (tau_s + dt)).max(0.0).min(1.0) }
                            } else { 1.0 };

                            let smoothed = if let Some(prev) = prev_tilt_smoothed_grav {
                                prev * (1.0 - alpha) + target * alpha
                            } else { target };

                            // Apply acceleration/deceleration limit
                            let mut accel_limited = smoothed;
                            if self.tilt_accel_limit.is_finite() {
                                if let Some(prev_tilt) = prev_tilt_smoothed_grav {
                                    if let Some(prev_ts_val) = prev_tilt_ts_grav {
                                        let dt = (*ts as f64 - prev_ts_val as f64) / 1_000_000.0;
                                        if dt > 0.0 {
                                            let max_change = self.tilt_accel_limit * dt;
                                            let change = smoothed - prev_tilt;
                                            if change.abs() > max_change {
                                                accel_limited = prev_tilt + change.signum() * max_change;
                                            }
                                        }
                                    }
                                }
                            }

                            prev_tilt_smoothed_grav = Some(accel_limited);
                            prev_tilt_ts_grav = Some(*ts);
                            dynamic_tilt_deg = accel_limited;
                        }

                        let total_horizonroll_deg = horizonroll + dynamic_tilt_deg;

                        let locked_ori = smoothed_ori.to_rotation_matrix() * Rotation3::from_axis_angle(&z_axis, -angle_corr + gv[0].simd_atan2(gv[1]) + total_horizonroll_deg * std::f64::consts::PI / 180.0);
                        *smoothed_ori = UnitQuaternion::from_rotation_matrix(&locked_ori).slerp(&smoothed_ori, 1.0 - horizonlockpercent / 100.0)
                    }
                    return;
                }
            }

            // Prepare tilt-smoothing state for non-gravity branch
            let mut prev_tilt_smoothed: Option<f64> = None;
            let mut prev_tilt_ts: Option<i64> = None;

            for (ts, smoothed_ori) in quats.iter_mut() {
                let timestamp_ms = *ts as f64 / 1000.0;
                let video_rotation = keyframes.value_at_gyro_timestamp(&KeyframeType::VideoRotation, timestamp_ms).unwrap_or(compute_params.video_rotation);
                let horizonroll = keyframes.value_at_gyro_timestamp(&KeyframeType::LockHorizonRoll, timestamp_ms).unwrap_or(self.horizonroll) + video_rotation;
                let horizonpitch = keyframes.value_at_gyro_timestamp(&KeyframeType::LockHorizonPitch, timestamp_ms).unwrap_or(self.horizonpitch);
                let lock_pitch = keyframes.value_at_gyro_timestamp(&KeyframeType::LockHorizonPitchEnabled, timestamp_ms).unwrap_or(if self.lock_pitch { 1.0 } else { 0.0 }) != 0.0;
                let horizonlockpercent = keyframes.value_at_gyro_timestamp(&KeyframeType::LockHorizonAmount, timestamp_ms).unwrap_or(self.horizonlockpercent);

                // Smooth ramping for dynamic tilt
                let mut dynamic_tilt_deg: f64 = 0.0;
                if self.automatic_lock {
                    let target = if let Some(&roll_rate) = roll_rates.get(ts) {
                        if roll_rate.abs() > self.turn_threshold { roll_rate * self.turn_multiplier } else { 0.0 }
                    } else { 0.0 };

                    let alpha = if let Some(prev_ts_val) = prev_tilt_ts {
                        let dt = (*ts as f64 - prev_ts_val as f64) / 1_000_000.0;
                        if tau_s <= 0.0 { 1.0 } else { (dt / (tau_s + dt)).max(0.0).min(1.0) }
                    } else { 1.0 };

                    let smoothed = if let Some(prev) = prev_tilt_smoothed {
                        prev * (1.0 - alpha) + target * alpha
                    } else { target };

                    // Apply acceleration/deceleration limit
                    let mut accel_limited = smoothed;
                    if self.tilt_accel_limit.is_finite() {
                        if let Some(prev_tilt) = prev_tilt_smoothed {
                            if let Some(prev_ts_val) = prev_tilt_ts {
                                let dt = (*ts as f64 - prev_ts_val as f64) / 1_000_000.0;
                                if dt > 0.0 {
                                    let max_change = self.tilt_accel_limit * dt;
                                    let change = smoothed - prev_tilt;
                                    if change.abs() > max_change {
                                        accel_limited = prev_tilt + change.signum() * max_change;
                                    }
                                }
                            }
                        }
                    }

                    prev_tilt_smoothed = Some(accel_limited);
                    prev_tilt_ts = Some(*ts);
                    dynamic_tilt_deg = accel_limited;
                }

                let total_horizonroll_deg = horizonroll + dynamic_tilt_deg;

                *smoothed_ori = lock_horizon_angle(smoothed_ori, total_horizonroll_deg * std::f64::consts::PI / 180.0, lock_pitch, horizonpitch * std::f64::consts::PI / 180.0).slerp(&smoothed_ori, 1.0 - horizonlockpercent / 100.0);
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
