// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

//! Vibration envelopes used by the auto-sync.
//!
//! The idea: the propellers produce a vibration that shows up in both signals - as
//! sound picked up by the microphone and as oscillation read by the gyroscope. When
//! the rotation changes, the intensity of that vibration changes in both.
//! Correlating the two intensity curves reveals the time shift between them.
//!
//! # Why energy envelopes and not the spectra
//!
//! The gyroscope samples at a relatively low rate `R` (200-1000 Hz typical), so it
//! only sees vibration up to `R/2` - the Nyquist limit. The blade passing frequency
//! is usually above that, and what the gyro records is its aliasing, at an apparent
//! frequency different from the real one.
//!
//! Comparing the spectra directly would therefore not work: the frequencies do not
//! match. But the vibration intensity over time survives the aliasing - when the
//! propellers spin up, energy rises in both signals, each in its own band. That is
//! why we correlate energy envelopes.

use std::f32::consts::PI;

use rustfft::{num_complex::Complex, FftPlanner};

/// STFT window size, in samples.
pub const STFT_WINDOW: usize = 2048;
/// Step between consecutive windows.
pub const STFT_HOP: usize = 512;

/// Default blade passing band, in Hz.
///
/// Covers the fundamental and the first harmonics of most consumer drones.
pub const DEFAULT_BAND_LO_HZ: f32 = 150.0;
pub const DEFAULT_BAND_HI_HZ: f32 = 900.0;

/// High-pass cutoff applied to the gyro, in Hz.
///
/// Removes intentional motion (pan, tilt, turns) and the DC level, leaving only the
/// vibration.
pub const DEFAULT_HIGHPASS_HZ: f32 = 30.0;

/// Below this the spectrum is ignored in the automatic band search: it is the range
/// of wind and handling noise, which do not carry the blade signature.
const AUTO_BAND_MIN_HZ: f32 = 80.0;

/// Relative band width around the peak, in the automatic detection.
const AUTO_BAND_WIDTH: f32 = 0.4;

/// Parameters of the envelope extraction.
///
/// Exposed in the `.gyroflow` so drones with an unusual frequency, or low-rate
/// gyros, can be adjusted without recompiling.
#[derive(Debug, Clone, Copy)]
pub struct FeatureParams {
    /// Start of the fixed band, in Hz.
    pub band_lo_hz: f32,
    /// End of the fixed band, in Hz.
    pub band_hi_hz: f32,
    /// If `true`, the band is detected from the signal itself and the limits above
    /// only serve as fallback.
    pub auto_band: bool,
    /// High-pass cutoff applied to the gyro, in Hz.
    pub highpass_hz: f32,
}

impl Default for FeatureParams {
    fn default() -> Self {
        Self {
            band_lo_hz: DEFAULT_BAND_LO_HZ,
            band_hi_hz: DEFAULT_BAND_HI_HZ,
            auto_band: true,
            highpass_hz: DEFAULT_HIGHPASS_HZ,
        }
    }
}

/// Hann window of `n` points.
///
/// Smooths the edges of each STFT window, keeping the artificial discontinuity of
/// the cut from spreading energy across the whole spectrum.
fn hann_window(n: usize) -> Vec<f32> {
    (0..n).map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / n as f32).cos())).collect()
}

/// Logarithmic compression.
///
/// Vibration energy varies by orders of magnitude; the log brings the scales
/// together and keeps a single peak from dominating the correlation.
fn log_compress(value: f32) -> f32 {
    (value + 1e-9).ln()
}

/// Average spectrum of the signal over time.
///
/// Used by the automatic band detection: the blade signature is persistent, so it
/// shows up in the average even when isolated events are more intense.
fn average_spectrum(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if samples.len() < STFT_WINDOW {
        return Vec::new();
    }

    let window = hann_window(STFT_WINDOW);
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(STFT_WINDOW);

    let bins = STFT_WINDOW / 2;
    let mut accum = vec![0.0f32; bins];
    let mut frames = 0usize;

    let mut buffer = vec![Complex::new(0.0f32, 0.0f32); STFT_WINDOW];

    let mut pos = 0;
    while pos + STFT_WINDOW <= samples.len() {
        for i in 0..STFT_WINDOW {
            buffer[i] = Complex::new(samples[pos + i] * window[i], 0.0);
        }
        fft.process(&mut buffer);

        for (bin, acc) in accum.iter_mut().enumerate() {
            *acc += buffer[bin].norm_sqr();
        }
        frames += 1;
        pos += STFT_HOP;
    }

    let _ = sample_rate;
    if frames > 0 {
        for value in &mut accum {
            *value /= frames as f32;
        }
    }
    accum
}

