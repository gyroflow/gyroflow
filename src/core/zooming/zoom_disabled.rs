// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021 Marc Roeschlin, Adrian, Maik

use super::*;
use std::collections::BTreeMap;

pub struct ZoomDisabled {
    fov_estimator: Box<dyn FieldOfViewAlgorithm>,
    compute_params: ComputeParams,
}

impl ZoomingAlgorithm for ZoomDisabled {
    fn compute(&self, timestamps: &[f64], _keyframes: &KeyframeManager, _method: ZoomMethod) -> Vec<((f64, f64), Point2D)> {
        if timestamps.is_empty() {
            return Vec::new();
        }

        let (fov_values, center_position) = self.fov_estimator.compute(timestamps, (self.compute_params.trim_start, self.compute_params.trim_end));

        fov_values.into_iter().map(|x| (1.0, x)).zip(center_position.into_iter()).collect()
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
    pub fn new(fov_estimator: Box<dyn FieldOfViewAlgorithm>, compute_params: ComputeParams) -> Self {
        Self {
            fov_estimator,
            compute_params
        }
    }
}
