// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026

use qmetaobject::QString;

use super::Controller;

impl Controller {
    pub(super) fn smooth_quaternion_range(&self, start_ms: f64, end_ms: f64, strength: i32) -> i32 {
        let count =
            self.stabilizer
                .smooth_quaternion_range(start_ms, end_ms, strength.clamp(1, 3) as u8);
        if count > 0 {
            self.request_recompute();
            self.chart_data_changed();
        }
        count as i32
    }

    pub(super) fn undo_quaternion_edit(&self) -> bool {
        let changed = self.stabilizer.undo_quaternion_edit();
        if changed {
            self.request_recompute();
            self.chart_data_changed();
        }
        changed
    }

    pub(super) fn redo_quaternion_edit(&self) -> bool {
        let changed = self.stabilizer.redo_quaternion_edit();
        if changed {
            self.request_recompute();
            self.chart_data_changed();
        }
        changed
    }

    pub(super) fn clear_quaternion_edits(&self) -> bool {
        let changed = self.stabilizer.clear_quaternion_edits();
        if changed {
            self.request_recompute();
            self.chart_data_changed();
        }
        changed
    }

    pub(super) fn save_fixed_quaternion_video(&self) -> QString {
        match self.stabilizer.save_fixed_quaternion_video() {
            Ok(output_url) => QString::from(output_url),
            Err(error) => {
                log::error!("Failed to save DJI quaternion repair: {error}");
                QString::default()
            }
        }
    }
}
