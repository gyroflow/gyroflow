use super::*;
use std::collections::BTreeMap;

pub struct ZoomDisabled {
    compute_params: ComputeParams,
}

impl ZoomingAlgorithm for ZoomDisabled {
    fn compute(&self, _timestamps: &[f64]) -> Vec<(f64, Point2D)> {
        Vec::new()
    }
    fn get_debug_points(&self) -> BTreeMap<i64, Vec<(f64, f64)>> { Default::default() }

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
