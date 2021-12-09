pub mod plain;
pub mod horizon;
pub mod fixed;
use super::gyro_source::TimeQuat;
pub use std::collections::HashMap;
use dyn_clone::{clone_trait_object, DynClone};

pub trait SmoothingAlgorithm: DynClone {
    fn get_name(&self) -> String;
    
    fn get_parameters_json(&self) -> simd_json::owned::Value;
    fn set_parameter(&mut self, name: &str, val: f64);

    fn smooth(&self, quats: &TimeQuat, duration: f64) -> TimeQuat;
}

clone_trait_object!(SmoothingAlgorithm);

#[derive(Clone)]
pub struct None { }
impl SmoothingAlgorithm for None {
    fn get_name(&self) -> String { "No smoothing".to_owned() }

    fn get_parameters_json(&self) -> simd_json::owned::Value { simd_json::json!([]) }
    fn set_parameter(&mut self, _name: &str, _val: f64) { }

    fn smooth(&self, quats: &TimeQuat, _duration: f64) -> TimeQuat { quats.clone() }
}

pub struct Smoothing {
    algs: Vec<Box<dyn SmoothingAlgorithm>>,
    current_id: usize
}
unsafe impl Send for Smoothing { }
unsafe impl Sync for Smoothing { }

impl Default for Smoothing {
    fn default() -> Self {
        Self {
            algs: vec![
                Box::new(None { }),
                Box::new(self::plain::Plain::default()),
                Box::new(self::horizon::HorizonLock::default()),
                Box::new(self::fixed::Fixed::default())
            ],
            current_id: 1
        }
    }
}

impl Smoothing {
    pub fn set_current(&mut self, id: usize) {
        assert!(id < self.algs.len());
        self.current_id = id;
    }
    pub fn current(&mut self) -> &mut Box<dyn SmoothingAlgorithm> {
        &mut self.algs[self.current_id]
    }
    pub fn get_names(&self) -> Vec<String> {
        self.algs.iter().map(|x| x.get_name()).collect()
    }
}

// "Yaw pitch roll smoothing", 
// "Horizon lock", 
// "Smooth angle limit (Aphobious)"