/// Finds the dominant vibration band from the signal itself.
///
/// Returns `(lo_hz, hi_hz)` around the peak of the average spectrum, ignoring
/// anything below [`AUTO_BAND_MIN_HZ`] - without assuming any known RPM.
///
/// Returns `None` when the signal is too short for a reliable estimate; the caller
/// then falls back to the fixed band.
pub fn detect_band(samples: &[f32], sample_rate: u32) -> Option<(f32, f32)> {
    let spectrum = average_spectrum(samples, sample_rate);
    if spectrum.is_empty() {
        return None;
    }

    let hz_per_bin = sample_rate as f32 / STFT_WINDOW as f32;
    let min_bin = (AUTO_BAND_MIN_HZ / hz_per_bin).ceil() as usize;
    if min_bin >= spectrum.len() {
        return None;
    }

    let (peak_bin, _) = spectrum[min_bin..]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;

    let peak_hz = (min_bin + peak_bin) as f32 * hz_per_bin;
    if peak_hz <= 0.0 {
        return None;
    }

    Some((peak_hz * (1.0 - AUTO_BAND_WIDTH), peak_hz * (1.0 + AUTO_BAND_WIDTH)))
}

/// Energy envelope of the audio in the blade band.
///
/// Works on the mono analysis downmix - never on the export audio. Returns the
/// envelope and its rate in Hz (`sample_rate / STFT_HOP`).
pub fn audio_envelope(mono: &[f32], sample_rate: u32, params: &FeatureParams) -> (Vec<f32>, f32) {
    if mono.len() < STFT_WINDOW || sample_rate == 0 {
        return (Vec::new(), 0.0);
    }

    // The automatic band falls back to the fixed one when the signal does not allow
    // an estimate, for example when auto-band locks onto wind.
    let (band_lo, band_hi) = if params.auto_band {
        detect_band(mono, sample_rate).unwrap_or((params.band_lo_hz, params.band_hi_hz))
    } else {
        (params.band_lo_hz, params.band_hi_hz)
    };

    let hz_per_bin = sample_rate as f32 / STFT_WINDOW as f32;
    let bin_lo = ((band_lo / hz_per_bin).floor() as usize).max(1);
    let bin_hi = ((band_hi / hz_per_bin).ceil() as usize).min(STFT_WINDOW / 2);
    if bin_hi <= bin_lo {
        return (Vec::new(), 0.0);
    }

    let window = hann_window(STFT_WINDOW);
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(STFT_WINDOW);

    let mut envelope = Vec::with_capacity(mono.len() / STFT_HOP + 1);
    let mut buffer = vec![Complex::new(0.0f32, 0.0f32); STFT_WINDOW];

    let mut pos = 0;
    while pos + STFT_WINDOW <= mono.len() {
        for i in 0..STFT_WINDOW {
            buffer[i] = Complex::new(mono[pos + i] * window[i], 0.0);
        }
        fft.process(&mut buffer);

        let energy: f32 = buffer[bin_lo..bin_hi].iter().map(|c| c.norm_sqr()).sum();
        envelope.push(log_compress(energy));

        pos += STFT_HOP;
    }

    (envelope, sample_rate as f32 / STFT_HOP as f32)
}

/// First-order high-pass filter.
///
/// Deliberately simple: the goal is to remove the slow component of intentional
/// motion, not to build a filter with an accurate response. What matters for the
/// correlation is the resulting envelope, not band fidelity.
fn highpass(signal: &[f32], sample_rate: f32, cutoff_hz: f32) -> Vec<f32> {
    if signal.is_empty() || sample_rate <= 0.0 {
        return Vec::new();
    }
    let rc = 1.0 / (2.0 * PI * cutoff_hz);
    let dt = 1.0 / sample_rate;
    let alpha = rc / (rc + dt);

    let mut out = Vec::with_capacity(signal.len());
    let mut prev_in = signal[0];
    let mut prev_out = 0.0;
    out.push(0.0);
    for &sample in &signal[1..] {
        let value = alpha * (prev_out + sample - prev_in);
        out.push(value);
        prev_out = value;
        prev_in = sample;
    }
    out
}

