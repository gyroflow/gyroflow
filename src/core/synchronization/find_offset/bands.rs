// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Synchronization by the optical analysis' band measurements (offset method "Optical analysis").
//!
//! The frames of the sync points are tracked once (`OpticalMotionAnalysis::for_sync`: KLT, persistent tracks, every
//! point at its own row's time), and the offset is found in two steps, both measuring the same tracks again:
//! - coarse: the picture's own rotation from frame to frame (the motion data set aside), correlated with the motion
//!   data's over the whole search range, all sync points together on a line - an offset and a drift, as the clocks of
//!   loggers and cameras drift apart;
//! - fine: each sync point measured at its offset - the rotation each band of rows moved by beyond the motion data -
//!   and the delay that brings the motion data's rotation closest to the picture's, measured again at the new offset
//!   until it settles.
//!
//! rs-sync (`rs_sync`) takes a translation of its own for every frame pair, and a timing error, mostly a rotation about
//! the axes across the view, is explained by such a translation about as well: rs-sync sees 4% (20° lens) to 26% (150°
//! fisheye) of it. Here parallax is told from rotation by the tracks' temporal high-pass instead. On 13 clips with the
//! camera's own timing (the `sync_ab` example) this found the offset within 0.27 ms (median), and none a frame off,
//! where rs-sync on DIS was 2.9 ms off and a third of the time more than a frame

use std::collections::HashMap;
use std::sync::{ Arc, atomic::{ AtomicBool, Ordering::Relaxed } };
use nalgebra::{ Matrix3, Quaternion, Rotation3, UnitQuaternion, Vector3 };
use parking_lot::RwLock;
use rayon::prelude::*;
use crate::gyro_source::GyroSource;
use super::super::{ SyncParams, optical_motion::OpticalMeasurements };

/// A band's χ² counts no more than this: what's left of the picture that moves on its own
const CAP: f64 = 25.0;
/// The median band χ² the noise floor brings the bands to: three degrees of freedom
const MEDIAN_CHI2: f64 = 2.366;
/// The delays tried, ms apart
const STEP_MS: f64 = 0.25;
/// Around the best delay tried, the parabola fitted over this far each way (ms): the χ² is rough at `STEP_MS`, and the
/// three best points alone let it pick between dips a fraction of a ms apart
const PARABOLA_MS: f64 = 1.5;
/// Measurements of a sync point at most, and the step below which it's settled (ms)
const ROUNDS: usize = 6;
const SETTLED_MS: f64 = 0.05;
/// How fast the motion data's clock may drift from the video's, a fraction: the coarse step finds the sync points on a
/// line, and each sync point's own fit (`fine`, ±3 frames) takes up what's off it. Crystals keep within 100 ppm, but
/// motion data timed by a nominal sample rate doesn't: a HOVER AQUA clip drifted 0.87%
const DRIFT_MAX: f64 = 0.02;
/// A sync point whose fit leaves more of the picture's rotation unexplained than this is dropped: up to 44% on the right
/// ones, 155% (worse than no motion data at all) on the one that ended up off the line on that HOVER clip
const MAX_UNEXPLAINED: f64 = 0.75;
/// The best line is trusted unless it's weak both ways: a pooled correlation below `MIN_NCC`, and no more than
/// `MIN_LEAD` above the best line 100 ms or more away from it. On 17 clips the two that went wrong were both
/// (0.29 / 0.004 lead, a nearly still clip 3.4 s off; 0.45 / 0.011, one 3.5 s off), the right ones at most one: 0.49
/// with a 0.25 lead, 0.56 with 0.004 (a vibrating drone, its correlation one broad plateau), most above 0.9
const MIN_NCC: f64 = 0.5;
const MIN_LEAD: f64 = 0.05;

