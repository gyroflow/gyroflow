// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use super::OpticalFlowPoints;
use crate::gyro_source::{Quat64, TimeQuat, GyroSource};
use crate::stabilization::{undistort_points_with_params, ComputeParams};
use nalgebra::{Matrix3, Vector3};
use rs_sync::SyncProblem;
use std::f64::consts::PI;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed, Ordering::SeqCst},
    Arc,
};

pub struct FindOffsetsRssync<'a> {
    sync : SyncProblem<'a>,
    gyro_source: &'a GyroSource,
    frame_readout_time : f64,
    sync_points : Vec::<(i64, i64)>,
    initial_offset: f64,
    search_size: f64,
    is_guess_orient: Arc<AtomicBool>,

    current_sync_point : Arc<AtomicUsize>,
    current_orientation : Arc<AtomicUsize>
}

impl FindOffsetsRssync<'_> {
    pub fn new<'a, F: Fn(f64) + Sync + 'a>(
        _ranges: &'a [(i64, i64)],
        matched_points: &Vec<Vec<((i64, OpticalFlowPoints), (i64, OpticalFlowPoints))>>,
        initial_offset: f64,
        search_size: f64,
        params: &'a ComputeParams,
        progress_cb: F,
        cancel_flag: Arc<AtomicBool>,
    ) -> FindOffsetsRssync<'a> {
        // Vec<(timestamp, offset, cost)>
        
        let mut frame_readout_time = params.frame_readout_time;
        if frame_readout_time == 0.0 {
            frame_readout_time = 1000.0 / params.scaled_fps / 2.0;
        }
        frame_readout_time /= 1000.0;
    
        let mut ret = FindOffsetsRssync {
            sync:SyncProblem::new(),
            gyro_source:&params.gyro,
            frame_readout_time:frame_readout_time,
            sync_points:Vec::new(),
            initial_offset:initial_offset,
            search_size:search_size,
            is_guess_orient:Arc::new(AtomicBool::new(false)),
            current_sync_point:Arc::new(AtomicUsize::new(0)),
            current_orientation:Arc::new(AtomicUsize::new(0))
        };
        
        
        {
            let num_sync_points = matched_points.len() as f64;
            let is_guess_orient = ret.is_guess_orient.clone();
            let cur_sync_point = ret.current_sync_point.clone();
            let cur_orientation = ret.current_orientation.clone();
            ret.sync.on_progress( move |progress| -> bool {
                let num_orientations  = if is_guess_orient.load(SeqCst) {48.0} else {1.0};
                progress_cb((cur_orientation.load(SeqCst) as f64 + ((cur_sync_point.load(SeqCst) as f64 + progress) / num_sync_points)) / num_orientations);
                !cancel_flag.load(Relaxed)
            });
        }
        
        for range in matched_points {
            if range.len() < 2 {
                log::warn!("Not enough data for sync! range.len: {}", range.len());
                continue;
            }
    
            let mut from_ts = -1;
            let mut to_ts = 0;
            for ((a_t, a_p), (b_t, b_p)) in range {
                if from_ts == -1 {
                    from_ts = *a_t;
                }
                to_ts = *b_t;
                let a = undistort_points_with_params(&a_p, Matrix3::identity(), None, None, params);
                let b = undistort_points_with_params(&b_p, Matrix3::identity(), None, None, params);
    
                let mut points3d_a = Vec::new();
                let mut points3d_b = Vec::new();
                let mut tss_a = Vec::new();
                let mut tss_b = Vec::new();
    
                assert!(a.len() == b.len());
    
                let height = params.height as f64;
                for (i, (ap, bp)) in a.iter().zip(b.iter()).enumerate() {
                    let ts_a = *a_t as f64 / 1000_000.0 + frame_readout_time * (a_p[i].1 / height);
                    let ts_b = *b_t as f64 / 1000_000.0 + frame_readout_time * (b_p[i].1 / height);
    
                    let ap = Vector3::new(ap.0, ap.1, 1.0).normalize();
                    let bp = Vector3::new(bp.0, bp.1, 1.0).normalize();
    
                    points3d_a.push((ap[0], ap[1], ap[2]));
                    points3d_b.push((bp[0], bp[1], bp[2]));
    
                    tss_a.push(ts_a);
                    tss_b.push(ts_b);
                }
    
                ret.sync.set_track_result(*a_t, &tss_a, &tss_b, &points3d_a, &points3d_b);
            }
            ret.sync_points.push((from_ts, to_ts));
    
        }
        ret
    }

    pub fn full_sync(&mut self) -> Vec<(f64, f64, f64)> {
        self.is_guess_orient.store(false, SeqCst);

        let mut offsets = Vec::new();
        set_quats(&mut self.sync, &self.gyro_source.quaternions);
        
        for (from_ts, to_ts) in &self.sync_points {

            let presync_step = 3.0;
            let presync_radius = self.search_size;
            let initial_delay = -self.initial_offset;
            
            if let Some(delay) = self.sync.full_sync(
                initial_delay / 1000.0,
                *from_ts,
                *to_ts,
                presync_step / 1000.0,
                presync_radius / 1000.0,
                4,
            ) {
                let offset = delay.1 * 1000.0;
                if (offset - initial_delay).abs() <= presync_radius {
                    let offset = -offset - (self.frame_readout_time * 1000.0 / 2.0);
                    offsets.push(((from_ts + to_ts) as f64 / 2.0 / 1000.0, offset, delay.0));
                } else {
                    log::warn!(
                        "Sync point out of acceptable range {} < {}",
                        presync_radius,
                        (offset - initial_delay).abs()
                    );
                }
            }
            self.current_sync_point.fetch_add(1, SeqCst);
        }
        offsets
    }     
    
    pub fn guess_orient(&mut self) -> Option<(String, f64)> {
        self.is_guess_orient.store(true, SeqCst);
        
        let mut clone_source = self.gyro_source.clone();

        let possible_orientations = [ 
            "YxZ", "Xyz", "XZy", "Zxy", "zyX", "yxZ", "ZXY", "zYx", "ZYX", "yXz", "YZX", "XyZ",
            "Yzx", "zXy", "YXz", "xyz", "yZx", "XYZ", "zxy", "xYz", "XYz", "zxY", "zXY", "xZy",
            "zyx", "xyZ", "Yxz", "xzy", "yZX", "yzX", "ZYx", "xYZ", "zYX", "ZxY", "yzx", "xZY",
            "Xzy", "XzY", "YzX", "Zyx", "XZY", "yxz", "xzY", "ZyX", "YXZ", "yXZ", "YZx", "ZXy"];        

        possible_orientations.iter().map(|orient| {
            clone_source.set_imu_orientation(orient.to_string());

            set_quats(&mut self.sync, &clone_source.quaternions);

            let total_cost : f64 = self.sync_points.iter().map(|(from_ts, to_ts)| {
                self.sync.pre_sync(
                    -self.initial_offset / 1000.0, 
                    *from_ts, 
                    *to_ts, 
                    3.0 / 1000.0, 
                    self.search_size / 1000.0
                ).unwrap_or((0.0,0.0))
            }).map(|v| {v.0}).sum();
            
            self.current_orientation.fetch_add(1, SeqCst);
            
            (orient.to_string(), total_cost)
        }).reduce(|a: (String, f64), b: (String, f64)| -> (String, f64) { if a.1 < b.1 { a } else { b } })
    }
}

fn set_quats (sync: &mut SyncProblem, source_quats: &TimeQuat) {
    let mut quats = Vec::new();
    let mut timestamps = Vec::new();
    let rotation = *Quat64::from_scaled_axis(Vector3::new(PI, 0.0, 0.0)).quaternion();
    
    for (ts, q) in source_quats {
        let q = Quat64::from(*q).quaternion() * rotation;
        let qv = q.as_vector();
        
        quats.push((qv[3], -qv[0], -qv[1], -qv[2])); // w, x, y, z
        timestamps.push(*ts);
    }
    sync.set_gyro_quaternions(&timestamps, &quats);
}
