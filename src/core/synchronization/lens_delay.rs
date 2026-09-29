// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

//! Estimates how many frames the lens metadata (the per-frame focal length) lags the picture.
//!
//! Lenses report their zoom position with a lens-specific delay: measured against the image, a Tamron
//! 28-200 is two frames late on an a7S III at 59.94 fps and on a ZV-E1 at both 59.94 and 119.88 fps,
//! Sony's own lens on time. The same two frames at both frame rates (16.7 ms at 120 fps, 33 ms at 60 fps)
//! show it's a pipeline depth in frames, the body polling the lens once per frame, not a time, so the
//! delay is kept in frames and carries over between frame rates. During a fast zoom it leaves a focal
//! length error of several percent in the projection and in the focal length compensation. The
//! synchronization already tracks features between consecutive frames; a similarity fitted to those
//! tracks after undistortion gives the picture's zoom step per frame pair, and the lag that best aligns
//! the metadata's zoom steps with it is the delay. The estimate is only trusted when the analyzed frames
//! actually contain a zoom and the two step series correlate well.

use super::PoseEstimator;
use crate::stabilization::{ ComputeParams, undistort_points_with_rolling_shutter };

pub struct LensDelayEstimate {
    /// Positive: the metadata lags the picture, so the lens state of a frame is found this many frames later in the metadata
    pub delay_frames: i32,
    /// The sub-frame refinement the integer above was rounded from
    pub delay_frames_exact: f64,
    pub correlation: f64,
    pub pairs: usize,
    /// Total zoom (sum of the absolute log steps) within the analyzed frames
    pub zoom_span: f64,
}

const MIN_PAIRS: usize = 20;
const MIN_ZOOM_SPAN: f64 = 0.05;
const MIN_CORRELATION: f64 = 0.7;
const MAX_LAG_FRAMES: i64 = 8;
const MIN_INLIERS: usize = 8;

/// Least-squares similarity `to = scale * R(angle) * from + t` with one round of outlier rejection at three
/// median residuals. Returns `ln(scale)` and the number of inliers
pub fn similarity_ln_scale(from: &[(f32, f32)], to: &[(f32, f32)]) -> Option<(f64, usize)> {
    fn fit(pairs: &[((f64, f64), (f64, f64))]) -> Option<(f64, f64, f64, f64)> { // a, b, tx, ty
        if pairs.len() < 4 { return None; }
        let n = pairs.len() as f64;
        let (mx, my, mu, mv) = pairs.iter().fold((0.0, 0.0, 0.0, 0.0), |acc, (p, q)| (acc.0 + p.0, acc.1 + p.1, acc.2 + q.0, acc.3 + q.1));
        let (mx, my, mu, mv) = (mx / n, my / n, mu / n, mv / n);
        let (mut sxx, mut sxu, mut sxv) = (0.0, 0.0, 0.0);
        for (p, q) in pairs {
            let (x, y, u, v) = (p.0 - mx, p.1 - my, q.0 - mu, q.1 - mv);
            sxx += x * x + y * y;
            sxu += x * u + y * v;
            sxv += x * v - y * u;
        }
        if sxx <= 0.0 { return None; }
        let (a, b) = (sxu / sxx, sxv / sxx);
        Some((a, b, mu - (a * mx - b * my), mv - (b * mx + a * my)))
    }
    let mut pairs: Vec<((f64, f64), (f64, f64))> = from.iter().zip(to)
        .filter(|(p, q)| p.0.is_finite() && p.1.is_finite() && q.0.is_finite() && q.1.is_finite())
        .map(|(p, q)| ((p.0 as f64, p.1 as f64), (q.0 as f64, q.1 as f64)))
        .collect();
    let (a, b, tx, ty) = fit(&pairs)?;
    let residual = |p: &(f64, f64), q: &(f64, f64)| ((a * p.0 - b * p.1 + tx - q.0).powi(2) + (b * p.0 + a * p.1 + ty - q.1).powi(2)).sqrt();
    let mut res: Vec<f64> = pairs.iter().map(|(p, q)| residual(p, q)).collect();
    res.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let threshold = (res[res.len() / 2] * 3.0).max(0.5);
    pairs.retain(|(p, q)| residual(p, q) <= threshold);
    if pairs.len() < MIN_INLIERS { return None; }
    let (a, b, _, _) = fit(&pairs)?;
    let scale = (a * a + b * b).sqrt();
    (scale > 0.0).then(|| (scale.ln(), pairs.len()))
}

