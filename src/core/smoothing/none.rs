// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Elvin Chen

use super::*;
use crate::gyro_source::TimeQuat;

#[derive(Default, Clone)]
pub struct None;

impl SmoothingAlgorithm for None {
    fn get_name(&self) -> String { "No smoothing".to_owned() }

    fn get_parameters_json(&self) -> serde_json::Value { serde_json::json!([]) }
    fn get_status_json(&self) -> serde_json::Value { serde_json::json!([]) }
    fn set_parameter(&mut self, _name: &str, _val: f64) { }
    fn get_parameter(&self, _name: &str) -> f64 { 0.0 }

    fn get_checksum(&self) -> u64 { 0 }
    fn smooth(&self, quats: &TimeQuat, _duration: f64, _: &StabilizationParams, _: &KeyframeManager) -> TimeQuat { quats.clone() }
}
