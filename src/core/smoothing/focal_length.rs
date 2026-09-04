// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

//! Focal length stabilization for cameras that record dynamic lens metadata (zoom lenses, Sony's
//! dynamic active stabilization, Clear Image Zoom, interpolated per-position lens profiles).
//!
//! Every frame is projected with its own camera matrix, so the *effective* focal length in output
//! pixels already includes the optical zoom, crop-driven zoom and the digital zoom factor, whichever
//! way the camera reports it (see `FrameTransform::get_lens_data_at_timestamp`): Sony's per-frame
//! pixel focal length and lens curve, Canon's pixel focal length, the millimetre focal length of a
//! Blackmagic, RED, Nikon or Z CAM body scaled into a single lens profile or interpolated between
//! several, or the metadata alone when no profile is loaded. Everything below works on that pixel
//! value, so the same code serves every camera. This
//! module turns that per-frame value into a smooth **upper envelope** `target >= raw`, and the
//! renderer applies the uniform digital zoom `comp = raw / target <= 1` on top of the adaptive zoom,
//! so the apparent focal length of the output follows `target` instead of the raw metadata.
//!
//! Why an upper envelope: the output magnification relative to the source is `target / raw` (see
//! `FrameTransform::at_timestamp`), so a target below the raw curve would ask for pixels outside the
//! captured frame. Only cropping can hide a zoom, exactly like the adaptive zoom can only crop to hide
//! motion. The envelope is the tightest curve above the raw one whose zoom rate stays below a limit:
//! a zoom slower than the limit passes through untouched (no crop), a faster one is spread out, with
//! the crop leading a zoom-in and lagging a zoom-out by just the time the limit requires, and fast
//! zoom pumping turns into a hold at its peak. Wherever that limit acts, the compensation also ramps
//! its speed over [`ZOOM_RAMP_TIME`] instead of switching it in one frame. Everything is done on
//! `ln(f)` because zoom is perceived logarithmically: a 10% change at 24 mm reads the same as a 10%
//! change at 200 mm.
//!
//! The lens metadata can lag the picture by a lens-specific number of frames (two on a Tamron 28-200 at
//! 60 and at 120 fps, none on Sony's own lenses): `lens_metadata_delay_frames` shifts every lens lookup,
//! the projection's as well as this module's, and `synchronization::lens_delay` measures it on the clip's
//! own zooms.
//!
//! The curves are computed in `StabilizationManager::apply_focal_length_smoothing` and stored in
//! `StabilizationParams`; every `ComputeParams::from_manager` copy (preview, render, plugins) then sees
//! the same data, so the preview and the export agree. The per-frame projection sweep is done once per file,
//! lens and video geometry ([`compute_base_curve`], cached without any setting in its key), and the settings
//! act on that base curve in O(frames) ([`derive_curves`]). The zoom accounts for the compensation in
//! `zooming::calculate_fovs`.

use crate::stabilization::{ ComputeParams, FrameTransform };
use crate::zooming::zoom_dynamic;

