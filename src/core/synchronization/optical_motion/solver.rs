// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Fits the correction spline δ(t) to the rotation the image measured and the motion data didn't.
//!
//! Every measurement comes from one band of rows of one pair of frames: the tracked points of that band moved by a
//! small rotation ρ more than the quaternions say, and with the camera at `R·exp(δ)` that is `ρ = M·δ(ta) - δ(tb)`,
//! `M` being the quaternions' own rotation between the two frames and `ta`, `tb` the band's times in each. The
//! measurements only ever see differences one frame apart, so the slow part of δ is left to the motion data: a ridge
//! pulls δ to zero wherever the image doesn't pin it down (which also makes it fade out where nothing was tracked),
//! and a small penalty on the curvature of the control points bridges the vertical blanking between two readouts

use nalgebra::{ Matrix3, Vector3 };
use crate::gyro_source::optical_correction::bspline_weights;

#[derive(Clone, Debug)]
pub struct BandMeasurement {
    /// The frame pair it was measured in
    pub pair: usize,
    /// Motion data times of the band in the first and the second frame, microseconds
    pub ta_us: f64,
    pub tb_us: f64,
    /// The rotation the band moved by beyond the quaternions, radians, in the quaternions' frame
    pub rho: Vector3<f64>,
    /// Inverse covariance of `rho`
    pub info: Matrix3<f64>,
    /// The quaternions' rotation from the first frame to the second, `R(tb)ᵀ·R(ta)`
    pub m: Matrix3<f64>,
}

#[derive(Clone, Debug)]
pub struct SolverParams {
    /// Control point spacing, microseconds
    pub spacing_us: f64,
    /// Ridge on δ, relative to the weight the measurements give each control point itself: where to hand over to
    /// the motion data. Relative to the local weight, so a stretch the image measured less surely (a band that moved
    /// by degrees) is held back no more than any other. 0.001 keeps 90% of 1 Hz and 97% from 2 Hz up
    pub ridge: f64,
    /// Penalty on the second difference of the control points, relative to the same
    pub curvature: f64,
    /// Passes that downweight measurements the fit disagrees with, and by how much (χ²) before they do
    pub robust_passes: usize,
    pub robust_chi2: f64,
}
impl Default for SolverParams {
    fn default() -> Self {
        Self { spacing_us: 2800.0, ridge: 0.001, curvature: 0.0001, robust_passes: 0, robust_chi2: 16.0 }
    }
}

/// The fitted control points and the time of the first one
pub struct Solution {
    pub start_us: f64,
    pub coeffs: Vec<Vector3<f64>>,
}
impl Solution {
    pub fn at(&self, t_us: f64, spacing_us: f64) -> Vector3<f64> {
        let (s, w) = bspline_weights((t_us - self.start_us) / spacing_us);
        let mut v = Vector3::zeros();
        for (i, w) in w.iter().enumerate() {
            let k = s + i as i64 - 1;
            if k >= 0 && (k as usize) < self.coeffs.len() { v += self.coeffs[k as usize] * *w; }
        }
        v
    }
}

pub fn solve(measurements: &[BandMeasurement], p: &SolverParams) -> Option<Solution> {
    if measurements.is_empty() || p.spacing_us <= 0.0 { return None; }
    let t_min = measurements.iter().map(|m| m.ta_us.min(m.tb_us)).fold(f64::MAX, f64::min);
    let t_max = measurements.iter().map(|m| m.ta_us.max(m.tb_us)).fold(f64::MIN, f64::max);
    let start_us = t_min - 2.0 * p.spacing_us;
    let knots = ((t_max - start_us) / p.spacing_us).ceil() as usize + 3;
    let n = knots * 3;

    // Half bandwidth, in knots: the farthest two control points one measurement touches
    let mut kb = 3usize;
    for m in measurements {
        let a = bspline_weights((m.ta_us - start_us) / p.spacing_us).0;
        let b = bspline_weights((m.tb_us - start_us) / p.spacing_us).0;
        kb = kb.max(((a - b).unsigned_abs() + 3) as usize);
    }
    let bw = kb * 3 + 2;

    let mut weights = vec![1.0f64; measurements.len()];
    let mut solution = None;
    for pass in 0..=p.robust_passes {
        let mut h = BandSym::new(n, bw);
        let mut g = vec![0.0f64; n];
        for (m, &wm) in measurements.iter().zip(weights.iter()) {
            if wm <= 0.0 { continue; }
            let blocks = jacobian_blocks(m, start_us, p.spacing_us, knots);
            let info = m.info * wm;
            for (ka, ja) in &blocks {
                let jai = ja.transpose() * info;
                let gv = jai * m.rho;
                for r in 0..3 { g[ka * 3 + r] += gv[r]; }
                for (kc, jc) in &blocks {
                    if kc > ka { continue; }
                    let blk = jai * jc;
                    for r in 0..3 {
                        for c in 0..3 {
                            let (i, j) = (ka * 3 + r, kc * 3 + c);
                            if j <= i { h.add(i, j, blk[(r, c)]); }
                        }
                    }
                }
            }
        }
        // Scale of the regularization at each control point: the most the image gives any control point within a frame
        // period of it, floored where that's (next to) nothing so δ fades to zero there. Not what the point gets itself:
        // one at the edge of what was measured (the blanking between two readouts, rows of sky with nothing to track)
        // only touches a few measurements with the tail of its curve, and held back by that little it takes whatever
        // value fits their noise - pixels of correction where the image saw nothing. A stretch measured less surely
        // all along (a band that moved by degrees) is still held back no more than any other
        let mut diag: Vec<f64> = (0..n).map(|i| h.get(i, i)).filter(|v| *v > 0.0).collect();
        if diag.is_empty() { return None; }
        diag.sort_by(|a, b| a.total_cmp(b));
        let floor = diag[diag.len() / 2] * 1e-3;
        let own: Vec<f64> = (0..knots).map(|k| (h.get(k * 3, k * 3) + h.get(k * 3 + 1, k * 3 + 1) + h.get(k * 3 + 2, k * 3 + 2)) / 3.0).collect();
        let mut periods: Vec<f64> = measurements.iter().map(|m| (m.tb_us - m.ta_us).abs()).collect();
        periods.sort_by(|a, b| a.total_cmp(b));
        let reach = (periods[periods.len() / 2] / p.spacing_us).ceil().max(1.0) as usize;
        let local: Vec<f64> = (0..knots).map(|k| own[k.saturating_sub(reach)..(k + reach + 1).min(knots)].iter().fold(floor, |a, b| a.max(*b))).collect();
        for k in 0..knots {
            for r in 0..3 { h.add(k * 3 + r, k * 3 + r, p.ridge * local[k]); }
        }
        for k in 1..knots.saturating_sub(1) {
            // (c[k-1] - 2 c[k] + c[k+1])², per axis
            let curv = p.curvature * local[k];
            let taps = [(k - 1, 1.0), (k, -2.0), (k + 1, 1.0)];
            for &(ka, wa) in &taps {
                for &(kc, wc) in &taps {
                    if kc > ka { continue; }
                    for r in 0..3 { h.add(ka * 3 + r, kc * 3 + r, curv * wa * wc); }
                }
            }
        }
        let x = h.solve(g)?;
        let coeffs: Vec<Vector3<f64>> = x.chunks(3).map(|c| Vector3::new(c[0], c[1], c[2])).collect();
        let sol = Solution { start_us, coeffs };

        if pass < p.robust_passes {
            // Measurements far off the fit (a band full of moving cars, water) lose weight, softly
            for (m, w) in measurements.iter().zip(weights.iter_mut()) {
                let r = m.rho - (m.m * sol.at(m.ta_us, p.spacing_us) - sol.at(m.tb_us, p.spacing_us));
                let chi2 = (r.transpose() * m.info * r)[0];
                *w = if chi2 > p.robust_chi2 { p.robust_chi2 / chi2 } else { 1.0 };
            }
        }
        solution = Some(sol);
    }
    solution
}

