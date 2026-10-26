// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! The camera path as an optimization: the virtual camera's orientation at every node (frame) is chosen to minimize
//! the energy of its angular motion - jerk, acceleration and velocity, each weighted by the smoothing time constant -
//! with a weak pull toward the real camera, subject to the output frame staying inside the source picture.
//!
//! Unconstrained, it is a zero-phase linear smoother with the response `1 / (1 + Σ cₖ·(ωτ)^2k)`: flat in the pass band,
//! falling off with the highest order present (36 dB per octave with jerk), and exact on constant-velocity motion, so a
//! pan never lags. A forward-backward exponential filter (`1 / (1 + (ωτ)²)`) lets shake through at the frequencies it
//! only halves. The constraint replaces the heuristics that otherwise keep the crop in check (less smoothing at high
//! velocities, lowering it again wherever the zoom gets over a limit): the path is as smooth as the crop allows, frame
//! by frame, and where it has to follow the camera it does so as smoothly as it can.
//!
//! Rotations don't form a vector space, so the problem is solved by Gauss-Newton on the manifold: around the current
//! path `S`, the update is `S' = S·exp(x)` with `x` a rotation vector per node, every residual is linearized in `x`,
//! the convex problem in `x` is solved, and the path is moved. The residuals themselves are always evaluated exactly,
//! so the iteration converges to a path where the linearized problem has nothing left to change. The inner problem is
//! a banded quadratic program, solved by a primal-dual interior point method in a few dozen `O(n)` factorizations.
//!
//! What keeps the outer iteration well-behaved on real footage:
//! - a trust radius on the step, grown while steps land where their linearization said and shrunk when one doesn't;
//! - a proximal term on any node whose step reverses, which settles a frame whose tightest crop points take turns;
//! - constraints of frames that even the real camera's own view misses (a glitch in the lens or motion data, a limit the
//!   lens can't meet) relaxed to where that view is, and all constraints elastic besides, so the problem is never
//!   infeasible - an interior point method sent at an infeasible one runs off to infinity;
//! - only the constraints within reach of the step in the inner problem, and any other its solution violates added and
//!   solved again, which gives the answer of all of them at the cost of the few near the crop's edge.

use nalgebra::{ Matrix3, Vector3 };
use super::banded::BandedMatrix;
use crate::gyro_source::Quat64;


/// One crop constraint linearized at a frame: its slack (source pixels, positive inside) is `s0 + g·x` for the update
/// `x` of the path at that frame's time
#[derive(Clone, Copy, Debug)]
pub struct Lin {
    pub s0: f64,
    pub g: [f64; 3],
    /// Which constraint of the frame this is, the same one at every linearization
    pub id: u32,
}

