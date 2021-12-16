use rayon::iter::{ ParallelIterator, IntoParallelIterator };
use crate::{ undistortion, undistortion::ComputeParams };
use super::PoseEstimator;

pub fn find_offsets(ranges: &[(usize, usize)], estimator: &PoseEstimator, initial_offset: f64, search_size: f64, params: &ComputeParams, for_rs: bool) -> Vec<(f64, f64, f64)> { // Vec<(timestamp, offset, cost)>
    let (w, h) = (params.width as i32, params.height as i32);

    let mut final_offsets = Vec::new();

    let next_frame_no = 2;
    let fps = params.fps;

    for (from_frame, to_frame) in ranges {
        let mut matched_points = Vec::new();
        for frame in (*from_frame..*to_frame).step_by(next_frame_no) {
            if let Some(lines) = estimator.get_of_lines_for_frame(&frame, 1.0, next_frame_no) {
                if !lines.0.is_empty() && lines.0.len() == lines.1.len() {
                    matched_points.push((frame, lines));
                } else {
                    eprintln!("Invalid point pairs {} {}", lines.0.len(), lines.1.len());
                }
            }
        }

        let calculate_distance = |offs, rs: Option<f64>| -> f64 {
            let mut total_dist = 0.0;
            let mut params_ref = params;
            let mut _params2 = None;
            if let Some(rs) = rs {
                _params2 = Some(params.clone());
                _params2.as_mut().unwrap().frame_readout_time = rs;
                params_ref = _params2.as_ref().unwrap();
            }

            for (frame, pts) in &matched_points {
                let timestamp_ms  = *frame as f64 * 1000.0 / fps;
                let timestamp_ms2 = (*frame + next_frame_no) as f64 * 1000.0 / fps;

                let undistorted_points1 = undistortion::undistort_points_with_rolling_shutter(&pts.0, timestamp_ms - offs, params_ref);
                let undistorted_points2 = undistortion::undistort_points_with_rolling_shutter(&pts.1, timestamp_ms2 - offs, params_ref);

                let mut distances = Vec::with_capacity(undistorted_points1.len());
                for (p1, p2) in undistorted_points1.iter().zip(undistorted_points2.iter()) {
                    if p1.0 > 0.0 && p1.0 < w as f64 && p1.1 > 0.0 && p1.1 < h as f64 &&
                       p2.0 > 0.0 && p2.0 < w as f64 && p2.1 > 0.0 && p2.1 < h as f64 {
                        let dist = ((p2.0 - p1.0) * (p2.0 - p1.0)) 
                                     + ((p2.1 - p1.1) * (p2.1 - p1.1));
                        distances.push(dist as u64);
                    }
                }
                distances.sort();

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
            dbg!(lowest);
            if let Some(lowest) = lowest {
                final_offsets.push((0.0, lowest.0, lowest.1));
            }
        } else {
            // First search every 1 ms
            let steps = search_size as usize;
            let lowest = (0..steps)
                .into_par_iter()
                .map(|i| {
                    let offs = initial_offset + (-(search_size / 2.0) + (i as f64));
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

            dbg!(&lowest);
            if let Some(lowest) = lowest {
                let middle_frame = from_frame + (to_frame - from_frame) / 2;
                let middle_timestamp = (middle_frame as f64 * 1000.0) / fps;

                // Only accept offsets that are within 90% of search size range
                if lowest.0.abs() < (search_size / 2.0) * 0.9 {
                    final_offsets.push((middle_timestamp, lowest.0, lowest.1));
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