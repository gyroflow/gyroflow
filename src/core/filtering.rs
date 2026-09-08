// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use biquad::{Biquad, Coefficients, Type, DirectForm2Transposed, ToHertz};
use nalgebra::UnitQuaternion;

use super::gyro_source::{ TimeIMU, TimeQuat };

pub struct Lowpass {
    filters: [DirectForm2Transposed<f64>; 6]
}

impl Lowpass {
    pub fn new(freq: f64, sample_rate: f64) -> Result<Self, biquad::Errors> {
        let coeffs = Coefficients::<f64>::from_params(Type::LowPass, sample_rate.hz(), freq.hz(), biquad::Q_BUTTERWORTH_F64)?;
        Ok(Self {
            filters: [
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
            ]
        })
    }
    pub fn run(&mut self, i: usize, data: f64) -> f64 {
        self.filters[i].run(data)
    }

    pub fn filter_gyro(&mut self, data: &mut [TimeIMU]) {
        for x in data {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = self.run(0, g[0]);
                g[1] = self.run(1, g[1]);
                g[2] = self.run(2, g[2]);
            }

            if let Some(a) = x.accl.as_mut() {
                a[0] = self.run(3, a[0]);
                a[1] = self.run(4, a[1]);
                a[2] = self.run(5, a[2]);
            }
        }
    }
    pub fn filter_gyro_forward_backward(freq: f64, sample_rate: f64, data: &mut [TimeIMU]) -> Result<(), biquad::Errors> {
        let mut forward = Self::new(freq, sample_rate)?;
        let mut backward = Self::new(freq, sample_rate)?;
        for x in data.iter_mut() {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = forward.run(0, g[0]);
                g[1] = forward.run(1, g[1]);
                g[2] = forward.run(2, g[2]);
            }
            if let Some(a) = x.accl.as_mut() {
                a[0] = forward.run(3, a[0]);
                a[1] = forward.run(4, a[1]);
                a[2] = forward.run(5, a[2]);
            }
        }
        for x in data.iter_mut().rev() {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = backward.run(0, g[0]);
                g[1] = backward.run(1, g[1]);
                g[2] = backward.run(2, g[2]);
            }
            if let Some(a) = x.accl.as_mut() {
                a[0] = backward.run(3, a[0]);
                a[1] = backward.run(4, a[1]);
                a[2] = backward.run(5, a[2]);
            }
        }
        Ok(())
    }
    pub fn filter_quats_forward_backward(freq: f64, sample_rate: f64, data: &mut TimeQuat) -> Result<(), biquad::Errors> {
        let mut forward = Self::new(freq, sample_rate)?;
        let mut backward = Self::new(freq, sample_rate)?;
        for (_ts, uq) in data.iter_mut() {
            let mut q = uq.quaternion().clone();
            q.coords[0] = forward.run(0, q.coords[0]);
            q.coords[1] = forward.run(1, q.coords[1]);
            q.coords[2] = forward.run(2, q.coords[2]);
            q.coords[3] = forward.run(3, q.coords[3]);
            *uq = crate::Quat64::from_quaternion(q);
        }
        for (_ts, uq) in data.iter_mut().rev() {
            let mut q = uq.quaternion().clone();
            q.coords[0] = backward.run(0, q.coords[0]);
            q.coords[1] = backward.run(1, q.coords[1]);
            q.coords[2] = backward.run(2, q.coords[2]);
            q.coords[3] = backward.run(3, q.coords[3]);
            *uq = crate::Quat64::from_quaternion(q);
        }
        Ok(())
    }
}

pub struct Median {
    filters: [median::Filter<f64>; 6]
}

impl Median {
    pub fn new(size: usize, _sample_rate: f64) -> Self {
        Self {
            filters: [
                median::Filter::new(size),
                median::Filter::new(size),
                median::Filter::new(size),
                median::Filter::new(size),
                median::Filter::new(size),
                median::Filter::new(size),
            ]
        }
    }
    pub fn run(&mut self, i: usize, data: f64) -> f64 {
        self.filters[i].consume(data)
    }

