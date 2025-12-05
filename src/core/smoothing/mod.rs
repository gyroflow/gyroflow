// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

pub mod horizon;
pub mod none;
pub mod plain;
pub mod fixed;
pub mod default_algo;
pub mod focal_length;

pub use nalgebra::*;
use super::gyro_source::{ TimeQuat, Quat64 };
pub use std::collections::HashMap;
use dyn_clone::{ clone_trait_object, DynClone };
use std::borrow::Cow;

use std::hash::Hasher;
use std::collections::hash_map::DefaultHasher;
use crate::ComputeParams;

pub trait SmoothingAlgorithm: DynClone {
    fn get_name(&self) -> String;

    fn get_parameters_json(&self) -> serde_json::Value;
    fn get_status_json(&self) -> serde_json::Value;
    fn set_parameter(&mut self, name: &str, val: f64);
    fn get_parameter(&self, name: &str) -> f64;

    fn get_checksum(&self) -> u64;

    fn smooth(&self, quats: &TimeQuat, duration: f64, _compute_params: &ComputeParams) -> TimeQuat;
}
clone_trait_object!(SmoothingAlgorithm);

struct Algs(Vec<Box<dyn SmoothingAlgorithm>>);
impl Default for Algs {
    fn default() -> Self {
        Self(vec![
            Box::new(self::none::None::default()),
            Box::new(self::default_algo::DefaultAlgo::default()),
            Box::new(self::plain::Plain::default()),
            Box::new(self::fixed::Fixed::default())
        ])
    }

}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Smoothing {
    #[serde(skip)]
    algs: Algs,
    current_id: usize,

    pub horizon_lock: horizon::HorizonLock
}
unsafe impl Send for Smoothing { }
unsafe impl Sync for Smoothing { }

impl Default for Smoothing {
    fn default() -> Self {
        Self {
            algs: Algs::default(),

            current_id: 1,

            horizon_lock: horizon::HorizonLock::default(),
        }
    }
}

impl Clone for Smoothing {
    fn clone(&self) -> Self {
        let mut ret = Self::default();
        ret.current_id = self.current_id;
        ret.horizon_lock = self.horizon_lock.clone();

        let parameters = self.current().get_parameters_json();
        if let serde_json::Value::Array(ref arr) = parameters {
            for v in arr {
                if let serde_json::Value::Object(obj) = v {
                    (|| -> Option<()> {
                        let name = obj.get("name").and_then(|x| x.as_str())?;
                        let value = obj.get("value").and_then(|x| x.as_f64())?;
                        ret.current_mut().set_parameter(name, value);
                        Some(())
                    })();
                }
            }
        }

        ret
    }
}

impl Smoothing {
    pub fn set_current(&mut self, id: usize) {
        self.current_id = id.min(self.algs.0.len() - 1);
    }

    pub fn current(&self) -> &Box<dyn SmoothingAlgorithm> {
        &self.algs.0[self.current_id]
    }
    pub fn current_mut(&mut self) -> &mut Box<dyn SmoothingAlgorithm> {
        &mut self.algs.0[self.current_id]
    }

    pub fn get_state_checksum(&self, gyro_checksum: u64) -> u64 {
        let mut hasher = DefaultHasher::new();
        hasher.write_u64(gyro_checksum);
        hasher.write_usize(self.current_id);
        hasher.write_u64(self.algs.0[self.current_id].get_checksum());
        hasher.write_u64(self.horizon_lock.get_checksum());
        hasher.finish()
    }

    pub fn get_names(&self) -> Vec<String> {
        self.algs.0.iter().map(|x| x.get_name()).collect()
    }

    pub fn get_trimmed_quats<'a>(quats: &'a TimeQuat, duration_ms: f64, trim_range_only: bool, trim_ranges: &[(f64, f64)]) -> Cow<'a, TimeQuat> {
        if trim_range_only && !trim_ranges.is_empty() {
            let mut quats_copy = quats.clone();
            let ranges = trim_ranges.iter().map(|x| ((x.0 * duration_ms * 1000.0).round() as i64, (x.1 * duration_ms * 1000.0).round() as i64)).collect::<Vec<_>>();
            let mut prev_q = quats.range(ranges.first().unwrap().0..).next().map(|(&a, &b)| (a, b));
            let mut next_q = prev_q;
            let mut range = ranges.first().unwrap();
            let mut current_range = 0;
            for (ts, q) in quats_copy.iter_mut() {
                while *ts > range.1 {
                    if let Some(next_range) = ranges.get(current_range + 1) {
                        current_range += 1;
                        range = next_range;
                    } else {
                        prev_q = quats.range(..ranges.last().unwrap().1).next_back().map(|(&a, &b)| (a, b));
                        next_q = prev_q;
                        range = &(i64::MAX, i64::MAX);
                        break;
                    }
                    prev_q = Some((*ts, q.clone()));
                    next_q = quats.range(range.0..).next().map(|(&a, &b)| (a, b));
                }
                if !(*ts >= range.0 && *ts <= range.1) {
                    if let Some(prev_q) = prev_q {
                        if let Some(next_q) = next_q {
                            let dist_to_next = if next_q.0 == prev_q.0 { 0.0 } else { (*ts - prev_q.0) as f64 / (next_q.0 - prev_q.0) as f64 };
                            if dist_to_next.abs() == 0.0 {
                                *q = prev_q.1;
                            } else {
                                *q = prev_q.1.slerp(&next_q.1, dist_to_next);
                            }
                        }
                    }
                }
            }
            Cow::Owned(quats_copy)
        } else {
            Cow::Borrowed(quats)
        }
    }

    pub fn get_max_angles(quats: &TimeQuat, smoothed_quats: &TimeQuat, params: &ComputeParams) -> (f64, f64, f64) { // -> (pitch, yaw, roll) in deg
        let ranges = params.trim_ranges.iter().map(|x| ((x.0 * params.scaled_duration_ms * 1000.0) as i64, (x.1 * params.scaled_duration_ms * 1000.0) as i64)).collect::<Vec<_>>();
        let identity_quat = Quat64::identity();

        let mut max_pitch = 0.0;
        let mut max_yaw = 0.0;
        let mut max_roll = 0.0;

        for (timestamp, quat) in smoothed_quats.iter() {
            let within_range = ranges.is_empty() || ranges.iter().any(|x| timestamp >= &x.0 && timestamp <= &x.1);
            if within_range {
                let dist = quat.inverse() * quats.get(timestamp).unwrap_or(&identity_quat);
                let euler_dist = dist.euler_angles();
                if euler_dist.2.abs() > max_roll  { max_roll  = euler_dist.2.abs(); }
                if euler_dist.0.abs() > max_pitch { max_pitch = euler_dist.0.abs(); }
                if euler_dist.1.abs() > max_yaw   { max_yaw   = euler_dist.1.abs(); }
            }
        }

        const RAD2DEG: f64 = 180.0 / std::f64::consts::PI;
        (max_pitch * RAD2DEG, max_yaw * RAD2DEG, max_roll * RAD2DEG)
    }
}