/// The offset of each sync point (the `ranges_us`, on the video's scaled timeline) as the other methods give them: the
/// middle of its range (ms), the offset (ms), and the cost - here the noise floor its fit took, µrad, lower for a
/// picture that follows the motion data more closely. Sync points without one are left out
pub fn find_offsets<F: Fn(f64) + Sync>(meas: &OpticalMeasurements, ranges_us: &[(i64, i64)], sync_params: &SyncParams, frame_ms: f64, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> Vec<(f64, f64, f64)> {
    let cancelled = || cancel_flag.load(Relaxed);
    let windows: Vec<(f64, f64)> = ranges_us.iter().map(|(a, b)| (*a as f64 / 1000.0, *b as f64 / 1000.0)).collect();
    // Looked up by the motion data's own timestamps from here on: one copy of it for the whole sync, without what only its
    // integration and smoothing read (a long clip's IMU readings and smoothed orientations), which every measurement at an
    // offset tried takes the sync points of (`fine`)
    let g = Arc::new(RwLock::new({ let mut g = meas.motion_for_lookups(true); g.clear_offsets(); g }));

    // The picture's own rotation, against no motion data
    let mut without_motion = meas.motion_for_lookups(false);
    without_motion.clear_offsets();
    let steps = match meas.remeasure_against(without_motion, true, cancelled) {
        Ok(vision) => vision_steps(&vision, frame_ms, &g.read()),
        Err(e) => { log::warn!("Optical sync: the picture's own rotation couldn't be measured ({e}), starting from the initial offset"); Vec::new() },
    };
    if cancelled() { return Vec::new(); }
    progress_cb(0.15);
    let window_steps: Vec<Vec<&Step>> = windows.iter().map(|(a, b)| steps.iter().filter(|s| (*a..=*b).contains(&s.t0)).collect()).collect();
    let starts = if steps.is_empty() {
        vec![sync_params.initial_offset; windows.len()]
    } else {
        // When the picture's own rotation was measured and follows the motion data nowhere, there's nothing to sync on
        match coarse(&window_steps, &windows, &g.read(), sync_params.initial_offset, sync_params.search_size) {
            Some(starts) => starts,
            None => return Vec::new(),
        }
    };
    if cancelled() { return Vec::new(); }
    progress_cb(0.25);

    let found = fine(meas, &g, &windows, &starts, frame_ms, |p| progress_cb(0.25 + 0.75 * p), &cancelled);
    found.into_iter().zip(&windows).filter_map(|(f, (a, b))| {
        let (offset, floor) = f?;
        // As the other methods: what's at the end of the search range is more likely the end than the offset
        if (offset - sync_params.initial_offset).abs() >= sync_params.search_size * 0.9 {
            log::warn!("Sync point out of acceptable range {} < {}", (offset - sync_params.initial_offset).abs(), sync_params.search_size * 0.9);
            return None;
        }
        Some(((a + b) / 2.0, offset, floor * 1e6))
    }).collect()
}

/// The shortest sync point this measures well, ms: tracks count only from 15 frame pairs on (`optical_motion`'s
/// high-pass), and need some to spare. "Time per sync point" at 0.5 s (enough for rs-sync, which works on frame pairs)
/// left 15 frames at 29.97 fps, no track long enough, and four of five sync points without a band
pub fn min_window_ms(fps: f64) -> f64 { (45.0 * 1000.0 / fps.max(1.0)).max(1500.0) }

/// The width the analysis tracks at, as "Analyze image optically" decodes for it (`rendering::analyze_optically`)
const TRACK_WIDTH: u32 = 960;

/// A frame of the synchronization's decoder (the processing resolution, up to the whole one) at about the width the
/// analysis is made for: area-averaged down to `TRACK_WIDTH` when it's over a quarter wider. None when it's fine as it is
pub fn shrink(width: u32, height: u32, stride: usize, pixels: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if width <= TRACK_WIDTH * 5 / 4 || height < 2 || pixels.len() < stride * (height as usize - 1) + width as usize { return None; }
    let tw = TRACK_WIDTH;
    let th = (((height as f64 * tw as f64 / width as f64) / 2.0).round() * 2.0).max(2.0) as u32;
    // Each output pixel covers [i·s, (i+1)·s) of the input: the input pixels it overlaps, by how much
    let spans = |n_out: u32, n_in: u32| -> Vec<Vec<(usize, f32)>> {
        let s = n_in as f64 / n_out as f64;
        (0..n_out).map(|i| {
            let (a, b) = (i as f64 * s, ((i + 1) as f64 * s).min(n_in as f64));
            (a.floor() as usize..b.ceil() as usize).filter_map(|x| {
                let w = (b.min(x as f64 + 1.0) - a.max(x as f64)) / s;
                (w > 0.0).then_some((x, w as f32))
            }).collect()
        }).collect()
    };
    let (cols, rows) = (spans(tw, width), spans(th, height));
    let tw = tw as usize;
    // Across first, into rows of floats, then down
    let mut across = vec![0f32; tw * height as usize];
    for (y, dst) in across.chunks_exact_mut(tw).enumerate() {
        let src = &pixels[y * stride..y * stride + width as usize];
        for (d, c) in dst.iter_mut().zip(&cols) { *d = c.iter().map(|(x, w)| src[*x] as f32 * w).sum(); }
    }
    let mut out = vec![0u8; tw * th as usize];
    for (dst, r) in out.chunks_exact_mut(tw).zip(&rows) {
        for (x, d) in dst.iter_mut().enumerate() {
            *d = r.iter().map(|(y, w)| across[y * tw + x] * w).sum::<f32>().round().clamp(0.0, 255.0) as u8;
        }
    }
    Some((tw as u32, th, out))
}

/// Where each sync point's fit starts from: on the line - the offset at the middle of the sync points, and how fast it
/// drifts along them, up to `DRIFT_MAX` - on which the picture's own rotation correlates best with the motion data's.
/// Every ms of offset and every ms of drift at the farthest sync point, within `radius` of `center`, all sync points
/// pooled: each counts by how much it moves, so one that hardly does can't outvote the others with a chance match. None
/// when the best line is weak both ways, see `MIN_NCC`
fn coarse(window_steps: &[Vec<&Step>], windows: &[(f64, f64)], g0: &GyroSource, center: f64, radius: f64) -> Option<Vec<f64>> {
    let mids: Vec<f64> = windows.iter().map(|(a, b)| (a + b) / 2.0).collect();
    let used: Vec<usize> = (0..windows.len()).filter(|w| window_steps[*w].len() >= 10).collect();
    if used.is_empty() { return None; }
    let t_ref = used.iter().map(|w| mids[*w]).sum::<f64>() / used.len() as f64;
    let reach = used.iter().map(|w| (mids[*w] - t_ref).abs()).fold(0.0, f64::max);
    let r = radius.round() as i64;
    let nb = (DRIFT_MAX * reach).floor() as i64;
    // Each sync point's sums at every ms its offset can be at: within `radius` of `center`, and as far again as it drifts
    let k = r + nb + 1;
    let parts: Vec<Vec<Option<(f64, f64, f64)>>> = used.iter().map(|w| {
        (0..(2 * k + 1) as usize).into_par_iter().with_min_len(64).map(|j| ncc_parts(&window_steps[*w], g0, center + (j as i64 - k) as f64)).collect()
    }).collect();
    // bi ms at the farthest sync point
    let drift_of = |bi: i64| if reach > 0.0 { bi as f64 / reach } else { 0.0 };
    let line = |ai: i64, shift: &[i64]| {
        let (mut num, mut vv, mut gg) = (0.0, 0.0, 0.0);
        for (p, s) in parts.iter().zip(shift) {
            if let Some(Some(p)) = p.get((ai + s) as usize) { num += p.0; vv += p.1; gg += p.2; }
        }
        if vv > 0.0 && gg > 0.0 { num / (vv * gg).sqrt() } else { f64::MIN }
    };
    let shifts = |bi: i64| -> Vec<i64> { used.iter().map(|w| (drift_of(bi) * (mids[*w] - t_ref)).round() as i64 + k).collect() };
    // For every offset at the middle, its best line, whatever the drift: what tells a peak from the rest
    let n = (2 * r + 1) as usize;
    let per_offset = (-nb..=nb).into_par_iter().fold(|| vec![f64::MIN; n], |mut acc, bi| {
        let shift = shifts(bi);
        for (ai, a) in (-r..=r).zip(acc.iter_mut()) { *a = a.max(line(ai, &shift)); }
        acc
    }).reduce(|| vec![f64::MIN; n], |a, b| a.into_iter().zip(b).map(|(a, b)| a.max(b)).collect());
    let (best_i, &score) = per_offset.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    let ai = best_i as i64 - r;
    let drift = drift_of((-nb..=nb).max_by(|x, y| line(ai, &shifts(*x)).total_cmp(&line(ai, &shifts(*y))))?);
    let at = center + ai as f64;
    // The best line 100 ms or more away at the middle, and how far the best stands out of all of them
    let runner_up = per_offset.iter().enumerate().filter(|(i, _)| (*i as i64 - best_i as i64).abs() > 100).map(|(_, s)| *s).fold(f64::MIN, f64::max);
    let finite: Vec<f64> = per_offset.iter().copied().filter(|s| *s > f64::MIN).collect();
    let mean = finite.iter().sum::<f64>() / finite.len().max(1) as f64;
    let sd = (finite.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / finite.len().max(1) as f64).sqrt();
    log::debug!("Optical sync: coarse offset {at:.0} ms at {t_ref:.0} ms, drift {:.0} ppm, correlation {score:.3}, next best {runner_up:.3} (more than 100 ms away), {:.1} sd above the mean {mean:.3}", drift * 1e6, (score - mean) / sd.max(1e-9));
    if score < MIN_NCC && score - runner_up < MIN_LEAD {
        log::warn!("Optical sync: the video's own rotation follows the motion data at no offset (correlation {score:.2} at best, {runner_up:.2} elsewhere), not enough motion to sync on");
        return None;
    }
    Some(mids.iter().map(|m| at + drift * (m - t_ref)).collect())
}

/// The band fit from `from`, each sync point on its own (they can differ, the clocks drift): measured at its offset, the
/// delay that fits, measured again at the new offset, until the next step is below `SETTLED_MS`. A measurement keeps the
/// slow part of the motion data's rotation at the offset it was taken at (the tracks' high-pass takes out the slow part
/// of what the offset gets wrong), so a delay found from it is short of the whole one: by 38% on a C50 clip, 73% on a
/// ZV-E1 one. From the second round on, the next offset is where the line through the last two delays crosses 0, and a
/// slope no measurement gives means the delays are down to the fit's noise: the sync point then gets the mean of its last
/// two. The fixed point doesn't depend on that slope: measured at the right offset, the delay found is 0.
/// Each sync point's offset, and the noise floor (rad) of its last fit. `g` is the motion data without sync points: each
/// round measures against it with the round's sync points, and the fits look it up without them again
fn fine(meas: &OpticalMeasurements, g: &Arc<RwLock<GyroSource>>, windows: &[(f64, f64)], from: &[f64], frame_ms: f64, progress: impl Fn(f64), cancelled: &dyn Fn() -> bool) -> Vec<Option<(f64, f64)>> {
    // An offset this far off is the coarse step's to find
    let range = (3.0 * frame_ms).max(40.0);
    let n = windows.len();
    let mut offsets = from.to_vec();
    let mut prev: Vec<Option<(f64, f64)>> = vec![None; n]; // the offset measured at and the delay found, last round
    let mut found: Vec<Option<(f64, f64)>> = vec![None; n];
    let mut open = vec![true; n];
    for round in 0..ROUNDS {
        if cancelled() { break; }
        // Each at the middle of its range, as the app sets a sync point: set on the one copy of the motion data for the
        // measurement, and taken off again for the fits, which look it up by its own timestamps
        g.write().set_offsets(windows.iter().zip(&offsets).filter(|(_, o)| o.is_finite()).map(|((a, b), o)| ((((a + b) / 2.0 - o) * 1000.0).round() as i64, *o)).collect());
        let measured = meas.remeasure_against_shared(g.clone(), false, cancelled);
        g.write().clear_offsets();
        let guard = g.read();
        let g0: &GyroSource = &guard;
        let bands = match measured {
            Ok(m) => bands_of(&m, g0),
            Err(e) => { log::warn!("Optical sync: measuring failed: {e}"); break; },
        };
        let todo: Vec<usize> = (0..n).filter(|w| open[*w]).collect();
        for w in todo {
            let o = offsets[w];
            let sel = in_window(&bands, windows[w], o);
            let Some((d, floor)) = fit_delay(&sel, g0, range) else {
                // Few bands: too little of the picture tracked long enough (blur, no texture, too short a sync point)
                log::warn!("Optical sync: sync point {w} measured at {o:.2} ms: no least χ² within ±{range:.0} ms ({} bands)", sel.len());
                open[w] = false;
                continue;
            };
            // What of the picture's rotation the motion data at that delay leaves unexplained
            let unexplained = sel.iter().map(|b| residual(b, g0, d).norm_squared()).sum::<f64>() / sel.iter().map(|b| log_rotation(&b.x).norm_squared()).sum::<f64>().max(1e-20);
            log::debug!("Optical sync: sync point {w} measured at {o:.2} ms: delay {d:.3} ms ({} bands, noise floor {:.1} µrad, {:.1}% of the picture's rotation unexplained)", sel.len(), floor * 1e6, unexplained * 100.0);
            let gain = prev[w].filter(|p| (o - p.0).abs() > 1e-3).map(|p| (d - p.1) / (o - p.0));
            let noise = gain.is_some_and(|g| !(0.1..=2.0).contains(&g));
            let step = gain.filter(|_| !noise).map(|g| (d / g).clamp(-range, range)).unwrap_or(d);
            if step.abs() < SETTLED_MS || noise || round + 1 == ROUNDS {
                // Not trusted when it can't be told either
                found[w] = if !(unexplained <= MAX_UNEXPLAINED) {
                    log::warn!("Optical sync: sync point {w} left out, the motion data at {:.2} ms leaves {:.0}% of the video's rotation unexplained", o - d, unexplained * 100.0);
                    None
                } else {
                    Some(match (noise, prev[w]) {
                        (true, Some(p)) => (((o - d) + (p.0 - p.1)) / 2.0, floor),
                        _ => (o - d, floor),
                    })
                };
                open[w] = false;
            } else {
                prev[w] = Some((o, d));
                offsets[w] = o - step;
            }
        }
        progress((round + 1) as f64 / ROUNDS as f64);
        if !open.iter().any(|x| *x) { break; }
    }
    found
}

/// A band measurement as the fit needs it: the picture's rotation X = exp(ρ)·M, its times on the motion data's
/// timeline (ms), the covariance of ρ, and the part of a video cut from recordings its frames are of
/// (`GyroSource::part_of_frame`), whose motion data the render reads them in
struct Band { x: Matrix3<f64>, ta: f64, tb: f64, cov: Matrix3<f64>, part: Option<usize> }

fn bands_of(m: &OpticalMeasurements, g0: &GyroSource) -> Vec<Band> {
    // A pair's two frames are of one part: the tracking starts over where the next part starts
    let pairs = if g0.segment_ranges().is_some() { m.pair_frames() } else { Vec::new() };
    let part = |pair: usize| pairs.get(pair).and_then(|(a, _)| g0.part_of_frame(*a));
    m.bands.iter().filter_map(|b| Some(Band { x: Rotation3::new(b.rho).into_inner() * b.m, ta: b.ta_us / 1000.0, tb: b.tb_us / 1000.0, cov: b.info.try_inverse()?, part: part(b.pair) })).collect()
}

/// The bands of a sync point's range: a band's time on the video's timeline is its time on the motion data's plus the
/// offset it was measured at
fn in_window(bands: &[Band], (a, b): (f64, f64), offset: f64) -> Vec<&Band> {
    bands.iter().filter(|x| (a..=b).contains(&(x.ta + offset))).collect()
}

/// What the motion data's rotation misses of the picture's, with the band's times moved by `delay` on the motion data's
/// timeline. `g0` has no sync points: its timestamps are the motion data's own, read as the render reads the band's
/// frames, in their part's motion data. Not the part the video shows at a time: one at a part's start moved before the
/// part's video starts would read the part before, of another moment
fn residual(b: &Band, g0: &GyroSource, delay: f64) -> Vector3<f64> {
    let q = |t: f64| g0.org_quat_at_motion_time(t + delay, b.part);
    let m = (q(b.tb).inverse() * q(b.ta)).to_rotation_matrix().into_inner();
    log_rotation(&(b.x * m.transpose()))
}

/// The rotation vector of a rotation matrix. `Rotation3::scaled_axis` takes the angle from the trace by an arc cosine it
/// doesn't clamp: NaN for a rotation so small that rounding puts the trace over 3 - the residual of a band that fits
/// exactly, which the cap then counted as an outlier. Through a quaternion the angle is an arc tangent
fn log_rotation(m: &Matrix3<f64>) -> Vector3<f64> {
    UnitQuaternion::from_rotation_matrix(&Rotation3::from_matrix_unchecked(*m)).scaled_axis()
}

/// Each band's weight, with `floor` (rad²) added to the variance of its rotation in every direction
fn weights(bands: &[&Band], floor: f64) -> Vec<Matrix3<f64>> {
    bands.iter().map(|b| (b.cov + Matrix3::identity() * floor).try_inverse().unwrap_or_else(Matrix3::zeros)).collect()
}

fn chi2(bands: &[&Band], w: &[Matrix3<f64>], g0: &GyroSource, delay: f64) -> f64 {
    bands.iter().zip(w).map(|(b, w)| { let r = residual(b, g0, delay); (r.transpose() * w * r)[(0, 0)].min(CAP) }).sum()
}

/// The variance (rad², the same in every direction) the bands' own covariance leaves out: what brings the median band's
/// χ² at `delay` to `MEDIAN_CHI2`. A band's covariance is the tracking's alone, while the motion data has an error of
/// its own over a frame and the picture a model error: without it nearly every band was capped even at the best delay
/// (χ² 20,650 of 21,300 on a C50 clip), the few that weren't made the curve rough, and on a long lens (a7S III, 29°) the
/// least of it was at the end of the range. It came out 0.05 to 1.5 mrad on clips the picture follows the motion data
/// in, and 3 to 9 on ones stabilized in the camera (HyperSmooth against the raw IMU)
fn noise_floor(bands: &[&Band], g0: &GyroSource, delay: f64) -> f64 {
    let rs: Vec<Vector3<f64>> = bands.iter().map(|b| residual(b, g0, delay)).collect();
    let median = |floor: f64| {
        let mut q: Vec<f64> = bands.iter().zip(&rs).map(|(b, r)| (b.cov + Matrix3::identity() * floor).try_inverse().map(|w| (r.transpose() * w * r)[(0, 0)]).unwrap_or(0.0)).collect();
        q.sort_by(f64::total_cmp);
        q[q.len() / 2]
    };
    if median(0.0) <= MEDIAN_CHI2 { return 0.0; }
    // Bisected in log, between nothing and ten times the largest residual
    let (mut lo, mut hi) = (1e-20f64, rs.iter().map(|r| r.norm_squared()).fold(1e-20, f64::max) * 10.0);
    for _ in 0..60 {
        let mid = (lo * hi).sqrt();
        if median(mid) > MEDIAN_CHI2 { lo = mid; } else { hi = mid; }
    }
    hi
}

/// The delay that fits the bands, within `range`, and the noise floor it took (rad): positive when the picture's motion
/// happens that much later on the motion data's timeline than the offset measured at puts it. The floor found at the
/// offset measured at is too large by what the delay adds to the residuals, which only widens the curve: it's found
/// again at the delay that gives. None with too few bands, or when the least χ² is at the end of the range
fn fit_delay(bands: &[&Band], g0: &GyroSource, range: f64) -> Option<(f64, f64)> {
    if bands.len() < 20 { return None; }
    let floor = noise_floor(bands, g0, 0.0);
    let d = least_chi2(bands, &weights(bands, floor), g0, range)?;
    let floor = noise_floor(bands, g0, d);
    Some((least_chi2(bands, &weights(bands, floor), g0, range)?, floor.sqrt()))
}

/// The delay with the least χ² for these weights, within `range`: tried every `STEP_MS`, then the vertex of a parabola
/// fitted around the best one
fn least_chi2(bands: &[&Band], w: &[Matrix3<f64>], g0: &GyroSource, range: f64) -> Option<f64> {
    let k = (range / STEP_MS).round() as i64;
    let pts: Vec<(f64, f64)> = (-k..=k).into_par_iter().map(|i| { let d = i as f64 * STEP_MS; (d, chi2(bands, w, g0, d)) }).collect();
    let (bi, &(bd, _)) = pts.iter().enumerate().min_by(|a, b| a.1.1.total_cmp(&b.1.1))?;
    if bi == 0 || bi + 1 == pts.len() { return None; }
    // c0 + c1·x + c2·x², x from the best one
    let (mut ata, mut atb) = (Matrix3::<f64>::zeros(), Vector3::<f64>::zeros());
    let mut n = 0;
    for p in pts.iter().filter(|p| (p.0 - bd).abs() <= PARABOLA_MS + 1e-9) {
        let x = p.0 - bd;
        let row = Vector3::new(1.0, x, x * x);
        ata += row * row.transpose();
        atb += row * p.1;
        n += 1;
    }
    if n < 5 { return Some(bd); }
    let c = ata.try_inverse()? * atb;
    let vertex = -c[1] / (2.0 * c[2]);
    Some(if c[2] > 0.0 && vertex.abs() <= PARABOLA_MS { bd + vertex } else { bd })
}

/// One step of the picture's own rotation: from the frame at `t0` to the next one at `t1` (ms, on the video's timeline,
/// with the frames' own timing), as a rotation vector
/// And the part of a video cut from recordings its frames are of, as `Band::part`
struct Step { t0: f64, t1: f64, v: Vector3<f64>, part: Option<usize> }

fn vision_steps(m: &OpticalMeasurements, frame_ms: f64, g0: &GyroSource) -> Vec<Step> {
    let q = |a: [f32; 4]| UnitQuaternion::from_quaternion(Quaternion::new(a[0] as f64, a[1] as f64, a[2] as f64, a[3] as f64));
    // In a video cut from recordings, each frame's part: no step goes from one part to the next, where the picture jumps
    // to another moment (the chained rotation goes on from where it was, and the motion data jumps between the parts')
    let frames = if g0.segment_ranges().is_some() { m.frames_by_time() } else { HashMap::new() };
    let part = |key: i64| frames.get(&key).and_then(|f| g0.part_of_frame(*f));
    m.video_base.windows(2).filter_map(|w| {
        let (t0, t1) = (w[0].0 as f64 / 1000.0, w[1].0 as f64 / 1000.0);
        let p = part(w[0].0);
        (t1 > t0 && t1 - t0 < frame_ms * 1.5 && p == part(w[1].0)).then(|| Step { t0, t1, v: (q(w[0].1).inverse() * q(w[1].1)).scaled_axis(), part: p })
    }).collect()
}

/// How much the motion data at `offset` rotates like the picture over these steps, as the sums of the normalized
/// cross-correlation of the rotation vectors (each without its mean): the products, the picture's energy, the motion
/// data's. Summed over sync points they pool them, see `coarse`. None with too few steps to tell
fn ncc_parts(steps: &[&Step], g0: &GyroSource, offset: f64) -> Option<(f64, f64, f64)> {
    if steps.len() < 10 { return None; }
    let g: Vec<Vector3<f64>> = steps.iter().map(|s| (g0.org_quat_at_motion_time(s.t0 - offset, s.part).inverse() * g0.org_quat_at_motion_time(s.t1 - offset, s.part)).scaled_axis()).collect();
    let n = steps.len() as f64;
    let vm = steps.iter().map(|s| s.v).sum::<Vector3<f64>>() / n;
    let gm = g.iter().sum::<Vector3<f64>>() / n;
    let mut p = (0.0, 0.0, 0.0);
    for (s, g) in steps.iter().zip(&g) {
        let (a, b) = (s.v - vm, g - gm);
        p.0 += a.dot(&b);
        p.1 += a.norm_squared();
        p.2 += b.norm_squared();
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::{ shrink, coarse, Step };
    use crate::gyro_source::GyroSource;
    use nalgebra::{ UnitQuaternion, Vector3 };

    #[test]
    fn shrink_averages_what_each_pixel_covers() {
        // Twice the width it's tracked at: columns of 0 and 200 average to 100, and the padding past the width (255)
        // doesn't get in
        let (w, h, stride) = (1920u32, 1080u32, 2048usize);
        let px: Vec<u8> = (0..stride * h as usize).map(|i| match i % stride { x if x >= w as usize => 255, x => if x % 2 == 0 { 0 } else { 200 } }).collect();
        let (tw, th, out) = shrink(w, h, stride, &px).unwrap();
        assert_eq!((tw, th), (960, 540));
        assert!(out.iter().all(|v| *v == 100), "{:?}", &out[..8]);
        // A ratio that isn't whole covers parts of pixels: their weights still add up to one
        let (tw, th, out) = shrink(1280, 720, 1280, &vec![77; 1280 * 720]).unwrap();
        assert_eq!((tw, th), (960, 540));
        assert!(out.iter().all(|v| *v == 77));
        // Narrow enough as it is
        assert!(shrink(1200, 676, 1200, &vec![0; 1200 * 676]).is_none());
    }

    #[test]
    fn coarse_follows_a_drifting_clock() {
        // Motion data turning at an irregular rate, and the picture's steps taken from it where a clock 1500 ppm off puts
        // them, at three sync points a minute apart: each starts within a ms and a half of its own offset
        let mut g = GyroSource::new();
        g.duration_ms = 200_000.0;
        let rate = |t: f64| Vector3::new((t / 170.0).sin() + 0.5 * (t / 53.0).cos(), (t / 97.0).cos(), 0.3 * (t / 31.0).sin());
        let mut q = UnitQuaternion::identity();
        let quats = std::sync::Arc::make_mut(&mut g.quaternions);
        for i in 0..200_000i64 {
            quats.insert(i * 1000, q);
            q = q * UnitQuaternion::from_scaled_axis(rate(i as f64) * 0.001);
        }
        let truth = |t: f64| 37.0 + 1.5e-3 * (t - 100_000.0);
        let windows = [(39_000.0, 41_000.0), (99_000.0, 101_000.0), (159_000.0, 161_000.0)];
        let frame = 1000.0 / 30.0;
        let steps: Vec<Vec<Step>> = windows.iter().map(|(a, b)| (0..((b - a) / frame) as usize).map(|k| {
            let (t0, t1) = (a + k as f64 * frame, a + (k + 1) as f64 * frame);
            let v = (g.org_quat_at_timestamp(t0 - truth(t0)).inverse() * g.org_quat_at_timestamp(t1 - truth(t1))).scaled_axis();
            Step { t0, t1, v, part: None }
        }).collect()).collect();
        let steps: Vec<Vec<&Step>> = steps.iter().map(|s| s.iter().collect()).collect();
        let starts = coarse(&steps, &windows, &g, 0.0, 1000.0).expect("a line");
        for ((a, b), s) in windows.iter().zip(&starts) {
            assert!((s - truth((a + b) / 2.0)).abs() < 1.5, "{s} vs {}", truth((a + b) / 2.0));
        }
        // 180 ms apart: one shared offset would leave the outer ones 90 ms off, past what the fine step takes up
        assert!((starts[2] - starts[0]).abs() > 150.0);
    }
}
