// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use crate::stabilization::KernelParams;

#[derive(Default, Clone)]
pub struct Insta360 { }

impl Insta360 {
    /// Radial part of the projection: image radius of a unified-sphere plane radius `m`
    #[inline] fn radial(m: f32, k1: f32, k2: f32, k3: f32) -> f32 {
        let m2 = m * m;
        m * (1.0 + k1*m2 + k2*m2*m2 + k3*m2*m2*m2)
    }
    /// `d(radial)/dm`
    #[inline] fn radial_derivative(m: f32, k1: f32, k2: f32, k3: f32) -> f32 {
        let m2 = m * m;
        1.0 + 3.0*k1*m2 + 5.0*k2*m2*m2 + 7.0*k3*m2*m2*m2
    }
    /// How far the *sphere* reaches: it folds at `θ = acos(-1/ξ)` (120° for the X5's ξ = 2), and for ξ ≤ 1
    /// it never folds but runs to infinity at `acos(-ξ)`, which [`Self::field_limit`] puts an epsilon short
    /// of and which is what the placeholder below stands for.
    ///
    /// Only half the story, and never a bound on the *image* radius on its own: the radial polynomial has a
    /// fold of its own wherever `radial'` first reaches zero, and it can come first. `radial(m_max)` is the
    /// largest radius the lens images only where the curve is still rising there - `radial_derivative` at
    /// `m_max` says whether it is - and past the fold the polynomial says whatever a degree-7 polynomial
    /// says: with the X5's own `k₃ = -3` and this placeholder it says -3·10²⁸, which read as a maximum
    /// image radius rejects every pixel in the frame
    #[inline] fn m_max(xi: f32) -> f32 {
        if xi > 1.0 { 1.0 / (xi * xi - 1.0).sqrt() } else { 1e4 }
    }
    /// Newton on `radial(m) = r`, kept on the branch the camera projects with.
    ///
    /// The bracket is what keeps it there. `radial` leaves 0 with slope 1 and the camera images the stretch
    /// up to its first fold; a Newton step that lands where the curve has stopped rising has stepped over
    /// that fold, and pulling the bracket's far end back to there - rather than letting the step stand -
    /// walks up to the fold instead of across it. So the fold never has to be solved for up front (it is
    /// the first positive root of a cubic in `m²`) and costs nothing at all where there is none, which is
    /// every calibration that folds later than the sphere does - the X5's own among them.
    ///
    /// The answer is only an answer if it reaches `r`: past the end of the rising branch there is no `m`
    /// with `radial(m) = r` and this returns the end of the branch instead, which the caller checks for
    #[inline] fn solve_radial(r: f32, guess: f32, m_max: f32, k1: f32, k2: f32, k3: f32) -> f32 {
        let (mut lo, mut hi) = (0.0f32, m_max);
        let mut m = guess.max(0.0).min(m_max);
        // Newton is done in three or four; the twenty are for the last ring of the image circle, where the
        // curve is nearly flat and a step overshoots the fold, and the way back is a bisection
        for _ in 0..20 {
            let d = Self::radial_derivative(m, k1, k2, k3);
            if d <= 1e-9 {
                // The curve has folded at or before `m`; the root, if there is one, is below it
                hi = m;
                m = 0.5 * (lo + hi);
            } else {
                let f = Self::radial(m, k1, k2, k3) - r;
                // Solved, judged on the residual: `radial(m)` is only good to a few ulp of itself, so this
                // is the floor the solve can reach, and the final check below asks for 1e-4. Not on the
                // size of the Newton step: below an ulp of `m` that step is one a GPU computes exactly as
                // `-f/d` and then rounds away when it adds it to `m`, and a test on the difference of those
                // two never came out true on an RTX 5090 (the CPU only passes it because its `next - m`
                // happens to be exactly zero)
                if f.abs() <= 2e-7 * (1.0 + r) { break; }
                if f < 0.0 { lo = m; } else { hi = m; }
                let next = m - f / d;
                // A step that lands on an end of the bracket is a step of nothing, and the end it lands on
                // is `m` itself - kept, so the next iteration is this one again. Read as *outside* the
                // bracket it would be replaced by the bracket's midpoint, and the far end of the bracket
                // is still the sphere's own `m_max` whenever Newton has only ever approached the root from
                // one side: that threw a converged answer 0.18 away and left the remaining iterations
                // bisecting their way back, to about 1e-4 - a coin flip against the check below, and on
                // the GO 3 at 0% correction a fifth of the frame was background, pixel by pixel
                m = if next >= lo && next <= hi { next } else { 0.5 * (lo + hi) }; // outside (or NaN): bisect
            }
        }
        m
    }

    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let k1 = params.k[0];
        let k2 = params.k[1];
        let k3 = params.k[2];
        let p1 = params.k[3];
        let p2 = params.k[4];
        let xi = params.k[5];

        let r_d = (point.0 * point.0 + point.1 * point.1).sqrt();
        if r_d < 1e-9 { return Some(point); }

        // Past the end of the projection's rising branch there is no ray at all - but only where the
        // sphere's own end *is* that end. Where the polynomial folds first, `radial(m_max)` is a value off
        // the far side of the fold and says nothing about the largest radius the lens images; the solve's
        // own answer, checked below, is what decides there
        let m_max = Self::m_max(xi);
        if Self::radial_derivative(m_max, k1, k2, k3) > 0.0 && Self::radial(m_max, k1, k2, k3) < r_d { return None; }

        // Strip the tangential terms, invert the radial polynomial, re-evaluate them: three passes are
        // already exact to ~1e-9 of the ray radius, and unlike a fixed-point iteration on the whole
        // 2D map it stays stable where the projection compresses hardest (the old 200-step loop
        // silently gave up well inside the valid field, from ~74° on the X5).
        let (mut ax, mut ay) = point;
        let (mut u, mut v, mut m, mut r_s) = (0.0f32, 0.0f32, r_d, r_d);
        for _ in 0..3 {
            r_s = (ax * ax + ay * ay).sqrt();
            m = Self::solve_radial(r_s, m, m_max, k1, k2, k3);
            let s = if r_s > 1e-12 { m / r_s } else { 0.0 };
            u = ax * s;
            v = ay * s;
            let r2 = u*u + v*v;
            ax = point.0 - (2.0*p1*u*v + p2*(r2 + 2.0*u*u));
            ay = point.1 - (2.0*p2*u*v + p1*(r2 + 2.0*v*v));
        }
        // `solve_radial` returns the end of the rising branch when `r` is past everything the lens images,
        // so the radius it actually reached is the test of whether there was a ray here at all. This is the
        // whole of the rejection wherever the polynomial folds inside the sphere - a ξ ≤ 1 body has no
        // sphere fold to be bounded by, and asking a folded polynomial for its maximum gives nonsense
        if (Self::radial(m, k1, k2, k3) - r_s).abs() > 1e-4 * (1.0 + r_s) { return None; }

        // Unified sphere -> ray angle. The sphere point is `(u·α, v·α, α - ξ)`, already a unit vector, so
        // the angle falls out of `atan2` - where the old `x/z` went to infinity at 90° and cost an X4/X5
        // the outer 10% of its own image circle
        let rho2 = u*u + v*v;
        let disc = 1.0 + (1.0 - xi*xi) * rho2;
        if disc < 0.0 { return None; }
        let alpha = (xi + disc.sqrt()) / (rho2 + 1.0);
        let rho = rho2.sqrt();
        if rho < 1e-12 { return Some((0.0, 0.0)); }
        let theta = (rho * alpha).atan2(alpha - xi);
        let s = theta / rho;

        Some((u * s, v * s))
    }

    pub fn distort_point(&self, mut x: f32, mut y: f32, z: f32, params: &KernelParams) -> (f32, f32) {
        let k1 = params.k[0];
        let k2 = params.k[1];
        let k3 = params.k[2];
        let p1 = params.k[3];
        let p2 = params.k[4];
        let xi = params.k[5];

        let len = (x.powi(2) + y.powi(2) + z.powi(2)).sqrt();

        x = (x / len) / ((z / len) + xi);
        y = (y / len) / ((z / len) + xi);

        let r2 = x*x + y*y;
        let r4 = r2 * r2;
        let r6 = r4 * r2;

        (
            x * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p1*x*y + p2*(r2 + 2.0*x*x),
            y * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p2*x*y + p1*(r2 + 2.0*y*y)
        )
    }
    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    /// The lens's own field end, in radians: the unified sphere folds at `acos(-1/ξ)` - 120° for the X5's
    /// ξ = 2, well past the 100° its image circle actually holds - and for ξ ≤ 1 it never folds but runs to
    /// infinity at `acos(-ξ)` instead. There is no ray past either
    pub fn field_limit(k: &[f64]) -> Option<f64> {
        let xi = *k.get(5)?;
        if xi > 1.0 { Some((-1.0 / xi).acos()) }
        else if xi > 1e-6 { Some((-xi).acos() - 1e-4) }
        else { Some(std::f64::consts::FRAC_PI_2 - 1e-4) } // ξ = 0 is a plain pinhole
    }

    /// `d(image radius)/dθ`, `<= 0` where the radial curve folds back. `dm/dθ` carries the unified
    /// sphere's own fold, `dr/dm` the polynomial's
    pub fn distortion_derivative(&self, theta: f64, k: &[f64]) -> Option<f64> {
        if k.len() < 6 { return None; }
        let (k1, k2, k3, xi) = (k[0], k[1], k[2], k[5]);
        let c = theta.cos();
        let d = c + xi;
        if d.abs() < 1e-9 { return None; }
        let m = theta.sin() / d;
        let m2 = m * m;
        let dm_dtheta = (1.0 + xi * c) / (d * d);
        let dr_dm = 1.0 + 3.0*k1*m2 + 5.0*k2*m2*m2 + 7.0*k3*m2*m2*m2;
        Some(dr_dm * dm_dtheta)
    }

    pub fn id() -> &'static str { "insta360" }
    pub fn name() -> &'static str { "Insta360" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("insta360.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("insta360.wgsl") }
}
