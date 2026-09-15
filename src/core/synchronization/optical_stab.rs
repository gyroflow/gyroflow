// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow contributors

//! Optical-only motion and a global residual (second-pass) stabilizer.
//!
//! Gyroflow already estimates camera rotation from optical flow when a clip has
//! no gyro/IMU: Auto sync analyzes the file and writes those rates as motion
//! data. That path is the first half of issue #45.
//!
//! This module is the reviewable second half, with honest scope:
//!
//! * **Motion source** — Auto keeps the existing "optical as gyro only when the
//!   file has no motion data" behavior. Optical-only makes Auto sync replace
//!   camera motion with the optical-flow estimate even when gyro exists (bad
//!   gyro, Runcam spikes, historical footage). Gyro-only never replaces it.
//! * **Residual pass** — after that camera-motion estimate (or after real gyro),
//!   leftover 2D similarity (translation / rotation / scale) is fitted to the
//!   tracks, accumulated, and high-pass filtered. The correction is a single
//!   global similarity per timestamp, applied in the existing render matrices.
//!   It is *not* a local Warp-Stabilizer mesh and *not* an AI optical-flow
//!   backend (#831).
//!
//! Fail-closed: sparse tracks, non-finite points, scene-cut sized jumps, and
//! impossible scales produce identity rather than a wild warp.

use nalgebra::{Matrix3, Rotation3, Vector3};
use std::collections::BTreeMap;

use crate::util::MapClosest;

pub use crate::stabilization_params::{ OpticalMotionMode, Similarity2D };

const MIN_INLIERS: usize = 8;
const MAX_SCALE: f64 = 1.25;
const MIN_SCALE: f64 = 0.80;
/// Reject an increment whose translation is this fraction of the shorter image side (cuts / failed tracks)
const CUT_TRANSLATION_FRAC: f64 = 0.20;
/// Reject an increment whose rotation is above this (radians, ~15°)
const CUT_ROTATION: f64 = 15.0 * std::f64::consts::PI / 180.0;

/// Feature tracks between two timestamps, optionally with a 3D rotation already explained by camera motion
#[derive(Clone, Debug)]
pub struct TrackedPair {
    pub timestamp_us: i64,
    pub next_timestamp_us: i64,
    pub from: Vec<(f32, f32)>,
    pub to: Vec<(f32, f32)>,
    pub remove_rotation: Option<Rotation3<f64>>,
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub width: u32,
    pub height: u32,
}

