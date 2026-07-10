// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Gyroflow contributors

use crate::StabilizationManager;

impl StabilizationManager {
    /// Replace the gyro rotation source with optical-flow-derived quaternions
    /// and re-run the full smoothing → zooming → undistortion pipeline.
    ///
    /// Call this after `pose_estimator.recalculate_gyro_data(fps, true)` so
    /// that `estimated_quats` is populated.  To revert to IMU gyro, reload the
    /// gyro data and call `recompute_gyro()`.
    pub fn use_optical_flow_for_stabilization(&self) {
        let of_quats = self.pose_estimator.estimated_quats.read().clone();
        if of_quats.is_empty() {
            log::warn!("use_optical_flow_for_stabilization: no optical flow quaternions available");
            return;
        }
        self.gyro.write().quaternions = of_quats;
        self.invalidate_smoothing();
    }
}