/// ∂ρ/∂c for the control points one measurement touches: `M·w` for the ones around `ta`, `-w` for the ones around `tb`
fn jacobian_blocks(m: &BandMeasurement, start_us: f64, spacing_us: f64, knots: usize) -> Vec<(usize, Matrix3<f64>)> {
    let mut out: Vec<(usize, Matrix3<f64>)> = Vec::with_capacity(8);
    let mut push = |k: i64, blk: Matrix3<f64>| {
        if k < 0 || k as usize >= knots { return; }
        if let Some(e) = out.iter_mut().find(|(kk, _)| *kk == k as usize) { e.1 += blk; } else { out.push((k as usize, blk)); }
    };
    let (sa, wa) = bspline_weights((m.ta_us - start_us) / spacing_us);
    for (i, w) in wa.iter().enumerate() { push(sa + i as i64 - 1, m.m * *w); }
    let (sb, wb) = bspline_weights((m.tb_us - start_us) / spacing_us);
    for (i, w) in wb.iter().enumerate() { push(sb + i as i64 - 1, -Matrix3::identity() * *w); }
    out
}

/// Symmetric positive definite band matrix, lower half, solved by Cholesky
struct BandSym {
    n: usize,
    bw: usize,
    data: Vec<f64>, // row i, column i - d at [i * (bw + 1) + d]
}
impl BandSym {
    fn new(n: usize, bw: usize) -> Self { Self { n, bw, data: vec![0.0; n * (bw + 1)] } }
    #[inline] fn idx(&self, i: usize, j: usize) -> usize { i * (self.bw + 1) + (i - j) }
    #[inline] fn get(&self, i: usize, j: usize) -> f64 { if i - j > self.bw { 0.0 } else { self.data[self.idx(i, j)] } }
    #[inline] fn add(&mut self, i: usize, j: usize, v: f64) {
        debug_assert!(j <= i && i - j <= self.bw);
        let k = self.idx(i, j);
        self.data[k] += v;
    }
    fn solve(mut self, mut b: Vec<f64>) -> Option<Vec<f64>> {
        let (n, bw) = (self.n, self.bw);
        for i in 0..n {
            let j0 = i.saturating_sub(bw);
            for j in j0..=i {
                let mut s = self.get(i, j);
                for k in j0.max(j.saturating_sub(bw))..j {
                    s -= self.get(i, k) * self.get(j, k);
                }
                let idx = self.idx(i, j);
                if i == j {
                    if s <= 0.0 || !s.is_finite() { return None; }
                    self.data[idx] = s.sqrt();
                } else {
                    self.data[idx] = s / self.get(j, j);
                }
            }
        }
        for i in 0..n {
            let mut s = b[i];
            for k in i.saturating_sub(bw)..i { s -= self.get(i, k) * b[k]; }
            b[i] = s / self.get(i, i);
        }
        for i in (0..n).rev() {
            let mut s = b[i];
            for k in (i + 1)..(i + bw + 1).min(n) { s -= self.get(k, i) * b[k]; }
            b[i] = s / self.get(i, i);
        }
        Some(b)
    }
}
