use gyroflow_core::{
    camera::Camera,
    config::Config,
    error::Error,
    frame::Frame,
    optical_flow::{OpticalFlowParams, OpticalFlowResult},
    stabilization_data::StabilizationMode,
    stabilizer::Stabilizer,
    transform::Transform,
};
use nalgebra::{Matrix4, Vector3};
use std::sync::Arc;

// Helper function to create a dummy image (BGR, 3 channels)
fn create_dummy_image(width: u32, height: u32, color_offset: u8) -> Arc<Vec<u8>> {
    let size = (width * height * 3) as usize;
    let mut data = Vec::with_capacity(size);
    for i in 0..size {
        data.push(((i % 255) + color_offset) as u8);
    }
    Arc::new(data)
}

#[test]
fn test_stabilize_optical_flow_only_first_frame() -> Result<(), Error> {
    let config = Arc::new(Config {
        stabilization_mode: StabilizationMode::OpticalFlowOnly,
        output_width: 640,
        output_height: 480,
        optical_flow_params: OpticalFlowParams::default(),
        ..Default::default()
    });
    let camera = Arc::new(Camera {
        width: 640,
        height: 480,
        ..Default::default()
    });

    let mut stabilizer = Stabilizer::new(config.clone(), camera.clone(), None)?;

    let frame1_image = create_dummy_image(camera.width, camera.height, 0);
    let frame1 = Frame {
        index: 0,
        timestamp: 0.0,
        image_data: Some(frame1_image),
        ..Default::default()
    };

    let stabilization_data = stabilizer.stabilize_frame(&frame1, None)?;

    assert_eq!(stabilization_data.frame_index, 0);
    assert_eq!(stabilization_data.timestamp, 0.0);
    assert_eq!(
        stabilization_data.stabilization_mode,
        StabilizationMode::OpticalFlowOnly
    );
    // First frame, no previous frame, so transforms should be identity
    assert_eq!(stabilization_data.original_transform, Transform::identity());
    assert_eq!(stabilization_data.stabilized_transform, Transform::identity());
    assert!(stabilization_data.optical_flow_result.is_none());

    Ok(())
}

#[test]
fn test_stabilize_optical_flow_only_subsequent_frames() -> Result<(), Error> {
    let config = Arc::new(Config {
        stabilization_mode: StabilizationMode::OpticalFlowOnly,
        output_width: 640,
        output_height: 480,
        optical_flow_params: OpticalFlowParams {
            max_features: 100,
            quality_level: 0.01,
            min_distance: 10.0,
            ..Default::default()
        },
        ..Default::default()
    });
    let camera = Arc::new(Camera {
        width: 640,
        height: 480,
        ..Default::default()
    });

    let mut stabilizer = Stabilizer::new(config.clone(), camera.clone(), None)?;

    // Frame 1 (initial)
    let frame1_image = create_dummy_image(camera.width, camera.height, 0);
    let frame1 = Frame {
        index: 0,
        timestamp: 0.0,
        image_data: Some(frame1_image),
        ..Default::default()
    };
    let _ = stabilizer.stabilize_frame(&frame1, None)?;

    // Frame 2 (no motion, should result in identity flow)
    let frame2_image = create_dummy_image(camera.width, camera.height, 0); // Same as frame1
    let frame2 = Frame {
        index: 1,
        timestamp: 1.0,
        image_data: Some(frame2_image),
        ..Default::default()
    };
    let stabilization_data_2 = stabilizer.stabilize_frame(&frame2, None)?;

    assert_eq!(stabilization_data_2.frame_index, 1);
    assert!(stabilization_data_2.optical_flow_result.is_some());
    let flow_result_2 = stabilization_data_2.optical_flow_result.unwrap();
    // Expect very small or zero flow for identical images
    assert!(flow_result_2.average_flow_x.abs() < 0.1, "Expected average_flow_x to be near zero, got {}", flow_result_2.average_flow_x);
    assert!(flow_result_2.average_flow_y.abs() < 0.1, "Expected average_flow_y to be near zero, got {}", flow_result_2.average_flow_y);
    assert_eq!(stabilization_data_2.original_transform, Transform::identity());
    assert_eq!(stabilization_data_2.stabilized_transform, Transform::identity());


    // Frame 3 (simulated motion - different image data)
    let frame3_image = create_dummy_image(camera.width, camera.height, 10); // Slightly different image
    let frame3 = Frame {
        index: 2,
        timestamp: 2.0,
        image_data: Some(frame3_image),
        ..Default::default()
    };
    let stabilization_data_3 = stabilizer.stabilize_frame(&frame3, None)?;

    assert_eq!(stabilization_data_3.frame_index, 2);
    assert!(stabilization_data_3.optical_flow_result.is_some());
    let flow_result_3 = stabilization_data_3.optical_flow_result.unwrap();

    // With dummy data, the flow might not be perfectly predictable,
    // but it should not be zero if images are different.
    // The `flow_to_transform` is a placeholder, so we mainly check if it ran.
    assert!(flow_result_3.features.len() > 0, "Expected features to be tracked");
    assert!(flow_result_3.average_flow_x.abs() > 0.0, "Expected some flow for different images");
    assert!(flow_result_3.average_flow_y.abs() > 0.0, "Expected some flow for different images");

    // The original and stabilized transforms should now be non-identity
    assert_ne!(stabilization_data_3.original_transform, Transform::identity());
    assert_ne!(stabilization_data_3.stabilized_transform, Transform::identity());
    // Stabilized should be inverse of original in this simple implementation
    assert_eq!(
        stabilization_data_3.stabilized_transform,
        stabilization_data_3.original_transform.inverse()
    );

    Ok(())
}