/// Energy envelope of the vibration read by the gyroscope.
///
/// - `gyro` are the `(timestamp_ms, [x, y, z])` samples in `f64`, as
///   `telemetry-parser` delivers them.
/// - `target_rate_hz` is the rate of the audio envelope, so both curves share a
///   time base and can be correlated.
///
/// The signal used is the magnitude `sqrt(x^2 + y^2 + z^2)`, which is independent of
/// the drone orientation.
pub fn gyro_envelope(gyro: &[(f64, [f64; 3])], target_rate_hz: f32, params: &FeatureParams) -> Vec<f32> {
    if gyro.len() < 2 || target_rate_hz <= 0.0 {
        return Vec::new();
    }

    let first_ms = gyro[0].0;
    let last_ms = gyro[gyro.len() - 1].0;
    let duration_s = (last_ms - first_ms) / 1000.0;
    if duration_s <= 0.0 {
        return Vec::new();
    }

    // Actual gyro sampling rate, derived from the timestamps: there is no ready-made
    // field for it in the core.
    let gyro_rate = (gyro.len() as f64 / duration_s) as f32;

    let magnitude: Vec<f32> = gyro
        .iter()
        .map(|(_, v)| ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()) as f32)
        .collect();

    let filtered = highpass(&magnitude, gyro_rate, params.highpass_hz);

    // Energy per window, with the window sized so the envelope comes out at exactly
    // the audio envelope rate.
    let samples_per_bucket = (gyro_rate / target_rate_hz).round().max(1.0) as usize;
    let bucket_count = filtered.len() / samples_per_bucket;

    let mut envelope = Vec::with_capacity(bucket_count);
    for bucket in 0..bucket_count {
        let from = bucket * samples_per_bucket;
        let to = (from + samples_per_bucket).min(filtered.len());
        let energy: f32 = filtered[from..to].iter().map(|v| v * v).sum();
        envelope.push(log_compress(energy));
    }

    envelope
}

// ---------------------------------------------------------------------------
// "Onset" mode: alignment by the start of motion
// ---------------------------------------------------------------------------
//
// The blade band method needs raw gyroscope data at a high rate. Cameras such as
// the DJI O4P do not expose that - they deliver only a few dozen already integrated
// and smoothed quaternions, typically below 1 Hz. The propeller vibration simply
// does not exist in that signal.
//
// But there is another event present on both sides: the start of motion. When the
// drone takes off, the gyro records rotation and the microphone records the motors
// spinning up. Correlating those two "how much is happening" curves works even at a
// low rate, because the event lasts seconds, not milliseconds.

/// Total energy envelope of the audio, with no band filter.
///
/// Unlike [`audio_envelope`], which isolates the blade band, here the whole signal
/// matters: what marks the takeoff is the overall level rising, not a specific
/// frequency.
///
/// Returns the envelope and its rate in Hz.
pub fn audio_energy_envelope(mono: &[f32], sample_rate: u32, target_rate_hz: f32) -> (Vec<f32>, f32) {
    if mono.is_empty() || sample_rate == 0 || target_rate_hz <= 0.0 {
        return (Vec::new(), 0.0);
    }

    let samples_per_bucket = (sample_rate as f32 / target_rate_hz).round().max(1.0) as usize;
    let bucket_count = mono.len() / samples_per_bucket;
    if bucket_count < 2 {
        return (Vec::new(), 0.0);
    }

    let mut envelope = Vec::with_capacity(bucket_count);
    for bucket in 0..bucket_count {
        let from = bucket * samples_per_bucket;
        let to = (from + samples_per_bucket).min(mono.len());
        let energy: f32 = mono[from..to].iter().map(|v| v * v).sum();
        envelope.push(log_compress(energy / (to - from) as f32));
    }

    (envelope, sample_rate as f32 / samples_per_bucket as f32)
}

