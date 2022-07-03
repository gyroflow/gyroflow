// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021 Marc Roeschlin, Adrian, Maik

use super::*;
use std::collections::BTreeMap;
use crate::keyframes::*;

pub struct ZoomDynamic {
    window: f64,
    fov_estimator: Box<dyn FieldOfViewAlgorithm>,
    compute_params: ComputeParams,
}

struct DataPerTimestamp {
    frames: usize,
    half_frames: isize,
    gaussian_window: Vec<f64>
}

impl ZoomingAlgorithm for ZoomDynamic {
    fn get_debug_points(&self) -> BTreeMap<i64, Vec<(f64, f64)>> { self.fov_estimator.get_debug_points() }

    fn compute(&self, timestamps: &[f64], keyframes: &KeyframeManager) -> Vec<(f64, Point2D)> {
        if timestamps.is_empty() {
            return Vec::new();
        }

        let (mut fov_values, center_position) = self.fov_estimator.compute(timestamps, (self.compute_params.trim_start, self.compute_params.trim_end));

        if keyframes.is_keyframed(&KeyframeType::ZoomingSpeed) {
            // Keyframed window
            let mut max_window = 0;
            let data_per_timestamp = timestamps.iter().map(|ts| {
                let window = keyframes.value_at_video_timestamp(&KeyframeType::ZoomingSpeed, *ts).unwrap_or(1.0);
                let frames = self.get_frames_per_window(window);
                if frames > max_window { max_window = frames; }
                DataPerTimestamp {
                    frames,
                    half_frames: (frames / 2) as isize,
                    gaussian_window: gaussian_window_normalized(frames, frames as f64 / 6.0)
                }
            }).collect::<Vec<_>>();

            let max_window_half = max_window / 2;

            let fov_values_pad = pad_edge(&fov_values, (max_window_half, max_window_half));
            let fov_min = min_rolling_dynamic(&fov_values_pad, max_window_half as isize, &data_per_timestamp);
            let fov_min_pad = pad_edge(&fov_min, (max_window_half, max_window_half));

            fov_values = convolve_dynamic(&fov_min_pad, max_window_half as isize, &data_per_timestamp);
        } else {
            // Static window
            let frames = self.get_frames_per_window(self.window);

            let fov_values_pad = pad_edge(&fov_values, (frames / 2, frames / 2));
            let fov_min = min_rolling(&fov_values_pad, frames);
            let fov_min_pad = pad_edge(&fov_min, (frames / 2, frames / 2));

            let gaussian = gaussian_window_normalized(frames, frames as f64 / 6.0);
            fov_values = convolve(&fov_min_pad, &gaussian);
        }

        fov_values.iter().copied().zip(center_position.iter().copied()).collect()
    }

    fn compute_params(&self) -> &ComputeParams {
        &self.compute_params
    }

    fn hash(&self, hasher: &mut dyn Hasher) {
        hasher.write_u64(self.window.to_bits());
    }
}

impl ZoomDynamic {
    pub fn new(window: f64, fov_estimator: Box<dyn FieldOfViewAlgorithm>, compute_params: ComputeParams) -> Self {
        Self {
            window,
            fov_estimator,
            compute_params,
        }
    }

    fn get_frames_per_window(&self, window: f64) -> usize {
        let mut frames = (window * self.compute_params.scaled_fps).floor() as usize;
        if frames % 2 == 0 {
            frames += 1;
        }
        frames
    }
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
    n.iter().map(|&v| (-v).powi(2) / sig2).collect()
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

// Dynamic windows

fn min_rolling_dynamic(a: &[f64], max_window_half: isize, data_per_timestamp: &[DataPerTimestamp]) -> Vec<f64> {
    let mut ret = Vec::with_capacity(a.len());

    for (di, data) in data_per_timestamp.iter().enumerate() {
        let i = di as isize + (max_window_half - data.half_frames);
        if i >= 0 && i as usize + data.frames <= a.len() {
            let i = i as usize;
            let window = &a[i..i + data.frames];
            ret.push(window.iter().copied().reduce(f64::min).unwrap())
        } else {
            log::error!("Something went wrong i: {i}, a.len: {}, frames: {}", a.len(), data.frames);
        }
    }
    ret
}

fn convolve_dynamic(a: &[f64], max_window_half: isize, data_per_timestamp: &[DataPerTimestamp]) -> Vec<f64> {
    let mut ret = Vec::with_capacity(a.len());
    for (di, data) in data_per_timestamp.iter().enumerate() {
        let i = di as isize + (max_window_half - data.half_frames);
        if i >= 0 && i as usize + data.frames <= a.len() {
            let i = i as usize;
            let window = &a[i..i + data.frames];
            let filter = &data.gaussian_window;
            if window.len() == filter.len() {
                ret.push(window.iter().zip(filter).map(|(x, y)| x * y).sum());
            } else {
                log::error!("Something went wrong window.len: {}, filter.len: {}", window.len(), filter.len());
            }
        } else {
            log::error!("Something went wrong i: {i}, a.len: {}, frames: {}", a.len(), data.frames);
        }
    }
    ret
}