/// A change between two consecutive values is a genuine jump (Clear Image Zoom switching, a lens change),
/// kept and never smoothed across, when it's larger than `DEQUANTIZE_JUMP_RATIO` of the focal length (no
/// optical zoom moves 25% in a frame, digital zoom toggles move 50% or more), or, once the quantization step is
/// known, larger than `DEQUANTIZE_JUMP_STEPS` steps at once and more than `DEQUANTIZE_MAX_STEP`. A fast zoom on
/// a lens that reports whole millimetres moves several units per frame (6 mm at 30 mm is 20%), but never dozens,
/// and a lens that reports whole millimetres at a short focal length steps by 9% at 11 mm, which must be joined
/// like any other quantization step even when the clip has too few steps to measure the quantization
const DEQUANTIZE_MAX_STEP: f64 = 0.05;
const DEQUANTIZE_JUMP_STEPS: f64 = 8.0;
const DEQUANTIZE_JUMP_RATIO: f64 = 0.25;
/// Metadata glitches: a few frames whose focal length is off by a large ratio and then returns to the previous
/// level (Canon's C50 writes a dozen frames referenced to a 2048x1080 area in the middle of a 6912x4608 clip, a
/// 4.3x drop). An optical zoom never moves 20% between two frames (that's 500%/s at 25 fps), and a digital zoom
/// toggle isn't undone within half a second, so such an excursion is replaced by a straight line between the
/// levels around it, before anything else looks at the data. Only a discontinuity that comes back triggers this:
/// a fast zoom that moves 15% per frame for a few frames is a sequence of steps below the ratio and passes
/// through, and a step that never comes back (a zoom toggle, at a clip end in particular) is a real zoom as far
/// as the metadata can tell and is kept, see `remove_outliers`
const OUTLIER_LN_RATIO: f64 = 0.2;
const OUTLIER_LN_RETURN: f64 = 0.05;
const OUTLIER_MAX_SECONDS: f64 = 0.6;
/// A run this many times longer than its neighbour is a static plateau (no zoom before or after a zoom),
/// so the true curve leaves its level at the run's edge rather than at its midpoint
const DEQUANTIZE_PLATEAU_RATIO: usize = 3;
/// The true focal length lies within half a quantization step of every reported value, so the reconstruction
/// may be smoothed (gaussian, sigma in frames) as long as it stays close to that band. This is what removes the
/// jitter of a fast zoom on a lens that reports whole millimetres and moves, say, 1.5 mm per frame: the reported
/// steps alternate between 1 and 2 mm, half a millimetre of noise on every frame. The band is a bit wider than
/// half a step: a ramp through the run midpoints touches the half-step edge at every run boundary, and clamping
/// exactly there would put a kink into every rounded corner. A slowly varying error of up to 0.75 step is
/// invisible, jitter is not
const DEQUANTIZE_BAND: f64 = 0.75;
const DEQUANTIZE_BAND_SIGMA: f64 = 2.0;
const DEQUANTIZE_BAND_ITERATIONS: usize = 3;
const DEQUANTIZE_MAX_WINDOW: usize = 121; // frames, odd
/// Time the compensation takes to reach the maximum zoom speed from rest, in seconds. The rate limit is
/// applied together with an acceleration limit of `max_zoom_rate / ZOOM_RAMP_TIME`, so the crop starts,
/// turns and stops along parabolic arcs instead of stepping its speed in one frame. The price: every ramp
/// starts half a ramp time earlier and ends half a ramp time later than the first-order envelope would, and
/// beside a sharp peak the arcs add up to `max_zoom_rate * ZOOM_RAMP_TIME / 2` of crop (4% at 50%/s); the
/// crop of a lead or a lag itself does not grow, and the rounding never overshoots a level. The arcs are
/// only placed where the rate limit acts: a zoom slower than the limit still passes through untouched,
/// however abruptly it starts, and the residual jitter of a dequantized curve is never "smoothed" into the
/// picture (the compensation would inject its inverse)
pub const ZOOM_RAMP_TIME: f64 = 0.15;

/// Size of the quantization step in `v`: the 10th percentile of the non-zero changes between consecutive
/// values. A lens that reports whole millimetres moving several millimetres per frame still produces plenty of
/// single-unit changes, and for continuous data this is tiny, which makes the band clamp a no-op
fn quantization_step(v: &[f64]) -> f64 {
    let mut steps: Vec<f64> = v.windows(2).map(|w| (w[1] - w[0]).abs()).filter(|d| *d > 0.0).collect();
    if steps.len() < 4 { return 0.0; }
    steps.sort_by(|a, b| a.partial_cmp(b).unwrap());
    steps[steps.len() / 10]
}

/// Effective focal length in output pixels for every frame, taken from the same lens data the renderer
/// projects with. `None` where the projection of the frame takes its focal length from the static profile
/// alone (no lens metadata within 100 ms of the frame), empty when the clip has no per-frame lens data at all.
/// Extracted without the lens metadata delay: frame `i` holds the metadata at frame `i`'s own time, and
/// [`shift_by_delay`] moves the curve by the delay afterwards, which is exactly what the delayed lookup does
/// (`ComputeParams::lens_timestamp_us` shifts by whole frames).
///
/// Whether a frame has one is the projection's own verdict (`get_lens_data_at_timestamp_with_metadata`), not
/// a guess from the shape of the metadata: the cameras report the focal length in pixels (Sony, Canon), in
/// millimetres with the capture area (Sony), in millimetres alone (RED, Blackmagic, Nikon, Z CAM) or as a
/// lens position to interpolate profiles by, and each reaches the camera matrix along a different path
pub fn extract_effective_focal_lengths(params: &ComputeParams) -> Vec<Option<f64>> {
    let gyro = params.gyro.read();
    let file_metadata = gyro.file_metadata.read();
    if file_metadata.lens_params.len() < 2 && file_metadata.lens_positions.len() < 2 {
        return Vec::new();
    }
    (0..params.frame_count).map(|frame| {
        let lens_timestamp_us = ComputeParams::lens_timestamp_us_with_delay(crate::timestamp_at_frame(frame as i32, params.scaled_fps), 0, params.scaled_fps);
        // On the metadata borrowed above: a second read of a lock this thread already holds deadlocks once a writer queues up
        let (camera_matrix, _, _, _, _, _, per_frame) = FrameTransform::get_lens_data_at_lens_timestamp(params, &file_metadata, lens_timestamp_us, false);
        if !per_frame { return None; }
        // Geometric mean: the compensation is one uniform scale applied to both axes
        let f = (camera_matrix[(0, 0)] * camera_matrix[(1, 1)]).sqrt();
        (f.is_finite() && f > 0.0).then_some(f)
    }).collect()
}

