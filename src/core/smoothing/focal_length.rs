// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

/// Apply Gaussian smoothing to focal length data.
///
/// Used for the short-kernel dequantization pass that kills metadata quantization stairs
/// before the main adaptive filter runs.
pub fn smooth_focal_lengths_gaussian(focal_lengths: &[Option<f64>], strength: f64, window_size: usize) -> Vec<Option<f64>> {
    if focal_lengths.is_empty() || strength <= 0.0 {
        return focal_lengths.to_vec();
    }

    let window_size = if window_size % 2 == 0 { window_size + 1 } else { window_size };
    let half_window = window_size / 2;

    let sigma = (window_size as f64 / 6.0) * (1.0 + strength * 2.0);
    let mut kernel = vec![0.0; window_size];
    let mut kernel_sum = 0.0;

    for i in 0..window_size {
        let x = i as f64 - half_window as f64;
        kernel[i] = (-x * x / (2.0 * sigma * sigma)).exp();
        kernel_sum += kernel[i];
    }
    for k in &mut kernel {
        *k /= kernel_sum;
    }

    let mut smoothed = Vec::with_capacity(focal_lengths.len());
    for i in 0..focal_lengths.len() {
        if focal_lengths[i].is_none() {
            smoothed.push(None);
            continue;
        }

        let mut weighted_sum = 0.0;
        let mut weight_sum = 0.0;
        for j in 0..window_size {
            let idx = (i as isize + j as isize - half_window as isize).max(0).min(focal_lengths.len() as isize - 1) as usize;
            if let Some(fl) = focal_lengths[idx] {
                weighted_sum += fl * kernel[j];
                weight_sum += kernel[j];
            }
        }

        if weight_sum > 0.0 {
            let smoothed_value = weighted_sum / weight_sum;
            if let Some(original) = focal_lengths[i] {
                smoothed.push(Some(original * (1.0 - strength) + smoothed_value * strength));
            } else {
                smoothed.push(Some(smoothed_value));
            }
        } else {
            smoothed.push(focal_lengths[i]);
        }
    }

    smoothed
}

/// Velocity-adaptive exponential smoothing for focal length.
///
/// Same idea as [`default_algo`](crate::smoothing::default_algo): at low velocity use a long
/// time constant (heavy smoothing, kills jitter), at high velocity use a short time constant
/// (light smoothing, tracks the real zoom without lag). Two-pass exponential filter (forward
/// + backward) cancels phase shift — the output is aligned in time with the input.
///
/// * `max_smoothness_time` — time constant (seconds) applied at low velocity.
/// * `min_smoothness_time` — time constant (seconds) applied at high velocity.
/// * `max_velocity` — relative velocity (1/s) above which the filter fully switches to
///   `min_smoothness_time`. Larger = filter stays at `max_smoothness_time` even during
///   deliberate zooms (smoother output, some lag). Smaller = opens up on any motion.
pub fn smooth_focal_lengths_adaptive(
    focal_lengths: &[Option<f64>],
    fps: f64,
    max_smoothness_time: f64,
    min_smoothness_time: f64,
    max_velocity: f64,
) -> Vec<Option<f64>> {
    if focal_lengths.len() < 2 || fps <= 0.0 {
        return focal_lengths.to_vec();
    }

    let dt = 1.0 / fps;
    let alpha_max = 1.0 - (-dt / max_smoothness_time.max(1e-3)).exp();
    let alpha_min = 1.0 - (-dt / min_smoothness_time.max(1e-3)).exp();

    let n = focal_lengths.len();

    // Relative velocity per sample. Using a relative rate (delta / value) keeps the threshold
    // meaningful across different lenses: a 1mm change on an 18mm lens is a bigger event than
    // the same 1mm change on a 200mm lens.
    let mut velocity = vec![0.0f64; n];
    for i in 1..n {
        if let (Some(prev), Some(curr)) = (focal_lengths[i - 1], focal_lengths[i]) {
            if prev > 0.0 {
                velocity[i] = ((curr - prev) * fps / prev).abs();
            }
        }
    }
    velocity[0] = velocity.get(1).copied().unwrap_or(0.0);

    // Smooth the velocity signal itself so a single noisy sample doesn't flip the alpha.
    for i in 1..n {
        velocity[i] = velocity[i - 1] * (1.0 - alpha_min) + velocity[i] * alpha_min;
    }
    for i in (0..n.saturating_sub(1)).rev() {
        velocity[i] = velocity[i + 1] * (1.0 - alpha_min) + velocity[i] * alpha_min;
    }

    // Per-sample alpha: interpolate between alpha_max (slow motion → small alpha, heavy
    // smoothing) and alpha_min (fast motion → larger alpha, light smoothing).
    let per_sample_alpha = |i: usize| -> f64 {
        let ratio = if max_velocity > 1e-6 { (velocity[i] / max_velocity).min(1.0) } else { 1.0 };
        alpha_max * (1.0 - ratio) + alpha_min * ratio
    };

    // Locate first valid sample to seed the filter state.
    let Some((start_idx, seed)) = focal_lengths.iter().enumerate().find_map(|(i, v)| v.map(|x| (i, x))) else {
        return focal_lengths.to_vec();
    };

    // Forward pass.
    let mut smoothed = vec![None; n];
    let mut state = seed;
    for i in start_idx..n {
        if let Some(x) = focal_lengths[i] {
            let a = per_sample_alpha(i);
            state = state * (1.0 - a) + x * a;
        }
        // For gaps (None), hold the last state — the backward pass will pick it back up.
        smoothed[i] = Some(state);
    }

    // Backward pass. Seed from the last forward-pass state.
    let mut state = smoothed[n - 1].unwrap_or(seed);
    for i in (start_idx..n).rev() {
        if let Some(x) = smoothed[i] {
            let a = per_sample_alpha(i);
            state = state * (1.0 - a) + x * a;
            smoothed[i] = Some(state);
        }
    }

    smoothed
}