#[test]
fn test_stabilize_optical_flow_missing_image_data() -> Result<(), Error> {
    let config = Arc::new(Config {
        stabilization_mode: StabilizationMode::OpticalFlowOnly,
        output_width: 640,
        output_height: 480,
        optical_flow_params: OpticalFlowParams::default(),
        ..Default::default()
    });
    let camera = Arc::new(Camera {
        width: 640,
        height: 480,
        ..Default::default()
    });

    let mut stabilizer = Stabilizer::new(config.clone(), camera.clone(), None)?;

    // First frame with image data
    let frame1_image = create_dummy_image(camera.width, camera.height, 0);
    let frame1 = Frame {
        index: 0,
        timestamp: 0.0,
        image_data: Some(frame1_image),
        ..Default::default()
    };
    let _ = stabilizer.stabilize_frame(&frame1, None)?;

    // Second frame without image data
    let frame2 = Frame {
        index: 1,
        timestamp: 1.0,
        image_data: None, // Missing image data
        ..Default::default()
    };

    let result = stabilizer.stabilize_frame(&frame2, None);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), Error::MissingImageDataForOpticalFlow));

    Ok(())
}

#[test]
fn test_stabilize_optical_flow_with_gyro_mode_selected() -> Result<(), Error> {
    let config = Arc::new(Config {
        stabilization_mode: StabilizationMode::GyroOnly, // Not OpticalFlowOnly
        output_width: 640,
        output_height: 480,
        optical_flow_params: OpticalFlowParams::default(),
        ..Default::default()
    });
    let camera = Arc::new(Camera {
        width: 640,
        height: 480,
        ..Default::default()
    });

    // OpticalFlow should not be initialized if mode is not OpticalFlowOnly
    let stabilizer = Stabilizer::new(config.clone(), camera.clone(), None)?;
    assert!(stabilizer.optical_flow.is_none());

    // Attempting to stabilize with GyroOnly mode and no IMU data should fail differently
    let frame1_image = create_dummy_image(camera.width, camera.height, 0);
    let frame1 = Frame {
        index: 0,
        timestamp: 0.0,
        image_data: Some(frame1_image),
        ..Default::default()
    };
    let result = stabilizer.stabilize_frame(&frame1, None);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), Error::MissingImuData));

    Ok(())
}