/// Zoom ring refinement (Sony tag 0x800B, a 16-bit fraction of the ring travel). On lenses that fill the focal
/// length fields with whole millimetres the ring is the only sub-step zoom information in the file, and it
/// tracks the picture closely: measured on a Tamron 28-200, 5.1 permille RMS against SIFT image scale steps,
/// versus 7.0-7.6 for every focal-length field or lens-curve derived estimate. The relation between ring
/// position and log focal length is fitted per clip as a cubic through the reported values, so the camera
/// matrix stays the reference: the ring only shapes the curve inside the quantization band of the reported
/// focal length, and it's ignored when the tag is missing, saturated at 0% or 100% (dead zones), spans too
/// little travel, is not monotonic, doesn't agree with the reported values to within their quantization, or
/// is coarser than they are: an a6700 kit lens reports its ring in steps of a fourteenth of the travel, three
/// changes over a 4 mm zoom, and such a ring would only turn the reconstruction back into a staircase. The
/// ring's own step, in log focal length, has to stay below `RING_MAX_STEP_FRACTION` of the focal length's
const RING_MIN_FRAMES: usize = 30;
const RING_MIN_TRAVEL: f64 = 5.0; // percent
const RING_MAX_RESIDUAL_FACTOR: f64 = 3.0;
const RING_MAX_STEP_FRACTION: f64 = 0.5;
/// Frames over which a ring-based estimate is cross-faded into the plateau reconstruction at its ends
const DEQUANTIZE_HINT_FADE: usize = 15;

/// Per frame: (focal length in mm, zoom ring position in percent) from the lens entry the projection uses, without
/// the lens metadata delay like [`extract_effective_focal_lengths`]
pub fn extract_lens_zoom(params: &ComputeParams) -> Vec<Option<(f64, f64)>> {
    let gyro = params.gyro.read();
    let file_metadata = gyro.file_metadata.read();
    (0..params.frame_count).map(|frame| {
        let timestamp_us = ComputeParams::lens_timestamp_us_with_delay(crate::timestamp_at_frame(frame as i32, params.scaled_fps), 0, params.scaled_fps);
        let lp = file_metadata.lens_params_closest(timestamp_us, 100000, |v| v.has_projection_data())?;
        Some((lp.focal_length? as f64, lp.zoom_ring_position? as f64))
    }).collect()
}

/// Least squares polynomial fit `y = sum c_k x^k`, coefficients by ascending power
fn polyfit(x: &[f64], y: &[f64], degree: usize) -> Option<Vec<f64>> {
    let a = nalgebra::DMatrix::from_fn(x.len(), degree + 1, |i, j| x[i].powi(j as i32));
    let b = nalgebra::DVector::from_column_slice(y);
    let ata = a.transpose() * &a;
    let atb = a.transpose() * b;
    ata.lu().solve(&atb).map(|c| c.iter().copied().collect())
}
fn polyval(c: &[f64], x: f64) -> f64 {
    c.iter().enumerate().map(|(k, ck)| ck * x.powi(k as i32)).sum()
}

