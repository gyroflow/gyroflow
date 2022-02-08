// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Marc Roeschlin

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use enterpolation::{ Curve, Merge, bspline::BSpline };
use crate::undistortion::{ self, ComputeParams };

#[derive(Default, Clone, Copy, Debug)]
pub struct Point2D(f64, f64);
impl Merge<f64> for Point2D {
    fn merge(self, other: Self, factor: f64) -> Self {
        Point2D(
            self.0 * (1.0 - factor) + other.0 * factor,
            self.1 * (1.0 - factor) + other.1 * factor
        )
    }
}

#[derive(PartialEq, Clone)]
enum Mode {
    Disabled,
    DynamicZoom(f64), // f64 - smoothing focus window in seconds
    StaticZoom
}

#[derive(Clone)]
pub struct AdaptiveZoom {
    compute_params: ComputeParams,
    input_dim: (f64, f64), 
    output_dim: (f64, f64), 
    fps: f64, 
    mode: Mode, 
    range: (f64, f64),
}

impl AdaptiveZoom {
    pub fn from_compute_params(mut compute_params: ComputeParams) -> Self {
        compute_params.fov_scale = 1.0;
        compute_params.fovs.clear();
        
        let ratio = compute_params.video_width as f64 / compute_params.video_output_width.max(1) as f64;
        // Use original video dimensions, because this is used to undistort points, and we need to find original image bounding box
        // Then we can use real `output_dim` to fit the fov
        compute_params.width = compute_params.video_width;
        compute_params.height = compute_params.video_height;
        compute_params.output_width = compute_params.video_width;
        compute_params.output_height = compute_params.video_height;

        let input_dim = (compute_params.video_width as f64, compute_params.video_height as f64);
        let output_dim = (compute_params.video_output_width as f64 * ratio, compute_params.video_output_height as f64 * ratio);

        Self {
            input_dim,
            output_dim,
            fps: compute_params.scaled_fps,
            range: (compute_params.trim_start, compute_params.trim_end),

            mode: if compute_params.adaptive_zoom_window < -0.9 {
                Mode::StaticZoom
            } else if compute_params.adaptive_zoom_window > 0.0001 {
                Mode::DynamicZoom(compute_params.adaptive_zoom_window)
            } else {
                Mode::Disabled
            },
            compute_params
        }
    }

    pub fn get_state_checksum(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        if self.compute_params.distortion_coeffs.len() >= 4 {
            hasher.write_u64(self.compute_params.distortion_coeffs[0].to_bits());
            hasher.write_u64(self.compute_params.distortion_coeffs[1].to_bits());
            hasher.write_u64(self.compute_params.distortion_coeffs[2].to_bits());
            hasher.write_u64(self.compute_params.distortion_coeffs[3].to_bits());
        }
        hasher.write_u64(self.input_dim.0.to_bits());
        hasher.write_u64(self.input_dim.1.to_bits());
        hasher.write_u64(self.output_dim.0.to_bits());
        hasher.write_u64(self.output_dim.1.to_bits());
        hasher.write_u64(self.compute_params.video_rotation.to_bits());
        hasher.write_u64(self.fps.to_bits());
        hasher.write_u64(self.range.0.to_bits());
        hasher.write_u64(self.range.1.to_bits());
        match self.mode {
            Mode::Disabled => hasher.write_u64(0),
            Mode::StaticZoom => hasher.write_u64(1),
            Mode::DynamicZoom(w) => hasher.write_u64(w.to_bits())
        }
        hasher.finish()
    }

    fn find_fcorr(&self, center: Point2D, polygon: &[Point2D]) -> (f64, usize) {
        let (output_width, output_height) = (self.output_dim.0 as f64, self.output_dim.1 as f64);
        let angle_output = (output_height as f64 / 2.0).atan2(output_width / 2.0);

        // fig, ax = plt.subplots()

        let polygon: Vec<Point2D> = polygon.iter().map(|p| Point2D(p.0 - center.0, p.1 - center.1)).collect();
        // ax.scatter(polygon[:,0], polygon[:,1])

        let dist_p: Vec<f64> = polygon.iter().map(|pt| ((pt.0*pt.0) + (pt.1*pt.1)).sqrt()).collect();
        let angles: Vec<f64> = polygon.iter().map(|pt| pt.1.atan2(pt.0).abs()).collect();

        // ax.plot(distP*np.cos(angles), distP*np.sin(angles), 'ro')
        // ax.plot(distP[mask]*np.cos(angles[mask]), distP[mask]*np.sin(angles[mask]), 'yo')
        // ax.add_patch(matplotlib.patches.Rectangle((-output_width/2,-output_height/2), output_width, output_height,color="yellow"))
        let d_width:  Vec<f64> = angles.iter().map(|a| ((output_width  / 2.0) / a.cos()).abs()).collect();
        let d_height: Vec<f64> = angles.iter().map(|a| ((output_height / 2.0) / a.sin()).abs()).collect();

        let mut ffactor: Vec<f64> = d_width.iter().zip(dist_p.iter()).map(|(v, d)| v / d).collect();

        ffactor.iter_mut().enumerate().for_each(|(i, v)| {
            if angle_output <= angles[i].abs() && angles[i].abs() < (std::f64::consts::PI - angle_output) {
                *v = d_height[i] / dist_p[i];
            }
        });

        // Find max value and it's index
        ffactor.iter().enumerate()
               .fold((0.0, 0), |max, (ind, &val)| if val > max.0 { (val, ind) } else { max })
    }