/// The lag that best aligns the metadata with the picture. `image`: `(t1 ms, t2 ms, ln scale)` per analyzed
/// frame pair, `meta_ln`: log focal length per frame of the metadata without any delay applied (`NaN` where
/// unknown). The lag is searched frame by frame within `MAX_LAG_FRAMES` and refined to a fraction of a frame
/// with a parabola through the RMS differences of the neighbouring lags
pub fn estimate_from_steps(image: &[(f64, f64, f64)], meta_ln: &[f64], fps: f64) -> Option<LensDelayEstimate> {
    if image.len() < MIN_PAIRS || meta_ln.len() < 2 || !(fps > 0.0) { return None; }
    let frame_of = |ts_ms: f64| -> Option<usize> {
        let f = crate::frame_at_timestamp(ts_ms, fps);
        (f >= 0 && (f as usize) < meta_ln.len() && meta_ln[f as usize].is_finite()).then(|| f as usize)
    };
    let meta_step = |t1: f64, t2: f64, lag: i64| -> Option<f64> {
        let shift = crate::timestamp_at_frame(lag as i32, fps);
        Some(meta_ln[frame_of(t2 + shift)?] - meta_ln[frame_of(t1 + shift)?])
    };
    let zoom_span: f64 = image.iter().filter_map(|(t1, t2, _)| meta_step(*t1, *t2, 0)).map(f64::abs).sum();
    if zoom_span < MIN_ZOOM_SPAN { return None; }

    let score = |lag: i64| -> Option<(f64, f64, usize)> { // rms, correlation, pairs
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        for (t1, t2, s) in image {
            if let Some(m) = meta_step(*t1, *t2, lag) { xs.push(*s); ys.push(m); }
        }
        if xs.len() < MIN_PAIRS { return None; }
        let n = xs.len() as f64;
        let (mx, my) = (xs.iter().sum::<f64>() / n, ys.iter().sum::<f64>() / n);
        let (mut sxy, mut sxx, mut syy, mut sq) = (0.0, 0.0, 0.0, 0.0);
        for (x, y) in xs.iter().zip(&ys) {
            sxy += (x - mx) * (y - my);
            sxx += (x - mx).powi(2);
            syy += (y - my).powi(2);
            sq += (x - y).powi(2);
        }
        let correlation = if sxx > 0.0 && syy > 0.0 { sxy / (sxx * syy).sqrt() } else { 0.0 };
        Some(((sq / n).sqrt(), correlation, xs.len()))
    };
    let scores: Vec<(i64, f64, f64, usize)> = (-MAX_LAG_FRAMES..=MAX_LAG_FRAMES).filter_map(|lag| score(lag).map(|(r, c, k)| (lag, r, c, k))).collect();
    let best = scores.iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())?;
    if best.2 < MIN_CORRELATION { return None; }
    let mut lag = best.0 as f64;
    if let (Some(l), Some(r)) = (scores.iter().find(|x| x.0 == best.0 - 1), scores.iter().find(|x| x.0 == best.0 + 1)) {
        let denom = l.1 - 2.0 * best.1 + r.1;
        if denom > 0.0 { lag += (0.5 * (l.1 - r.1) / denom).clamp(-0.5, 0.5); }
    }
    Some(LensDelayEstimate { delay_frames: lag.round() as i32, delay_frames_exact: lag, correlation: best.2, pairs: best.3, zoom_span })
}

/// Where to look: up to `max_windows` windows of `window_ms` (in the same time base as `fps`) around the
/// strongest zooms of the metadata, non-overlapping, each holding at least `MIN_ZOOM_SPAN / 2` of zoom.
/// `meta_ln`: log focal length per frame (`NaN` where unknown). Returns `(from_ms, to_ms)` pairs
pub fn zoom_ranges(meta_ln: &[f64], fps: f64, window_ms: f64, max_windows: usize) -> Vec<(f64, f64)> {
    let n = meta_ln.len();
    let w = ((window_ms / 1000.0 * fps).round() as usize).max(2).min(n);
    if n < 2 || !(fps > 0.0) || max_windows == 0 { return Vec::new(); }
    let step = |i: usize| if meta_ln[i].is_finite() && meta_ln[i + 1].is_finite() { (meta_ln[i + 1] - meta_ln[i]).abs() } else { 0.0 };
    // Zoom inside every window, by a running sum
    let mut sum = 0.0;
    let mut scores: Vec<(f64, usize)> = Vec::with_capacity(n);
    for i in 0..n - 1 {
        sum += step(i);
        if i + 1 >= w { sum -= step(i + 1 - w); }
        if i + 1 >= w - 1 { scores.push((sum, i + 2 - w)); } // window starts at i + 2 - w, covers w frames
    }
    scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let mut taken: Vec<(usize, usize)> = Vec::new();
    for (score, start) in scores {
        if taken.len() >= max_windows || score < MIN_ZOOM_SPAN / 2.0 { break; }
        let (a, b) = (start, (start + w).min(n));
        if taken.iter().any(|(ta, tb)| a < *tb && *ta < b) { continue; }
        taken.push((a, b));
    }
    taken.sort_unstable();
    taken.into_iter().map(|(a, b)| (crate::timestamp_at_frame(a as i32, fps), crate::timestamp_at_frame(b as i32, fps))).collect()
}