/// Sub-step estimate of the effective focal length from the zoom ring, for the frames the fit covers
pub fn ring_hint(raw_px: &[f64], lens_zoom: &[Option<(f64, f64)>]) -> Option<Vec<Option<f64>>> {
    let samples: Vec<(f64, f64)> = lens_zoom.iter()
        .filter_map(|s| s.and_then(|(mm, ring)| (mm > 0.0 && ring > 0.01 && ring < 99.99).then(|| (ring, mm.ln()))))
        .collect();
    if samples.len() < RING_MIN_FRAMES { return None; }
    let (lo, hi) = samples.iter().fold((f64::MAX, f64::MIN), |(lo, hi), s| (lo.min(s.0), hi.max(s.0)));
    if hi - lo < RING_MIN_TRAVEL { return None; }

    // Fit on a normalized ring position for conditioning; a cubic first, a straight line if that isn't monotonic
    let norm = |r: f64| (r - lo) / (hi - lo) * 2.0 - 1.0;
    let xs: Vec<f64> = samples.iter().map(|s| norm(s.0)).collect();
    let ys: Vec<f64> = samples.iter().map(|s| s.1).collect();
    let monotonic = |c: &[f64]| (0..200).all(|i| polyval(c, -1.0 + i as f64 / 100.0) < polyval(c, -1.0 + (i + 1) as f64 / 100.0));
    let coeffs = [3usize, 1].iter().find_map(|&deg| polyfit(&xs, &ys, deg).filter(|c| monotonic(c)))?;

    // The fit has to agree with the reported values to within their quantization: a rounded value is off by
    // step / sqrt(12) on average, in log units step / (mm * sqrt(12))
    let mms: Vec<f64> = samples.iter().map(|s| s.1.exp()).collect();
    let step = quantization_step(&mms);
    let expected = if step > 0.0 { mms.iter().map(|mm| step / mm).sum::<f64>() / mms.len() as f64 / 12f64.sqrt() } else { 0.0 };
    let residual = (xs.iter().zip(&ys).map(|(x, y)| (y - polyval(&coeffs, *x)).powi(2)).sum::<f64>() / xs.len() as f64).sqrt();
    if residual > RING_MAX_RESIDUAL_FACTOR * expected + 0.002 { return None; }
    // Only a ring that resolves finer than the reported focal length carries any sub-step information: its
    // quantum (the smallest change it ever makes: a quantized value only moves by whole quanta, and a fast zoom
    // makes every per-frame change large, so a percentile would measure the zoom speed instead), taken through
    // the fit into log focal length, has to be well below the focal length's step
    let rings: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let ring_changes = rings.windows(2).filter(|w| w[1] != w[0]).count();
    let ring_quantum = rings.windows(2).map(|w| (w[1] - w[0]).abs()).filter(|d| *d > 0.0).fold(f64::MAX, f64::min);
    let (ymin, ymax) = ys.iter().fold((f64::MAX, f64::MIN), |(a, b), y| (a.min(*y), b.max(*y)));
    let ring_quantum_ln = ring_quantum * (ymax - ymin) / (hi - lo);
    if ring_changes < 4 || (step > 0.0 && ring_quantum_ln > RING_MAX_STEP_FRACTION * expected * 12f64.sqrt()) { return None; }

    Some(lens_zoom.iter().zip(raw_px).map(|(s, px)| {
        let (mm, ring) = (*s)?;
        if !(mm > 0.0) || ring < lo || ring > hi { return None; }
        Some(px * (polyval(&coeffs, norm(ring)).exp() / mm))
    }).collect())
}

