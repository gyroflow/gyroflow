// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };

pub struct Insta360 { }

impl Insta360 {
    /// Radial part of the projection: image radius of a unified-sphere plane radius `m`
    fn radial_projection(m: f32, k1: f32, k2: f32, k3: f32) -> f32 {
        let m2 = m * m;
        m * (1.0 + k1*m2 + k2*m2*m2 + k3*m2*m2*m2)
    }
    /// `d(radial_projection)/dm`
    fn radial_derivative(m: f32, k1: f32, k2: f32, k3: f32) -> f32 {
        let m2 = m * m;
        1.0 + 3.0*k1*m2 + 5.0*k2*m2*m2 + 7.0*k3*m2*m2*m2
    }
    /// How far the *sphere* reaches: it folds at `θ = acos(-1/ξ)` (120° for the X5's ξ = 2), and for ξ ≤ 1
    /// it never folds but runs to infinity, which the placeholder stands for. Never a bound on the image
    /// radius on its own - the radial polynomial folds where `radial_derivative` first reaches zero, and
    /// that can come first. See `distortion_models::insta360`
    fn m_max(xi: f32) -> f32 {
        if xi > 1.0 { 1.0 / (xi * xi - 1.0).sqrt() } else { 1e4 }
    }
    /// Newton on `radial_projection(m) = r`, kept on the branch the camera projects with: a step that lands
    /// where the curve has stopped rising has stepped over its fold, and pulling the bracket in to there
    /// walks up to the fold instead of across it. Returns the end of the branch when `r` is past everything
    /// the lens images
    fn solve_radial(r: f32, guess: f32, m_max: f32, k1: f32, k2: f32, k3: f32) -> f32 {
        let (mut lo, mut hi) = (0.0f32, m_max);
        let mut m = guess.max(0.0).min(m_max);
        let mut i = 0; while i < 20 { // Newton is done in a few; the rest are the fold's bisection
            let d = Self::radial_derivative(m, k1, k2, k3);
            if d <= 1e-9 {
                hi = m; // folded at or before `m`; the root, if there is one, is below it
                m = 0.5 * (lo + hi);
            } else {
                let f = Self::radial_projection(m, k1, k2, k3) - r;
                // Solved, judged on the residual and not on the size of the step: a step below an ulp of
                // `m` is one a GPU computes exactly and then rounds away, and a test on it never passes.
                // See `distortion_models::insta360`
                if f.abs() <= 2e-7 * (1.0 + r) { break; }
                if f < 0.0 { lo = m; } else { hi = m; }
                let next = m - f / d;
                // A step landing on an end of the bracket is a step of nothing - kept, not read as outside
                // the bracket and replaced by the midpoint of a bracket whose far end may still be `m_max`
                m = if next >= lo && next <= hi { next } else { 0.5 * (lo + hi) }; // outside (or NaN): bisect
            }
            i += 1;
        }
        m
    }

    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        let k1 = params.k1.x;
        let k2 = params.k1.y;
        let k3 = params.k1.z;
        let p1 = params.k1.w;
        let p2 = params.k2.x;
        let xi = params.k2.y;

        let r_d = point.length();
        if r_d < 1e-9 { return point; }

        // Past the end of the projection's rising branch there is no ray at all - but only where the
        // sphere's own end *is* that end; where the polynomial folds first, `radial_projection(m_max)` is
        // off the far side of the fold and says nothing about the largest radius the lens images
        let m_max = Self::m_max(xi);
        if Self::radial_derivative(m_max, k1, k2, k3) > 0.0 && Self::radial_projection(m_max, k1, k2, k3) < r_d { return vec2(-99999.0, -99999.0); }

        // Strip the tangential terms, invert the radial polynomial, re-evaluate them
        let mut a = point;
        let mut uv = vec2(0.0, 0.0);
        let mut m = r_d;
        let mut r_s = r_d;
        let mut i = 0; while i < 3 {
            r_s = a.length();
            m = Self::solve_radial(r_s, m, m_max, k1, k2, k3);
            uv = if r_s > 1e-12 { a * (m / r_s) } else { vec2(0.0, 0.0) };
            let r2 = uv.dot(uv);
            a = point - vec2(2.0*p1*uv.x*uv.y + p2*(r2 + 2.0*uv.x*uv.x),
                             2.0*p2*uv.x*uv.y + p1*(r2 + 2.0*uv.y*uv.y));
            i += 1;
        }
        // The radius the solve actually reached is the test of whether there was a ray here at all
        if (Self::radial_projection(m, k1, k2, k3) - r_s).abs() > 1e-4 * (1.0 + r_s) { return vec2(-99999.0, -99999.0); }

        // Unified sphere -> ray angle. The sphere point is `(u·α, v·α, α - ξ)`, already a unit vector, so
        // the angle falls out of `atan2` - where the old `x/z` went to infinity at 90°
        let rho2 = uv.dot(uv);
        let disc = 1.0 + (1.0 - xi*xi) * rho2;
        if disc < 0.0 { return vec2(-99999.0, -99999.0); }
        let alpha = (xi + disc.sqrt()) / (rho2 + 1.0);
        let rho = rho2.sqrt();
        if rho < 1e-12 { return vec2(0.0, 0.0); }
        uv * ((rho * alpha).atan2(alpha - xi) / rho)
    }

    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let k1 = params.k1.x;
        let k2 = params.k1.y;
        let k3 = params.k1.z;
        let p1 = params.k1.w;
        let p2 = params.k2.x;
        let xi = params.k2.y;

        let len = point.length();

        let x = (point.x / len) / ((point.z / len) + xi);
        let y = (point.y / len) / ((point.z / len) + xi);

        let r2 = x*x + y*y;
        let r4 = r2 * r2;
        let r6 = r4 * r2;

        vec2(
            x * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p1*x*y + p2*(r2 + 2.0*x*x),
            y * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p2*x*y + p1*(r2 + 2.0*y*y)
        )
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
