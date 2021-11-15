pub mod plain;
pub mod fixed;
use super::gyro_source::TimeQuat;
pub use std::collections::HashMap;

pub trait SmoothingAlgorithm {
    fn get_name(&self) -> String;
    
    fn get_parameters_json(&self) -> simd_json::owned::Value;
    fn set_parameter(&mut self, name: &str, val: f64);

    fn smooth(&self, quats: &TimeQuat, duration: f64) -> TimeQuat;
}

pub struct None { }
impl SmoothingAlgorithm for None {
    fn get_name(&self) -> String { "No smoothing".to_owned() }

    fn get_parameters_json(&self) -> simd_json::owned::Value { simd_json::json!([]) }
    fn set_parameter(&mut self, _name: &str, _val: f64) { }

    fn smooth(&self, quats: &TimeQuat, _duration: f64) -> TimeQuat { quats.clone() }
}

pub fn get_smoothing_algorithms() -> Vec<Box<dyn SmoothingAlgorithm>> {
    vec![
        Box::new(None { }),
        Box::new(self::plain::Plain::default()),
        Box::new(self::fixed::Fixed::default())
    ]
}

// "Yaw pitch roll smoothing", 
// "Horizon lock", 
// "Smooth angle limit (Aphobious)"