/// Replaces excursions of at most `max_frames` frames that leave the previous level by more than
/// `OUTLIER_LN_RATIO` and come back to within `OUTLIER_LN_RETURN` of it (see the constants) with a straight
/// line between the levels around them. Only such a confirmed excursion is touched, where the metadata
/// contradicts itself: a step that never comes back within `max_frames`, at a clip end in particular, may just
/// as well be a real zoom toggle (Clear Image Zoom engaged half a second before the recording stopped), and
/// holding the old level over it would project those frames with a focal length off by the whole step. So the
/// metadata is trusted there, and the curve leaves the dequantization band of the metadata only across a
/// confirmed glitch (`FrameTransform::dequantize_camera_matrix` relies on that). Returns the number of frames
/// replaced.
pub fn remove_outliers(v: &mut [f64], max_frames: usize) -> usize {
    let n = v.len();
    if n < 3 || max_frames == 0 { return 0; }
    let (mut removed, mut i) = (0, 1);
    while i < n {
        let base = v[i - 1];
        if base > 0.0 && v[i] > 0.0 && (v[i] / base).ln().abs() > OUTLIER_LN_RATIO {
            let back = (i + 1..=(i + max_frames).min(n - 1)).find(|&j| v[j] > 0.0 && (v[j] / base).ln().abs() <= OUTLIER_LN_RETURN);
            if let Some(j) = back {
                for k in i..j {
                    let t = (k - i + 1) as f64 / (j - i + 1) as f64;
                    v[k] = base + (v[j] - base) * t;
                }
                removed += j - i;
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    removed
}

/// Linear interpolation across missing frames, the ends are held. Returns `false` when no frame is valid.
fn fill_gaps(v: &mut [Option<f64>]) -> bool {
    let Some(first) = v.iter().position(|x| x.is_some()) else { return false; };
    let last = v.iter().rposition(|x| x.is_some()).unwrap();
    for i in 0..first { v[i] = v[first]; }
    for i in last + 1..v.len() { v[i] = v[last]; }
    let mut prev = first;
    for i in first + 1..=last {
        if v[i].is_some() {
            if i > prev + 1 {
                let (a, b) = (v[prev].unwrap(), v[i].unwrap());
                for j in prev + 1..i {
                    let t = (j - prev) as f64 / (i - prev) as f64;
                    v[j] = Some(a + (b - a) * t);
                }
            }
            prev = i;
        }
    }
    true
}

/// Lens encoders often report the focal length in coarse steps that the optics don't have (whole
/// millimetres on many Sony lenses): the true focal length moves smoothly while the metadata sits on
/// plateaus. The compensation is a ratio against this curve, so any step left in it becomes a visible
/// jump in the output while the optics zoom smoothly.
///
/// Runs of identical values whose neighbour differs by at most `DEQUANTIZE_MAX_STEP` are joined by
/// straight lines between knots: the midpoint of a run during a steady zoom (the true curve crosses the
/// reported level halfway through the run), or the run's edge when it is much longer than its neighbour
/// (a static plateau, where the true curve only leaves the level at the edge). A gaussian of half a
/// typical run then removes the slope kinks at the knots; the slope between knots is at most one step per
/// run, so this smoothing is off by a fraction of a step at most. Continuous data (a new value every
/// frame) comes back unchanged, and genuine jumps are kept and not smoothed across.
///
/// `hint` is an optional sub-step estimate per frame (see [`ring_hint`]) that replaces the reconstruction
/// wherever it exists; the band clamp still keeps it within the quantization of `v`
pub fn dequantize(v: &[f64], hint: Option<&[Option<f64>]>) -> Vec<f64> {
    let mut out = v.to_vec();
    // (start, length, value) of every run of identical values
    let mut runs: Vec<(usize, usize, f64)> = Vec::new();
    for (i, &x) in v.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if run.2 == x => run.1 += 1,
            _ => runs.push((i, 1, x))
        }
    }
    if runs.len() < 3 { return out; }

    let mut lengths: Vec<usize> = runs.iter().map(|r| r.1).collect();
    lengths.sort_unstable();
    let typical = lengths[lengths.len() / 2];

    let midpoint = |run: &(usize, usize, f64)| run.0 as f64 + (run.1 - 1) as f64 / 2.0;
    let is_plateau = |run: &(usize, usize, f64), neighbour: &(usize, usize, f64)| run.1 > DEQUANTIZE_PLATEAU_RATIO * neighbour.1 && run.1 > typical;
    let q = quantization_step(v);
    let is_jump = |a: f64, b: f64| {
        let d = (b - a).abs();
        d > DEQUANTIZE_JUMP_RATIO * a.min(b) || (q > 0.0 && d > DEQUANTIZE_JUMP_STEPS * q && d > DEQUANTIZE_MAX_STEP * a.min(b))
    };

    // Segments between genuine jumps are smoothed separately
    let mut segment_starts = vec![0usize];
    for pair in runs.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if is_jump(a.2, b.2) {
            segment_starts.push(b.0);
            continue;
        }
        // Knot of `a` towards `b`: the boundary is at `a.0 + a.1 - 0.5`, the true curve reaches the level of
        // `a` half a neighbouring run before it. Same for `b` towards `a`
        let ka = if is_plateau(a, b) { ((a.0 + a.1) as f64 - 0.5 - b.1 as f64 / 2.0).max(midpoint(a)) } else { midpoint(a) };
        let kb = if is_plateau(b, a) { (b.0 as f64 - 0.5 + a.1 as f64 / 2.0).min(midpoint(b)) } else { midpoint(b) };
        if kb <= ka { continue; }
        for j in (ka.ceil() as usize)..=(kb.floor() as usize) {
            let t = (j as f64 - ka) / (kb - ka);
            out[j] = a.2 + (b.2 - a.2) * t;
        }
    }

    segment_starts.push(v.len());
    let band = if q > 0.0 { Some(q * DEQUANTIZE_BAND) } else { None };
    let smooth_segments = |out: &mut Vec<f64>, frames: usize| {
        let gaussian = zoom_dynamic::gaussian_window_normalized(frames, frames as f64 / 6.0);
        for pair in segment_starts.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if end <= start { continue; }
            let padded = zoom_dynamic::pad_edge(&out[start..end], (frames / 2, frames / 2));
            let smoothed = zoom_dynamic::convolve(&padded, &gaussian);
            for (i, s) in smoothed.into_iter().enumerate() {
                out[start + i] = match band { Some(half) => s.clamp(v[start + i] - half, v[start + i] + half), None => s };
            }
        }
    };
    if typical >= 2 {
        // 6 sigma wide, sigma = half a run. Capped: a long clip whose lens was touched once has a median run of
        // tens of thousands of frames, and the convolution is O(n * window) on every recompute; the kinks of such
        // a reconstruction are one step per run and don't need more rounding than this anyway
        smooth_segments(&mut out, ((typical * 3) | 1).min(DEQUANTIZE_MAX_WINDOW));
    }
    if let Some(hint) = hint {
        for (o, h) in out.iter_mut().zip(hint) {
            if let Some(h) = h { *o = *h; }
        }
    }
    if band.is_some() {
        let frames = ((DEQUANTIZE_BAND_SIGMA * 6.0).round() as usize) | 1;
        for _ in 0..DEQUANTIZE_BAND_ITERATIONS {
            smooth_segments(&mut out, frames);
        }
    }
    // The hint is already smooth and more precise than the smoothing (which biases an accelerating zoom by
    // sigma^2 * f'' / 2 per pass), so it's final wherever it exists, only the band clamp applies. At the ends of
    // a hint region (the ring's dead zones, where its fit is least reliable) it's cross-faded into the smoothed
    // reconstruction so a fit offset there doesn't turn into a step
    if let Some(hint) = hint {
        let mut i = 0;
        while i < hint.len().min(out.len()) {
            if hint[i].is_none() { i += 1; continue; }
            let start = i;
            while i < hint.len().min(out.len()) && hint[i].is_some() { i += 1; }
            let end = i;
            for j in start..end {
                let h = hint[j].unwrap();
                let w_start = if start == 0 { 1.0 } else { ((j - start + 1) as f64 / (DEQUANTIZE_HINT_FADE + 1) as f64).min(1.0) };
                let w_end = if end == hint.len() { 1.0 } else { ((end - j) as f64 / (DEQUANTIZE_HINT_FADE + 1) as f64).min(1.0) };
                let w = w_start.min(w_end);
                let blended = out[j] * (1.0 - w) + h * w;
                out[j] = match band { Some(half) => blended.clamp(v[j] - half, v[j] + half), None => blended };
            }
        }
    }
    out
}

