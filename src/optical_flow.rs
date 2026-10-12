use crate::error::Error;
use opencv::{
    core::{Mat, MatTraitConst, Point2f, Size, CV_8U},
    imgproc::{cvt_color, COLOR_BGR2GRAY},
    prelude::*,
    video::{calc_optical_flow_pyr_lk, PyrLkParams},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpticalFlowParams {
    pub max_features: usize,
    pub quality_level: f64,
    pub min_distance: f64,
    pub block_size: i32,
    pub use_harris_detector: bool,
    pub k: f64,
    pub pyr_lk_max_level: i32,
    pub pyr_lk_win_size: Size,
    pub pyr_lk_max_iterations: i32,
    pub pyr_lk_epsilon: f64,
}

impl Default for OpticalFlowParams {
    fn default() -> Self {
        Self {
            max_features: 500,
            quality_level: 0.01,
            min_distance: 10.0,
            block_size: 3,
            use_harris_detector: false,
            k: 0.04,
            pyr_lk_max_level: 3,
            pyr_lk_win_size: Size::new(21, 21),
            pyr_lk_max_iterations: 30,
            pyr_lk_epsilon: 0.01,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpticalFlowFeature {
    pub previous_point: Point2f,
    pub current_point: Point2f,
    pub status: u8, // 1 if found, 0 if not
    pub error: f32,
}

#[derive(Debug, Clone)]
pub struct OpticalFlowResult {
    pub features: Vec<OpticalFlowFeature>,
    pub average_flow_x: f32,
    pub average_flow_y: f32,
}

pub struct OpticalFlow {
    width: i32,
    height: i32,
    params: OpticalFlowParams,
    pyr_lk_params: PyrLkParams,
}

impl OpticalFlow {
    pub fn new(width: i32, height: i32, params: OpticalFlowParams) -> Result<Self, Error> {
        let pyr_lk_params = PyrLkParams {
            max_level: params.pyr_lk_max_level,
            win_size: params.pyr_lk_win_size,
            max_iterations: params.pyr_lk_max_iterations,
            epsilon: params.pyr_lk_epsilon,
        };

        Ok(Self {
            width,
            height,
            params,
            pyr_lk_params,
        })
    }

    /// Calculates optical flow between two frames.
    /// `prev_image_data` and `current_image_data` are expected to be raw image bytes (e.g., BGR).
    pub fn calculate_flow(
        &mut self,
        prev_image_data: &Arc<Vec<u8>>,
        current_image_data: &Arc<Vec<u8>>,
    ) -> Result<OpticalFlowResult, Error> {
        let prev_mat = Mat::from_slice(prev_image_data)
            .map_err(|e| Error::OpenCvError(format!("Failed to create Mat from prev_image: {}", e)))?
            .reshape(3, self.height)
            .map_err(|e| Error::OpenCvError(format!("Failed to reshape prev_image: {}", e)))?;

        let current_mat = Mat::from_slice(current_image_data)
            .map_err(|e| Error::OpenCvError(format!("Failed to create Mat from current_image: {}", e)))?
            .reshape(3, self.height)
            .map_err(|e| Error::OpenCvError(format!("Failed to reshape current_image: {}", e)))?;

        let mut prev_gray = Mat::default();
        let mut current_gray = Mat::default();

        cvt_color(&prev_mat, &mut prev_gray, COLOR_BGR2GRAY, 0)
            .map_err(|e| Error::OpenCvError(format!("Failed to convert prev_image to gray: {}", e)))?;
        cvt_color(&current_mat, &mut current_gray, COLOR_BGR2GRAY, 0)
            .map_err(|e| Error::OpenCvError(format!("Failed to convert current_image to gray: {}", e)))?;

        let mut prev_points = opencv::core::VectorOfPoint2f::new();
        opencv::imgproc::good_features_to_track(
            &prev_gray,
            &mut prev_points,
            self.params.max_features as i32,
            self.params.quality_level,
            self.params.min_distance,
            &opencv::core::no_array(), // mask
            self.params.block_size,
            self.params.use_harris_detector,
            self.params.k,
        )
        .map_err(|e| Error::OpenCvError(format!("Failed to find good features to track: {}", e)))?;

        if prev_points.is_empty() {
            return Ok(OpticalFlowResult {
                features: Vec::new(),
                average_flow_x: 0.0,
                average_flow_y: 0.0,
            });
        }

        let mut current_points = opencv::core::VectorOfPoint2f::new();
        let mut status = opencv::core::VectorOfu8::new();
        let mut err = opencv::core::VectorOff32::new();

        calc_optical_flow_pyr_lk(
            &prev_gray,
            &current_gray,
            &prev_points,
            &mut current_points,
            &mut status,
            &mut err,
            self.pyr_lk_params.win_size,
            self.pyr_lk_params.max_level,
            self.pyr_lk_params.criteria(),
            0, // flags
            0.0, // min_eigen_threshold
        )
        .map_err(|e| Error::OpenCvError(format!("Failed to calculate optical flow: {}", e)))?;

        let mut features = Vec::new();
        let mut total_flow_x = 0.0;
        let mut total_flow_y = 0.0;
        let mut valid_features_count = 0;

        for i in 0..prev_points.len() {
            let s = *status.get(i).unwrap_or(&0);
            let e = *err.get(i).unwrap_or(&0.0);
            let prev_p = *prev_points.get(i).unwrap();
            let curr_p = *current_points.get(i).unwrap();

            features.push(OpticalFlowFeature {
                previous_point: prev_p,
                current_point: curr_p,
                status: s,
                error: e,
            });

            if s == 1 {
                total_flow_x += curr_p.x - prev_p.x;
                total_flow_y += curr_p.y - prev_p.y;
                valid_features_count += 1;
            }
        }

        let average_flow_x = if valid_features_count > 0 {
            total_flow_x / valid_features_count as f32
        } else {
            0.0
        };
        let average_flow_y = if valid_features_count > 0 {
            total_flow_y / valid_features_count as f32
        } else {
            0.0
        };

        Ok(OpticalFlowResult {
            features,
            average_flow_x,
            average_flow_y,
        })
    }
}