/// Motion intensity envelope of the drone.
///
/// Uses the angular velocity magnitude without a high-pass: here the intentional
/// motion is exactly what matters, not noise to be removed.
///
/// `target_rate_hz` should be the audio envelope rate, so both curves share a time
/// base.
pub fn gyro_motion_envelope(gyro: &[(f64, [f64; 3])], target_rate_hz: f32) -> Vec<f32> {
    if gyro.len() < 2 || target_rate_hz <= 0.0 {
        return Vec::new();
    }

    let first_ms = gyro[0].0;
    let last_ms = gyro[gyro.len() - 1].0;
    let duration_s = (last_ms - first_ms) / 1000.0;
    if duration_s <= 0.0 {
        return Vec::new();
    }

    let bucket_count = (duration_s as f32 * target_rate_hz).round().max(2.0) as usize;
    let bucket_duration_s = duration_s / bucket_count as f64;

    // Resample by interpolation: with few samples (dozens of quaternions for dozens
    // of seconds), grouping by index would leave empty buckets.
    let mut envelope = Vec::with_capacity(bucket_count);
    for bucket in 0..bucket_count {
        let t_ms = first_ms + (bucket as f64 + 0.5) * bucket_duration_s * 1000.0;

        // Same linear interpolation by timestamp used in
        // synchronization/optimsync.rs:40-51.
        let idx = gyro.partition_point(|(t, _)| *t < t_ms);
        let magnitude = if idx == 0 {
            let v = gyro[0].1;
            (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
        } else if idx >= gyro.len() {
            let v = gyro[gyro.len() - 1].1;
            (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
        } else {
            let (t0, v0) = gyro[idx - 1];
            let (t1, v1) = gyro[idx];
            let span = t1 - t0;
            let f = if span > 0.0 { (t_ms - t0) / span } else { 0.0 };
            let m0 = (v0[0] * v0[0] + v0[1] * v0[1] + v0[2] * v0[2]).sqrt();
            let m1 = (v1[0] * v1[0] + v1[1] * v1[1] + v1[2] * v1[2]).sqrt();
            m0 + (m1 - m0) * f
        };

        envelope.push(log_compress(magnitude as f32));
    }

    envelope
}

/// Emphasizes the transitions of an envelope, discarding the absolute level.
///
/// Onset alignment does not compare how loud each signal is - the scales of a
/// microphone and a gyroscope are unrelated - but when each one changes.
/// Differentiating and keeping only the increases isolates those instants: the
/// takeoff shows up as a peak in both.
///
/// Known in audio processing as half-wave rectified spectral flux.
pub fn onset_strength(envelope: &[f32]) -> Vec<f32> {
    if envelope.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(envelope.len());
    out.push(0.0);
    for i in 1..envelope.len() {
        // Increases only: energy drops do not mark the start of an event.
        out.push((envelope[i] - envelope[i - 1]).max(0.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sine at `freq_hz` with an amplitude that varies over time.
    fn tone(freq_hz: f32, sample_rate: u32, seconds: f32, amplitude: impl Fn(f32) -> f32) -> Vec<f32> {
        let n = (sample_rate as f32 * seconds) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                amplitude(t) * (2.0 * PI * freq_hz * t).sin()
            })
            .collect()
    }

    #[test]
    fn envelope_follows_the_intensity_variation() {
        // A 400 Hz tone that gets louder in the second half.
        let sr = 8000;
        let signal = tone(400.0, sr, 2.0, |t| if t < 1.0 { 0.1 } else { 1.0 });
        let params = FeatureParams { auto_band: false, ..Default::default() };
        let (env, rate) = audio_envelope(&signal, sr, &params);

        assert!(!env.is_empty());
        assert!((rate - sr as f32 / STFT_HOP as f32).abs() < 0.01);

        // The energy in the second half must be clearly higher.
        let mid = env.len() / 2;
        let first_half: f32 = env[..mid].iter().sum::<f32>() / mid as f32;
        let second_half: f32 = env[mid..].iter().sum::<f32>() / (env.len() - mid) as f32;
        assert!(second_half > first_half + 1.0, "first={first_half}, second={second_half}");
    }

    #[test]
    fn automatic_detection_finds_the_dominant_tone() {
        let sr = 8000;
        let signal = tone(500.0, sr, 2.0, |_| 1.0);
        let (lo, hi) = detect_band(&signal, sr).expect("should detect the band");
        // The band must contain the real 500 Hz.
        assert!(lo < 500.0 && hi > 500.0, "detected band: {lo}..{hi}");
    }

    #[test]
    fn band_ignores_the_wind_range() {
        let sr = 8000;
        // Strong rumble at 30 Hz plus a weaker signature at 600 Hz.
        let mut signal = tone(30.0, sr, 2.0, |_| 3.0);
        let blades = tone(600.0, sr, 2.0, |_| 1.0);
        for (s, b) in signal.iter_mut().zip(blades.iter()) {
            *s += *b;
        }
        let (lo, hi) = detect_band(&signal, sr).expect("should detect the band");
        // Even though the rumble is stronger, the band must land on the 600 Hz.
        assert!(lo > AUTO_BAND_MIN_HZ, "band started inside the wind: {lo}");
        assert!(lo < 600.0 && hi > 600.0, "detected band: {lo}..{hi}");
    }

    #[test]
    fn short_signal_produces_no_envelope() {
        let (env, rate) = audio_envelope(&[0.0; 100], 8000, &FeatureParams::default());
        assert!(env.is_empty());
        assert_eq!(rate, 0.0);
    }

    #[test]
    fn gyro_envelope_comes_out_at_the_requested_rate() {
        // 10 s of gyro at 500 Hz, with vibration that doubles halfway through.
        let gyro_rate = 500.0;
        let n = 5000;
        let gyro: Vec<(f64, [f64; 3])> = (0..n)
            .map(|i| {
                let t = i as f64 / gyro_rate;
                let amp = if t < 5.0 { 0.1 } else { 1.0 };
                let v = amp * (2.0 * std::f64::consts::PI * 80.0 * t).sin();
                (t * 1000.0, [v, 0.0, 0.0])
            })
            .collect();

        let target_rate = 15.625; // 8000/512
        let env = gyro_envelope(&gyro, target_rate, &FeatureParams::default());

        assert!(!env.is_empty());
        // ~10 s at 15.625 Hz is about 156 points, with slack for rounding.
        assert!(env.len() > 100 && env.len() < 200, "length={}", env.len());

        let mid = env.len() / 2;
        let first: f32 = env[..mid].iter().sum::<f32>() / mid as f32;
        let second: f32 = env[mid..].iter().sum::<f32>() / (env.len() - mid) as f32;
        assert!(second > first, "first={first}, second={second}");
    }

    #[test]
    fn onset_marks_the_instant_of_the_rise() {
        // Envelope that jumps at position 5 and then falls.
        let env = vec![0.0, 0.0, 0.0, 0.0, 0.0, 5.0, 5.0, 5.0, 1.0, 1.0];
        let onset = onset_strength(&env);

        // The onset peak must land exactly on the transition.
        let (peak_idx, _) = onset.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
        assert_eq!(peak_idx, 5, "onset={onset:?}");

        // The drop at 8 must not become a peak: only increases count.
        assert_eq!(onset[8], 0.0);
    }

    #[test]
    fn motion_envelope_works_with_few_samples() {
        // The DJI O4P case: ~30 quaternions for ~45 s of video. The drone stays
        // still and then moves.
        let gyro: Vec<(f64, [f64; 3])> = (0..30)
            .map(|i| {
                let t_ms = i as f64 * 1500.0; // one sample every 1.5 s
                let v = if i < 15 { 0.001 } else { 0.5 }; // still, then moving
                (t_ms, [v, 0.0, 0.0])
            })
            .collect();

        // Even asking for 15 Hz out of 0.66 Hz of data, the interpolation fills the
        // grid without leaving holes.
        let env = gyro_motion_envelope(&gyro, 15.0);
        assert!(env.len() > 100, "length={}", env.len());

        let mid = env.len() / 2;
        let before: f32 = env[..mid].iter().sum::<f32>() / mid as f32;
        let after: f32 = env[mid..].iter().sum::<f32>() / (env.len() - mid) as f32;
        assert!(after > before + 1.0, "before={before}, after={after}");

        // And the onset marks the transition near the middle.
        let onset = onset_strength(&env);
        let (peak, _) = onset.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
        let relative_error = (peak as f32 - mid as f32).abs() / env.len() as f32;
        assert!(relative_error < 0.15, "peak at {peak}, expected near {mid}");
    }

    #[test]
    fn audio_energy_ignores_the_band() {
        let sr = 8000;
        // Silence and then sound: what matters is the level, not the frequency.
        let mut signal = vec![0.0f32; sr as usize];
        signal.extend(tone(300.0, sr, 1.0, |_| 0.8));

        let (env, rate) = audio_energy_envelope(&signal, sr, 15.0);
        assert!(!env.is_empty() && rate > 0.0);

        let mid = env.len() / 2;
        assert!(env[mid + 5] > env[mid - 5] + 1.0, "the rise should show up");
    }

    #[test]
    fn highpass_removes_the_dc_level() {
        // Constant signal: only DC, no vibration.
        let dc = vec![5.0f32; 1000];
        let filtered = highpass(&dc, 500.0, 30.0);
        // After the initial transient the result must stay near zero.
        assert!(filtered[500..].iter().all(|v| v.abs() < 0.1));
    }
}
