use super::*;
use super::field_of_view::FieldOfView;

#[derive(Clone)]
pub struct ZoomDisabled {
    compute_params: ComputeParams,
}

impl ZoomingAlgorithm for ZoomDisabled {
    fn compute(&self, timestamps: &[f64]) -> Vec<(f64, Point2D)> {
        let fov_est = FieldOfView::new(self.compute_params.clone());
        let (fov_values, center_position) = fov_est.compute(timestamps, (self.compute_params.trim_start, self.compute_params.trim_end));

        fov_values.iter().copied().zip(center_position.iter().copied()).collect()
    }

    fn compute_params(&self) -> &ComputeParams {
        &self.compute_params
    }

    fn hash(&self, hasher: &mut dyn Hasher) {
        // this is for mode, 0 = disabled
        // TODO: this should be handled in a call to this, once zooming::Mode is in the compute struct
        hasher.write_u64(0);
    }  
}

impl ZoomDisabled {
    pub fn new(compute_params: ComputeParams) -> Self {
        Self {
            compute_params
        }
    }
}
