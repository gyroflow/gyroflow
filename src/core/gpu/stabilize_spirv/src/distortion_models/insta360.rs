// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3, Vec4, vec4 };

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
        // And only past what the terms that aren't radial add to it: they carry the image of the sphere's rim out of that
        // circle in some directions, and the 2D solve below decides there (see insta360_v6.rs / insta360.rs)
        let sphere_ends = Self::radial_derivative(m_max, k1, k2, k3) > 0.0;
        if sphere_ends && Self::radial_projection(m_max, k1, k2, k3) + 4.0 * m_max * m_max * (p1.abs() + p2.abs()) < r_d { return vec2(-99999.0, -99999.0); }
        // Between the two only a point the solve reaches is one: what it stalls at short of a point out there is the edge of
        // the image, and the point past it
        let past_the_rim = sphere_ends && Self::radial_projection(m_max, k1, k2, k3) < r_d;

        // Strip the tangential terms, invert the radial polynomial, re-evaluate them
        let mut a = point;
        let mut uv = vec2(0.0, 0.0);
        let mut m = r_d;
        let mut i = 0; while i < 3 {
            let r_s = a.length();
            m = Self::solve_radial(r_s, m, m_max, k1, k2, k3);
            uv = if r_s > 1e-12 { a * (m / r_s) } else { vec2(0.0, 0.0) };
            let r2 = uv.dot(uv);
            a = point - vec2(2.0*p1*uv.x*uv.y + p2*(r2 + 2.0*uv.x*uv.x),
                             2.0*p2*uv.x*uv.y + p1*(r2 + 2.0*uv.y*uv.y));
            i += 1;
        }
        // Large tangential terms leave that pixels off: Newton on the whole map finishes from there, inside the
        // sphere's fold and never from past a fold of the map (see `polish_inverse` in the core's distortion_models)
        let mut e = Self::plane(uv, params) - point;
        let mut err = e.x.abs().max(e.y.abs());
        let tol = 5e-7 * (1.0 + r_d);
        let mut exact_only = past_the_rim;
        let mut attempt = 0; while attempt < 2 {
            let mut it = 0; while it < 8 {
                if !(err > tol) { break; }
                let j = Self::plane_jacobian(uv, params);
                let det = j.x * j.w - j.y * j.z;
                if !(det > 0.0) { break; }
                let step = vec2(j.w * e.x - j.y * e.y, j.x * e.y - j.z * e.x) / det;
                let mut lambda = 1.0;
                let mut moved = false;
                let mut b = 0; while b < 4 {
                    let c = uv - step * lambda;
                    if c.dot(c) <= m_max * m_max {
                        let ce = Self::plane(c, params) - point;
                        let cerr = ce.x.abs().max(ce.y.abs());
                        if cerr < err { uv = c; e = ce; err = cerr; moved = true; break; }
                    }
                    lambda *= 0.5;
                    b += 1;
                }
                if !moved { break; }
                it += 1;
            }
            let jf = Self::plane_jacobian(uv, params);
            if jf.x * jf.w - jf.y * jf.z > 0.0 { break; }
            // A point past a fold of the map has no ray: the camera images the other branch. Where that fold is the edge of
            // the image, the point it images can be just short of it: back towards the axis until the map keeps its orientation,
            // and Newton from there; only a point it then reaches counts (see polish_inverse in distortion_models/mod.rs)
            err = 1e30;
            if attempt == 1 { break; }
            let uv0 = uv;
            let mut s = 1e-3;
            let mut onto = false;
            let mut r = 0; while r < 8 {
                let c = uv0 * (1.0 - s);
                let cj = Self::plane_jacobian(c, params);
                if cj.x * cj.w - cj.y * cj.z > 0.0 { uv = c; onto = true; break; }
                s *= 2.0;
                r += 1;
            }
            if !onto { break; }
            e = Self::plane(uv, params) - point;
            err = e.x.abs().max(e.y.abs());
            exact_only = true;
            attempt += 1;
        }
        // How far the map lands from the point is the test of whether there was a ray here at all
        let accept_err = if exact_only { 1e-6 * (1.0 + r_d) } else { 1e-4 * (1.0 + r_d) };
        if !(err <= accept_err) { return vec2(-99999.0, -99999.0); }

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

    /// The projection of a unified-sphere plane point, as [`Self::distort_point`] has it
    fn plane(uv: Vec2, params: &KernelParams) -> Vec2 {
        let (k1, k2, k3, p1, p2) = (params.k1.x, params.k1.y, params.k1.z, params.k1.w, params.k2.x);
        let r2 = uv.dot(uv);
        let rad = 1.0 + r2 * (k1 + r2 * (k2 + r2 * k3));
        vec2(uv.x * rad + 2.0*p1*uv.x*uv.y + p2*(r2 + 2.0*uv.x*uv.x),
             uv.y * rad + 2.0*p2*uv.x*uv.y + p1*(r2 + 2.0*uv.y*uv.y))
    }
    /// Its Jacobian: `(∂x/∂u, ∂x/∂v, ∂y/∂u, ∂y/∂v)`
    fn plane_jacobian(uv: Vec2, params: &KernelParams) -> Vec4 {
        let (k1, k2, k3, p1, p2) = (params.k1.x, params.k1.y, params.k1.z, params.k1.w, params.k2.x);
        let (u, v) = (uv.x, uv.y);
        let r2 = u*u + v*v;
        let rad = 1.0 + r2 * (k1 + r2 * (k2 + r2 * k3));
        let drad = k1 + r2 * (2.0*k2 + 3.0*k3*r2); // d(rad)/d(r²)
        vec4(rad + 2.0*u*u*drad + 2.0*p1*v + 6.0*p2*u,
             2.0*u*v*drad       + 2.0*p1*u + 2.0*p2*v,
             2.0*u*v*drad       + 2.0*p2*v + 2.0*p1*u,
             rad + 2.0*v*v*drad + 2.0*p2*u + 6.0*p1*v)
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
