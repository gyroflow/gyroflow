// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use crate::stabilization::ComputeParams;
use crate::gyro_source::Quat64;
use nalgebra::{ Matrix3, Vector3 };
use rs_sync::SyncProblem;
use super::OpticalFlowPoints;
use std::f64::consts::PI;

// use super::cpp_wrapper;
// const SAVE_DEBUG_DATA: bool = false;

pub fn find_offsets(
    _ranges: &[(i64, i64)],
    matched_points: &Vec<Vec<((i64, OpticalFlowPoints), (i64, OpticalFlowPoints))>>,
    initial_offset: f64,
    search_size: f64,
    params: &ComputeParams,
) -> Vec<(f64, f64, f64)> { // Vec<(timestamp, offset, cost)>

    let mut offsets = Vec::new();

    let mut frame_readout_time = params.frame_readout_time;
    if frame_readout_time == 0.0 {
        frame_readout_time = 1000.0 / params.scaled_fps / 2.0;
    }
    frame_readout_time /= 1000.0;

    let mut quats = Vec::new();
    let mut timestamps = Vec::new();
    for (ts, q) in &params.gyro.quaternions {
        let rotation = Quat64::from_scaled_axis(Vector3::new(PI, 0.0, 0.0));
        let q = Quat64::from(*q).quaternion() * rotation.quaternion();
        let qv = q.as_vector();

        quats.push((qv[3], -qv[0], -qv[1], -qv[2])); // w, x, y, z
        timestamps.push(*ts);
    }

    // let mut ser = cpp_wrapper::Serialized::default();
    // if SAVE_DEBUG_DATA {
    //     ser.quats = quats.clone();
    //     ser.timestamps = timestamps.clone();
    // }

    let mut sync = SyncProblem::new();
    sync.set_gyro_quaternions(&timestamps, &quats);

    for range in matched_points {
        let mut from_ts = -1;
        let mut to_ts = 0;
        for ((a_t, a_p), (b_t, b_p)) in range {
            if from_ts == -1 {
                from_ts = *a_t;
            }
            to_ts = *b_t;
            let a = crate::stabilization::undistort_points_with_params(
                &a_p,
                Matrix3::identity(),
                None,
                None,
                params,
            );
            let b = crate::stabilization::undistort_points_with_params(
                &b_p,
                Matrix3::identity(),
                None,
                None,
                params,
            );

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

            let frame = crate::frame_at_timestamp(*a_t as f64 / 1000.0, params.scaled_fps);
            sync.set_track_result(frame, &tss_a, &tss_b, &points3d_a, &points3d_b);

            // if SAVE_DEBUG_DATA {
            //     ser.perframe.push(cpp_wrapper::PerFrame {
            //         frame,
            //         pointsa: points3d_a,
            //         pointsb: points3d_b,
            //         tsa: tss_a,
            //         tsb: tss_b,
            //     });
            // }
        }

        let start_frame = crate::frame_at_timestamp(from_ts as f64 / 1000.0, params.scaled_fps);
        let end_frame = crate::frame_at_timestamp(to_ts as f64 / 1000.0, params.scaled_fps);
        let presync_step = 2.0;
        let presync_radius = search_size;
        let initial_delay = initial_offset;

        // if SAVE_DEBUG_DATA {
        //     ser.frame_ro = frame_readout_time;
        //     ser.start_frame = start_frame;
        //     ser.end_frame = end_frame;
        //     ser.presync_step = presync_step;
        //     ser.presync_radius = presync_radius;
        //     ser.initial_delay = initial_delay;
        //     cpp_wrapper::save_data_to_file(&ser, &format!("D:/tests/data-{}.bin", from_ts));
        // }

        let mut delay = sync.pre_sync(initial_delay / 1000.0, start_frame, end_frame, presync_step / 1000.0, presync_radius / 1000.0);
        for _ in 0..4 {
            delay = sync.sync(delay.1, start_frame, end_frame);   
        }
        let offset = delay.1 * 1000.0;

        let offset = -offset - (frame_readout_time * 1000.0 / 2.0);

        offsets.push(((from_ts + to_ts) as f64 / 2.0 / 1000.0, offset, delay.0));
    }
    offsets
}
