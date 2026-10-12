use crate::camera::Camera;
use crate::config::Config;
use crate::error::Error;
use crate::frame::Frame;
use crate::imu::ImuData;
use crate::lens_profile::LensProfile;
use crate::optical_flow::{OpticalFlow, OpticalFlowResult};
use crate::stabilization_data::{StabilizationData, StabilizationMode};
use crate::transform::Transform;
use crate::util::interpolate_imu_data;
use nalgebra::{Matrix4, Vector3};
use std::sync::Arc;

pub struct Stabilizer {
    config: Arc<Config>,
    camera: Arc<Camera>,
    lens_profile: Option<Arc<LensProfile>>,
    optical_flow: Option<OpticalFlow>,
    prev_frame_data: Option<FrameData>,
}

struct FrameData {
    frame_index: usize,
    timestamp: f64,
    transform: Transform,
    stabilized_transform: Transform,
    image_data: Option<Arc<Vec<u8>>>, // Store image data for optical flow
}

impl Stabilizer {
    pub fn new(
        config: Arc<Config>,
        camera: Arc<Camera>,
        lens_profile: Option<Arc<LensProfile>>,
    ) -> Result<Self, Error> {
        let optical_flow = if config.stabilization_mode == StabilizationMode::OpticalFlowOnly {
            Some(OpticalFlow::new(
                camera.width as i32,
                camera.height as i32,
                config.optical_flow_params.clone(),
            )?)
        } else {
            None
        };

        Ok(Stabilizer {
            config,
            camera,
            lens_profile,
            optical_flow,
            prev_frame_data: None,
        })
    }

    /// Processes a single frame to calculate its stabilized transform.
    ///
    /// This function handles different stabilization modes:
    /// - `GyroOnly`: Uses IMU data to calculate stabilization.
    /// - `OpticalFlowOnly`: Uses optical flow between frames to calculate stabilization.
    /// - `Hybrid`: (Future) Combines gyro and optical flow.
    /// - `None`: No stabilization, returns identity transform.
    pub fn stabilize_frame(
        &mut self,
        frame: &Frame,
        imu_data: Option<&[ImuData]>,
    ) -> Result<StabilizationData, Error> {
        let mut stabilization_data = StabilizationData {
            frame_index: frame.index,
            timestamp: frame.timestamp,
            original_transform: Transform::identity(),
            stabilized_transform: Transform::identity(),
            stabilization_mode: self.config.stabilization_mode,
            optical_flow_result: None,
        };

        match self.config.stabilization_mode {
            StabilizationMode::None => {
                // No stabilization, original and stabilized transforms are identity
            }
            StabilizationMode::GyroOnly => {
                if let Some(imu_data) = imu_data {
                    self.stabilize_gyro_only(frame, imu_data, &mut stabilization_data)?;
                } else {
                    return Err(Error::MissingImuData);
                }
            }
            StabilizationMode::OpticalFlowOnly => {
                self.stabilize_optical_flow_only(frame, &mut stabilization_data)?;
            }
            StabilizationMode::Hybrid => {
                // TODO: Implement hybrid stabilization
                return Err(Error::NotImplemented("Hybrid stabilization".to_string()));
            }
        }

        self.prev_frame_data = Some(FrameData {
            frame_index: frame.index,
            timestamp: frame.timestamp,
            transform: stabilization_data.original_transform, // Store original for next frame's optical flow
            stabilized_transform: stabilization_data.stabilized_transform,
            image_data: frame.image_data.clone(),
        });

        Ok(stabilization_data)
    }