/// The frames the crop holds the path to, each at the time the render reads it at
pub trait Constraints: Sync {
    fn len(&self) -> usize;
    /// Time (ms, in the motion's own timeline) of frame `j`
    fn time(&self, j: usize) -> f64;
    /// The orientation the render corrects at frame `j`: a virtual camera `S` turns the output rays by `S⁻¹·target`
    fn target(&self, j: usize) -> Quat64;
    /// The virtual camera that doesn't smooth at all, at frame `j`: the motion handed to the smoothing
    fn unsmoothed(&self, j: usize) -> Quat64;
    /// The constraints of frame `j` with the output turned by `c` (`S⁻¹·target`), linearized in the update `x` of the
    /// path there (`S' = S·exp(x)`, ie. `c' = exp(-x)·c`). Only those with a slack below `near` are needed
    fn linearize(&self, j: usize, c: &Quat64, near: f64, out: &mut Vec<Lin>);
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// Node spacing, s
    pub h: f64,
    /// Time of the first node, ms
    pub t0_ms: f64,
    /// `c₁, c₂, c₃`: the weights of velocity, acceleration and jerk in the smoothing energy, in units of `τ`, the time
    /// constant (see the module docs)
    pub order_weights: [f64; 3],
    /// Source pixels: the crop constraint pushes back from this far inside the frame's edge
    pub margin: f64,
    pub max_outer: usize,
}
impl Default for Settings {
    fn default() -> Self {
        Self { h: 1.0 / 30.0, t0_ms: 0.0, order_weights: [0.0, 0.0, 1.0], margin: 2.0, max_outer: 16 }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub outer_iterations: usize,
    pub newton_iterations: usize,
    /// Source pixels, at the returned path: the deepest a constraint is violated (`0` when none is), beyond the
    /// relaxation of the frames the unsmoothed view doesn't fit
    pub max_violation: f64,
    /// Frames whose constraints had to be relaxed, and by how many pixels at most, and the frames not held at all
    pub relaxed_frames: usize,
    pub max_relaxation: f64,
    pub dropped_frames: usize,
    pub constrained_frames: usize,
    pub linearize_ms: f64,
    pub newton_ms: f64,
}


/// Radians: the longest step any node may take in one outer iteration when there are constraints, at first; it doubles
/// while the steps land where their linearization said, up to `MAX_OUTER_STEP_LIMIT`, and halves when one doesn't
const MAX_OUTER_STEP: f64 = 0.15;
const MAX_OUTER_STEP_LIMIT: f64 = 0.6;

#[inline] fn log(q: &Quat64) -> Vector3<f64> { q.scaled_axis() }
#[inline] fn exp(v: &Vector3<f64>) -> Quat64 { Quat64::from_scaled_axis(*v) }

/// `(-1)^(k-t)·C(k, t)`: the `k`-th forward difference
const DIFF: [&[f64]; 4] = [&[1.0], &[-1.0, 1.0], &[1.0, -2.0, 1.0], &[-1.0, 3.0, -3.0, 1.0]];

/// The inverse of SO(3)'s right Jacobian at `φ`: `log(exp(φ)·exp(ε)) = φ + J_r⁻¹(φ)·ε` to first order in `ε`. The left one
/// is `J_l⁻¹(φ) = J_r⁻¹(-φ)`: `log(exp(ε)·exp(φ)) = φ + J_l⁻¹(φ)·ε`
fn jr_inv(phi: &Vector3<f64>) -> Matrix3<f64> {
    let th = phi.norm();
    let k = phi.cross_matrix();
    // 1/θ² - (1 + cos θ)/(2θ·sin θ), which tends to 1/12
    let c = if th < 1e-4 { 1.0 / 12.0 + th * th / 720.0 } else { 1.0 / (th * th) - (1.0 + th.cos()) / (2.0 * th * th.sin()) };
    Matrix3::identity() + k * 0.5 + k * k * c
}

/// Adds `Bᵢᵀ·W·Bⱼ` for every pair of blocks of a residual `e0 + Σ Bᵤ·x_{node(u)}` to `q`, and `-Bᵤᵀ·W·e0` to `b`
fn add_residual(q: &mut BandedMatrix, b: &mut [f64], blocks: &[(usize, Matrix3<f64>)], e0: &Vector3<f64>, w: &Vector3<f64>) {
    for (u, (nu, bu)) in blocks.iter().enumerate() {
        let wbu = Matrix3::from_diagonal(w) * bu; // W·Bᵤ
        let g = bu.transpose() * (Matrix3::from_diagonal(w) * e0);
        for r in 0..3 { b[3 * nu + r] -= g[r]; }
        for (nv, bv) in blocks.iter().take(u + 1) {
            let m = bv.transpose() * wbu; // Bᵥᵀ·W·Bᵤ: row is v's axis, column u's
            for r in 0..3 {
                for c in 0..3 {
                    let (i, j) = (3 * nv + r, 3 * nu + c);
                    // Each unordered pair of entries once: the diagonal block is symmetric, so only its lower half
                    if nu == nv && c > r { continue; }
                    q.add(i, j, m[(r, c)]);
                }
            }
        }
    }
}

/// The quadratic part of the inner problem, `½xᵀQx - bᵀx`, around the path `s`: fidelity to `raw` (weight 1 per node and
/// axis) and the smoothing energy. `tau` is the time constant per node and axis
fn assemble(s: &[Quat64], raw: &[Quat64], tau: &[[f64; 3]], st: &Settings) -> (BandedMatrix, Vec<f64>) {
    let n = s.len();
    // Per interval: the rotation between the nodes, v = a + P·x_{i+1} - M·x_i. Not with SO(3)'s Jacobians here, though
    // `log(exp(-xᵢ)·A·exp(xᵢ₊₁))` has them: the smoothing residuals are large at the solution by design (that's the
    // energy being traded against the crop), so Gauss-Newton's dropped second-order terms aren't small next to them,
    // and the `½[a]×` coupling then promises a payoff in directions held only by the fidelity (a roll, for a camera
    // shaking in pitch and yaw) that the real energy doesn't have. With `P = M = I` the energy is that of the path's
    // differences in its own moving frame - the same to first order in the rotation per node - and the model is exact
    let a: Vec<Vector3<f64>> = s.windows(2).map(|w| log(&(w[0].inverse() * w[1]))).collect();
    let p = vec![Matrix3::identity(); a.len()];
    let m = p.clone();
    let mut q = BandedMatrix::zeros(3 * n, 11);
    let mut b = vec![0.0; 3 * n];
    let ones = Vector3::new(1.0, 1.0, 1.0);
    for i in 0..n {
        // log(S'⁻¹·R) = log(exp(-x)·exp(r)) = r - J_l⁻¹(r)·x
        let r = log(&(s[i].inverse() * raw[i]));
        add_residual(&mut q, &mut b, &[(i, -jr_inv(&-r))], &r, &ones);
    }
    let mut blocks: Vec<(usize, Matrix3<f64>)> = Vec::with_capacity(4);
    for k in 1..=3usize {
        let ck = st.order_weights[k - 1];
        if ck <= 0.0 || n <= k { continue; }
        let c = DIFF[k - 1];
        for j in 0..n - k {
            let mid = j + k / 2;
            let w = Vector3::from_iterator(tau[mid].iter().map(|t| ck * (t / st.h).powi(2 * k as i32)));
            if !(w.max() > 0.0) { continue; }
            // The k-th difference of the path: the (k-1)-th difference of its interval rotations,
            // Σₜ cₜ·v_{j+t} = Σₜ cₜ·a_{j+t} + Σᵤ (c_{u-1}·P_{j+u-1} - c_u·M_{j+u})·x_{j+u}
            let e0: Vector3<f64> = c.iter().enumerate().map(|(t, ct)| a[j + t] * *ct).sum();
            blocks.clear();
            for u in 0..=k {
                let mut bu = Matrix3::zeros();
                if u >= 1 { bu += p[j + u - 1] * c[u - 1]; }
                if u < k { bu -= m[j + u] * c[u]; }
                blocks.push((j + u, bu));
            }
            add_residual(&mut q, &mut b, &blocks, &e0, &w);
        }
    }
    (q, b)
}

/// The path's rotation from each node to the next
pub fn intervals(s: &[Quat64]) -> Vec<Vector3<f64>> {
    s.windows(2).map(|w| log(&(w[0].inverse() * w[1]))).collect()
}

#[inline]
fn hermite(u: f64) -> (f64, f64, f64) {
    let (u2, u3) = (u * u, u * u * u);
    (-2.0 * u3 + 3.0 * u2, u3 - 2.0 * u2 + u, u3 - u2)
}

/// Catmull-Rom tangent at node `k` from the intervals `a`, one-sided at the ends
#[inline]
fn tangent(a: &[Vector3<f64>], k: usize) -> Vector3<f64> {
    if k == 0 { a[0] } else if k >= a.len() { a[a.len() - 1] } else { (a[k - 1] + a[k]) * 0.5 }
}

/// Node `i` and the position `u` in `[0, 1)` from it, for the node coordinate `u` (`(t - t0)/h`), clamped to the path
#[inline]
fn segment(n: usize, u: f64) -> (usize, f64) {
    let u = u.clamp(0.0, (n - 1) as f64);
    let i = (u.floor() as usize).min(n - 2);
    (i, u - i as f64)
}

/// The path at the node coordinate `u`: a cubic Hermite curve in the tangent space of each node, with Catmull-Rom
/// tangents (`a` from `intervals`). This is the path everything between the nodes - the frames, the gyro samples - gets
pub fn sample(s: &[Quat64], a: &[Vector3<f64>], u: f64) -> Quat64 {
    let (i, u) = segment(s.len(), u);
    let (h01, h10, h11) = hermite(u);
    s[i] * exp(&(a[i] * h01 + tangent(a, i) * h10 + tangent(a, i + 1) * h11))
}

/// How an update `x` of the nodes moves the path at the node coordinate `u`, to first order: the same curve drawn on
/// the nodes' updates, as weights on the four nodes from the returned first one
fn sample_weights(n: usize, u: f64) -> (usize, [f64; 4]) {
    let (i, u) = segment(n, u);
    let (h01, h10, h11) = hermite(u);
    let base = i.saturating_sub(1).min(n - 4);
    let mut w = [0.0; 4];
    let mut add = |k: usize, v: f64| w[k - base] += v;
    // x(u) = x_i + h01·(x_{i+1} - x_i) + h10·T_i + h11·T_{i+1}, with the tangents as `tangent` makes them
    add(i, 1.0 - h01);
    add(i + 1, h01);
    for (k, h) in [(i, h10), (i + 1, h11)] {
        if k == 0 { add(1, h); add(0, -h); }
        else if k >= n - 1 { add(n - 1, h); add(n - 2, -h); }
        else { add(k + 1, 0.5 * h); add(k - 1, -0.5 * h); }
    }
    (base, w)
}

/// A linearized constraint in the inner problem: `g·x(t) ≥ bound` on the update of the path at its frame's time, which
/// is `Σ w·x` over the four nodes from `base`. Elastic: it may be missed by `e` pixels at a price of `rho` per pixel
#[derive(Clone, Copy)]
struct Active { base: usize, w: [f64; 4], g: [f64; 3], bound: f64, rho: f64 }

impl Active {
    #[inline]
    fn eval(&self, x: &[f64]) -> f64 {
        let mut v = 0.0;
        for (k, w) in self.w.iter().enumerate() {
            if *w == 0.0 { continue; }
            let o = 3 * (self.base + k);
            v += w * (self.g[0] * x[o] + self.g[1] * x[o + 1] + self.g[2] * x[o + 2]);
        }
        v
    }
    /// Adds `coef·∂(g·x(t))/∂x` to `v`
    #[inline]
    fn add_to(&self, v: &mut [f64], coef: f64) {
        for (k, w) in self.w.iter().enumerate() {
            if *w == 0.0 { continue; }
            let o = 3 * (self.base + k);
            for r in 0..3 { v[o + r] += coef * w * self.g[r]; }
        }
    }
    /// Adds `d·aᵀa` to `h`, `a` the constraint's row
    fn add_outer(&self, h: &mut BandedMatrix, d: f64) {
        for k in 0..4 {
            if self.w[k] == 0.0 { continue; }
            for l in 0..=k {
                if self.w[l] == 0.0 { continue; }
                let coef = d * self.w[k] * self.w[l];
                let (ok, ol) = (3 * (self.base + k), 3 * (self.base + l));
                for r in 0..3 {
                    for c in 0..3 {
                        // The block on the diagonal is symmetric: only its lower half, each pair of entries once
                        if k == l && c > r { continue; }
                        h.add(ok + r, ol + c, coef * self.g[r] * self.g[c]);
                    }
                }
            }
        }
    }
}

/// `min ½xᵀQx - bᵀx + Σ ρ·e` subject to `a·x - s + e = bound`, `s, e ≥ 0` for every constraint, by a primal-dual
/// interior point method (Mehrotra's predictor-corrector). Each constraint shapes every step through its barrier, so
/// the iterations don't grow with how many become active the way an active-set method's do, and the reduced system
/// `(Q + AᵀDA)·Δx` keeps `Q`'s band, as a constraint only couples the four nodes around its frame.
///
/// The constraints are elastic (an exact penalty): a frame the crop can't hold at all - a lens or motion glitch, a limit
/// the lens can't meet - costs `ρ` per pixel it's missed by instead of making the problem infeasible, which sends an
/// interior point method's iterates to infinity. With `ρ` above the multipliers of the constraints that can be met, the
/// answer is that of the hard constraints. Starts at `x = 0`, feasible or not. Returns the minimizer and the iterations
fn interior_point(q: &BandedMatrix, b: &[f64], cons: &[Active]) -> (Vec<f64>, usize) {
    let n = b.len();
    let m = cons.len();
    let mut x = vec![0.0; n];
    if m == 0 {
        let Some(chol) = q.cholesky() else { return (x, 0); };
        x.copy_from_slice(b);
        chol.solve_in_place(&mut x);
        // Iterative refinement, see `solve` below
        let mut res = vec![0.0; n];
        q.mul_vec(&x, &mut res);
        res.iter_mut().zip(b).for_each(|(r, b)| *r = b - *r);
        chol.solve_in_place(&mut res);
        x.iter_mut().zip(&res).for_each(|(x, r)| *x += r);
        return (x, 1);
    }
    // Slacks of the constraint (s) and of its elasticity (e), the multiplier z of the constraint and w = ρ - z of the
    // elasticity. Away from the boundary to start with, at the scale of a pixel
    let mut s: Vec<f64> = cons.iter().map(|c| (c.eval(&x) - c.bound).max(1.0)).collect();
    let mut z: Vec<f64> = {
        // Multipliers of the size that balances the start: the gradient there spread over the constraints
        let qx_b = b.iter().fold(0.0f64, |a, v| a.max(v.abs()));
        let g_max = cons.iter().map(|c| c.g.iter().fold(0.0f64, |a, v| a.max(v.abs()))).fold(1e-12, f64::max);
        let z0 = (qx_b / g_max).max(1.0) * 1e-3;
        cons.iter().map(|c| z0.min(0.5 * c.rho)).collect()
    };
    // The elasticity starts as centred as the constraint itself, `e·w = s·z`: with `w` near the price, which is many
    // orders above `z`, an `e` of a pixel would make the elasticity's complementarity all of `μ`, and the centring step
    // then asks the slacks for moves no step length allows
    let mut e: Vec<f64> = (0..m).map(|k| (s[k] * z[k] / (cons[k].rho - z[k])).max(1e-300)).collect();
    let b_scale = b.iter().fold(1.0f64, |a, v| a.max(v.abs()));
    let mut qx = vec![0.0; n];
    let mut rhs = vec![0.0; n];
    let mut r3 = vec![0.0; m];
    let mut r4 = vec![0.0; m];
    // Solves the reduced system for the right-hand sides of the two complementarity equations, and returns
    // (Δx, Δs, Δe, Δz). With G = s/z + e/w and D = 1/G:
    //   H·Δx = b - Qx + Aᵀz + Aᵀ·D·(-rp + r3/z - r4/w),  rp = A·x - s + e - bound
    //   Δz = D·(-rp + r3/z - r4/w - A·Δx),  Δs = (r3 - s·Δz)/z,  Δe = (r4 + e·Δz)/w
    let solve = |h: &BandedMatrix, chol: &super::banded::BandedCholesky, x: &[f64], s: &[f64], e: &[f64], z: &[f64], qx: &[f64], r3: &[f64], r4: &[f64], rhs: &mut Vec<f64>| -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
        rhs.iter_mut().zip(b.iter().zip(qx)).for_each(|(r, (b, qx))| *r = b - qx);
        let mut t = vec![0.0; cons.len()];
        for (k, c) in cons.iter().enumerate() {
            let w = c.rho - z[k];
            let d = 1.0 / (s[k] / z[k] + e[k] / w);
            let rp = c.eval(x) - s[k] + e[k] - c.bound;
            t[k] = -rp + r3[k] / z[k] - r4[k] / w;
            c.add_to(rhs, z[k] + d * t[k]);
        }
        let mut dx = rhs.clone();
        chol.solve_in_place(&mut dx);
        // One step of iterative refinement: the slow modes of the path are held only by the fidelity, a million times
        // weaker than the smoothing, and the factorization alone loses them in rounding
        let mut res = vec![0.0; dx.len()];
        h.mul_vec(&dx, &mut res);
        res.iter_mut().zip(rhs.iter()).for_each(|(r, b)| *r = b - *r);
        chol.solve_in_place(&mut res);
        dx.iter_mut().zip(&res).for_each(|(d, r)| *d += r);
        let mut ds = vec![0.0; cons.len()];
        let mut de = vec![0.0; cons.len()];
        let mut dz = vec![0.0; cons.len()];
        for (k, c) in cons.iter().enumerate() {
            let w = c.rho - z[k];
            let d = 1.0 / (s[k] / z[k] + e[k] / w);
            dz[k] = d * (t[k] - c.eval(&dx));
            ds[k] = (r3[k] - s[k] * dz[k]) / z[k];
            de[k] = (r4[k] + e[k] * dz[k]) / w;
        }
        (dx, ds, de, dz)
    };
    // The longest step in (0, 1] keeping `v + α·dv` positive
    let max_step = |v: &[f64], dv: &[f64]| -> f64 {
        let mut a = 1.0f64;
        for (v, d) in v.iter().zip(dv) { if *d < 0.0 { a = a.min(-v / d); } }
        a
    };
    // ... and keeping both z and w = ρ - z positive
    let max_dual_step = |z: &[f64], dz: &[f64]| -> f64 {
        let mut a = 1.0f64;
        for ((z, d), c) in z.iter().zip(dz).zip(cons) {
            if *d < 0.0 { a = a.min(-z / d); }
            if *d > 0.0 { a = a.min((c.rho - z) / d); }
        }
        a
    };
    for it in 0..MAX_IPM {
        q.mul_vec(&x, &mut qx);
        let mu = (0..m).map(|k| s[k] * z[k] + e[k] * (cons[k].rho - z[k])).sum::<f64>() / (2 * m) as f64;
        let rp_max = cons.iter().enumerate().map(|(k, c)| (c.eval(&x) - s[k] + e[k] - c.bound).abs()).fold(0.0, f64::max);
        let mut rd = qx.iter().zip(b).map(|(qx, b)| qx - b).collect::<Vec<f64>>();
        for (k, c) in cons.iter().enumerate() { c.add_to(&mut rd, -z[k]); }
        let rd_max = rd.iter().fold(0.0f64, |a, v| a.max(v.abs()));
        let z_max = z.iter().fold(0.0f64, |a, v| a.max(*v));
        // Converged: complementarity at a ten-thousandth of a pixel on the binding constraints, primal feasible, and the
        // dual residual below what moves the path a micro-radian even along its slowest modes (curvature 1, the fidelity)
        // (or what rounding leaves of `Qx - b` at a long time constant, where its terms are 10¹¹)
        let qx_max = qx.iter().fold(b_scale, |a, v| a.max(v.abs()));
        if rp_max < 1e-3 && rd_max < 1e-6 + 1e-13 * qx_max && mu < 1e-4 * z_max.max(1e-12) {
            return (x, it);
        }

        let mut h = q.clone();
        for (k, c) in cons.iter().enumerate() {
            c.add_outer(&mut h, 1.0 / (s[k] / z[k] + e[k] / (c.rho - z[k])));
        }
        let Some(chol) = h.cholesky() else { return (x, it); };

        // Predictor: the affine scaling direction
        for k in 0..m { r3[k] = -s[k] * z[k]; r4[k] = -e[k] * (cons[k].rho - z[k]); }
        let (_, ds_a, de_a, dz_a) = solve(&h, &chol, &x, &s, &e, &z, &qx, &r3, &r4, &mut rhs);
        let ap = max_step(&s, &ds_a).min(max_step(&e, &de_a));
        let ad = max_dual_step(&z, &dz_a);
        let mu_aff = (0..m).map(|k| {
            let w = cons[k].rho - z[k];
            (s[k] + ap * ds_a[k]) * (z[k] + ad * dz_a[k]) + (e[k] + ap * de_a[k]) * (w - ad * dz_a[k])
        }).sum::<f64>() / (2 * m) as f64;
        let sigma = (mu_aff / mu).powi(3).clamp(0.0, 1.0);

        // Corrector: centred, with the second order terms of the predictor (Δw = -Δz)
        for k in 0..m {
            let w = cons[k].rho - z[k];
            r3[k] = -s[k] * z[k] - ds_a[k] * dz_a[k] + sigma * mu;
            r4[k] = -e[k] * w + de_a[k] * dz_a[k] + sigma * mu;
        }
        let (dx, ds, de, dz) = solve(&h, &chol, &x, &s, &e, &z, &qx, &r3, &r4, &mut rhs);
        let ap = (0.995 * max_step(&s, &ds).min(max_step(&e, &de))).min(1.0);
        let ad = (0.995 * max_dual_step(&z, &dz)).min(1.0);
        x.iter_mut().zip(&dx).for_each(|(x, d)| *x += ap * d);
        s.iter_mut().zip(&ds).for_each(|(s, d)| *s = (*s + ap * d).max(1e-14));
        e.iter_mut().zip(&de).for_each(|(e, d)| *e = (*e + ap * d).max(1e-14));
        for (k, c) in cons.iter().enumerate() { z[k] = (z[k] + ad * dz[k]).clamp(1e-30, c.rho * (1.0 - 1e-12)); }
        log::trace!("  interior point {it}: mu {mu:.3e}, rp {rp_max:.3e}, rd {rd_max:.3e} (b {b_scale:.1e}), sigma {sigma:.3}, steps {ap:.3}/{ad:.3}");
    }
    (x, MAX_IPM)
}

