// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use rayon::iter::{ ParallelIterator, IntoParallelIterator };
use crate::{ stabilization, stabilization::ComputeParams };
use std::sync::{ Arc, atomic::{ AtomicBool, Ordering::Relaxed } };
use super::super::{ PoseEstimator, SyncParams };
use parking_lot::RwLock;

pub fn find_offsets<F: Fn(f64) + Sync>(estimator: &PoseEstimator, ranges: &[(i64, i64)], sync_params: &SyncParams, params_arg: &ComputeParams, for_rs: bool, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> Vec<(f64, f64, f64)> { // Vec<(timestamp, offset, cost)>
    let mut params = params_arg.clone();
    params.gyro = Arc::new(RwLock::new(params_arg.gyro.read().clone()));
    if !for_rs {
        params.gyro.write().clear_offsets();
    }

    let (w, h) = (params.width as i32, params.height as i32);

    let mut final_offsets = Vec::new();

    let next_frame_no = 2;
    let fps = params.scaled_fps;
    let ranges_len = ranges.len() as f64;

    let keys: Vec<i64> = estimator.sync_results.read().keys().copied().collect();

    for (i, (from_ts, to_ts)) in ranges.iter().enumerate() {
        if cancel_flag.load(Relaxed) { break; }
        progress_cb(i as f64 / ranges_len);

        let mut matched_points = Vec::new();
        for ts in &keys {
            if (*from_ts..*to_ts).contains(&ts) {
                match estimator.get_of_lines_for_timestamp(&ts, 0, 1.0, next_frame_no, true) {
                    (Some(lines), Some(_frame_size)) => {
                        if !lines.0.1.is_empty() && lines.0.1.len() == lines.1.1.len() {
                            matched_points.push(lines);
                        } else {
                            log::warn!("Invalid point pairs {} {}", lines.0.1.len(), lines.1.1.len());
                        }
                    },
                    _ => {
                        log::warn!("No detected features for ts {}", ts);
                    }
                }
            }
        }

        let calculate_distance = |offs, rs: Option<f64>| -> f64 {
            let mut total_dist = 0.0;
            let mut params_ref = &params;
            let mut _params2 = None;
            if let Some(rs) = rs {
                _params2 = Some(params.clone());
                _params2.as_mut().unwrap().frame_readout_time = rs;
                params_ref = _params2.as_ref().unwrap();
            }

            for ((ts, pts1), (next_ts, pts2)) in &matched_points {
                let timestamp_ms  = *ts as f64 / 1000.0;
                let timestamp_ms2 = *next_ts as f64 / 1000.0;

                let undistorted_points1 = stabilization::undistort_points_with_rolling_shutter(&pts1, timestamp_ms - offs, params_ref, 1.0, false);
                let undistorted_points2 = stabilization::undistort_points_with_rolling_shutter(&pts2, timestamp_ms2 - offs, params_ref, 1.0, false);

                let mut distances = Vec::with_capacity(undistorted_points1.len());
                for (p1, p2) in undistorted_points1.iter().zip(undistorted_points2.iter()) {
                    if p1.0 > 0.0 && p1.0 < w as f32 && p1.1 > 0.0 && p1.1 < h as f32 &&
                       p2.0 > 0.0 && p2.0 < w as f32 && p2.1 > 0.0 && p2.1 < h as f32 {
                        let dist = ((p2.0 - p1.0) * (p2.0 - p1.0))
                                      + ((p2.1 - p1.1) * (p2.1 - p1.1));
                        distances.push(dist as u64);
                    }
                }
                distances.sort_unstable();

                // Use only 90% of lines, discard the longest ones as they are often wrongly computed point matches
                for dist in &distances[0..(distances.len() as f64 * 0.9) as usize] {
                    total_dist += *dist as f64;
                }
            }
            total_dist
        };

        let find_min = |a: (f64, f64), b: (f64, f64)| -> (f64, f64) { if a.1 < b.1 { a } else { b } };

        if for_rs { // Estimate rolling shutter
            // First search every 1 ms
            let max_rs = 1000.0 / fps;
            let steps = max_rs as isize;
            let lowest = (-steps..steps)
                .into_par_iter()
                .map(|i| {
                    (i as f64, calculate_distance(0.0, Some(i as f64)))
                })
                .reduce_with(find_min)
                .and_then(|lowest| {
                    // Then refine to 0.01 ms
                    (0..200)
                        .into_par_iter()
                        .map(|i| {
                            let rs = lowest.0 - 1.0 + (i as f64 * 0.01);
                            (rs, calculate_distance(0.0, Some(rs)))
                        })
                        .reduce_with(find_min)
                });
            log::debug!("lowest: {:?}", &lowest);
            if let Some(lowest) = lowest {
                final_offsets.push((0.0, lowest.0, lowest.1));
            }
        } else {
            // First search every 1 ms
            let steps = sync_params.search_size as usize;
            let lowest = (0..steps)
                .into_par_iter()
                .map(|i| {
                    let offs = sync_params.initial_offset + (-(sync_params.search_size / 2.0) + (i as f64));
                    (offs, calculate_distance(offs, None))
                })
                .reduce_with(find_min)
                .and_then(|lowest| {
                    // Then refine to 0.01 ms
                    (0..200)
                        .into_par_iter()
                        .map(|i| {
                            let offs = lowest.0 - 1.0 + (i as f64 * 0.01);
                            (offs, calculate_distance(offs, None))
                        })
                        .reduce_with(find_min)
                });

            log::debug!("lowest: {:?}", &lowest);
            if let Some(lowest) = lowest {
                let middle_timestamp = (*from_ts as f64 + (to_ts - from_ts) as f64 / 2.0) / 1000.0;

                // Only accept offsets that are within 90% of search size range
                if (lowest.0 - sync_params.initial_offset).abs() < sync_params.search_size * 0.9 {
                    final_offsets.push((middle_timestamp, lowest.0, lowest.1));
                } else {
                    log::warn!("Sync point out of acceptable range {} < {}", (lowest.0 - sync_params.initial_offset).abs(), sync_params.search_size * 0.9);
                }
            }
        }
    }

    final_offsets
}



/////////////////////// DEBUG ///////////////////////
/*let l = self.sync_results.read();
if let Some(curr) = l.get(&frame) {
    if let Some(next) = l.get(&(frame + 2/*every_nth_frame*/)) {
        let mut curr = curr.item.clone();
        let mut next = next.item.clone();
        drop(l);
        match (curr, next) {
            #[cfg(feature = "use-opencv")]
            (EstimatorItem::OpenCV(ref mut curr), EstimatorItem::OpenCV(ref mut next)) => {
                use ::opencv::types::VectorOfi32;
                use ::opencv::core::{Mat, Size, Point, Scalar, Point2f, TermCriteria, CV_8UC1, CV_32FC1, BORDER_CONSTANT};
                use ::opencv::prelude::MatTraitConst;
                use ::opencv::imgproc::INTER_LINEAR;
                use std::os::raw::c_void;

                let (w, h) = curr.size;
                let mut inp = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, curr.img_bytes.as_mut_ptr() as *mut c_void, w as usize) }.unwrap();
                let mut inp2 = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, next.img_bytes.as_mut_ptr() as *mut c_void, w as usize) }.unwrap();

                let k_cv = Mat::from_slice_2d(&[
                    [scaled_k[(0, 0)], scaled_k[(0, 1)], scaled_k[(0, 2)]],
                    [scaled_k[(1, 0)], scaled_k[(1, 1)], scaled_k[(1, 2)]],
                    [scaled_k[(2, 0)], scaled_k[(2, 1)], scaled_k[(2, 2)]]
                ]).unwrap();
                let new_k_cv = Mat::from_slice_2d(&[
                    [new_k[(0, 0)], new_k[(0, 1)], new_k[(0, 2)]],
                    [new_k[(1, 0)], new_k[(1, 1)], new_k[(1, 2)]],
                    [new_k[(2, 0)], new_k[(2, 1)], new_k[(2, 2)]]
                ]).unwrap();
                let r_cv = Mat::from_slice_2d(&[
                    [r[(0, 0)], r[(0, 1)], r[(0, 2)]],
                    [r[(1, 0)], r[(1, 1)], r[(1, 2)]],
                    [r[(2, 0)], r[(2, 1)], r[(2, 2)]]
                ]).unwrap();
                let r2_cv = Mat::from_slice_2d(&[
                    [r2[(0, 0)], r2[(0, 1)], r2[(0, 2)]],
                    [r2[(1, 0)], r2[(1, 1)], r2[(1, 2)]],
                    [r2[(2, 0)], r2[(2, 1)], r2[(2, 2)]]
                ]).unwrap();
                let coeffs_cv = Mat::from_slice(&distortion_coeffs).unwrap();
                for j in 0..2 {
                    let mut outp1 = Mat::default();
                    let mut outp = Mat::default();
                    //::opencv::imgproc::resize(&inp, &mut outp1, Size::new(3840, 2160), 0.0, 0.0, ::opencv::imgproc::INTER_LINEAR);
                    ::opencv::imgproc::cvt_color(if j == 0 { &inp } else { &inp2 }, &mut outp1, ::opencv::imgproc::COLOR_GRAY2RGB, 0).unwrap();
                    let mut map1 = Mat::default();
                    let mut map2 = Mat::default();
                    ::opencv::calib3d::fisheye_init_undistort_rectify_map(&k_cv, &coeffs_cv, if j == 0 { &r_cv } else { &r2_cv }, &new_k_cv, Size::new(out_dim_small.0 as i32, out_dim_small.1 as i32), CV_32FC1, &mut map1, &mut map2).unwrap();
                    for (&p1, &p2) in pts.0.iter().zip(pts.1.iter()) {
                        ::opencv::imgproc::line(&mut outp1, Point::new(p1.0 as i32, p1.1 as i32), Point::new(p2.0 as i32, p2.1 as i32), Scalar::new(255.0, 0.0, 0.0, 0.0), 1, ::opencv::imgproc::LINE_8, 0).unwrap();
                    }
                    ::opencv::imgproc::remap(&outp1, &mut outp, &map1, &map2, INTER_LINEAR, BORDER_CONSTANT, Scalar::default()).unwrap();

                    for (&p1, &p2) in undistorted_points1.iter().zip(undistorted_points2.iter()) {
                        ::opencv::imgproc::line(&mut outp, Point::new(p1.0 as i32, p1.1 as i32), Point::new(p2.0 as i32, p2.1 as i32), Scalar::new(0.0, 0.0, 255.0, 0.0), 1, ::opencv::imgproc::LINE_8, 0).unwrap();
                    }
                    ::opencv::imgcodecs::imwrite(&format!("D:/test-{:.3}-{}.jpg", offs, j), &outp, &VectorOfi32::new()).unwrap();
                }
                /*let mut pts1: Vec<Point2f> = Vec::new();
                let mut pts2: Vec<Point2f> = Vec::new();
                for x in &pts.0 {
                    pts1.push(Point2f::new(x.0 as f32, x.1 as f32));
                }
                for x in &pts.1 {
                    pts2.push(Point2f::new(x.0 as f32, x.1 as f32));
                }
                let distorted1 = Mat::from_slice(&pts1).unwrap();
                let distorted2 = Mat::from_slice(&pts2).unwrap();
                let mut undistorted1 = Mat::default();
                let mut undistorted2 = Mat::default();
                ::opencv::calib3d::fisheye_undistort_points(&distorted1, &mut undistorted1, &k_cv, &coeffs_cv, &r_cv, &new_k_cv).unwrap();
                ::opencv::calib3d::fisheye_undistort_points(&distorted2, &mut undistorted2, &k_cv, &coeffs_cv, &r2_cv, &new_k_cv).unwrap();
                undistorted_points1.clear();
                undistorted_points2.clear();
                for i in 0..undistorted1.cols() {
                    let pt = undistorted1.at::<Point2f>(i).unwrap();
                    undistorted_points1.push(((pt.x+1.0) as f64, (pt.y+1.0) as f64));
                }
                for i in 0..undistorted2.cols() {
                    let pt = undistorted2.at::<Point2f>(i).unwrap();
                    undistorted_points2.push(((pt.x+1.0) as f64, (pt.y+1.0) as f64));
                }
                for (&p1, &p2) in undistorted_points1.iter().zip(undistorted_points2.iter()) {
                    ::opencv::imgproc::line(&mut outp, Point::new(p1.0 as i32, p1.1 as i32), Point::new(p2.0 as i32, p2.1 as i32), Scalar::new(0.0, 255.0, 255.0, 0.0), 1, ::opencv::imgproc::LINE_8, 0).unwrap();
                }*/
            }
            _ => { }
        }
    }
}*/
/////////////////////// DEBUG ///////////////////////