/// Tightest upper envelope of the effective focal length whose zoom rate never exceeds `max_zoom_rate`
/// (`d ln(f) / dt` in 1/s: 0.5 is roughly 50% of magnification per second), with its speed changes
/// ramped over [`ZOOM_RAMP_TIME`] wherever the limit acts.
///
/// The forward pass bounds how fast the envelope may fall (the crop lags a zoom-out), the backward pass
/// how fast it may rise (the crop leads a zoom-in); each pass keeps the other's bound and both keep the
/// envelope above `raw`, so the result is the minimal such curve. Wherever the limit is not hit the
/// envelope is exactly `raw`, so the compensation is exactly 1 and the feature is a no-op there: any
/// deviation from `raw` shows up in the picture as a zoom the optics didn't make. The corners that the
/// limit creates are then rounded by [`round_corners`]. Frames outside the trim ranges don't constrain
/// the envelope, like they don't constrain the zoom.
pub fn upper_envelope(raw: &[f64], fps: f64, max_zoom_rate: f64, trim_ranges: &[(f64, f64)]) -> Vec<f64> {
    let n = raw.len();
    if n == 0 || !(fps > 0.0) { return raw.to_vec(); }

    let mut s: Vec<f64> = raw.iter().map(|f| f.max(1e-9).ln()).collect();

    if !trim_ranges.is_empty() {
        let l = (n - 1) as f64;
        let within = |i: usize| trim_ranges.iter().any(|r| i >= (l * r.0).floor() as usize && i <= (l * r.1).ceil() as usize);
        if let Some(least_restrictive) = s.iter().enumerate().filter(|(i, _)| within(*i)).map(|(_, v)| *v).reduce(f64::min) {
            for (i, v) in s.iter_mut().enumerate() {
                if !within(i) { *v = least_restrictive; }
            }
        }
    }
    let floor = s.clone();

    let max_step = max_zoom_rate.max(1e-6) / fps;
    for i in 1..n {
        s[i] = s[i].max(s[i - 1] - max_step);
    }
    for i in (0..n - 1).rev() {
        s[i] = s[i].max(s[i + 1] - max_step);
    }

    round_corners(&mut s, &floor, max_step, max_step / (ZOOM_RAMP_TIME * fps));

    // Never below the captured focal length, or the compensation would need pixels outside the frame
    s.iter().zip(raw).map(|(e, f)| e.exp().max(*f)).collect()
}

