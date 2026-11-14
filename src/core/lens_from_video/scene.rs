// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Anton Belkin

//! A rough scene for the bundle adjustment to start from, made with a lens taken as given: two frames far enough
//! apart for the rays of their common points to open up give the pose of one against the other and the first points;
//! every other frame is placed against the points it sees, and adds the points that it's the first to see well.
//! It only has to be in the right valley: the bundle adjustment settles it before it lets the lens go.

use std::collections::HashMap;
use nalgebra::{ Matrix2x3, Matrix3, Matrix6, Rotation3, SMatrix, Vector2, Vector3, Vector6 };
use super::{ Tracks, lens::Lens, bundle::{ Scene, Sighting } };

/// A track takes part once its rays have opened up by this much (degrees), from the two frames it starts in ...
const MIN_ANGLE_START: f64 = 0.7;
/// ... or from a later frame; and a point is made again when a frame sees it at an angle this many times wider
const MIN_ANGLE: f64 = 1.0;
const WIDER: f64 = 1.3;
/// How far apart the two frames to start from are looked for, frames: until the median angle of their points is `GOOD_START`
const GAPS: [usize; 7] = [8, 12, 16, 24, 32, 48, 64];
const GOOD_START: f64 = 1.5;
/// Fewer common tracks than this are no pair to start from
const MIN_COMMON: usize = 60;
/// With the rays of the best pair of frames opening up by less than this (degrees, median), the camera didn't move
/// through space: there's no scene to make, and nothing that tells the focal length
pub const MIN_PARALLAX: f64 = 0.25;
/// ... and the same of the fitted scene (`Bundle::parallax`), over the whole life of each track: the rolling shutter of
/// a camera that only turns bends its rays apart by more than `MIN_PARALLAX`, until the fit has found its readout time
pub const MIN_PARALLAX_FITTED: f64 = 0.5;

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct SceneInfo {
    /// The two frames it started from were this far apart
    pub gap: usize,
    /// Median angle between the rays of their common points, degrees: how much the camera moved through space
    pub parallax: f64,
    /// Tracks that got a point, and those given to the fit
    pub tracks_with_point: usize,
    pub points: usize,
    pub frames: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoScene {
    /// Too little was tracked: a few frames, or nothing in the picture to hold on to
    TooFewTracks,
    /// The camera turned but didn't move through space
    NoTranslation,
    /// A scene was made, and the fit of it found no step to take or lost its numbers: it says nothing of the lens,
    /// least of all that the lens stayed where it was
    NoVerdict,
}
impl NoScene {
    /// For the UI to put into words (and languages)
    pub fn code(&self) -> &'static str {
        match self { Self::TooFewTracks => "too_few_tracks", Self::NoTranslation => "no_translation", Self::NoVerdict => "no_verdict" }
    }
}

