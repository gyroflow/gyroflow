use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration error: {0}")]
    ConfigError(String),
    #[error("Camera error: {0}")]
    CameraError(String),
    #[error("IMU data error: {0}")]
    ImuDataError(String),
    #[error("Lens profile error: {0}")]
    LensProfileError(String),
    #[error("Video processing error: {0}")]
    VideoProcessingError(String),
    #[error("Missing IMU data for stabilization")]
    MissingImuData,
    #[error("IMU data not found for timestamp: {0}")]
    ImuDataNotFoundForTimestamp(f64),
    #[error("Not implemented: {0}")]
    NotImplemented(String),
    #[error("OpenCV error: {0}")]
    OpenCvError(String),
    #[error("Optical flow not initialized")]
    OpticalFlowNotInitialized,
    #[error("Missing image data for optical flow calculation")]
    MissingImageDataForOpticalFlow,
    // Add other error types as needed
}