/// Bounds the second difference of the rate-limited envelope `s` (log units) by `max_accel` per frame²,
/// keeping it above `floor` and its first difference within `max_step`, and only around the places where
/// the rate limit acted (`s > floor`, widened by one full turnaround `2 * max_step / max_accel`).
///
/// The limit leaves three kinds of corners. Peaks and shoulders (a slope decreasing faster than the bound)
/// get an arc of curvature `-max_accel` through the corner point itself, continuing the gentler of the two
/// sides and lifting the curve only over the steeper one (zero slope at a peak), then continuing at the
/// speed limit like the passes would. Continuing the gentler side keeps the rounding monotone wherever the
/// envelope is: an arc with the mean of the two slopes would be a smaller lift, but at the end of a lead it
/// carries the rise past the plateau and back down, a zoom-in-and-out the picture never made. Valleys (a
/// slope increasing faster than the bound: the start of a lead, the end of a lag) are filled from above by
/// the least curve of bounded upward curvature, which is `a t² / 2` plus the concave majorant of
/// `s - a t² / 2`; chords of that majorant that bridge a valley the limit didn't create are dropped, so the
/// raw curve's own features and its residual dequantization jitter are never touched. Every step raises the
/// curve, so `s >= floor` holds throughout, and every arc and chord keeps the slope within the limit.
pub fn round_corners(s: &mut [f64], floor: &[f64], max_step: f64, max_accel: f64) {
    let n = s.len();
    if n < 3 || !(max_accel > 0.0) || !(max_step > 0.0) { return; }
    let (vd, ad) = (max_step, max_accel);
    let blend = ((2.0 * vd / ad).ceil() as usize).max(1);

    let mut mask = vec![false; n];
    let mut last_active = None;
    for i in 0..n {
        if s[i] > floor[i] + 1e-12 { last_active = Some(i); }
        if let Some(l) = last_active { if i - l <= blend { mask[i] = true; } }
    }
    let mut next_active = None;
    for i in (0..n).rev() {
        if s[i] > floor[i] + 1e-12 { next_active = Some(i); }
        if let Some(a) = next_active { if a - i <= blend { mask[i] = true; } }
    }
    if !mask.iter().any(|m| *m) { return; }

    // Peaks and shoulders
    let e = s.to_vec();
    for i in 1..n - 1 {
        if !mask[i] || e[i + 1] - 2.0 * e[i] + e[i - 1] >= -ad * (1.0 + 1e-6) { continue; }
        let (before, after) = (e[i] - e[i - 1], e[i + 1] - e[i]);
        let w = if before * after <= 0.0 { 0.0 } else if before.abs() < after.abs() { before } else { after }.clamp(-vd, vd);
        for dir in [1i64, -1] {
            let mut k = 1i64;
            loop {
                let j = i as i64 + dir * k;
                if j < 0 || j >= n as i64 { break; }
                let delta = (dir * k) as f64;
                let slope = w - ad * delta;
                if (dir > 0 && slope < -vd) || (dir < 0 && slope > vd) { break; }
                let val = e[i] + w * delta - ad * delta * delta / 2.0;
                // Below the envelope the arc stays below it until a sharper corner, whose own arc dominates
                if val <= e[j as usize] { break; }
                if val > s[j as usize] { s[j as usize] = val; }
                k += 1;
            }
        }
    }
    // The arcs continue at the speed limit
    for i in 1..n {
        s[i] = s[i].max(s[i - 1] - vd);
    }
    for i in (0..n - 1).rev() {
        s[i] = s[i].max(s[i + 1] - vd);
    }

    // Valleys
    let h: Vec<f64> = s.iter().enumerate().map(|(j, v)| v - ad * (j * j) as f64 / 2.0).collect();
    let mut hull: Vec<usize> = Vec::with_capacity(n);
    for j in 0..n {
        while hull.len() >= 2 {
            let (p, q) = (hull[hull.len() - 2], hull[hull.len() - 1]);
            let cross = (q - p) as f64 * (h[j] - h[p]) - (h[q] - h[p]) * (j - p) as f64;
            if cross >= 0.0 { hull.pop(); } else { break; }
        }
        hull.push(j);
    }
    for pair in hull.windows(2) {
        let (p, q) = (pair[0], pair[1]);
        if q <= p + 1 || !mask[p..=q].iter().any(|m| *m) { continue; }
        for j in p + 1..q {
            let t = (j - p) as f64 / (q - p) as f64;
            let v = h[p] + (h[q] - h[p]) * t + ad * (j * j) as f64 / 2.0;
            if v > s[j] { s[j] = v; }
        }
    }
}