    fn find_fov(&self, center: Point2D, polygon: &[Point2D]) -> Option<f64> {
        let num_int_points = 20;
        // let (original_width, original_height) = self.calib_dimension;
        let (fcorr, idx) = self.find_fcorr(center, polygon);
        if idx < 1 { return None; }
        let n_p = polygon.len();
        let relevant_p = [
            polygon[(idx - 1) % n_p], 
            polygon[idx],
            polygon[(idx + 1) % n_p]
        ];

        // TODO: `distance` should be used in interpolation for more accurate results. It's the x axis for `scipy.interp1d`
        // let distance = {
        //     let mut sum = 0.0;
        //     let mut d: Vec<f64> = relevant_p[1..].iter().enumerate().map(|(i, v)| {
        //         sum += ((v.0 - relevant_p[i].0).powf(2.0) + (v.1 - relevant_p[i].1).powf(2.0)).sqrt();
        //         sum
        //     }).collect();
        //     d.insert(0, 0.0);
        //     d.iter_mut().for_each(|v| *v /= sum);
        //     d
        // };

        let bspline = BSpline::builder()
                    .clamped()
                    .elements(&relevant_p)
                    .equidistant::<f64>()
                    .degree(2) // 1 - linear, 2 - quadratic, 3 - cubic
                    .normalized()
                    .constant::<3>()
                    .build();
        if let Err(ref err) = bspline {
            log::error!("{:?}", err);
        }
        let bspline = bspline.ok()?;

        // let alpha: Vec<f64> = (0..numIntPoints).map(|i| i as f64 * (1.0 / numIntPoints as f64)).collect();
        let interpolated_points: Vec<Point2D> = bspline.take(num_int_points).collect();

        let (fcorr_i, _) = self.find_fcorr(center, &interpolated_points);

        // plt.plot(polygon[:,0], polygon[:,1], 'ro')
        // plt.plot(relevantP[:,0], relevantP[:,1], 'bo')
        // plt.plot(interpolated_points[:,0], interpolated_points[:,1], 'yo')
        // plt.show()

        Some(1.0 / fcorr.max(fcorr_i))
    }
    
    pub fn compute(&self, timestamps: &[f64]) -> Vec<(f64, Point2D)> { // Vec<fovValue, focalCenter>
        if self.mode == Mode::Disabled || timestamps.is_empty() {
            return Vec::new();
        }
        let boundary_polygons: Vec<Vec<Point2D>> = timestamps.iter().map(|&ts| self.bounding_polygon(ts, 9)).collect();
        // let focus_windows: Vec<Point2D> = boundary_boxes.iter().map(|b| self.find_focal_center(b, output_dim)).collect();

        // TODO: implement smoothing of position of crop, s.t. cropping area can "move" anywhere within bounding polygon
        let crop_center_positions: Vec<Point2D> = timestamps.into_iter().map(|_| Point2D(self.input_dim.0 / 2.0, self.input_dim.1 / 2.0)).collect();

        // if smoothing_center > 0 {
        //     let mut smoothing_num_frames = (smoothing_center * fps).floor() as usize;
        //     if smoothing_num_frames % 2 == 0 {
        //         smoothing_num_frames += 1;
        //     }
        //     let focus_windows_pad = pad_edge(&focus_windows, (smoothing_num_frames / 2, smoothing_num_frames / 2));
        //     let gaussian = gaussian_window_normalized(smoothing_num_frames, smoothing_num_frames as f64 / 6.0);
        //     focus_windows = convolve(&focus_windows_pad.map(|v| v.0).collect(), &gaussian).iter().zip(
        //         convolve(&focus_windows_pad.map(|v| v.1).collect(), &gaussian).iter()
        //     ).map(|v| Point2D(v.0, v.1)).collect()
        // }
        let mut fov_values: Vec<f64> = crop_center_positions.iter()
                                                            .zip(boundary_polygons.iter())
                                                            .filter_map(|(&center, polygon)| 
                                                                self.find_fov(center, polygon)
                                                            ).collect();

        if self.range.0 > 0.0 || self.range.1 < 1.0 {
            // Only within render range.
            if let Some(max_fov) = fov_values.iter().copied().reduce(f64::max) {
                let l = (timestamps.len() - 1) as f64;
                let first_ind = (l * self.range.0).floor() as usize;
                let last_ind  = (l * self.range.1).ceil() as usize;
                if fov_values.len() > first_ind {
                    fov_values[0..first_ind].iter_mut().for_each(|v| *v = max_fov);
                }
                if fov_values.len() > last_ind {
                    fov_values[last_ind..].iter_mut().for_each(|v| *v = max_fov);
                }
            }
        }

        match self.mode {
            Mode::DynamicZoom(window_s) => {
                let mut frames = (window_s * self.fps).floor() as usize;
                if frames % 2 == 0 {
                    frames += 1;
                }
    
                let fov_values_pad = pad_edge(&fov_values, (frames / 2, frames / 2));
                let fov_min = min_rolling(&fov_values_pad, frames);
                let fov_min_pad = pad_edge(&fov_min, (frames / 2, frames / 2));
    
                let gaussian = gaussian_window_normalized(frames, frames as f64 / 6.0);
                fov_values = convolve(&fov_min_pad, &gaussian);
            },
            Mode::StaticZoom => {
                if let Some(max_f) = fov_values.iter().copied().reduce(f64::min) {
                    fov_values.iter_mut().for_each(|v| *v = max_f);
                } else {
                    log::warn!("Unable to find min of fov_values, len: {}", fov_values.len());
                }
            }
            _ => { }
        }

        fov_values.iter().copied().zip(crop_center_positions.iter().copied()).collect()
    }

