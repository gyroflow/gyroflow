pub mod adaptive;

use dyn_clone::{ clone_trait_object, DynClone };
use enterpolation::Merge;
use crate::undistortion::{ ComputeParams };


#[derive(PartialEq, Clone)]
pub enum Mode {
    Disabled,
    DynamicZoom(f64), // f64 - smoothing focus window in seconds
    StaticZoom
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Point2D(f64, f64);
impl Merge<f64> for Point2D {
    fn merge(self, other: Self, factor: f64) -> Self {
        Point2D(
            self.0 * (1.0 - factor) + other.0 * factor,
            self.1 * (1.0 - factor) + other.1 * factor
        )
    }
}


pub trait ZoomingAlgorithm : DynClone {
    fn get_state_checksum(&self) -> u64;
    fn compute(&self, timestamps: &[f64]) -> Vec<(f64, Point2D)>;
}
clone_trait_object!(ZoomingAlgorithm);


pub fn from_compute_params(mut compute_params: ComputeParams) -> Box<dyn ZoomingAlgorithm> {
    compute_params.fov_scale = 1.0;
    compute_params.fovs.clear();
    
    // Use original video dimensions, because this is used to undistort points, and we need to find original image bounding box
    // Then we can use real `output_dim` to fit the fov
    compute_params.width = compute_params.video_width;
    compute_params.height = compute_params.video_height;
    compute_params.output_width = compute_params.video_width;
    compute_params.output_height = compute_params.video_height;
    

    let mode = if compute_params.adaptive_zoom_window < -0.9 {
        Mode::StaticZoom
    } else if compute_params.adaptive_zoom_window > 0.0001 {
        Mode::DynamicZoom(compute_params.adaptive_zoom_window)
    } else {
        Mode::Disabled
    };

    Box::new(adaptive::AdaptiveZoom::new(compute_params, mode))
}
