use super::*;
use super::field_of_view::FieldOfView;

#[derive(Clone)]
pub struct ZoomStatic {
    compute_params: ComputeParams
}

impl ZoomingAlgorithm for ZoomStatic {
    fn compute(&self, timestamps: &[f64]) -> Vec<(f64, Point2D)> {
        let fov_est = FieldOfView::new(self.compute_params.clone());
        let (mut fov_values, center_position) = fov_est.compute(timestamps, (self.compute_params.trim_start, self.compute_params.trim_end));

        if let Some(max_f) = fov_values.iter().copied().reduce(f64::min) {
            fov_values.iter_mut().for_each(|v| *v = max_f);
        } else {
            log::warn!("Unable to find min of fov_values, len: {}", fov_values.len());
        }

        fov_values.iter().copied().zip(center_position.iter().copied()).collect()
    }

    fn compute_params(&self) -> &ComputeParams {
        &self.compute_params
    }

    fn hash(&self, hasher: &mut dyn Hasher) {
        // this is for mode, 1 = static
        // TODO: this should be handled in a call to this, once zooming::Mode is in the compute struct
        hasher.write_u64(1);
    }
}

impl ZoomStatic {
    pub fn new(compute_params: ComputeParams) -> Self {
        Self {
            compute_params
        }
    }
}