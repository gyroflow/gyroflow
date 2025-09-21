// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2025

use super::*;
use std::sync::Mutex;
use crate::synchronization::PoseQuality;

pub struct MotionDirection {
    pub time_constant: f64,
    pub min_inlier_ratio: f64,
    pub max_epi_err: f64,
    pub motion_window_ms: f64,
    pub smoothing_enabled: bool,
    status: Mutex<Vec<serde_json::Value>>,
}

impl Default for MotionDirection {
    fn default() -> Self {
        Self {
            time_constant: 0.25,
            min_inlier_ratio: 0.2,
            max_epi_err: 2.0,
            motion_window_ms: 50.0,
            smoothing_enabled: true,
            status: Mutex::new(Vec::new()),
        }
    }
}

impl Clone for MotionDirection {
    fn clone(&self) -> Self {
        let status = self.status.lock().unwrap().clone();
        Self {
            time_constant: self.time_constant,
            min_inlier_ratio: self.min_inlier_ratio,
            max_epi_err: self.max_epi_err,
            motion_window_ms: self.motion_window_ms,
            smoothing_enabled: self.smoothing_enabled,
            status: Mutex::new(status)
        }
    }
}

impl SmoothingAlgorithm for MotionDirection {
    fn get_name(&self) -> String { "Motion direction".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "time_constant" => self.time_constant = val,
            "min_inlier_ratio" => self.min_inlier_ratio = val,
            "max_epi_err" => self.max_epi_err = val,
            "motion_window_ms" => self.motion_window_ms = val,
            "smoothing_enabled" => self.smoothing_enabled = val >= 0.5,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameter(&self, name: &str) -> f64 {
        match name {
            "time_constant" => self.time_constant,
            "min_inlier_ratio" => self.min_inlier_ratio,
            "max_epi_err" => self.max_epi_err,
            "motion_window_ms" => self.motion_window_ms,
            "smoothing_enabled" => if self.smoothing_enabled { 1.0 } else { 0.0 },
            _ => 0.0
        }
    }

    fn get_parameters_json(&self) -> serde_json::Value {
        serde_json::json!([
        {
            "name": "smoothing_enabled",
            "description": "Enable smoothing (blend towards target)",
            "type": "CheckBox",
            "value": self.smoothing_enabled,
            "default": true,
            "custom_qml": "id: mdSmoothingEnabled"
        },
        {
            "name": "time_constant",
            "description": "Smoothness",
            "type": "SliderWithField",
            "from": 0.01,
            "to": 10.0,
            "value": self.time_constant,
            "default": 0.25,
            "unit": "s",
            "keyframe": "SmoothingParamTimeConstant",
            "custom_qml": "enabled: mdSmoothingEnabled.checked"
        },{
            "name": "min_inlier_ratio",
            "description": "Min inlier ratio",
            "type": "SliderWithField",
            "from": 0.0,
            "to": 1.0,
            "value": self.min_inlier_ratio,
            "default": 0.2,
            "precision": 3,
            "unit": ""
        },{
            "name": "max_epi_err",
            "description": "Max epipolar error",
            "type": "SliderWithField",
            "from": 0.0,
            "to": 5.0,
            "value": self.max_epi_err,
            "default": 2.0,
            "precision": 3,
            "unit": ""
        },{
            "name": "motion_window_ms",
            "description": "Motion sampling window",
            "type": "SliderWithField",
            "from": 5.0,
            "to": 500.0,
            "value": if self.motion_window_ms > 0.0 { self.motion_window_ms } else { 50.0 },
            "default": 50.0,
            "precision": 0,
            "unit": "ms"
        }])
    }
    fn get_status_json(&self) -> serde_json::Value { serde_json::Value::Array(self.status.lock().unwrap().clone()) }
    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.time_constant.to_bits());
        hasher.write_u64(self.min_inlier_ratio.to_bits());
        hasher.write_u64(self.max_epi_err.to_bits());
        hasher.write_u64(self.motion_window_ms.to_bits());
        hasher.write_u64(if self.smoothing_enabled { 1u64 } else { 0u64 });
        hasher.finish()
    }

    fn smooth(&self, quats: &TimeQuat, duration_ms: f64, compute_params: &ComputeParams) -> TimeQuat {
        println!("MotionDirection::smooth() called with {} quats, duration_ms: {}", quats.len(), duration_ms);
        
        if quats.is_empty() || duration_ms <= 0.0 { 
            println!("MotionDirection::smooth() early return - empty quats or invalid duration");
            return quats.clone(); 
        }

        let sample_rate: f64 = quats.len() as f64 / (duration_ms / 1000.0);
        let get_alpha = |time_constant: f64| 1.0 - (-(1.0 / sample_rate) / time_constant).exp();
        let alpha = if self.smoothing_enabled {
            if self.time_constant > 0.0 { get_alpha(self.time_constant) } else { 1.0 }
        } else { 1.0 };

        let mut out = TimeQuat::new();

        // Check if pose estimator has any motion data
        println!("MotionDirection::smooth() checking for motion data...");
        let has_motion_data = {
            // Use try_read to avoid blocking the UI thread
            if let Some(sync_results) = compute_params.pose_estimator.sync_results.try_read() {
                let count = sync_results.len();
                let has_motion = sync_results.values().any(|fr| fr.translation_dir_cam.is_some());
                println!("MotionDirection::smooth() got sync_results lock, {} frames, has_motion: {}", count, has_motion);
                has_motion
            } else {
                // If we can't get the lock immediately, assume no motion data to avoid blocking
                println!("MotionDirection::smooth() could not acquire sync_results lock, assuming no motion data");
                false
            }
        };
        
        if !has_motion_data {
            println!("MotionDirection::smooth() no motion data available, returning original quats");
            *self.status.lock().unwrap() = vec![serde_json::json!({
                "type": "Label",
                "text": "Motion direction: no motion samples available. Check pose estimation settings or re-run synchronization."
            })];
            return quats.clone();
        }

        println!("MotionDirection::smooth() processing {} quats with motion data", quats.len());

        // TODO: Pre-compute motion data mapping to avoid expensive lookups for every frame?
        //let window_us: i64 = ((if self.motion_window_ms > 0.0 { self.motion_window_ms } else { 50.0 }) * 1000.0) as i64;
        
        // Get all available motion data once and create a lookup map
        let motion_data_map: std::collections::BTreeMap<i64, ([f64; 3], PoseQuality)> = if let Some(sync_results) = compute_params.pose_estimator.sync_results.try_read() {
            sync_results.iter()
                .filter_map(|(ts, fr)| {
                    if let Some(tdir) = fr.translation_dir_cam {
                        let quality = fr.pose_quality.clone().unwrap_or_default();
                        Some((*ts, (tdir, quality)))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            println!("MotionDirection::smooth() could not acquire sync_results lock for pre-computation, falling back to original quats");
            return quats.clone();
        };
        
        println!("MotionDirection::smooth() pre-computed {} motion data points", motion_data_map.len());

        // Pre-compute gyro quaternions to avoid repeated lock acquisitions
        let gyro_quaternions: std::collections::BTreeMap<i64, nalgebra::UnitQuaternion<f64>> = if let Some(gyro) = compute_params.gyro.try_read() {
            gyro.quaternions.clone()
        } else {
            println!("MotionDirection::smooth() could not acquire gyro lock for pre-computation, falling back to original quats");
            return quats.clone();
        };
        println!("MotionDirection::smooth() pre-computed {} gyro quaternions", gyro_quaternions.len());

        // Debug counters to avoid silent fallbacks
        let mut frames_total: usize = 0;
        let mut targets_used: usize = 0;
        let mut skipped_no_sample: usize = 0;
        let mut skipped_no_gyro: usize = 0;
        let mut skipped_degenerate: usize = 0;

        // Iterate timestamps, blend towards motion direction look-at if quality OK, else keep gyro orientation
        for (ts, q) in quats.iter() {
            frames_total += 1;
            if frames_total % 5000 == 0 {
                println!("MotionDirection::smooth() processed {} frames so far", frames_total);
            }
            if frames_total % 10000 == 0 {
                println!("MotionDirection::smooth() starting frame {} processing", frames_total);
            }
            
            // Find averaged motion data within the window (match visualization semantics)
            let window_vis_us: i64 = 50_000 * 20; // 1s window, same as visualization
            let cam_dir_opt: Option<nalgebra::Vector3<f64>> = if let Some((tdir, _qual)) = compute_params.pose_estimator.get_translation_dir_cam_near(*ts, window_vis_us, true) {
                let vec = nalgebra::Vector3::new(tdir[0], tdir[1], tdir[2]);
                if vec.norm() > 1e-9 { Some(vec) } else { None }
            } else {
                skipped_no_sample += 1;
                None
            };

            if frames_total % 10000 == 0 {
                println!("MotionDirection::smooth() processing frame {} - cam_dir_opt: {}", frames_total, cam_dir_opt.is_some());
            }

            let target = if let Some(tvec_cam) = cam_dir_opt {
                // 1) Convert camera-frame translation direction into world frame using current gyro orientation
                let maybe_world_q = {
                    if frames_total % 10000 == 0 {
                        println!("MotionDirection::smooth() frame {} - searching pre-computed gyro quaternions for timestamp {}", frames_total, *ts);
                    }
                    // Use pre-computed gyro data to avoid lock acquisitions
                    let b = gyro_quaternions.range(..=ts).next_back();
                    if frames_total % 10000 == 0 {
                        println!("MotionDirection::smooth() frame {} - found previous gyro quaternion: {}", frames_total, b.is_some());
                    }
                    let a = gyro_quaternions.range(ts..).next();
                    if frames_total % 10000 == 0 {
                        println!("MotionDirection::smooth() frame {} - found next gyro quaternion: {}", frames_total, a.is_some());
                    }
                    match (b, a) {
                        (Some((tsb, qb)), Some((tsa, qa))) => if ts - *tsb <= *tsa - ts { Some((*qb).clone()) } else { Some((*qa).clone()) },
                        (Some((_, qb)), None) => Some((*qb).clone()),
                        (None, Some((_, qa))) => Some((*qa).clone()),
                        _ => None
                    }
                };
                if let Some(world_q) = maybe_world_q {
                    if frames_total % 10000 == 0 {
                        println!("MotionDirection::smooth() frame {} - transforming vector", frames_total);
                    }
                    let world_dir = world_q.transform_vector(&tvec_cam);
                    if world_dir.norm() > 1e-6 {
                        if frames_total % 10000 == 0 {
                            println!("MotionDirection::smooth() frame {} - building look-at rotation", frames_total);
                        }
                        // 2) Build a look-at rotation with world up ~ Y axis to minimize roll
                        let forward = world_dir.normalize();
                        let world_up = nalgebra::Vector3::<f64>::new(0.0, 1.0, 0.0);
                        let mut right = forward.cross(&world_up);
                        if right.norm() < 1e-9 { right = nalgebra::Vector3::new(1.0, 0.0, 0.0); }
                        right = right.normalize();
                        let up2 = right.cross(&forward);
                        let rot = nalgebra::Rotation3::from_matrix_unchecked(nalgebra::Matrix3::from_columns(&[right, up2, forward]));
                        Some(nalgebra::UnitQuaternion::from_rotation_matrix(&rot))
                    } else { skipped_degenerate += 1; None }
                } else { skipped_no_gyro += 1; None }
            } else { None };

            let new_q = if let Some(target_q) = target { 
                targets_used += 1;
                if frames_total % 10000 == 0 {
                    println!("MotionDirection::smooth() frame {} - performing slerp", frames_total);
                }
                q.slerp(&target_q, alpha) 
            } else { *q };
            if frames_total % 10000 == 0 {
                println!("MotionDirection::smooth() frame {} - inserting quaternion", frames_total);
            }
            out.insert(*ts, new_q);
        }

        if !self.smoothing_enabled {
            println!("MotionDirection::smooth() smoothing disabled, skipping reverse pass smoothing");
        } else {
            // Reverse pass smoothing (avoid mutable + immutable borrow by iterating over a snapshot)
            println!("MotionDirection::smooth() starting reverse pass smoothing with {} quaternions", out.len());
            let mut snapshot: Vec<(i64, nalgebra::UnitQuaternion<f64>)> = out.iter().map(|(ts, q)| (*ts, q.clone())).collect();
            println!("MotionDirection::smooth() created snapshot, sorting...");
            snapshot.sort_by_key(|(ts, _)| *ts);
            println!("MotionDirection::smooth() snapshot sorted, starting reverse pass...");
            if let Some((_, mut acc)) = snapshot.last().cloned() {
                let reverse_count = snapshot.len().saturating_sub(1);
                println!("MotionDirection::smooth() reverse pass processing {} quaternions", reverse_count);
                for (i, (ts, q)) in snapshot[..reverse_count].iter().rev().enumerate() {
                    if i % 10000 == 0 {
                        println!("MotionDirection::smooth() reverse pass processed {} / {}", i, reverse_count);
                    }
                    acc = q.slerp(&acc, alpha);
                    out.insert(*ts, acc.clone());
                }
            }
            println!("MotionDirection::smooth() reverse pass completed");
        }

        // Store status for UI
        let mut msgs: Vec<serde_json::Value> = Vec::new();
        if frames_total > 0 {
            let used_percent = (targets_used as f64 * 100.0 / frames_total as f64).round();
            msgs.push(serde_json::json!({
                "type": "Label",
                "text": format!("Motion direction: used {}% of frames ({} / {})", used_percent as i64, targets_used, frames_total)
            }));
            if targets_used == 0 {
                msgs.push(serde_json::json!({
                    "type": "Label",
                    "text": "No valid motion directions found within the window. Consider increasing the window or adjusting pose estimation method."
                }));
            }
            if skipped_no_sample > 0 {
                msgs.push(serde_json::json!({
                    "type": "Label",
                    "text": format!("No motion sample near timestamp for {} frames. Increase motion window.", skipped_no_sample)
                }));
            }
            if skipped_no_gyro > 0 {
                msgs.push(serde_json::json!({
                    "type": "Label",
                    "text": format!("No gyro orientation near timestamp for {} frames.", skipped_no_gyro)
                }));
            }
            if skipped_degenerate > 0 {
                msgs.push(serde_json::json!({
                    "type": "Label",
                    "text": format!("Degenerate motion direction for {} frames.", skipped_degenerate)
                }));
            }
        }
        *self.status.lock().unwrap() = msgs;

        out
    }
}