/// Midpoint of two rays (bearings in each camera's space, cameras world → camera): the point, the angle between the
/// rays in degrees, and whether it's in front of both
fn triangulate(ra: &Rotation3<f64>, ta: &Vector3<f64>, ba: &Vector3<f64>, rb: &Rotation3<f64>, tb: &Vector3<f64>, bb: &Vector3<f64>) -> (Vector3<f64>, f64, bool) {
    let ca = -(ra.inverse() * ta);
    let cb = -(rb.inverse() * tb);
    let da = ra.inverse() * ba;
    let db = rb.inverse() * bb;
    let w = ca - cb;
    let c = da.dot(&db);
    let (d, e) = (da.dot(&w), db.dot(&w));
    let den = (1.0 - c * c).max(1e-12);
    let s = (c * e - d) / den;
    let u = (e - c * d) / den;
    ((ca + da * s + cb + db * u) * 0.5, c.clamp(-1.0, 1.0).acos().to_degrees(), s > 0.0 && u > 0.0)
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() { return 0.0; }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// The pose of the second frame against the first (X₂ = R X₁ + t, |t| = 1) from the bearings of their common points,
/// and which of them agree with it. `px`: one pixel as an angle.
///
/// The frames are a fraction of a second of video apart, so the rotation that best turns one set of bearings into the
/// other is close; with it the direction of the translation is a linear fit; and both are then refined together on the
/// distance of each bearing from its epipolar plane, robustly. Tried from a few starts, the best kept
fn two_view(a: &[Vector3<f64>], b: &[Vector3<f64>], px: f64) -> Option<(Rotation3<f64>, Vector3<f64>, Vec<bool>)> {
    let n = a.len();
    if n < 8 { return None; }
    let sigma = 2.0 * px;
    let epi = |r: &Rotation3<f64>, t: &Vector3<f64>, i: usize| -> f64 {
        let c = t.cross(&(r * a[i]));
        b[i].dot(&c) / c.norm().max(1e-9)
    };
    let cost = |r: &Rotation3<f64>, t: &Vector3<f64>| -> f64 { (0..n).map(|i| { let e = epi(r, t, i) / sigma; (e * e).ln_1p() }).sum() };

    // Rotation alone, the points that don't follow it (near ones, moving ones) weighing less each round
    let mut rot = Rotation3::identity();
    for _ in 0..6 {
        let res: Vec<f64> = (0..n).map(|i| (b[i] - rot * a[i]).norm()).collect();
        let s = median(res.clone()).max(sigma);
        let mut h = Matrix3::zeros();
        for i in 0..n { h += a[i] * b[i].transpose() / (1.0 + (res[i] / s) * (res[i] / s)); }
        let svd = h.svd(true, true);
        let (u, vt) = (svd.u?, svd.v_t?);
        let v = vt.transpose();
        let d = Matrix3::from_diagonal(&Vector3::new(1.0, 1.0, (v * u.transpose()).determinant().signum()));
        rot = Rotation3::from_matrix_unchecked(v * d * u.transpose());
    }

    let mut best: Option<(f64, Rotation3<f64>, Vector3<f64>)> = None;
    for r0 in [rot, Rotation3::identity()] {
        // The translation that fits this rotation: perpendicular to every (R a) × b
        let mut m = Matrix3::zeros();
        for i in 0..n { let c = (r0 * a[i]).cross(&b[i]); m += c * c.transpose(); }
        let eig = m.symmetric_eigen();
        let t_lin: Vector3<f64> = eig.eigenvectors.column(eig.eigenvalues.imin()).into_owned();
        for t0 in [t_lin, Vector3::x(), Vector3::y(), Vector3::z()] {
            let (mut r, mut t) = (r0, t0.normalize());
            let mut c = cost(&r, &t);
            let mut lambda = 1e-3;
            for _ in 0..30 {
                // The translation moves on the sphere
                let b1 = t.cross(&if t.x.abs() < 0.9 { Vector3::x() } else { Vector3::y() }).normalize();
                let b2 = t.cross(&b1);
                let step = |d: &SMatrix<f64, 5, 1>| (Rotation3::new(Vector3::new(d[0], d[1], d[2])) * r, (t + b1 * d[3] + b2 * d[4]).normalize());
                let mut jtj = SMatrix::<f64, 5, 5>::zeros();
                let mut jtr = SMatrix::<f64, 5, 1>::zeros();
                let eps = 1e-6;
                let moved: Vec<_> = (0..5).map(|k| { let mut d = SMatrix::<f64, 5, 1>::zeros(); d[k] = eps; step(&d) }).collect();
                for i in 0..n {
                    let e0 = epi(&r, &t, i);
                    let w = 1.0 / (1.0 + (e0 / sigma) * (e0 / sigma));
                    let j = SMatrix::<f64, 5, 1>::from_fn(|k, _| (epi(&moved[k].0, &moved[k].1, i) - e0) / eps);
                    jtj += j * j.transpose() * w;
                    jtr += j * (e0 * w);
                }
                let mut improved = false;
                while lambda < 1e6 {
                    let mut h = jtj;
                    for k in 0..5 { h[(k, k)] += lambda * jtj[(k, k)].max(1e-12); }
                    let Some(d) = h.try_inverse().map(|inv| -(inv * jtr)) else { lambda *= 5.0; continue };
                    let (r2, t2) = step(&d);
                    let c2 = cost(&r2, &t2);
                    if c2 < c {
                        improved = c - c2 > 1e-9 * c;
                        r = r2; t = t2; c = c2; lambda = (lambda / 4.0).max(1e-9);
                        break;
                    }
                    lambda *= 5.0;
                }
                if !improved { break; }
            }
            if best.as_ref().is_none_or(|x| c < x.0) { best = Some((c, r, t)); }
        }
    }
    let (_, r, mut t) = best?;

    // The points are in front of the cameras for one of the two signs
    let inliers: Vec<bool> = (0..n).map(|i| epi(&r, &t, i).abs() < sigma).collect();
    let (id, zero) = (Rotation3::identity(), Vector3::zeros());
    let front = |t: &Vector3<f64>| (0..n).filter(|i| inliers[*i] && triangulate(&id, &zero, &a[*i], &r, t, &b[*i]).2).count();
    if front(&-t) > front(&t) { t = -t; }
    Some((r, t, inliers))
}

/// The pose of a frame (world → camera) against the points it sees, from the pose of the frame next to it. Robust
/// (Cauchy), then once more on the points that agree. `None` when fewer than 10 do
fn place(points: &[Vector3<f64>], bearings: &[Vector3<f64>], r0: &Rotation3<f64>, t0: &Vector3<f64>, px: f64) -> Option<(Rotation3<f64>, Vector3<f64>)> {
    let sigma = 2.0 * px;
    let (mut r, mut t) = (*r0, *t0);
    // Each bearing's two directions across it
    let bases: Vec<(Vector3<f64>, Vector3<f64>)> = bearings.iter().map(|b| {
        let e1 = b.cross(&if b.x.abs() < 0.9 { Vector3::x() } else { Vector3::y() }).normalize();
        (e1, b.cross(&e1))
    }).collect();
    let residual = |r: &Rotation3<f64>, t: &Vector3<f64>, i: usize| -> (Vector2<f64>, Vector3<f64>, Vector3<f64>) {
        let xr = r * points[i];
        let xc = xr + t;
        let d = xc / xc.norm().max(1e-12);
        (Vector2::new(bases[i].0.dot(&d), bases[i].1.dot(&d)), xr, xc)
    };
    let inlier = |r: &Rotation3<f64>, t: &Vector3<f64>, i: usize| { let (e, _, xc) = residual(r, t, i); e.norm() < 6.0 * px && xc.dot(&bearings[i]) > 0.0 };
    let mut only: Option<Vec<bool>> = None;
    for round in 0..2 {
        let weight = |e: &Vector2<f64>, i: usize| match &only { Some(m) => if m[i] { 1.0 } else { 0.0 }, None => 1.0 / (1.0 + e.norm_squared() / (sigma * sigma)) };
        let cost = |r: &Rotation3<f64>, t: &Vector3<f64>| -> f64 {
            (0..points.len()).map(|i| { let e = residual(r, t, i).0; match &only { Some(m) => if m[i] { e.norm_squared() } else { 0.0 }, None => sigma * sigma * (e.norm_squared() / (sigma * sigma)).ln_1p() } }).sum()
        };
        let mut c = cost(&r, &t);
        let mut lambda = 1e-4;
        for _ in 0..(if round == 0 { 15 } else { 6 }) {
            let mut h = Matrix6::<f64>::zeros();
            let mut g = Vector6::<f64>::zeros();
            for i in 0..points.len() {
                let (e, xr, xc) = residual(&r, &t, i);
                let w = weight(&e, i);
                if w == 0.0 { continue; }
                let norm = xc.norm().max(1e-12);
                let d = xc / norm;
                let dd = (Matrix3::identity() - d * d.transpose()) / norm;
                let b = Matrix2x3::from_rows(&[bases[i].0.transpose(), bases[i].1.transpose()]) * dd;
                let sk = Matrix3::new(0.0, xr.z, -xr.y,  -xr.z, 0.0, xr.x,  xr.y, -xr.x, 0.0);
                let mut j = SMatrix::<f64, 2, 6>::zeros();
                j.fixed_view_mut::<2, 3>(0, 0).copy_from(&(b * sk));
                j.fixed_view_mut::<2, 3>(0, 3).copy_from(&b);
                h += j.transpose() * j * w;
                g += j.transpose() * e * w;
            }
            let mut improved = false;
            while lambda < 1e8 {
                let mut hd = h;
                for k in 0..6 { hd[(k, k)] += lambda * h[(k, k)].max(1e-12); }
                let Some(d) = hd.try_inverse().map(|inv| -(inv * g)) else { lambda *= 5.0; continue };
                let r2 = Rotation3::new(Vector3::new(d[0], d[1], d[2])) * r;
                let t2 = t + Vector3::new(d[3], d[4], d[5]);
                let c2 = cost(&r2, &t2);
                if c2 < c {
                    improved = c - c2 > 1e-10 * c;
                    r = r2; t = t2; c = c2; lambda = (lambda / 4.0).max(1e-9);
                    break;
                }
                lambda *= 5.0;
            }
            if !improved { break; }
        }
        let agree: Vec<bool> = (0..points.len()).map(|i| inlier(&r, &t, i)).collect();
        if agree.iter().filter(|x| **x).count() < 10 { return None; }
        only = Some(agree);
    }
    Some((r, t))
}

/// The scene of these tracks as this lens sees it, with at most `max_points` tracks picked for the fit (the longest,
/// spread over time and over the frame). Every `hold_out`-th track is held out, from here on: no pose is made from it
/// here either, so what the held-out tracks say of a fit owes nothing to them
pub fn rough_scene(tracks: &Tracks, lens: &Lens, max_points: usize, hold_out: usize) -> Result<(Scene, SceneInfo), NoScene> {
    let (nf, nt) = (tracks.frames, tracks.tracks);
    if nf < GAPS[0] + 2 || nt < MIN_COMMON { return Err(NoScene::TooFewTracks); }
    let px = 1.0 / lens.fx;
    let held: Vec<bool> = (0..nt).map(|k| hold_out > 0 && k % hold_out == 0).collect();
    let bearings: Vec<Vector3<f64>> = tracks.uv.iter().map(|uv| lens.unproject(*uv)).collect();
    let mut of_frame: Vec<Vec<usize>> = vec![Vec::new(); nf];
    for (row, f) in tracks.frame.iter().enumerate() { of_frame[*f as usize].push(row); }
    let common = |fa: usize, fb: usize| -> (Vec<usize>, Vec<usize>) {
        let in_a: HashMap<u32, usize> = of_frame[fa].iter().map(|r| (tracks.track[*r], *r)).collect();
        of_frame[fb].iter().filter_map(|rb| in_a.get(&tracks.track[*rb]).map(|ra| (*ra, *rb))).unzip()
    };

    // ---- The two frames to start from. The first frames are often blurred or swung: tried from the frames with most
    // tracks too
    let per: Vec<usize> = of_frame.iter().map(|x| x.len()).collect();
    let most = per.iter().copied().max().unwrap_or(0);
    let mut order: Vec<usize> = (0..nf.saturating_sub(8).max(1)).collect();
    order.sort_by_key(|f| std::cmp::Reverse(per[*f]));
    let mut from: Vec<usize> = if per[0] as f64 >= 0.7 * most as f64 { vec![0] } else { Vec::new() };
    from.extend(order.into_iter().step_by((nf / 12).max(1)).take(4));

    struct Start { a: usize, gap: usize, r: Rotation3<f64>, t: Vector3<f64>, rows: (Vec<usize>, Vec<usize>), points: Vec<(Vector3<f64>, f64, bool)>, parallax: f64 }
    let mut best: Option<Start> = None;
    let mut enough_common = false;
    'starts: for a0 in from {
        for gap in GAPS {
            if a0 + gap >= nf { break; }
            let rows = common(a0, a0 + gap);
            if rows.0.len() < MIN_COMMON { break; }
            enough_common = true;
            // The pose of the pair is of the tracks the fit will have
            let rows: (Vec<usize>, Vec<usize>) = rows.0.into_iter().zip(rows.1).filter(|(ra, _)| !held[tracks.track[*ra] as usize]).unzip();
            let a: Vec<_> = rows.0.iter().map(|r| bearings[*r]).collect();
            let b: Vec<_> = rows.1.iter().map(|r| bearings[*r]).collect();
            let Some((r, t, inliers)) = two_view(&a, &b, px) else { continue };
            let (id, zero) = (Rotation3::identity(), Vector3::zeros());
            let points: Vec<_> = (0..a.len()).map(|i| { let (x, angle, ok) = triangulate(&id, &zero, &a[i], &r, &t, &b[i]); (x, angle, ok && inliers[i]) }).collect();
            let angles: Vec<f64> = points.iter().filter(|p| p.2).map(|p| p.1).collect();
            let parallax = if angles.len() > 40 { median(angles) } else { 0.0 };
            if best.as_ref().is_none_or(|x| parallax > x.parallax) { best = Some(Start { a: a0, gap, r, t, rows, points, parallax }); }
            if parallax > GOOD_START { break 'starts; }
        }
    }
    let Some(start) = best else { return Err(if enough_common { NoScene::NoTranslation } else { NoScene::TooFewTracks }) };
    if start.parallax < MIN_PARALLAX { return Err(NoScene::NoTranslation); }

    // ---- Every frame placed, the points made as it goes
    let (a0, b0) = (start.a, start.a + start.gap);
    let mut rot = vec![Rotation3::<f64>::identity(); nf];
    let mut tr = vec![Vector3::<f64>::zeros(); nf];
    let mut point = vec![Vector3::<f64>::zeros(); nt];
    let mut angle = vec![0.0f64; nt];
    let mut first: Vec<Option<(usize, usize)>> = vec![None; nt]; // The frame a track was first seen from, and its row there
    rot[b0] = start.r; tr[b0] = start.t;
    for (i, row) in start.rows.0.iter().enumerate() {
        let k = tracks.track[*row] as usize;
        first[k] = Some((a0, *row));
        let (x, an, ok) = start.points[i];
        if ok && an > MIN_ANGLE_START { point[k] = x; angle[k] = an; }
    }
    let mut register = |f: usize, prev: usize| {
        let rows = &of_frame[f];
        let (r_prev, t_prev) = (rot[prev], tr[prev]);
        rot[f] = r_prev; tr[f] = t_prev;
        let known: Vec<usize> = rows.iter().copied().filter(|r| { let k = tracks.track[*r] as usize; angle[k] > 0.0 && !held[k] }).collect();
        if known.len() >= 12 {
            let xs: Vec<_> = known.iter().map(|r| point[tracks.track[*r] as usize]).collect();
            let bs: Vec<_> = known.iter().map(|r| bearings[*r]).collect();
            if let Some((r, t)) = place(&xs, &bs, &r_prev, &t_prev, px) { rot[f] = r; tr[f] = t; }
        }
        for &row in rows {
            let k = tracks.track[row] as usize;
            match first[k] {
                None => first[k] = Some((f, row)),
                Some((f0, row0)) if f0 != f => {
                    // From the first frame that saw it and this one
                    let (x, an, ok) = triangulate(&rot[f0], &tr[f0], &bearings[row0], &rot[f], &tr[f], &bearings[row]);
                    if ok && an > MIN_ANGLE && an > WIDER * angle[k] { point[k] = x; angle[k] = an; }
                },
                _ => { }
            }
        }
    };
    for f in a0 + 1..b0 { register(f, f - 1); }
    for f in b0 + 1..nf { register(f, f - 1); }
    for f in (0..a0).rev() { register(f, f + 1); }

    // ---- Tracks for the fit: the longest first, spread over time and over the frame
    let good: Vec<usize> = (0..nt).filter(|k| angle[*k] > 0.0).collect();
    if good.len() < MIN_COMMON { return Err(NoScene::TooFewTracks); }
    let mut count = vec![0usize; nt];
    let mut mean = vec![[0.0f64; 2]; nt];
    for (row, k) in tracks.track.iter().enumerate() {
        let k = *k as usize;
        count[k] += 1; mean[k][0] += tracks.uv[row][0]; mean[k][1] += tracks.uv[row][1];
    }
    let (w, h) = (tracks.size.0 as f64, tracks.size.1 as f64);
    let cell = |k: usize| -> usize {
        let (u, v) = (mean[k][0] / count[k] as f64, mean[k][1] / count[k] as f64);
        (first[k].map(|x| x.0).unwrap_or(0) / 12) * 100 + ((u / w * 6.0) as usize).min(5) * 10 + ((v / h * 4.0) as usize).min(3)
    };
    let mut order = good.clone();
    order.sort_by_key(|k| std::cmp::Reverse(count[*k])); // stable
    let mut seen: HashMap<usize, usize> = HashMap::new();
    let mut ranked: Vec<(usize, usize)> = order.iter().map(|k| { let n = seen.entry(cell(*k)).or_insert(0); *n += 1; (*n - 1, *k) }).collect();
    ranked.sort_by_key(|x| x.0); // stable
    let mut chosen: Vec<usize> = ranked.into_iter().take(max_points).map(|x| x.1).collect();
    chosen.sort_unstable();

    let mut index = vec![u32::MAX; nt];
    for (i, k) in chosen.iter().enumerate() { index[*k] = i as u32; }
    let mut sightings: Vec<Sighting> = (0..tracks.track.len()).filter(|row| index[tracks.track[*row] as usize] != u32::MAX)
        .map(|row| Sighting { cam: tracks.frame[row], point: index[tracks.track[row] as usize], uv: tracks.uv[row] }).collect();
    sightings.sort_by_key(|s| (s.point, s.cam));

    let info = SceneInfo { gap: start.gap, parallax: start.parallax, tracks_with_point: good.len(), points: chosen.len(), frames: nf };
    Ok((Scene {
        size: tracks.size,
        times: if tracks.times.len() == nf { tracks.times.clone() } else { (0..nf).map(|i| i as f64).collect() },
        rotations: rot.iter().map(super::bundle::rotation_vector).collect(),
        translations: tr,
        points: chosen.iter().map(|k| point[*k]).collect(),
        held_out: chosen.iter().map(|k| held[*k]).collect(),
        sightings,
    }, info))
}