    fn stabilize_gyro_only(
        &self,
        frame: &Frame,
        imu_data: &[ImuData],
        stabilization_data: &mut StabilizationData,
    ) -> Result<(), Error> {
        // Interpolate IMU data for the current frame's timestamp
        let interpolated_imu = interpolate_imu_data(imu_data, frame.timestamp)
            .ok_or(Error::ImuDataNotFoundForTimestamp(frame.timestamp))?;

        // Calculate the original camera orientation based on IMU data
        // This is a simplified example; a real implementation would integrate gyro data
        // over time and apply camera-specific rotations.
        let original_orientation = Transform::from_rotation_euler_angles(
            interpolated_imu.gyro.x,
            interpolated_imu.gyro.y,
            interpolated_imu.gyro.z,
        );
        stabilization_data.original_transform = original_orientation;

        // For gyro-only, the stabilized transform aims to counteract the original motion.
        // This often involves smoothing the original motion over a window.
        // For simplicity, let's assume we want to keep the camera "level" or at a reference.
        // A more advanced approach would involve a PID controller or similar.
        let stabilized_orientation = Transform::identity(); // Target level orientation
        stabilization_data.stabilized_transform = stabilized_orientation;

        Ok(())
    }

    fn stabilize_optical_flow_only(
        &mut self,
        frame: &Frame,
        stabilization_data: &mut StabilizationData,
    ) -> Result<(), Error> {
        let optical_flow = self
            .optical_flow
            .as_mut()
            .ok_or(Error::OpticalFlowNotInitialized)?;

        let current_image = frame
            .image_data
            .as_ref()
            .ok_or(Error::MissingImageDataForOpticalFlow)?;

        if let Some(prev_frame_data) = &self.prev_frame_data {
            let prev_image = prev_frame_data
                .image_data
                .as_ref()
                .ok_or(Error::MissingImageDataForOpticalFlow)?;

            // Perform optical flow calculation between previous and current frame
            let flow_result = optical_flow.calculate_flow(prev_image, current_image)?;
            stabilization_data.optical_flow_result = Some(flow_result.clone());

            // Convert optical flow result (e.g., average translation/rotation) into a transform
            // This is a simplified example. A real implementation would use the flow
            // to estimate camera motion and then smooth it.
            let estimated_motion = self.flow_to_transform(&flow_result);

            // The original transform is the estimated motion from the previous frame
            stabilization_data.original_transform = estimated_motion;

            // To stabilize, we want to counteract this motion.
            // A simple approach is to apply the inverse of the estimated motion.
            // More advanced methods would smooth the motion over several frames.
            let stabilized_transform = estimated_motion.inverse(); // Simple counter-motion
            stabilization_data.stabilized_transform = stabilized_transform;
        } else {
            // First frame, no previous frame to compare with.
            // Assume no motion or identity transform.
            stabilization_data.original_transform = Transform::identity();
            stabilization_data.stabilized_transform = Transform::identity();
        }

        Ok(())
    }

    /// Converts an OpticalFlowResult into a Transform representing camera motion.
    /// This is a placeholder and needs a robust implementation.
    ///
    /// A proper implementation would:
    /// 1. Filter good features.
    /// 2. Use a robust estimator (e.g., RANSAC) to estimate a homography or essential matrix.
    /// 3. Decompose the matrix to get rotation and translation.
    /// 4. Account for lens distortion (if lens_profile is available).
    fn flow_to_transform(&self, flow_result: &OpticalFlowResult) -> Transform {
        if flow_result.features.is_empty() {
            return Transform::identity();
        }

        // Simple average translation for demonstration
        let mut avg_dx = 0.0;
        let mut avg_dy = 0.0;
        for feature in &flow_result.features {
            avg_dx += feature.current_point.x - feature.previous_point.x;
            avg_dy += feature.current_point.y - feature.previous_point.y;
        }
        let num_features = flow_result.features.len() as f64;
        avg_dx /= num_features;
        avg_dy /= num_features;

        // Convert pixel translation to a conceptual camera translation/rotation.
        // This is highly simplified and needs proper camera intrinsics and 3D reconstruction.
        // For now, let's map pixel shift to a small rotation.
        // A large pixel shift might correspond to a small angular change.
        let rotation_scale = 0.001; // Arbitrary scale factor
        let rot_y = -avg_dx * rotation_scale; // X pixel shift -> Y-axis rotation (yaw)
        let rot_x = -avg_dy * rotation_scale; // Y pixel shift -> X-axis rotation (pitch)

        // Create a transform from these estimated rotations
        Transform::from_rotation_euler_angles(rot_x, rot_y, 0.0)
    }
}