/// Least-squares similarity with one round of median-residual outlier rejection.
/// Same estimator as `lens_delay::similarity_ln_scale`, returning the full transform.
pub fn fit_similarity(from: &[(f32, f32)], to: &[(f32, f32)]) -> Option<Similarity2D> {
    fn fit(pairs: &[((f64, f64), (f64, f64))]) -> Option<(f64, f64, f64, f64)> {
        if pairs.len() < 4 { return None; }
        let n = pairs.len() as f64;
        let (mx, my, mu, mv) = pairs.iter().fold((0.0, 0.0, 0.0, 0.0), |acc, (p, q)| {
            (acc.0 + p.0, acc.1 + p.1, acc.2 + q.0, acc.3 + q.1)
        });
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
    if pairs.len() < MIN_INLIERS { return None; }
    let (a, b, tx, ty) = fit(&pairs)?;
    let residual = |p: &(f64, f64), q: &(f64, f64), a: f64, b: f64, tx: f64, ty: f64| {
        ((a * p.0 - b * p.1 + tx - q.0).powi(2) + (b * p.0 + a * p.1 + ty - q.1).powi(2)).sqrt()
    };
    let mut res: Vec<f64> = pairs.iter().map(|(p, q)| residual(p, q, a, b, tx, ty)).collect();
    res.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let threshold = (res[res.len() / 2] * 3.0).max(0.5);
    pairs.retain(|(p, q)| residual(p, q, a, b, tx, ty) <= threshold);
    if pairs.len() < MIN_INLIERS { return None; }
    let (a, b, tx, ty) = fit(&pairs)?;
    let scale = (a * a + b * b).sqrt();
    if !(scale.is_finite() && scale > 0.0) { return None; }
    let rot = b.atan2(a);
    let s = Similarity2D { tx, ty, rot, scale };
    s.is_finite().then_some(s)
}

/// Warp `from` by a pinhole camera rotation so a following similarity fit sees only leftover motion
pub fn apply_camera_rotation(
    points: &[(f32, f32)],
    rot: &Rotation3<f64>,
    fx: f64,
    fy: f64,
    cx: f64,
    cy: f64,
) -> Vec<(f32, f32)> {
    let fx = fx.max(1e-6);
    let fy = fy.max(1e-6);
    points.iter().map(|&(x, y)| {
        let dir = Vector3::new((x as f64 - cx) / fx, (y as f64 - cy) / fy, 1.0);
        let p = rot * dir;
        if !p.z.is_finite() || p.z.abs() < 1e-8 {
            return (x, y);
        }
        ((fx * p.x / p.z + cx) as f32, (fy * p.y / p.z + cy) as f32)
    }).collect()
}

fn is_cut(s: &Similarity2D, width: u32, height: u32) -> bool {
    let short = width.min(height).max(1) as f64;
    let trans = (s.tx * s.tx + s.ty * s.ty).sqrt();
    trans > CUT_TRANSLATION_FRAC * short
        || s.rot.abs() > CUT_ROTATION
        || s.scale < MIN_SCALE
        || s.scale > MAX_SCALE
        || !s.is_finite()
}

/// Incremental leftover similarities, one per tracked pair. Cuts start a new identity segment.
pub fn residual_increments(pairs: &[TrackedPair]) -> BTreeMap<i64, Similarity2D> {
    let mut out = BTreeMap::new();
    for p in pairs {
        if p.from.len() != p.to.len() || p.from.len() < MIN_INLIERS {
            continue;
        }
        let predicted = if let Some(rot) = p.remove_rotation.as_ref() {
            apply_camera_rotation(&p.from, rot, p.fx, p.fy, p.cx, p.cy)
        } else {
            p.from.clone()
        };
        let Some(inc) = fit_similarity(&predicted, &p.to) else { continue; };
        if is_cut(&inc, p.width, p.height) {
            // Scene change / tracking failure: do not carry motion across the cut
            out.insert(p.timestamp_us, Similarity2D::identity());
            continue;
        }
        out.insert(p.timestamp_us, inc);
    }
    out
}

/// Accumulate increments into a path, then high-pass it: correction = path ∘ smooth(path)⁻¹
pub fn residual_corrections(increments: &BTreeMap<i64, Similarity2D>, window_s: f64) -> BTreeMap<i64, Similarity2D> {
    if increments.is_empty() { return BTreeMap::new(); }
    let keys: Vec<i64> = increments.keys().copied().collect();
    let mut path = Vec::with_capacity(keys.len());
    let mut acc = Similarity2D::identity();
    for k in &keys {
        if let Some(inc) = increments.get(k) {
            acc = acc.compose(*inc);
        }
        path.push(acc);
    }
    let smoothed = smooth_path(&keys, &path, window_s);
    keys.iter().zip(path.iter()).zip(smoothed.iter()).map(|((k, p), s)| {
        (*k, s.compose(p.inverse()))
    }).collect()
}

fn smooth_path(keys: &[i64], path: &[Similarity2D], window_s: f64) -> Vec<Similarity2D> {
    if keys.is_empty() { return Vec::new(); }
    let window_us = (window_s.max(0.0) * 1_000_000.0).round() as i64;
    if window_us <= 0 {
        return path.to_vec();
    }
    let mut out = Vec::with_capacity(path.len());
    let mut left = 0usize;
    let mut right = 0usize;
    for i in 0..keys.len() {
        let t = keys[i];
        while left < i && keys[left] < t - window_us { left += 1; }
        if right < i { right = i; }
        while right + 1 < keys.len() && keys[right + 1] <= t + window_us { right += 1; }
        let n = (right - left + 1) as f64;
        let (mut tx, mut ty, mut rot, mut scale) = (0.0, 0.0, 0.0, 0.0);
        for j in left..=right {
            tx += path[j].tx;
            ty += path[j].ty;
            rot += path[j].rot;
            scale += path[j].scale;
        }
        out.push(Similarity2D { tx: tx / n, ty: ty / n, rot: rot / n, scale: scale / n });
    }
    out
}

/// Interpolate a residual correction at `timestamp_us`. Returns identity when the map is empty or the gap is huge.
pub fn interpolate_similarity(map: &BTreeMap<i64, Similarity2D>, timestamp_us: i64) -> Similarity2D {
    if map.is_empty() { return Similarity2D::identity(); }
    if let Some(exact) = map.get(&timestamp_us) { return *exact; }
    let before = map.range(..=timestamp_us).next_back();
    let after = map.range(timestamp_us..).next();
    match (before, after) {
        (Some((t0, a)), Some((t1, b))) if *t1 != *t0 => {
            let t = (timestamp_us - *t0) as f64 / (*t1 - *t0) as f64;
            Similarity2D {
                tx: a.tx + (b.tx - a.tx) * t,
                ty: a.ty + (b.ty - a.ty) * t,
                rot: a.rot + (b.rot - a.rot) * t,
                scale: a.scale + (b.scale - a.scale) * t,
            }
        }
        (Some((_, s)), None) | (None, Some((_, s))) => *s,
        _ => map.get_closest(&timestamp_us, 200_000).copied().unwrap_or_default(),
    }
}

/// Output-space residual matrix, or `None` when the pass is off / identity (so callers skip the multiply)
pub fn residual_matrix(
    enabled: bool,
    strength: f64,
    map: &BTreeMap<i64, Similarity2D>,
    timestamp_ms: f64,
    output_width: usize,
    output_height: usize,
) -> Option<Matrix3<f64>> {
    if !enabled || map.is_empty() { return None; }
    let strength = strength.clamp(0.0, 1.0);
    if strength <= 0.0 { return None; }
    let s = interpolate_similarity(map, (timestamp_ms * 1000.0).round() as i64).scale_strength(strength);
    if s.is_near_identity() || !s.is_finite() { return None; }
    Some(s.as_matrix_about(output_width as f64 / 2.0, output_height as f64 / 2.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: i32) -> Vec<(f32, f32)> {
        let mut pts = Vec::new();
        for y in 0..n {
            for x in 0..n {
                pts.push((80.0 + x as f32 * 40.0, 60.0 + y as f32 * 30.0));
            }
        }
        pts
    }
    fn apply_all(s: Similarity2D, pts: &[(f32, f32)]) -> Vec<(f32, f32)> {
        pts.iter().map(|p| {
            let q = s.apply((p.0 as f64, p.1 as f64));
            (q.0 as f32, q.1 as f32)
        }).collect()
    }

    #[test]
    fn motion_mode_matches_existing_autosync_default() {
        assert!(!OpticalMotionMode::Auto.uses_optical_as_motion(true));
        assert!(OpticalMotionMode::Auto.uses_optical_as_motion(false));
        assert!(OpticalMotionMode::OpticalOnly.uses_optical_as_motion(true));
        assert!(OpticalMotionMode::OpticalOnly.uses_optical_as_motion(false));
        assert!(!OpticalMotionMode::GyroOnly.uses_optical_as_motion(true));
        assert!(!OpticalMotionMode::GyroOnly.uses_optical_as_motion(false));
        assert_eq!(OpticalMotionMode::from(0), OpticalMotionMode::Auto);
        assert_eq!(OpticalMotionMode::from(1), OpticalMotionMode::OpticalOnly);
        assert_eq!(OpticalMotionMode::from(2), OpticalMotionMode::GyroOnly);
    }

    #[test]
    fn similarity_recovers_translation_rotation_and_scale() {
        let from = grid(6);
        let truth = Similarity2D { tx: 12.5, ty: -7.25, rot: 0.08, scale: 1.03 };
        let to = apply_all(truth, &from);
        let est = fit_similarity(&from, &to).expect("fit");
        assert!((est.tx - truth.tx).abs() < 1e-4, "{est:?}");
        assert!((est.ty - truth.ty).abs() < 1e-4, "{est:?}");
        assert!((est.rot - truth.rot).abs() < 1e-5, "{est:?}");
        assert!((est.scale - truth.scale).abs() < 1e-5, "{est:?}");
    }

    #[test]
    fn similarity_rejects_sparse_and_non_finite_tracks() {
        assert!(fit_similarity(&[(1.0, 1.0)], &[(2.0, 2.0)]).is_none());
        let from = grid(5);
        let mut to = apply_all(Similarity2D { tx: 4.0, ty: 1.0, rot: 0.0, scale: 1.0 }, &from);
        to[0] = (f32::NAN, 0.0);
        // still enough finite pairs
        assert!(fit_similarity(&from, &to).is_some());
        let empty: Vec<(f32, f32)> = Vec::new();
        assert!(fit_similarity(&empty, &empty).is_none());
    }

    #[test]
    fn similarity_rejects_a_wrong_match() {
        let from = grid(6);
        let mut to = apply_all(Similarity2D { tx: 5.0, ty: 3.0, rot: 0.02, scale: 1.0 }, &from);
        to[3] = (900.0, 900.0);
        let est = fit_similarity(&from, &to).expect("inliers enough");
        assert!((est.tx - 5.0).abs() < 0.05 && (est.ty - 3.0).abs() < 0.05, "{est:?}");
    }

    #[test]
    fn compose_and_inverse_round_trip() {
        let a = Similarity2D { tx: 3.0, ty: -2.0, rot: 0.1, scale: 1.05 };
        let b = Similarity2D { tx: -1.0, ty: 4.0, rot: -0.04, scale: 0.98 };
        let p = (120.0, 80.0);
        let q = b.apply(a.apply(p));
        let back = a.compose(b).inverse().apply(q);
        assert!((back.0 - p.0).abs() < 1e-9 && (back.1 - p.1).abs() < 1e-9, "{back:?}");
    }

    #[test]
    fn strength_zero_is_identity() {
        let s = Similarity2D { tx: 10.0, ty: 4.0, rot: 0.2, scale: 1.1 }.scale_strength(0.0);
        assert!(s.is_near_identity(), "{s:?}");
        let half = Similarity2D { tx: 10.0, ty: 4.0, rot: 0.2, scale: 1.1 }.scale_strength(0.5);
        assert!((half.tx - 5.0).abs() < 1e-12 && (half.rot - 0.1).abs() < 1e-12);
        assert!((half.scale - 1.05).abs() < 1e-12);
    }

    #[test]
    fn removing_a_known_rotation_leaves_the_translation() {
        let from = grid(6);
        let fx = 800.0;
        let fy = 800.0;
        let cx = 320.0;
        let cy = 180.0;
        let rot = Rotation3::from_axis_angle(&Vector3::z_axis(), 0.03);
        let rotated = apply_camera_rotation(&from, &rot, fx, fy, cx, cy);
        let leftover = Similarity2D { tx: 6.0, ty: -2.5, rot: 0.0, scale: 1.0 };
        let to = apply_all(leftover, &rotated);
        let pair = TrackedPair {
            timestamp_us: 0,
            next_timestamp_us: 33333,
            from: from.clone(),
            to,
            remove_rotation: Some(rot),
            fx, fy, cx, cy,
            width: 640,
            height: 360,
        };
        let inc = residual_increments(&[pair]);
        let est = inc.get(&0).expect("increment");
        assert!((est.tx - 6.0).abs() < 0.4, "{est:?}");
        assert!((est.ty - -2.5).abs() < 0.4, "{est:?}");
        assert!(est.rot.abs() < 0.01, "{est:?}");
    }

    #[test]
    fn cuts_do_not_enter_the_path() {
        let from = grid(6);
        let huge = apply_all(Similarity2D { tx: 400.0, ty: 0.0, rot: 0.0, scale: 1.0 }, &from);
        let pair = TrackedPair {
            timestamp_us: 1000,
            next_timestamp_us: 2000,
            from, to: huge,
            remove_rotation: None,
            fx: 800.0, fy: 800.0, cx: 320.0, cy: 180.0,
            width: 640,
            height: 360,
        };
        let inc = residual_increments(&[pair]);
        assert!(inc.get(&1000).unwrap().is_near_identity());
    }

    #[test]
    fn residual_pass_keeps_slow_motion_and_cancels_jitter() {
        // Slow drift of +0.5 px/frame plus ±4 px of every-other-frame jitter
        let mut increments = BTreeMap::new();
        for i in 0..40 {
            let jitter = if i % 2 == 0 { 4.0 } else { -4.0 };
            increments.insert(i as i64 * 33_333, Similarity2D {
                tx: 0.5 + jitter,
                ty: 0.0,
                rot: 0.0,
                scale: 1.0,
            });
        }
        let corr = residual_corrections(&increments, 0.25);
        // Mean correction translation should sit near the opposite of the jitter, not the drift
        let mean_tx: f64 = corr.values().map(|s| s.tx).sum::<f64>() / corr.len() as f64;
        assert!(mean_tx.abs() < 0.6, "mean correction {mean_tx} should not fight the drift");
        let even = corr.get(&0).unwrap().tx;
        let odd = corr.get(&33_333).unwrap().tx;
        assert!(even.signum() != odd.signum() || even.abs() > 1.0, "jitter should be opposed: {even} / {odd}");
        assert!((even - odd).abs() > 3.0, "corrections {even} and {odd} should differ by about the 8 px peak-to-peak jitter");
    }

    #[test]
    fn interpolate_and_matrix_move_a_point() {
        let mut map = BTreeMap::new();
        map.insert(0, Similarity2D { tx: 0.0, ty: 0.0, rot: 0.0, scale: 1.0 });
        map.insert(1000, Similarity2D { tx: 10.0, ty: 0.0, rot: 0.0, scale: 1.0 });
        let mid = interpolate_similarity(&map, 500);
        assert!((mid.tx - 5.0).abs() < 1e-9);
        let m = residual_matrix(true, 1.0, &map, 1.0, 200, 100).unwrap();
        let p = m * nalgebra::Vector3::new(100.0, 50.0, 1.0);
        assert!((p.x - 110.0).abs() < 1e-9 && (p.y - 50.0).abs() < 1e-9, "{p:?}");
        assert!(residual_matrix(false, 1.0, &map, 1.0, 200, 100).is_none());
        assert!(residual_matrix(true, 0.0, &map, 1.0, 200, 100).is_none());
        assert!(residual_matrix(true, 1.0, &BTreeMap::new(), 1.0, 200, 100).is_none());
    }

    #[test]
    fn empty_path_is_identity() {
        assert!(residual_increments(&[]).is_empty());
        assert!(residual_corrections(&BTreeMap::new(), 0.5).is_empty());
        assert!(interpolate_similarity(&BTreeMap::new(), 0).is_near_identity());
    }
}