    pub fn filter_gyro(&mut self, data: &mut [TimeIMU]) {
        for x in data {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = self.run(0, g[0]);
                g[1] = self.run(1, g[1]);
                g[2] = self.run(2, g[2]);
            }

            if let Some(a) = x.accl.as_mut() {
                a[0] = self.run(3, a[0]);
                a[1] = self.run(4, a[1]);
                a[2] = self.run(5, a[2]);
            }
        }
    }
    pub fn filter_gyro_forward_backward(size: i32, sample_rate: f64, data: &mut [TimeIMU]) {
        let mut forward = Self::new(size as _, sample_rate);
        let mut backward = Self::new(size as _, sample_rate);
        for x in data.iter_mut() {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = forward.run(0, g[0]);
                g[1] = forward.run(1, g[1]);
                g[2] = forward.run(2, g[2]);
            }
            if let Some(a) = x.accl.as_mut() {
                a[0] = forward.run(3, a[0]);
                a[1] = forward.run(4, a[1]);
                a[2] = forward.run(5, a[2]);
            }
        }
        for x in data.iter_mut().rev() {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = backward.run(0, g[0]);
                g[1] = backward.run(1, g[1]);
                g[2] = backward.run(2, g[2]);
            }
            if let Some(a) = x.accl.as_mut() {
                a[0] = backward.run(3, a[0]);
                a[1] = backward.run(4, a[1]);
                a[2] = backward.run(5, a[2]);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Glitch repair
// ---------------------------------------------------------------------------
//
// This is not a frequency-domain filter like `Lowpass`/`Median` - it's an
// outlier-detection + gap-interpolation ("inpainting") pass. Some cameras
// occasionally emit a short burst of corrupt attitude data: the
// rate swings wildly and oscillates ("rings"), unlike a genuine fast rotation
// which tracks its own recent trend even when quick. A low-pass just smears
// such a burst into its neighbours; a median needs a window wide enough to
// also destroy real motion. Instead we:
//   1. compute a HIGH-PASS residual: gyro rate minus a short local trend,
//   2. flag samples whose residual spikes far above the file's own baseline,
//   3. grow each flagged core outward through its decaying "ringdown" tail,
//   4. bridge the bad span with a quaternion SLERP between the last-good and
//      first-good sample around it (the smooth path the camera would have
//      taken), and
//   5. repeat for a couple of passes so smaller anomalies masked by the big
//      ones get caught once the big ones are gone.
// Discovered by Gene Matocha

#[derive(Clone, Copy)]
pub struct GlitchRepairParams {
    pub residual_window: f64,      // Smoothing window (s) the local trend residual is computed against
    pub threshold_multiplier: f64, // Flag samples above N x the file's 99th-percentile residual magnitude
    pub absolute_floor: f64,       // Never flag below this residual (deg/s)
    pub merge_gap: f64,            // Merge flagged samples within N seconds into one core region
    pub expand_multiplier: f64,    // Grow regions while local residual stays above N x baseline
    pub quiet_duration: f64,       // Stop growing once quiet for N seconds
    pub smoothing_window: f64,     // Rolling-max window (s) used while growing regions
    pub max_duration: f64,         // Candidate cores longer than this are treated as possible real motion
    pub force_long_regions: bool,  // Patch regions longer than max_duration anyway
    pub passes: usize,             // Detection passes, recomputing the baseline each time
}
impl Default for GlitchRepairParams {
    fn default() -> Self {
        Self {
            residual_window:      0.04,
            threshold_multiplier: 1.5,
            absolute_floor:       195.0,
            merge_gap:            0.5,
            expand_multiplier:    1.15,
            quiet_duration:       0.05,
            smoothing_window:     0.01,
            max_duration:         2.5,
            force_long_regions:   false,
            passes:               2,
        }
    }
}
impl GlitchRepairParams {
    pub fn from_strength(s: f64) -> Self {
        let s = s.max(0.0);
        let mut p = Self::default();
        p.absolute_floor = 195.0 * 2.0_f64.powf((50.0 - s) / 30.0);  // ~618 (0%) .. 195 (50%) .. ~62 (100%)
        p.max_duration   = 0.75 + s * 0.015;                         // 0.75s (0%) .. 1.5s (50%) .. 2.25s (100%)
        p.passes         = (1 + (s / 33.0).floor() as usize).max(1); // 1 (0%) .. 2 (50%) .. 4 (100%)
        p
    }
}

pub struct GlitchRepair;
impl GlitchRepair {
    /// Detect and repair bad quaternion bursts in place. Returns the number of
    /// samples that were replaced.
    pub fn repair_quats(data: &mut TimeQuat, params: &GlitchRepairParams) -> usize {
        let n = data.len();
        if n < 8 { return 0; }

        let keys: Vec<i64> = data.keys().copied().collect();
        let times: Vec<f64> = keys.iter().map(|k| *k as f64 / 1_000_000.0).collect();
        let mut quats: Vec<UnitQuaternion<f64>> = data.values().copied().collect();

        let mut total_patched = 0;
        for pass in 0..params.passes.max(1) {
            let mags = compute_residual_magnitudes(&times, &quats, params.residual_window);
            let (regions, suspicious) = detect_bad_regions(&times, &mags, params);

            if !suspicious.is_empty() {
                for (lo, hi, dur) in &suspicious {
                    log::debug!("Glitch repair: region [{:.3}s, {:.3}s] (dur {:.3}s) exceeded max_duration, left as possible real motion", times[*lo], times[*hi], dur);
                }
            }
            if regions.is_empty() {
                if pass == 0 { log::debug!("Glitch repair: no bad regions detected"); }
                break;
            }

            for (lo, hi) in &regions {
                log::info!("Glitch repair: patching [{:.3}s, {:.3}s] ({} samples)", times[*lo], times[*hi], hi - lo + 1);
            }

            patch_regions(&times, &mut quats, &regions);
            total_patched += regions.iter().map(|(lo, hi)| hi - lo + 1).sum::<usize>();
        }

        if total_patched > 0 {
            for (i, k) in keys.iter().enumerate() {
                data.insert(*k, quats[i]);
            }
        }
        total_patched
    }
}

/// Angular velocity (deg/s, reference frame) between two orientations `q1 -> q2`
/// separated by `dt` seconds, as the exact rotation vector `scaled_axis(q2·q1⁻¹) / dt`.
/// Returns zero for non-positive `dt`.
fn quat_angular_velocity_deg_s(q1: &UnitQuaternion<f64>, q2: &UnitQuaternion<f64>, dt: f64) -> [f64; 3] {
    if dt <= 0.0 { return [0.0, 0.0, 0.0]; }
    let w = (q2 * q1.inverse()).scaled_axis() * (180.0 / std::f64::consts::PI / dt);
    [w.x, w.y, w.z]
}

/// O(n) centered moving average via a running sum.
fn moving_average(values: &[f64], window: usize) -> Vec<f64> {
    let n = values.len();
    if window < 1 { return values.to_vec(); }
    let half = (window / 2) as i64;
    let mut out = vec![0.0; n];
    let mut running = 0.0;
    let (mut lo, mut hi): (i64, i64) = (0, -1);
    for i in 0..n as i64 {
        let want_lo = (i - half).max(0);
        let want_hi = (i + half).min(n as i64 - 1);
        while hi < want_hi { hi += 1; running += values[hi as usize]; }
        while lo < want_lo { running -= values[lo as usize]; lo += 1; }
        out[i as usize] = running / (hi - lo + 1) as f64;
    }
    out
}

/// High-pass gyro-rate magnitude per sample: the raw gyro vector minus a
/// locally smoothed trend, magnitude of the remainder.
fn compute_residual_magnitudes(times: &[f64], quats: &[UnitQuaternion<f64>], smoothing_window_s: f64) -> Vec<f64> {
    let n = times.len();

    // Nominal sample interval from the first non-zero gap.
    let mut dt = 0.0;
    for i in 1..n.min(50) {
        let d = times[i] - times[i - 1];
        if d > 0.0 { dt = d; break; }
    }
    if dt <= 0.0 { dt = 0.0005; }
    let mut window = (3.0_f64).max((smoothing_window_s / dt).round()) as usize;
    if window % 2 == 0 { window += 1; }

    let (mut gx, mut gy, mut gz) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    for i in 1..n {
        let v = quat_angular_velocity_deg_s(&quats[i - 1], &quats[i], times[i] - times[i - 1]);
        gx[i] = v[0]; gy[i] = v[1]; gz[i] = v[2];
    }
    let sx = moving_average(&gx, window);
    let sy = moving_average(&gy, window);
    let sz = moving_average(&gz, window);

    (0..n).map(|i| {
        ((gx[i] - sx[i]).powi(2) + (gy[i] - sy[i]).powi(2) + (gz[i] - sz[i]).powi(2)).sqrt()
    }).collect()
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() { return 0.0; }
    let mut s = values.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let k = (s.len() - 1) as f64 * p;
    let f = k.floor() as usize;
    let c = k.ceil() as usize;
    if f == c { return s[f]; }
    s[f] + (s[c] - s[f]) * (k - f as f64)
}

fn rolling_max(mags: &[f64], times: &[f64], i: usize, half_window_s: f64) -> f64 {
    let n = mags.len();
    let t0 = times[i] - half_window_s;
    let t1 = times[i] + half_window_s;
    let mut lo = i;
    while lo > 0 && times[lo - 1] >= t0 { lo -= 1; }
    let mut hi = i;
    while hi < n - 1 && times[hi + 1] <= t1 { hi += 1; }
    mags[lo..=hi].iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

/// Grow `[lo, hi]` outward through a decaying ringdown tail.
fn expand_region(times: &[f64], mags: &[f64], mut lo: usize, mut hi: usize, expand_threshold: f64, quiet_duration: f64, smoothing_window: f64) -> (usize, usize) {
    let n = times.len();
    let half_win = smoothing_window / 2.0;

    let mut i = lo;
    let mut quiet_since: Option<f64> = None;
    while i > 0 {
        i -= 1;
        if rolling_max(mags, times, i, half_win) > expand_threshold {
            quiet_since = None;
            lo = i;
        } else {
            let qs = *quiet_since.get_or_insert(times[i]);
            if qs - times[i] >= quiet_duration { break; }
        }
    }

    let mut j = hi;
    quiet_since = None;
    while j < n - 1 {
        j += 1;
        if rolling_max(mags, times, j, half_win) > expand_threshold {
            quiet_since = None;
            hi = j;
        } else {
            let qs = *quiet_since.get_or_insert(times[j]);
            if times[j] - qs >= quiet_duration { break; }
        }
    }
    (lo, hi)
}

/// Returns `(regions, suspicious)` where `regions` are `[lo, hi]` index ranges
/// to patch and `suspicious` are `(lo, hi, duration)` cores that exceeded
/// `max_duration` and were left alone.
fn detect_bad_regions(times: &[f64], mags: &[f64], p: &GlitchRepairParams) -> (Vec<(usize, usize)>, Vec<(usize, usize, f64)>) {
    let baseline_p99 = percentile(mags, 0.99);
    let threshold = (baseline_p99 * p.threshold_multiplier).max(p.absolute_floor);
    let expand_threshold = (baseline_p99 * p.expand_multiplier)
        .max(p.absolute_floor * p.expand_multiplier / p.threshold_multiplier);

    let flagged: Vec<usize> = mags.iter().enumerate().filter(|&(_, &m)| m > threshold).map(|(i, _)| i).collect();
    if flagged.is_empty() { return (Vec::new(), Vec::new()); }

    // Merge flagged samples within merge_gap seconds into cores.
    let mut core_regions = Vec::new();
    let mut start = flagged[0];
    let mut prev = flagged[0];
    for &i in &flagged[1..] {
        if times[i] - times[prev] > p.merge_gap {
            core_regions.push((start, prev));
            start = i;
        }
        prev = i;
    }
    core_regions.push((start, prev));

    // Duration safety valve.
    let mut accepted = Vec::new();
    let mut suspicious = Vec::new();
    for (lo, hi) in core_regions {
        let dur = times[hi] - times[lo];
        if dur > p.max_duration && !p.force_long_regions {
            suspicious.push((lo, hi, dur));
        } else {
            accepted.push((lo, hi));
        }
    }
    if accepted.is_empty() { return (Vec::new(), suspicious); }

    let expanded: Vec<(usize, usize)> = accepted.into_iter()
        .map(|(lo, hi)| expand_region(times, mags, lo, hi, expand_threshold, p.quiet_duration, p.smoothing_window))
        .collect();

    // Merge overlapping/adjacent expanded regions.
    let mut merged: Vec<(usize, usize)> = vec![expanded[0]];
    for (lo, hi) in expanded.into_iter().skip(1) {
        let last = merged.last_mut().unwrap();
        if lo <= last.1 + 1 {
            last.1 = last.1.max(hi);
        } else {
            merged.push((lo, hi));
        }
    }
    (merged, suspicious)
}

fn patch_regions(times: &[f64], quats: &mut [UnitQuaternion<f64>], regions: &[(usize, usize)]) {
    let n = quats.len();
    for &(lo, hi) in regions {
        // Region touches the very start and end - nothing good to anchor to.
        if lo == 0 && hi + 1 >= n { continue; }

        // Region touches one edge: hold the single good neighbour constant.
        if lo == 0 || hi + 1 >= n {
            let anchor = if lo == 0 { quats[hi + 1] } else { quats[lo - 1] };
            for i in lo..=hi { quats[i] = anchor; }
            continue;
        }

        let q0 = quats[lo - 1];
        let q1 = quats[hi + 1];
        let t0 = times[lo - 1];
        let t1 = times[hi + 1];
        let span = t1 - t0;
        for i in lo..=hi {
            let frac = if span <= 0.0 { 0.0 } else { ((times[i] - t0) / span).clamp(0.0, 1.0) };
            // `try_slerp` takes the shortest arc and returns None only when the anchors are
            // ~180° apart (numerically ambiguous); there the endpoints coincide, so fall back to q0.
            quats[i] = q0.try_slerp(&q1, frac, 1e-6).unwrap_or(q0);
        }
    }
}
