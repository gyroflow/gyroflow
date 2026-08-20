// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

//! Cross-correlation between the audio and gyroscope vibration envelopes.
//!
//! Takes the two intensity curves produced by [`super::features`] and returns the
//! time shift that best aligns them, along with a confidence score.
//!
//! The correlation goes through an FFT because the direct version is O(n^2): a
//! few minutes of envelope at 15 Hz is tens of thousands of points, and the
//! difference between O(n^2) and O(n log n) is the difference between seconds and
//! milliseconds.
//!
//! The logic here is independent of UI and files: `(audio_env, gyro_env,
//! env_rate_hz)` in, `(offset_seconds, confidence)` out.

use rustfft::{num_complex::Complex, FftPlanner};

/// Result of the automatic alignment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncResult {
    /// Estimated shift in seconds, following `t_audio = t_video + offset`.
    pub offset_seconds: f64,
    /// Match quality, from 0 to 1.
    ///
    /// The normalized correlation coefficient at the peak. Low values mean the
    /// two signals have no clear common signature - for example when the gimbal
    /// isolates the blade vibration, or when the audio was recorded far from the
    /// drone.
    pub confidence: f32,
}

/// Normalizes an envelope to zero mean and unit standard deviation.
///
/// Without this the correlation would be dominated by the absolute amplitude of
/// each signal, which is unrelated between a microphone and a gyroscope.
fn normalize(signal: &[f32]) -> Vec<f32> {
    if signal.is_empty() {
        return Vec::new();
    }
    let mean = signal.iter().sum::<f32>() / signal.len() as f32;
    let variance = signal.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / signal.len() as f32;
    let std_dev = variance.sqrt();

    if std_dev < 1e-9 {
        // Constant signal: carries no alignment information.
        return vec![0.0; signal.len()];
    }
    signal.iter().map(|v| (v - mean) / std_dev).collect()
}

/// Cross-correlates two signals via FFT.
///
/// Returns the full correlation vector, with lags from `-(b.len()-1)` to
/// `+(a.len()-1)`, where index `i` maps to lag `i - (b.len() - 1)`.
fn cross_correlate_fft(a: &[f32], b: &[f32]) -> Vec<f32> {
    let result_len = a.len() + b.len() - 1;
    // The FFT works best with powers of two, and the padding does not change the
    // linear correlation result.
    let fft_len = result_len.next_power_of_two();

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_len);
    let ifft = planner.plan_fft_inverse(fft_len);

    let mut buf_a = vec![Complex::new(0.0f32, 0.0f32); fft_len];
    let mut buf_b = vec![Complex::new(0.0f32, 0.0f32); fft_len];

    for (i, v) in a.iter().enumerate() {
        buf_a[i] = Complex::new(*v, 0.0);
    }
    // `b` goes in reversed: correlation is convolution with the mirrored signal.
    for (i, v) in b.iter().enumerate() {
        buf_b[b.len() - 1 - i] = Complex::new(*v, 0.0);
    }

    fft.process(&mut buf_a);
    fft.process(&mut buf_b);

    // Multiplication in the frequency domain is convolution in the time domain.
    for (x, y) in buf_a.iter_mut().zip(buf_b.iter()) {
        *x *= *y;
    }

    ifft.process(&mut buf_a);

    let scale = 1.0 / fft_len as f32;
    buf_a[..result_len].iter().map(|c| c.re * scale).collect()
}

