pub mod plain;
pub mod horizon;
pub mod fixed;
pub mod velocity_dampened;
pub mod velocity_dampened2;

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
                Box::new(self::velocity_dampened::VelocityDampened::default()),
                Box::new(self::velocity_dampened2::VelocityDampened2::default()),
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
}