const MAX_IPM: usize = 60;

/// `f` for every index below `n`, spread over threads of its own. Not rayon: the smoothing runs on a rayon pool's
/// thread holding the gyro lock, and a thread waiting in a rayon join runs other jobs of its pool meanwhile - another
/// recompute, which then waits for the lock this very thread holds
pub(super) fn parallel_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map(|x| x.get()).unwrap_or(1).min(n / 32).max(1);
    if threads <= 1 { return (0..n).map(&f).collect(); }
    let chunk = n.div_ceil(threads);
    let f = &f;
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads).map(|t| scope.spawn(move || ((t * chunk)..((t + 1) * chunk).min(n)).map(f).collect::<Vec<T>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}

/// The smoothed path at the nodes `raw` (equally spaced, `st.h` apart from `st.t0_ms`), with the time constant `tau`
/// per node and axis, holding every frame of `constraints`
pub fn solve(raw: &[Quat64], tau: &[[f64; 3]], st: &Settings, constraints: Option<&dyn Constraints>) -> (Vec<Quat64>, Stats) {
    let n = raw.len();
    let mut stats = Stats::default();
    if n < 4 { return (raw.to_vec(), stats); }
    let mut s = raw.to_vec();
    // Per frame: where it is on the node grid, the nodes its path depends on, and its constraints as last linearized
    let frames = constraints.map(|c| c.len()).unwrap_or(0);
    let weights: Vec<(usize, [f64; 4])> = (0..frames).map(|j| sample_weights(n, (constraints.unwrap().time(j) - st.t0_ms) / (st.h * 1000.0))).collect();
    let coords: Vec<f64> = (0..frames).map(|j| (constraints.unwrap().time(j) - st.t0_ms) / (st.h * 1000.0)).collect();
    let mut lins: Vec<Vec<Lin>> = vec![Vec::new(); frames];
    // Per node: how far it moved in the last outer iteration, and by what
    let mut moved = vec![f64::INFINITY; n];
    let mut last_x = vec![Vector3::zeros(); n];
    // Pixels: how far a frame's constraints are moved out because even the unsmoothed view misses them there, and the
    // frames that miss them by so much they aren't held at all
    let mut relax = vec![0.0f64; frames];
    let mut dropped = vec![false; frames];
    // A proximal weight per node, raised where the node's step keeps reversing (see below)
    let mut prox = vec![0.0f64; n];
    // The trust radius, how far the last step went, and its jitter (see the convergence test below)
    let mut radius = MAX_OUTER_STEP;
    let mut last_step = f64::INFINITY;
    let mut last_jitter = f64::INFINITY;
    let frame_moved = |j: usize, moved: &[f64]| -> f64 {
        let (base, w) = weights[j];
        (0..4).filter(|k| w[*k] != 0.0).map(|k| moved[base + k]).fold(0.0, f64::max)
    };
    for outer in 0..st.max_outer.max(1) {
        let (mut q, b) = assemble(&s, raw, tau, st);
        for (i, p) in prox.iter().enumerate() {
            if *p > 0.0 { for d in 0..3 { q.add(3 * i + d, 3 * i + d, *p); } }
        }

        let mut cons = Vec::new();
        let mut worst = 0.0f64;
        if let Some(c) = constraints {
            let t = std::time::Instant::now();
            let a = intervals(&s);
            if outer == 0 {
                // The view of no smoothing at all. A frame that misses the crop even so - a limit the lens can't meet,
                // the horizon lock turning it past the edge - can't be held by any correction worth having, and as a hard
                // constraint it would leave no path at all: its constraints are moved out to where that view is. One that
                // misses it by more than `DROP_BEYOND` is a glitch in the motion or lens data, and isn't held at all:
                // moved out that far, it would still pull the path toward wherever the glitch put the frame
                let missed = parallel_map(frames, |j| {
                    let mut out = Vec::new();
                    c.linearize(j, &(c.unsmoothed(j).inverse() * c.target(j)), st.margin + 1.0, &mut out);
                    out.iter().map(|l| (l.s0, (l.g[0] * l.g[0] + l.g[1] * l.g[1] + l.g[2] * l.g[2]).sqrt())).fold((f64::INFINITY, 0.0), |a, b| if b.0 < a.0 { b } else { a })
                });
                for (j, (s0, g)) in missed.into_iter().enumerate() {
                    if s0 < st.margin + 1.0 {
                        relax[j] = st.margin + 1.0 - s0;
                        if g > 0.0 && relax[j] / g > DROP_BEYOND { dropped[j] = true; }
                    }
                }
                stats.relaxed_frames = relax.iter().zip(&dropped).filter(|(r, d)| **r > 0.0 && !**d).count();
                stats.dropped_frames = dropped.iter().filter(|d| **d).count();
                stats.max_relaxation = relax.iter().zip(&dropped).filter(|(_, d)| !**d).map(|(r, _)| *r).fold(0.0, f64::max);
            }
            let stale: Vec<usize> = (0..frames).filter(|j| !dropped[*j] && frame_moved(*j, &moved) > REUSE_BELOW).collect();
            // Every frame keeps its constraints however far inside it is: one left out lets the path run off toward
            // the unconstrained one in a single step, out of the frame, and the next iteration pull it back
            let fresh = parallel_map(stale.len(), |k| {
                let j = stale[k];
                let mut out = Vec::new();
                c.linearize(j, &(sample(&s, &a, coords[j]).inverse() * c.target(j)), f64::INFINITY, &mut out);
                out
            });
            // A frame whose nodes have hardly moved keeps its linearization, carried along to where the path moved
            for j in 0..frames {
                if frame_moved(j, &moved) <= REUSE_BELOW {
                    let (base, w) = weights[j];
                    let x: Vector3<f64> = (0..4).map(|k| last_x[base + k] * w[k]).sum();
                    for l in lins[j].iter_mut() { l.s0 += l.g[0] * x[0] + l.g[1] * x[1] + l.g[2] * x[2]; }
                }
            }
            for (j, ls) in stale.into_iter().zip(fresh) { lins[j] = ls; }
            stats.linearize_ms += t.elapsed().as_secs_f64() * 1000.0;

            stats.constrained_frames = lins.iter().filter(|l| !l.is_empty()).count();
            for (j, ls) in lins.iter().enumerate() {
                if dropped[j] { continue; }
                let (base, w) = weights[j];
                // The price of missing a constraint by a pixel: what holding the frame's nodes `ELASTIC_PRICE` radians
                // off their preferred place costs in smoothing, far above the multiplier of any constraint that can be met
                let qd = (0..4).filter(|k| w[*k] != 0.0).flat_map(|k| (0..3).map(move |d| 3 * (base + k) + d)).map(|i| q.get(i, i)).fold(0.0, f64::max);
                for l in ls {
                    if !l.s0.is_finite() || !l.g.iter().all(|v| v.is_finite()) { continue; }
                    worst = worst.max(-l.s0 - relax[j]);
                    // slack(x) = s0 + g·x(t) ≥ margin
                    let g_norm = (l.g[0] * l.g[0] + l.g[1] * l.g[1] + l.g[2] * l.g[2]).sqrt();
                    if !(g_norm > 1e-9) { continue; }
                    cons.push(Active { base, w, g: l.g, bound: st.margin - relax[j] - l.s0, rho: ELASTIC_PRICE * qd / g_norm });
                }
            }
        }
        stats.max_violation = worst;
        // Converged, constraints or not, when the last step had no jitter left in it and left nothing outside the crop.
        // The smoothing residuals are linearized with the identity, which leaves an error that follows the frame to frame
        // motion until the iteration takes it out, and that error is what a step's third difference measures. What a
        // step may still carry after that is a smooth drift, most of it at the ends of the path, where the smoothing
        // holds it least - by fractions of a degree, and inside the crop. Whether a step left a frame outside, only the
        // linearization where it landed tells: a long smooth step has no jitter either
        if outer > 0 && last_jitter < CONVERGED_JITTER && worst <= 0.5 { break; }
        stats.outer_iterations = outer + 1;
        if outer > 0 {
            // The last step was taken on a linearization: where it left nothing outside, the next may go further
            if worst <= 0.5 { radius = (radius * 2.0).min(MAX_OUTER_STEP_LIMIT); } else if worst > 2.0 { radius = (radius * 0.5).max(0.02); }
        }

        // Most constraints are far from the crop's edge and can't bind: the inner problem starts with the near ones
        // only, and whatever its solution then violates of the others is added and it's solved again. The result is
        // that of all of them, at the cost of the few that matter
        let t = std::time::Instant::now();
        // Within reach is about twice as far as the last step went, never further than the trust radius
        let reach = radius.min(2.0 * last_step).max(NEAR_STEP);
        let g_norm = |c: &Active| (c.g[0] * c.g[0] + c.g[1] * c.g[1] + c.g[2] * c.g[2]).sqrt();
        let mut in_set: Vec<bool> = cons.iter().map(|c| -c.bound < g_norm(c) * reach).collect();
        let mut x;
        let mut iters = 0;
        let mut round = 0;
        loop {
            let working: Vec<Active> = cons.iter().zip(&in_set).filter(|(_, i)| **i).map(|(c, _)| *c).collect();
            let (xs, it) = interior_point(&q, &b, &working);
            x = xs;
            iters += it;
            // The ones it violates, and the ones it nearly does, which would only come in a round later; after a few
            // rounds, all of them
            let mut added = 0;
            for (k, c) in cons.iter().enumerate() {
                if !in_set[k] && (round >= MAX_ROUNDS || c.eval(&x) < c.bound + NEAR_STEP * 0.5 * g_norm(c)) { in_set[k] = true; added += 1; }
            }
            if added == 0 { break; }
            round += 1;
        }
        stats.newton_ms += t.elapsed().as_secs_f64() * 1000.0;
        stats.newton_iterations += iters;
        let max_step = (0..n).map(|i| Vector3::new(x[3 * i], x[3 * i + 1], x[3 * i + 2]).norm()).fold(0.0, f64::max);
        if log::log_enabled!(log::Level::Trace) {
            let (im, _) = (0..n).map(|i| (i, Vector3::new(x[3 * i], x[3 * i + 1], x[3 * i + 2]).norm())).fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
            let dev = log(&(s[im].inverse() * raw[im]));
            log::trace!("  largest step at node {im}/{n}: {:?}, deviation there {:?}", [x[3 * im], x[3 * im + 1], x[3 * im + 2]], dev);
        }
        // The constraints are linear only near where they were linearized: a longer step is shortened (as a whole, so
        // the path stays smooth) and the rest is left to the next linearization
        let scale = if constraints.is_some() && max_step > radius { radius / max_step } else { 1.0 };
        last_step = max_step * scale;
        for i in 0..n {
            let v = Vector3::new(x[3 * i], x[3 * i + 1], x[3 * i + 2]) * scale;
            // A node whose step turns back on the last one is flipping between two linearizations - at a corner of the
            // crop, two outline points taking turns as the tightest. Levenberg-Marquardt: a proximal term `ρ·|x|²` on it
            // in the next inner problem shortens its step until it settles, and vanishes at the fixed point, x = 0
            if v.norm() > 1e-4 && v.dot(&last_x[i]) < -0.5 * v.norm() * last_x[i].norm() {
                let qd = (0..3).map(|d| q.get(3 * i + d, 3 * i + d)).fold(0.0, f64::max);
                prox[i] = (prox[i] * 4.0).max(qd);
            } else {
                prox[i] *= 0.25;
                if prox[i] < 1e-3 { prox[i] = 0.0; }
            }
            s[i] = s[i] * exp(&v);
            moved[i] = v.norm();
            last_x[i] = v;
        }
        // The step's own jitter: its third difference, which a smooth update hardly has
        last_jitter = last_x.windows(4).map(|w| (w[3] - w[2] * 3.0 + w[1] * 3.0 - w[0]).norm()).fold(0.0, f64::max);
        log::trace!("outer {outer}: {} constraints, {iters} iterations, step {max_step:.2e} (x{scale:.2}), jitter {last_jitter:.2e}, violation {worst:.2} px", cons.len());
    }
    if let Some(c) = constraints {
        // The violation at the path being returned
        let a = intervals(&s);
        stats.max_violation = parallel_map(frames, |j| {
            if dropped[j] { return 0.0; }
            let mut out = Vec::new();
            c.linearize(j, &(sample(&s, &a, coords[j]).inverse() * c.target(j)), 0.0, &mut out);
            out.iter().map(|l| -l.s0 - relax[j]).fold(0.0, f64::max)
        }).into_iter().fold(0.0, f64::max);
    }
    (s, stats)
}

/// Radians: the outer iteration stops when a step's third difference stays below this, a twentieth of a pixel of jitter
/// at a focal length of 3000 px
const CONVERGED_JITTER: f64 = 2e-5;

/// Radians: constraints closer to the crop's edge than a rotation of this always go into the inner problem from the start
const NEAR_STEP: f64 = 0.02;

/// Rounds of adding the constraints the inner problem's solution violates, before all the rest go in
const MAX_ROUNDS: usize = 3;

/// Radians: a frame whose unsmoothed view misses the crop by more than a rotation of this isn't held (see `solve`)
const DROP_BEYOND: f64 = 0.1;

/// Radians: the elasticity of a constraint costs as much per pixel as holding its node this far off in smoothing
const ELASTIC_PRICE: f64 = 10.0;

/// Radians: a frame whose nodes moved less than this in the last outer iteration keeps its linearization
const REUSE_BELOW: f64 = 3e-4;

#[cfg(test)]
mod tests {
    use super::*;

    fn yaw(a: f64) -> Quat64 { Quat64::from_scaled_axis(Vector3::new(0.0, a, 0.0)) }

    /// Every node is a frame, with the constraints `f` makes for the output turned by `c`
    struct NodeFrames<'a, F: Fn(usize, &Quat64, &mut Vec<Lin>) + Sync> { raw: &'a [Quat64], h: f64, f: F }
    impl<F: Fn(usize, &Quat64, &mut Vec<Lin>) + Sync> Constraints for NodeFrames<'_, F> {
        fn len(&self) -> usize { self.raw.len() }
        fn time(&self, j: usize) -> f64 { j as f64 * self.h * 1000.0 }
        fn target(&self, j: usize) -> Quat64 { self.raw[j] }
        fn unsmoothed(&self, j: usize) -> Quat64 { self.raw[j] }
        fn linearize(&self, j: usize, c: &Quat64, _: f64, out: &mut Vec<Lin>) { (self.f)(j, c, out) }
    }

    /// Keeps the yaw deviation within ±`lim`, in "pixels" of 1000 per radian
    fn box1(lim: f64, c: &Quat64, out: &mut Vec<Lin>) {
        let dev = c.scaled_axis()[1];
        // c' = exp(-x)·c: the deviation drops by x
        out.push(Lin { s0: (lim - dev) * 1000.0, g: [0.0, 1000.0, 0.0], id: 0 });
        out.push(Lin { s0: (lim + dev) * 1000.0, g: [0.0, -1000.0, 0.0], id: 1 });
    }

    #[test]
    fn keeps_a_constant_velocity_pan_and_removes_shake() {
        let h = 1.0 / 60.0;
        let n = 600;
        let pan: Vec<f64> = (0..n).map(|i| i as f64 * h * 0.5).collect();
        let raw: Vec<Quat64> = pan.iter().enumerate().map(|(i, p)| yaw(p + 0.01 * (i as f64 * h * 2.0 * std::f64::consts::PI * 8.0).sin())).collect();
        let tau = vec![[0.3; 3]; n];
        let (s, _) = solve(&raw, &tau, &Settings { h, ..Default::default() }, None);
        // Away from the ends the 8 Hz shake is gone and the pan is untouched
        for i in 100..n - 100 {
            let err = (yaw(pan[i]).inverse() * s[i]).angle();
            assert!(err < 2e-4, "node {i}: {err} rad off the pan");
        }
    }

    #[test]
    fn iterates_to_the_end_without_constraints() {
        // A fast tumbling camera: the identity-linearized smoothing leaves a frame to frame error after one step, which
        // the iteration has to take out
        let h = 1.0 / 30.0;
        let n = 400;
        let raw: Vec<Quat64> = (0..n).map(|i| {
            let t = i as f64 * h;
            Quat64::from_scaled_axis(Vector3::new(0.6 * (1.3 * t).sin(), 2.0 * t, 0.4 * (0.7 * t).cos())) * yaw(0.02 * (i as f64 * 2.1).sin())
        }).collect();
        let tau = vec![[0.5; 3]; n];
        // RMS of the third difference of the path, away from the ends
        let jitter = |s: &[Quat64]| { let a = intervals(s); let d: Vec<f64> = a.windows(3).skip(30).take(a.len() - 62).map(|w| (w[2] - w[1] * 2.0 + w[0]).norm_squared()).collect(); (d.iter().sum::<f64>() / d.len() as f64).sqrt() };
        let (one, _) = solve(&raw, &tau, &Settings { h, max_outer: 1, ..Default::default() }, None);
        let (s, stats) = solve(&raw, &tau, &Settings { h, ..Default::default() }, None);
        assert!(stats.outer_iterations > 1, "{stats:?}");
        assert!(jitter(&s) < 0.3 * jitter(&one), "jitter {} after converging, {} after one step, {stats:?}", jitter(&s), jitter(&one));
    }

    #[test]
    fn stays_within_the_constraint() {
        let h = 1.0 / 30.0;
        let n = 300;
        // A step of 0.3 rad: an unconstrained smoother would lag it by far more than 0.05 rad
        let raw: Vec<Quat64> = (0..n).map(|i| yaw(if i < 150 { 0.0 } else { 0.3 })).collect();
        let tau = vec![[2.0; 3]; n];
        let cons = NodeFrames { raw: &raw, h, f: |_, c: &Quat64, out: &mut Vec<Lin>| box1(0.05, c, out) };
        let (s, stats) = solve(&raw, &tau, &Settings { h, margin: 0.0, ..Default::default() }, Some(&cons));
        let worst = (0..n).map(|i| (s[i].inverse() * raw[i]).angle()).fold(0.0, f64::max);
        assert!(worst < 0.0505, "deviation {worst} rad, {stats:?}");
    }

    #[test]
    fn holds_frames_between_the_nodes() {
        // Frames at a third of the node spacing, half a frame off the grid as with a sync offset, of a camera that turns
        // fast and wiggles between the nodes: the frames themselves have to be held, not just the nodes
        let (h, n) = (1.0 / 30.0, 200);
        let at = |t_ms: f64| { let u = t_ms / (h * 1000.0); yaw(0.3 * ((u - 98.0) / 3.0).clamp(0.0, 1.0) + 0.03 * (u * std::f64::consts::TAU / 1.5).sin()) };
        let raw: Vec<Quat64> = (0..n).map(|i| at(i as f64 * h * 1000.0)).collect();
        let tau = vec![[2.0; 3]; n];
        struct Frames<'a> { times: Vec<f64>, at: &'a (dyn Fn(f64) -> Quat64 + Sync) }
        impl Constraints for Frames<'_> {
            fn len(&self) -> usize { self.times.len() }
            fn time(&self, j: usize) -> f64 { self.times[j] }
            fn target(&self, j: usize) -> Quat64 { (self.at)(self.times[j]) }
            fn unsmoothed(&self, j: usize) -> Quat64 { (self.at)(self.times[j]) }
            fn linearize(&self, _: usize, c: &Quat64, _: f64, out: &mut Vec<Lin>) { box1(0.05, c, out) }
        }
        let frames = Frames { times: (0..3 * (n - 1)).map(|k| (k as f64 + 0.5) * h * 1000.0 / 3.0).collect(), at: &at };
        let worst_at_frames = |s: &[Quat64]| { let a = intervals(s); frames.times.iter().map(|t| (sample(s, &a, t / (h * 1000.0)).inverse() * at(*t)).angle()).fold(0.0, f64::max) };
        let (s, stats) = solve(&raw, &tau, &Settings { h, margin: 0.0, ..Default::default() }, Some(&frames));
        assert!(worst_at_frames(&s) < 0.0505, "deviation {} rad at a frame, {stats:?}", worst_at_frames(&s));
        // Held at the nodes alone, the path leaves the band between them: this needs the frames themselves
        let nodes = Frames { times: (0..n).map(|i| i as f64 * h * 1000.0).collect(), at: &at };
        let (s, _) = solve(&raw, &tau, &Settings { h, margin: 0.0, ..Default::default() }, Some(&nodes));
        assert!(worst_at_frames(&s) > 0.0505, "{} rad at the frames with the nodes held alone", worst_at_frames(&s));
    }

    #[test]
    fn a_frame_that_cant_fit_leaves_the_rest_alone() {
        let h = 1.0 / 30.0;
        let n = 300;
        let raw: Vec<Quat64> = (0..n).map(|i| yaw(0.2 * (i as f64 * h * 2.0).sin() + 0.01 * (i as f64 * 1.7).sin())).collect();
        let tau = vec![[1.0; 3]; n];
        // Both sides violated whatever the correction at frame 150: by a little it's relaxed, by a lot (a glitch) it's
        // left out
        for (miss, relaxed, dropped) in [(0.01, 1, 0), (0.3, 0, 1)] {
            let cons = NodeFrames { raw: &raw, h, f: |j, c: &Quat64, out: &mut Vec<Lin>| box1(if j == 150 { -miss } else { 0.05 }, c, out) };
            let (s, stats) = solve(&raw, &tau, &Settings { h, margin: 0.0, ..Default::default() }, Some(&cons));
            assert!(s.iter().all(|q| q.coords.iter().all(|v| v.is_finite())), "{stats:?}");
            assert_eq!((stats.relaxed_frames, stats.dropped_frames), (relaxed, dropped), "{stats:?}");
            // Everywhere but around the impossible frame the constraint holds
            let worst = (0..n).filter(|i| (*i as i64 - 150).abs() > 2).map(|i| (s[i].inverse() * raw[i]).angle()).fold(0.0, f64::max);
            assert!(worst < 0.0505, "deviation {worst} rad, {stats:?}");
            assert!(stats.outer_iterations < 16, "{stats:?}");
        }
    }

    /// Like the crop: a box in pitch and yaw, each edge made of a few nearly parallel constraints, slightly nonlinear,
    /// differentiated numerically in the update the way `crop::CropModel` does it
    fn crop2(lim: [f64; 2], c: &Quat64, out: &mut Vec<Lin>) {
        let slacks = |c: &Quat64| -> Vec<f64> {
            let v = c.scaled_axis();
            let mut out = Vec::new();
            for (a, lim) in [(0usize, lim[0]), (1usize, lim[1])] {
                for sign in [1.0, -1.0] {
                    for k in 0..3 {
                        // A point of the edge: its position depends a little on the other axes too
                        let tilt = (k as f64 - 1.0) * 0.1;
                        let pos = v[a] + tilt * v[1 - a] + 0.2 * v[a] * v[a].abs();
                        out.push((lim - sign * pos) * 1000.0);
                    }
                }
            }
            out
        };
        const EPS: f64 = 1e-6;
        let s0 = slacks(c);
        let d: Vec<Vec<f64>> = (0..3).map(|j| { let mut e = Vector3::zeros(); e[j] = -EPS; slacks(&(Quat64::from_scaled_axis(e) * c)) }).collect();
        for (id, s) in s0.iter().enumerate() {
            out.push(Lin { s0: *s, g: [0, 1, 2].map(|j| (d[j][id] - s) / EPS), id: id as u32 });
        }
    }

    #[test]
    fn converges_on_a_busy_crop() {
        let h = 1.0 / 30.0;
        let n = 500;
        // Wandering motion in pitch and yaw, far more than the box holds, with shake on top
        let mut rng = 12345u64;
        let mut rand = || { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; (rng % 10000) as f64 / 10000.0 - 0.5 };
        let (mut q, mut vp, mut vy) = (Quat64::identity(), 0.0f64, 0.0f64);
        let raw: Vec<Quat64> = (0..n).map(|_| {
            // Integrated like a gyro's samples, in the camera's own frame
            vp = 0.97 * vp + 0.006 * rand(); vy = 0.97 * vy + 0.006 * rand();
            q *= Quat64::from_scaled_axis(Vector3::new(vp, vy, 0.0));
            q * Quat64::from_scaled_axis(Vector3::new(0.01 * rand(), 0.01 * rand(), 0.0))
        }).collect();
        let tau = vec![[1.0; 3]; n];
        let cons = NodeFrames { raw: &raw, h, f: |_, c: &Quat64, out: &mut Vec<Lin>| crop2([0.08, 0.12], c, out) };
        let (s, stats) = solve(&raw, &tau, &Settings { h, ..Default::default() }, Some(&cons));
        let worst = (0..n).map(|i| { let v = (s[i].inverse() * raw[i]).scaled_axis(); (v[0].abs() / 0.08).max(v[1].abs() / 0.12) }).fold(0.0, f64::max);
        assert!(worst < 1.01, "{worst} of the box, {stats:?}");
        assert!(stats.outer_iterations < 16, "{stats:?}");
    }

    #[test]
    fn rides_a_curved_edge() {
        // A camera circling in pitch and yaw, far wider than the round "crop" holds: the path rides the edge of the disc
        // all the way around. The motion is smooth, so is the first step, which lands far outside - its lack of jitter
        // mustn't end the iteration there
        let h = 1.0 / 30.0;
        let n = 400;
        let raw: Vec<Quat64> = (0..n).map(|i| {
            let a = i as f64 * h * std::f64::consts::TAU / 4.0;
            Quat64::from_scaled_axis(Vector3::new(0.15 * a.cos(), 0.15 * a.sin(), 0.0))
        }).collect();
        let tau = vec![[1.0; 3]; n];
        let disc = |_: usize, c: &Quat64, out: &mut Vec<Lin>| {
            let slack = |c: &Quat64| { let v = c.scaled_axis(); (0.05 - (v[0] * v[0] + v[1] * v[1]).sqrt()) * 1000.0 };
            const EPS: f64 = 1e-6;
            let s0 = slack(c);
            out.push(Lin { s0, g: [0, 1, 2].map(|a| { let mut e = Vector3::zeros(); e[a] = EPS; (slack(&(exp(&-e) * c)) - s0) / EPS }), id: 0 });
        };
        let cons = NodeFrames { raw: &raw, h, f: disc };
        let (s, stats) = solve(&raw, &tau, &Settings { h, margin: 0.0, ..Default::default() }, Some(&cons));
        let worst = (0..n).map(|i| { let v = (s[i].inverse() * raw[i]).scaled_axis(); (v[0] * v[0] + v[1] * v[1]).sqrt() }).fold(0.0, f64::max);
        assert!(worst < 0.0505, "deviation {worst} rad, {stats:?}");
    }

    #[test]
    fn sample_weights_follow_the_curve() {
        // Moving one node moves the sampled path by its weight, to first order
        let n = 8;
        let s: Vec<Quat64> = (0..n).map(|i| yaw(0.1 * i as f64 + 0.02 * (i as f64).powi(2))).collect();
        for u in [0.0, 0.3, 2.5, 5.9, 6.7, 7.0] {
            let (base, w) = sample_weights(n, u);
            for k in 0..4 {
                let mut moved = s.clone();
                let eps = 1e-6;
                moved[base + k] = moved[base + k] * yaw(eps);
                let d = (sample(&s, &intervals(&s), u).inverse() * sample(&moved, &intervals(&moved), u)).scaled_axis()[1] / eps;
                assert!((d - w[k]).abs() < 1e-4, "u {u}, node {}: {d} vs weight {}", base + k, w[k]);
            }
        }
    }
}
