// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Insta360's `offset_v6` lens (and `offset_v8`'s, at each AF code): the unified sphere of `insta360`, with a 13-term
//! distortion block in place of its Brown one. `k` is `d0..d12, ξ`:
//!
//! ```text
//! m  = (x, y) / (z + ξ·‖p‖)                         the unified sphere, as in `insta360`
//! r² = |m|²,  R = 1 + d0·r² + d1·r⁴ + d2·r⁶ + d3·r⁸ + d4·r¹⁰
//! xd = mx·R + (r² + 2mx²)(d5 + d7·r²) + 2·mx·my·(d6 + d8·r²) + d9·r²  + d11·r⁴
//! yd = my·R + (r² + 2my²)(d6 + d8·r²) + 2·mx·my·(d5 + d7·r²) + d10·r² + d12·r⁴
//! ```
//!
//! The decentering pair is in the opposite order to `insta360`'s `p1 p2`, and its second pair grows with `r²`.

use crate::stabilization::KernelParams;
use super::{ Real, c };

#[derive(Default, Clone)]
pub struct Insta360V6 { }

/// The end the bracket of the radial solve has for `ξ ≤ 1`, where the sphere never folds. Lower than `insta360`'s
/// 1e4: its polynomial is of degree 11, and 1e4 to that power is past `f32`
const M_MAX_UNFOLDED: f32 = 1e3;

