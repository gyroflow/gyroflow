use crate::transform::Transform;
use crate::optical_flow::OpticalFlowResult; // Import OpticalFlowResult
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StabilizationMode {
    None,
    GyroOnly,
    OpticalFlowOnly,
    Hybrid, // Future: Combine gyro and optical flow
}

#[derive(Debug, Clone)]
pub struct StabilizationData {
    pub frame_index: usize,
    pub timestamp: f64,
    pub original_transform: Transform,
    pub stabilized_transform: Transform,
    pub stabilization_mode: StabilizationMode,
    pub optical_flow_result: Option<OpticalFlowResult>, // Add optical flow result
}