    fn bounding_polygon(&self, timestamp_ms: f64, num_points: usize) -> Vec<Point2D> {
        if num_points < 1 { return Vec::new(); }
        let (w, h) = (self.input_dim.0, self.input_dim.1);

        let pts = num_points - 1;
        let dim_ratio = ((w / pts as f64), (h / pts as f64));
        let mut distorted_points: Vec<(f64, f64)> = Vec::with_capacity(pts * 4);
        for i in 0..pts { distorted_points.push((i as f64 * dim_ratio.0,              0.0)); }
        for i in 0..pts { distorted_points.push((w,                                   i as f64 * dim_ratio.1)); }
        for i in 0..pts { distorted_points.push(((pts - i) as f64 * dim_ratio.0,      h)); }
        for i in 0..pts { distorted_points.push((0.0,                                 (pts - i) as f64 * dim_ratio.1)); }

        let undistorted_points = undistortion::undistort_points_with_rolling_shutter(&distorted_points, timestamp_ms, &self.compute_params);

        undistorted_points.into_iter().map(|v| Point2D(v.0, v.1)).collect()
    }

    /*fn find_focal_center(&self, box_: (f64, f64, f64, f64), output_dim: (usize, usize)) -> Point2D {
        let (mleft, mright, mtop, mbottom) = box_;
        let (mut window_width, mut window_height) = (output_dim.0 as f64, output_dim.1 as f64);

        let max_x = mright - mleft;
        let max_y = mbottom - mtop;

        let ratio = max_x / max_y;
        let output_ratio = output_dim.0 as f64 / output_dim.1 as f64;

        if max_x / output_ratio < max_y {
            window_width = max_x;
            window_height = max_x / output_ratio;
            let mut f_x = mleft + window_width / 2.0;
            let mut f_y = self.compute_params.height as f64 / 2.0;
            if f_y + window_height / 2.0 > mbottom {
                f_y = mbottom - window_height / 2.0;
            } else if f_y - window_height / 2.0 < mtop {
                f_y = mtop + window_height / 2.0;
            }
            Point2D(f_x, f_y)
        } else {
            window_height = max_y;
            window_width = max_y * output_ratio;
            let mut f_y = mtop + window_height / 2.0;
            let mut f_x = self.compute_params.width as f64 / 2.0;
            if f_x + window_width / 2.0 > mright {
                f_x = mright - window_width / 2.0;
            } else if f_x - window_width / 2.0 < mleft {
                f_x = mleft + window_width / 2.0;
            }
            Point2D(f_x, f_y)
        }
    }*/
}

fn min_rolling(a: &[f64], window: usize) -> Vec<f64> {
    a.windows(window).filter_map(|window| {
        window.iter().copied().reduce(f64::min)
    }).collect()
}

fn convolve(v: &[f64], filter: &[f64]) -> Vec<f64> {
    v.windows(filter.len()).map(|window| {
        window.iter().zip(filter).map(|(x, y)| x * y).sum()
    }).collect()
}

fn gaussian_window(m: usize, std: f64) -> Vec<f64> {
    let step = 1.0 / m as f64;
    let n: Vec<f64> = (0..m).map(|i| (i as f64 * step) - (m as f64 - 1.0) / 2.0).collect();
    let sig2 = 2.0 * std * std;
    n.iter().map(|&v| (-v).powf(2.0) / sig2).collect()
}
fn gaussian_window_normalized(m: usize, std: f64) -> Vec<f64> {
    let mut w = gaussian_window(m, std);
    let sum: f64 = w.iter().sum();
    w.iter_mut().for_each(|v| *v /= sum);
    w
}

fn pad_edge(arr: &[f64], pad_to: (usize, usize)) -> Vec<f64> {
    let first = *arr.first().unwrap_or(&0.0);
    let last = *arr.last().unwrap_or(&0.0);

    let mut new_arr = vec![0.0; arr.len() + pad_to.0 + pad_to.1];
    new_arr[pad_to.0..pad_to.0 + arr.len()].copy_from_slice(arr);

    for i in 0..pad_to.0 { new_arr[i] = first; }
    for i in pad_to.0 + arr.len()..new_arr.len() { new_arr[i] = last; }

    new_arr
}
