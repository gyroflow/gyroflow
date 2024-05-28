// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021 Marc Roeschlin, Adrian, Maik

use super::*;
use crate::keyframes::*;

struct DataPerTimestamp {
    fps: f64,
    window: f64,
    frames: usize,
    half_frames: isize,
    gaussian_window: Vec<f64>
}

pub fn compute(compute_params: &ComputeParams, mut fov_values: Vec<f64>, timestamps: &[f64], method: ZoomMethod) -> (Vec<f64>, Vec<f64>) {
    let window = compute_params.adaptive_zoom_window;

    let fov_minimal = fov_values.clone();

    let keyframes = &compute_params.keyframes;

    if keyframes.is_keyframed(&KeyframeType::ZoomingSpeed) || (compute_params.video_speed_affects_zooming && (compute_params.video_speed != 1.0 || keyframes.is_keyframed(&KeyframeType::VideoSpeed))) {
        // Keyframed window
        let mut max_window = 0;
        let data_per_timestamp = timestamps.iter().map(|ts| {
            let mut window = keyframes.value_at_video_timestamp(&KeyframeType::ZoomingSpeed, *ts).unwrap_or(window);
            if compute_params.video_speed_affects_zooming {
                let vid_speed = keyframes.value_at_video_timestamp(&KeyframeType::VideoSpeed, *ts).unwrap_or(compute_params.video_speed);
                window *= vid_speed;
            }
            let frames = get_frames_per_window(compute_params);
            if frames > max_window { max_window = frames; }
            DataPerTimestamp {
                window,
                fps: compute_params.scaled_fps,
                frames,
                half_frames: (frames / 2) as isize,
                gaussian_window: gaussian_window_normalized(frames, frames as f64 / 6.0)
            }
        }).collect::<Vec<_>>();

        match method {
            ZoomMethod::GaussianFilter => {
                let max_window_half = max_window / 2;
                let fov_values_pad = pad_edge(&fov_values, (max_window_half, max_window_half));
                let fov_min = min_rolling_dynamic(&fov_values_pad, max_window_half as isize, &data_per_timestamp);
                let fov_min_pad = pad_edge(&fov_min, (max_window_half, max_window_half));
                fov_values = convolve_dynamic(&fov_min_pad, max_window_half as isize, &data_per_timestamp);
            },
            ZoomMethod::EnvelopeFollower => {
                let second_pass_alpha = 1.0 - (-(1.0 / compute_params.scaled_fps) / 0.2).exp();
                fov_values = envelope_follower(&fov_values, &data_per_timestamp, None);
                fov_values = envelope_follower(&fov_values, &data_per_timestamp, Some(second_pass_alpha));
            }
        }
    } else {
        match method {
            ZoomMethod::GaussianFilter => {
                // Static window
                let frames = get_frames_per_window(compute_params);

                let fov_values_pad = pad_edge(&fov_values, (frames / 2, frames / 2));
                let fov_min = min_rolling(&fov_values_pad, frames);
                let fov_min_pad = pad_edge(&fov_min, (frames / 2, frames / 2));

                let gaussian = gaussian_window_normalized(frames, frames as f64 / 6.0);
                fov_values = convolve(&fov_min_pad, &gaussian);
            },
            ZoomMethod::EnvelopeFollower => {
                let first_pass_alpha  = 1.0 - (-(1.0 / compute_params.scaled_fps) / window).exp();
                let second_pass_alpha = 1.0 - (-(1.0 / compute_params.scaled_fps) / 0.2).exp();

                fov_values = envelope_follower(&fov_values, &[], Some(first_pass_alpha));
                fov_values = envelope_follower(&fov_values, &[], Some(second_pass_alpha));
            }
        }
    }

    (fov_values, fov_minimal)
}

fn get_frames_per_window(compute_params: &ComputeParams) -> usize {
    let mut frames = (compute_params.adaptive_zoom_window * compute_params.scaled_fps).floor() as usize;
    if frames % 2 == 0 {
        frames += 1;
    }
    frames
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

fn gaussian_window(width: isize, std: f64) -> Vec<f64> {
    let sig2 = 2.0 * std.powi(2);
    (-width / 2..=width / 2).map(|x| (-(x.pow(2) as f64) / sig2).exp()).collect()
}

fn gaussian_window_normalized(m: usize, std: f64) -> Vec<f64> {
    let mut w = gaussian_window(m as isize, std);
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

fn envelope_follower(a: &[f64], data_per_timestamp: &[DataPerTimestamp], alpha: Option<f64>) -> Vec<f64> {
    if a.is_empty() { return Vec::new(); }

    let alphas = if let Some(alpha) = alpha {
        vec![alpha; a.len()]
    } else {
        data_per_timestamp.iter().map(|dpt| {
            1.0 - (-(1.0 / dpt.fps) / dpt.window).exp()
        }).collect::<Vec<_>>()
    };

    let mut q = *a.iter().next_back().unwrap();
    let smoothed_rev = a.iter().zip(&alphas).rev().map(|(&x, coeff)| {
        q = x.min(x * coeff + q * (1.0-coeff));
        q
    }).collect::<Vec<_>>();

    let mut q = *smoothed_rev.iter().next_back().unwrap();
    let smoothed2 = smoothed_rev.iter().rev().zip(&alphas).map(|(&x, coeff)| {
        q = x.min(x * coeff + q * (1.0-coeff));
        q
    }).collect::<Vec<_>>();

    smoothed2
}
