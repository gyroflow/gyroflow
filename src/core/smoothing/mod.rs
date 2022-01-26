// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

pub mod plain;
pub mod horizon;
pub mod fixed;
// pub mod velocity_dampened_v1;
pub mod velocity_dampened;
pub mod velocity_dampened_axis;

use super::gyro_source::TimeQuat;
pub use std::collections::HashMap;
use dyn_clone::{ clone_trait_object, DynClone };

use std::hash::Hasher;
use std::collections::hash_map::DefaultHasher;

pub trait SmoothingAlgorithm: DynClone {
    fn get_name(&self) -> String;
    
    fn get_parameters_json(&self) -> serde_json::Value;
    fn get_status_json(&self) -> serde_json::Value;
    fn set_parameter(&mut self, name: &str, val: f64);

    fn get_checksum(&self) -> u64;

    fn smooth(&mut self, quats: &TimeQuat, duration: f64, _params: &crate::BasicParams) -> TimeQuat;
}
clone_trait_object!(SmoothingAlgorithm);

#[derive(Clone)]
pub struct None { }
impl SmoothingAlgorithm for None {
    fn get_name(&self) -> String { "No smoothing".to_owned() }

    fn get_parameters_json(&self) -> serde_json::Value { serde_json::json!([]) }
    fn get_status_json(&self) -> serde_json::Value { serde_json::json!([]) }
    fn set_parameter(&mut self, _name: &str, _val: f64) { }

    fn get_checksum(&self) -> u64 { 0 }

    fn smooth(&mut self, quats: &TimeQuat, _duration: f64, _params: &crate::BasicParams) -> TimeQuat { quats.clone() }
}

pub struct Smoothing {
    algs: Vec<Box<dyn SmoothingAlgorithm>>,
    current_id: usize,
    quats_checksum: u64
}
unsafe impl Send for Smoothing { }
unsafe impl Sync for Smoothing { }

impl Default for Smoothing {
    fn default() -> Self {
        Self {
            algs: vec![
                Box::new(None { }),
                Box::new(self::plain::Plain::default()),
                // Box::new(self::velocity_dampened_v1::VelocityDampened::default()),
                Box::new(self::velocity_dampened::VelocityDampened::default()),
                Box::new(self::velocity_dampened_axis::VelocityDampenedAxis::default()),
                Box::new(self::horizon::HorizonLock::default()),
                Box::new(self::fixed::Fixed::default())
            ],
            quats_checksum: 0,
            current_id: 1
        }
    }
}

impl Smoothing {
    pub fn set_current(&mut self, id: usize) {
        self.current_id = id.min(self.algs.len() - 1);
    }

    pub fn current(&mut self) -> &mut Box<dyn SmoothingAlgorithm> {
        &mut self.algs[self.current_id]
    }

    pub fn get_state_checksum(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        hasher.write_u64(self.quats_checksum);
        hasher.write_usize(self.current_id);
        hasher.write_u64(self.algs[self.current_id].get_checksum());
        hasher.finish()
    }

    pub fn update_quats_checksum(&mut self, quats: &TimeQuat) {
        let mut hasher = DefaultHasher::new();
        for (&k, v) in quats {
            hasher.write_i64(k);
            let vec = v.quaternion().as_vector();
            hasher.write_u64(vec[0].to_bits());
            hasher.write_u64(vec[1].to_bits());
            hasher.write_u64(vec[2].to_bits());
            hasher.write_u64(vec[3].to_bits());
        }
        self.quats_checksum = hasher.finish();
    }

    pub fn get_names(&self) -> Vec<String> {
        self.algs.iter().map(|x| x.get_name()).collect()
    }

    pub fn get_max_angles(quats: &TimeQuat, smoothed_quats: &TimeQuat, params: &crate::BasicParams) -> (f64, f64, f64) { // -> (pitch, yaw, roll) in deg
        let start_ts = (params.trim_start * params.get_scaled_duration_ms() * 1000.0) as i64;
        let end_ts   = (params.trim_end   * params.get_scaled_duration_ms() * 1000.0) as i64;
        let identity_quat = crate::Quat64::identity();

        let mut max_pitch = 0.0;
        let mut max_yaw = 0.0;
        let mut max_roll = 0.0;
        
        for (timestamp, quat) in smoothed_quats.iter() {
            if timestamp >= &start_ts && timestamp <= &end_ts {
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