/// The synchronization stamps every frame it analyzes with the frame's own time offset added
/// (`AutosyncProcess::feed_frame`; Sony's exceeds a whole frame at 60 fps), so a stamp can't be rounded to a frame
/// index. This inverts the stamp exactly: every frame whose offset could have produced it is tried, and the one
/// whose own stamp reproduces it wins, the nearest of them should the offsets make two frames collide. The
/// offsets don't have to vary slowly for this (a capture area change moves Sony's by more than a frame between
/// two neighbours, which would send a single subtraction to the wrong frame), and a stamp no frame explains is
/// `None` rather than a guess
pub fn frame_of_stamp(stamp_us: i64, per_frame_time_offsets: &[f64], fps: f64) -> Option<usize> {
    if !(fps > 0.0) { return None; }
    let offset_us = |f: usize| (per_frame_time_offsets.get(f).copied().unwrap_or(0.0) * 1000.0).round() as i64;
    let guess = crate::frame_at_timestamp(stamp_us as f64 / 1000.0, fps).max(0) as usize;
    let reach = per_frame_time_offsets.iter().fold(0.0f64, |m, o| m.max(o.abs()));
    let reach = crate::frame_at_timestamp(reach, fps).max(0) as usize + 1;
    (guess.saturating_sub(reach)..=guess + reach).filter_map(|f| {
        let own_ms = (stamp_us - offset_us(f)) as f64 / 1000.0; // the frame's own timestamp, before the offset was added
        let error = (own_ms - crate::timestamp_at_frame(f as i32, fps)).abs();
        (crate::frame_at_timestamp(own_ms, fps) == f as i32).then_some((f, error))
    }).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(f, _)| f)
}

/// Estimates the delay from the synchronization's feature tracks. `params` are the process' own, without the
/// delay under test and without the adaptive zoom, and `meta_ln` is the log focal length per frame extracted
/// from them (`NaN` where unknown), the same curve the analyzed windows were picked on. `None` when the analyzed
/// frames hold no zoom to speak of, when the tracks are too few, or when the picture and the metadata don't agree
pub fn estimate(estimator: &PoseEstimator, params: &ComputeParams, meta_ln: &[f64]) -> Option<LensDelayEstimate> {
    if meta_ln.is_empty() { return None; }
    let fps = params.scaled_fps;
    let offsets: Vec<f64> = params.gyro.read().file_metadata.read().per_frame_time_offsets.clone();

    // Stamp and decode counter of every analyzed frame. The counter runs over the decoded frames (from the first
    // frame of the first analyzed range), not over the video's, so it can't index the metadata by itself; but its
    // difference between the two frames of a pair is exact, where inverting the second stamp as well could land
    // on a neighbour
    let frames: Vec<(i64, usize)> = estimator.sync_results.read().iter().map(|(ts, r)| (*ts, r.frame_no)).collect();
    let frame_no_of = |ts: i64| frames.binary_search_by_key(&ts, |x| x.0).ok().map(|i| frames[i].1);
    let (w, h) = (params.width as f32, params.height as f32);
    let inside = |p: &(f32, f32)| p.0 > -w && p.0 < 2.0 * w && p.1 > -h && p.1 < 2.0 * h;
    let mut steps = Vec::new();
    for (ts, _) in &frames {
        // The optical flow is cached to the next frame, or to the one after for the essential matrix method
        let (lines, frame_size) = [1usize, 2].iter().map(|d| estimator.get_of_lines_for_timestamp(ts, 0, 1.0, *d, false)).find(|x| x.0.is_some()).unwrap_or((None, None));
        let (Some(((ts1, pts1), (ts2, pts2))), Some(frame_size)) = (lines, frame_size) else { continue };
        if pts1.len() != pts2.len() || pts1.len() < MIN_INLIERS || frame_size.0 == 0 { continue; }
        let (Some(f1), Some(no1), Some(no2)) = (frame_of_stamp(ts1, &offsets, fps), frame_no_of(ts1), frame_no_of(ts2)) else { continue };
        if no2 <= no1 { continue; }
        let f2 = f1 + (no2 - no1);
        let scale = params.width as f32 / frame_size.0 as f32;
        let scaled = |pts: &[(f32, f32)]| pts.iter().map(|p| (p.0 * scale, p.1 * scale)).collect::<Vec<_>>();
        // Frame-exact times, the renderer's convention (it adds the per-frame offset itself for the gyro lookup)
        let (t1, t2) = (crate::timestamp_at_frame(f1 as i32, fps), crate::timestamp_at_frame(f2 as i32, fps));
        // No field clamp: these are tracked image features, and a feature the lens has no image of has to
        // drop out of `inside()` below rather than come back as a stand-in at the edge of the field
        let u1 = undistort_points_with_rolling_shutter(&scaled(&pts1), t1, Some(f1), params, 1.0, false, false);
        let u2 = undistort_points_with_rolling_shutter(&scaled(&pts2), t2, Some(f2), params, 1.0, false, false);
        let (from, to): (Vec<_>, Vec<_>) = u1.iter().zip(&u2).filter(|(p, q)| inside(p) && inside(q)).map(|(p, q)| (*p, *q)).unzip();
        if let Some((ln_scale, _)) = similarity_ln_scale(&from, &to) {
            steps.push((t1, t2, ln_scale));
        }
    }
    estimate_from_steps(&steps, meta_ln, fps)
}