/// Estimates the shift between the audio and gyroscope envelopes.
///
/// `env_rate_hz` is the common rate of both envelopes, in Hz - it converts the
/// lag in samples into seconds.
///
/// The offset convention matches the rest of the module: `t_audio = t_video +
/// offset`. A positive offset means the event appears later in the audio than in
/// the video.
pub fn cross_correlate(audio_env: &[f32], gyro_env: &[f32], env_rate_hz: f32) -> SyncResult {
    if audio_env.len() < 2 || gyro_env.len() < 2 || env_rate_hz <= 0.0 {
        return SyncResult { offset_seconds: 0.0, confidence: 0.0 };
    }

    let a = normalize(audio_env);
    let g = normalize(gyro_env);

    let correlation = cross_correlate_fft(&a, &g);
    if correlation.is_empty() {
        return SyncResult { offset_seconds: 0.0, confidence: 0.0 };
    }

    let (peak_index, peak_value) = correlation
        .iter()
        .enumerate()
        .max_by(|x, y| x.1.partial_cmp(y.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, v)| (i, *v))
        .unwrap_or((0, 0.0));

    // Index 0 corresponds to the most negative lag possible.
    let lag_samples = peak_index as i64 - (g.len() as i64 - 1);
    let offset_seconds = lag_samples as f64 / env_rate_hz as f64;

    // Normalizing by the number of overlapping points turns the raw sum into the
    // correlation coefficient, which stays in 0..1 and is comparable across clips
    // of different durations.
    let overlap = (a.len().min(g.len())) as f32;
    let confidence = (peak_value / overlap).clamp(0.0, 1.0);

    SyncResult { offset_seconds, confidence }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic envelope with a gaussian pulse at `center`, imitating what
    /// happens when the propellers spin up: a localized energy increase, which is
    /// exactly what the correlation looks for.
    fn pulse(len: usize, center: f32, width: f32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let d = (i as f32 - center) / width;
                (-d * d).exp()
            })
            .collect()
    }

    #[test]
    fn recovers_zero_offset() {
        let signal = pulse(500, 250.0, 20.0);
        let r = cross_correlate(&signal, &signal, 100.0);
        assert!(r.offset_seconds.abs() < 0.02, "offset={}", r.offset_seconds);
        assert!(r.confidence > 0.5, "confidence={}", r.confidence);
    }

    #[test]
    fn recovers_known_positive_offset() {
        // The audio pulse happens 50 samples AFTER the gyro pulse. At a 100 Hz
        // envelope rate that is +0.5 s.
        let audio = pulse(500, 300.0, 20.0);
        let gyro = pulse(500, 250.0, 20.0);
        let r = cross_correlate(&audio, &gyro, 100.0);
        assert!((r.offset_seconds - 0.5).abs() < 0.03, "offset={}", r.offset_seconds);
    }

    #[test]
    fn recovers_known_negative_offset() {
        // Now the audio comes 50 samples BEFORE: -0.5 s.
        let audio = pulse(500, 200.0, 20.0);
        let gyro = pulse(500, 250.0, 20.0);
        let r = cross_correlate(&audio, &gyro, 100.0);
        assert!((r.offset_seconds + 0.5).abs() < 0.03, "offset={}", r.offset_seconds);
    }

    #[test]
    fn confidence_drops_for_unrelated_signals() {
        // Two different patterns, with no common signature.
        let audio: Vec<f32> = (0..500).map(|i| ((i as f32) * 0.7).sin()).collect();
        let gyro: Vec<f32> = (0..500).map(|i| ((i as f32) * 0.013).cos()).collect();

        let matched = cross_correlate(&audio, &audio, 100.0);
        let unmatched = cross_correlate(&audio, &gyro, 100.0);

        assert!(
            unmatched.confidence < matched.confidence,
            "unrelated={}, matched={}",
            unmatched.confidence,
            matched.confidence
        );
    }

    #[test]
    fn constant_signal_produces_no_false_alignment() {
        let flat = vec![1.0f32; 300];
        let signal = pulse(300, 150.0, 10.0);
        let r = cross_correlate(&signal, &flat, 100.0);
        // With no variation in the gyro there is nothing to match.
        assert!(r.confidence < 0.1, "confidence={}", r.confidence);
    }

    #[test]
    fn too_short_input_is_rejected() {
        let r = cross_correlate(&[1.0], &[1.0], 100.0);
        assert_eq!(r.confidence, 0.0);
        assert_eq!(r.offset_seconds, 0.0);
    }

    #[test]
    fn envelope_rate_converts_correctly() {
        let audio = pulse(500, 300.0, 20.0);
        let gyro = pulse(500, 250.0, 20.0);

        // The same 50-sample lag becomes a different offset depending on the
        // envelope rate.
        let r100 = cross_correlate(&audio, &gyro, 100.0);
        let r50 = cross_correlate(&audio, &gyro, 50.0);
        assert!((r100.offset_seconds - 0.5).abs() < 0.03);
        assert!((r50.offset_seconds - 1.0).abs() < 0.06);
    }
}