/// Digital zoom factor the renderer multiplies into `fov` for `frame`: `raw / target`, never above 1.
/// Frames past the end of the curves use the last value, like `fovs` do. Anything that draws in the
/// output space (the zoom debug polygon, overlays) has to scale by the same factor.
pub fn compensation(focal_lengths: &[Option<f64>], smoothed_focal_lengths: &[Option<f64>], enabled: bool, frame: usize) -> f64 {
    if !enabled { return 1.0; }
    let at = |v: &[Option<f64>]| v.get(frame).or(v.last()).copied().flatten();
    match (at(focal_lengths), at(smoothed_focal_lengths)) {
        (Some(raw), Some(target)) if raw > 0.0 && target > 0.0 => (raw / target).min(1.0),
        _ => 1.0
    }
}

/// [`compensation`] for the curves stored in `ComputeParams`
pub fn compensation_at(params: &ComputeParams, frame: usize) -> f64 {
    compensation(&params.focal_lengths, &params.smoothed_focal_lengths, params.focal_length_smoothing_enabled, frame)
}

/// The dequantized effective focal length per frame, without the lens metadata delay: the one expensive step (the
/// projection of every frame) and what `StabilizationManager::apply_focal_length_smoothing` caches. Every setting
/// the final curves depend on, the delay, the rate limit and the trim, acts on it afterwards in O(frames)
/// ([`derive_curves`]), so a slider tick never repeats the sweep. Empty when the clip has no per-frame lens data
pub fn compute_base_curve(params: &ComputeParams) -> Vec<f64> {
    let mut raw = extract_effective_focal_lengths(params);
    if !fill_gaps(&mut raw) {
        return Vec::new();
    }
    let mut raw: Vec<f64> = raw.into_iter().map(|x| x.unwrap()).collect();
    let removed = remove_outliers(&mut raw, (OUTLIER_MAX_SECONDS * params.scaled_fps).ceil() as usize);
    if removed > 0 {
        log::warn!("Focal length metadata: {removed} frames replaced as glitches (isolated excursions of more than {:.0}%)", (OUTLIER_LN_RATIO.exp() - 1.0) * 100.0);
    }
    let hint = ring_hint(&raw, &extract_lens_zoom(params));
    dequantize(&raw, hint.as_deref())
}

/// The curve as the picture sees it with the metadata `delay_frames` late: frame `i` takes the value of frame
/// `i + delay`, the ends are held. The lens lookups shift by whole frames (`ComputeParams::lens_timestamp_us`), so
/// this is what extracting with the delay gives; dequantizing the shifted metadata instead could only differ within
/// the last `|delay|` frames, where the shifted metadata is held too
pub fn shift_by_delay(base: &[f64], delay_frames: i32) -> Vec<f64> {
    let n = base.len() as i64;
    (0..n).map(|i| base[(i + delay_frames as i64).clamp(0, n - 1) as usize]).collect()
}

/// The curves stored in `StabilizationParams` and `ComputeParams`, (`focal_lengths`, `smoothed_focal_lengths`), from
/// the base curve of [`compute_base_curve`] and the settings.
///
/// `focal_lengths` is the dequantized effective focal length, the renderer's estimate of the true optical state and
/// the denominator of the compensation. `smoothed_focal_lengths` is the envelope target, empty when smoothing is
/// disabled. Both are empty when the clip has no per-frame lens data.
pub fn derive_curves(base: &[f64], delay_frames: i32, enabled: bool, max_zoom_rate: f64, fps: f64, trim_ranges: &[(f64, f64)]) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    if base.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let dequantized = shift_by_delay(base, delay_frames);
    if !enabled {
        return (dequantized.into_iter().map(Some).collect(), Vec::new());
    }
    // The envelope follows the dequantized curve only. The renderer's magnification is `target / dequantized`
    // (the metadata focal length cancels out of the projection), so `target >= dequantized` is all that keeps
    // the compensated view inside the frame, and an envelope above the raw staircase would inherit every step
    // edge as a sawtooth in the compensation: a lens that reports whole millimetres would visibly jump on
    // every step while the optics zoom smoothly
    let target = upper_envelope(&dequantized, fps, max_zoom_rate, trim_ranges);
    (dequantized.into_iter().map(Some).collect(), target.into_iter().map(Some).collect())
}

/// [`derive_curves`] of a fresh [`compute_base_curve`], with the delay and the trim of `params`
pub fn compute_curves(params: &ComputeParams, enabled: bool, max_zoom_rate: f64) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    derive_curves(&compute_base_curve(params), params.lens_metadata_delay_frames, enabled, max_zoom_rate, params.scaled_fps, &params.trim_ranges)
}
