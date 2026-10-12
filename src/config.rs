use crate::stabilization_data::StabilizationMode;
use crate::optical_flow::OpticalFlowParams; // Import OpticalFlowParams
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub stabilization_mode: StabilizationMode,
    pub output_width: u32,
    pub output_height: u32,
    pub crop_factor: f32,
    pub enable_rolling_shutter_correction: bool,
    pub optical_flow_params: OpticalFlowParams, // Add optical flow parameters
    // Other configuration parameters...
}

impl Default for Config {
    fn default() -> Self {
        Self {
            stabilization_mode: StabilizationMode::GyroOnly,
            output_width: 1920,
            output_height: 1080,
            crop_factor: 1.0,
            enable_rolling_shutter_correction: true,
            optical_flow_params: OpticalFlowParams::default(), // Default optical flow params
            // Default values for other parameters...
        }
    }
}