impl Insta360V6 {
    /// Radial part of the projection: image radius of a unified-sphere plane radius `m`
    #[inline] fn radial(m: f32, d: &[f32]) -> f32 {
        let m2 = m * m;
        m * (1.0 + m2 * (d[0] + m2 * (d[1] + m2 * (d[2] + m2 * (d[3] + m2 * d[4])))))
    }
    /// `d(radial)/dm`, in either precision (see `Real`): the render's and [`Self::distortion_derivative`]'s
    #[inline] fn radial_derivative<T: Real>(m: T, d: &[T]) -> T {
        let m2 = m * m;
        c::<T>(1.0) + m2 * (c::<T>(3.0)*d[0] + m2 * (c::<T>(5.0)*d[1] + m2 * (c::<T>(7.0)*d[2] + m2 * (c::<T>(9.0)*d[3] + m2 * c::<T>(11.0)*d[4]))))
    }
    /// A bound on how far the terms that aren't radial move a point of plane radius `√m2`: past `radial(m) + this`
    /// no point of radius up to `m` lands
    #[inline] fn non_radial_bound(m2: f32, d: &[f32]) -> f32 {
        m2 * (4.0 * (d[5].abs() + d[6].abs() + (d[7].abs() + d[8].abs()) * m2) + d[9].abs() + d[10].abs() + (d[11].abs() + d[12].abs()) * m2)
    }
    /// The terms that aren't radial: the decentering and the thin prism, at the sphere-plane point `(u, v)`
    #[inline] fn non_radial(u: f32, v: f32, d: &[f32]) -> (f32, f32) {
        let r2 = u*u + v*v;
        let (px, py) = (d[5] + d[7]*r2, d[6] + d[8]*r2);
        (
            (r2 + 2.0*u*u) * px + 2.0*u*v * py + d[9]  * r2 + d[11] * r2*r2,
            (r2 + 2.0*v*v) * py + 2.0*u*v * px + d[10] * r2 + d[12] * r2*r2,
        )
    }
    /// How far the *sphere* reaches: it folds at `θ = acos(-1/ξ)` (104.5° for ξ = 4), and for ξ ≤ 1 it never folds,
    /// which [`M_MAX_UNFOLDED`] stands for. Never a bound on the image radius on its own: the radial polynomial has a
    /// fold of its own, which can come first. See `insta360`
    #[inline] fn m_max(xi: f32) -> f32 {
        if xi > 1.0 { 1.0 / (xi * xi - 1.0).sqrt() } else { M_MAX_UNFOLDED }
    }
    /// Newton on `radial(m) = r`, bracketed to the branch the camera projects with, as `insta360`'s: a step that lands
    /// where the curve has stopped rising has stepped over its fold, and pulling the bracket in to there walks up to
    /// the fold instead of across it. Returns the end of the branch when `r` is past everything the lens images
    #[inline] fn solve_radial(r: f32, guess: f32, m_max: f32, d: &[f32]) -> f32 {
        let (mut lo, mut hi) = (0.0f32, m_max);
        let mut m = guess.max(0.0).min(m_max);
        for _ in 0..20 {
            let dr = Self::radial_derivative(m, d);
            if dr <= 1e-9 {
                hi = m; // folded at or before `m`; the root, if there is one, is below it
                m = 0.5 * (lo + hi);
            } else {
                let f = Self::radial(m, d) - r;
                // Solved, judged on the residual and not on the size of the step (see `insta360`)
                if f.abs() <= 2e-7 * (1.0 + r) { break; }
                if f < 0.0 { lo = m; } else { hi = m; }
                let next = m - f / dr;
                m = if next >= lo && next <= hi { next } else { 0.5 * (lo + hi) }; // outside (or NaN): bisect
            }
        }
        m
    }

    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let d = &params.k[..13];
        let xi = params.k[13];

        let r_d = (point.0 * point.0 + point.1 * point.1).sqrt();
        if r_d < 1e-9 { return Some(point); }

        // Past the end of the projection's rising branch there is no ray - but only where the sphere's own end is
        // that end; where the polynomial folds first, `radial(m_max)` says nothing (see `insta360`). And only past
        // what the terms that aren't radial add to it: they carry the image of the sphere's rim out of the circle
        // `radial(m_max)` in some directions, and the 2D solve below decides there
        let m_max = Self::m_max(xi);
        let mm = m_max * m_max;
        let sphere_ends = Self::radial_derivative(m_max, d) > 0.0;
        if sphere_ends && Self::radial(m_max, d) + Self::non_radial_bound(mm, d) < r_d { return None; }
        // Between the two only a point the solve reaches is one (`converged`): what it stalls at short of a point out there
        // is the edge of the image, and the point past it
        let past_the_rim = sphere_ends && Self::radial(m_max, d) < r_d;

        // Strip the terms that aren't radial, invert the radial polynomial, re-evaluate them. They are small next to
        // the radial ones, so each pass shrinks the error by about their own size
        let (mut ax, mut ay) = point;
        let (mut u, mut v, mut m) = (0.0f32, 0.0f32, r_d);
        for _ in 0..4 {
            let r_s = (ax * ax + ay * ay).sqrt();
            m = Self::solve_radial(r_s, m, m_max, d);
            let s = if r_s > 1e-12 { m / r_s } else { 0.0 };
            u = ax * s;
            v = ay * s;
            let (tx, ty) = Self::non_radial(u, v, d);
            ax = point.0 - tx;
            ay = point.1 - ty;
        }
        // Large ones leave that pixels off, though: Newton on the whole map finishes from there, inside the sphere's fold
        let err;
        ((u, v), err) = super::polish_inverse(point, (u, v), |u, v| Self::map_with_jacobian(u, v, d), |u, v| u*u + v*v <= mm);
        // How far the map lands from the point is the test of whether there was a ray here at all: `solve_radial` returns
        // the end of the rising branch when the point is past everything the lens images, and a point past a fold of
        // the map has no ray either (an infinite `err`)
        let accept = if past_the_rim { super::converged(point) } else { 1e-4 * (1.0 + r_d) };
        if !(err <= accept) { return None; }

        // Unified sphere -> ray angle: the sphere point `(u·α, v·α, α - ξ)` is a unit vector
        let rho2 = u*u + v*v;
        let disc = 1.0 + (1.0 - xi*xi) * rho2;
        if disc < 0.0 { return None; }
        let alpha = (xi + disc.sqrt()) / (rho2 + 1.0);
        let rho = rho2.sqrt();
        if rho < 1e-12 { return Some((0.0, 0.0)); }
        let s = (rho * alpha).atan2(alpha - xi) / rho;
        Some((u * s, v * s))
    }

    pub fn distort_point(&self, x: f32, y: f32, z: f32, params: &KernelParams) -> (f32, f32) {
        let d = &params.k[..13];
        let xi = params.k[13];

        let len = (x * x + y * y + z * z).sqrt();
        let u = (x / len) / ((z / len) + xi);
        let v = (y / len) / ((z / len) + xi);

        let r2 = u*u + v*v;
        let radial = 1.0 + r2 * (d[0] + r2 * (d[1] + r2 * (d[2] + r2 * (d[3] + r2 * d[4]))));
        let (tx, ty) = Self::non_radial(u, v, d);
        (u * radial + tx, v * radial + ty)
    }
    /// The projection of a unified-sphere plane point, as [`Self::distort_point`] has it, and its Jacobian:
    /// `(x, y, ∂x/∂u, ∂x/∂v, ∂y/∂u, ∂y/∂v)`
    #[inline]
    fn map_with_jacobian(u: f32, v: f32, d: &[f32]) -> (f32, f32, f32, f32, f32, f32) {
        let r2 = u*u + v*v;
        let rad = 1.0 + r2 * (d[0] + r2 * (d[1] + r2 * (d[2] + r2 * (d[3] + r2 * d[4]))));
        let drad = d[0] + r2 * (2.0*d[1] + r2 * (3.0*d[2] + r2 * (4.0*d[3] + r2 * 5.0*d[4]))); // d(rad)/d(r²)
        let (px, py) = (d[5] + d[7]*r2, d[6] + d[8]*r2);
        let (tx, ty) = Self::non_radial(u, v, d);
        // d(non_radial)/d(r²), with the `u`, `v` of its other factors held still
        let qx = (r2 + 2.0*u*u) * d[7] + 2.0*u*v * d[8] + d[9]  + 2.0 * d[11] * r2;
        let qy = (r2 + 2.0*v*v) * d[8] + 2.0*u*v * d[7] + d[10] + 2.0 * d[12] * r2;
        (
            u * rad + tx,
            v * rad + ty,
            rad + 2.0*u*u*drad + 6.0*u*px + 2.0*v*py + 2.0*u*qx,
            2.0*u*v*drad       + 2.0*v*px + 2.0*u*py + 2.0*v*qx,
            2.0*u*v*drad       + 2.0*u*py + 2.0*v*px + 2.0*u*qy,
            rad + 2.0*v*v*drad + 6.0*v*py + 2.0*u*px + 2.0*v*qy,
        )
    }
    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    /// The lens's own field end, in radians, as `insta360`'s: the sphere folds at `acos(-1/ξ)`, and for ξ ≤ 1 runs to
    /// infinity at `acos(-ξ)` instead
    pub fn field_limit(k: &[f64]) -> Option<f64> {
        let xi = *k.get(13)?;
        if xi > 1.0 { Some((-1.0 / xi).acos()) }
        else if xi > 1e-6 { Some((-xi).acos() - 1e-4) }
        else { Some(std::f64::consts::FRAC_PI_2 - 1e-4) } // ξ = 0 is a plain pinhole
    }

    /// `d(image radius)/dθ`, `<= 0` where the radial curve folds back: `dm/dθ` carries the sphere's fold, `dr/dm`
    /// the polynomial's
    pub fn distortion_derivative(&self, theta: f64, k: &[f64]) -> Option<f64> {
        if k.len() < 14 { return None; }
        let xi = k[13];
        let c = theta.cos();
        let den = c + xi;
        if den.abs() < 1e-9 { return None; }
        let m = theta.sin() / den;
        let dm_dtheta = (1.0 + xi * c) / (den * den);
        Some(Self::radial_derivative(m, &k[..5]) * dm_dtheta)
    }

    pub fn id() -> &'static str { "insta360_v6" }
    pub fn name() -> &'static str { "Insta360 (13-term)" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("insta360_v6.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("insta360_v6.wgsl") }
}
