// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: https://github.com/opencv/opencv/blob/c3cbd302cbfbaefdef9a011b2615f8d8f58556dd/modules/calib3d/src/undistort.dispatch.cpp#L491-L538

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3, Vec4, vec4 };
pub struct OpenCVStandard { }

impl OpenCVStandard {
    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        let (mut x, mut y) = (point.x, point.y);
        let (x0, y0) = (point.x, point.y);

        // compensate distortion iteratively
        let mut i = 0; while i < 20 {
        // for _ in 0..20 {
            let r2 = x * x + y * y;
            let icdist = (1.0 + ((params.k2.w * r2 + params.k2.z) * r2 + params.k2.y) * r2) / (1.0 + ((params.k2.x * r2 + params.k1.y) * r2 + params.k1.x) * r2);
            if icdist < 0.0 {
                // log::warn!("icdist < 0");
                return vec2(-99999.0, -99999.0);
            }
            let delta_x = 2.0 * params.k1.z * x * y + params.k1.w * (r2 + 2.0 * x * x) + params.k3.x  * r2 + params.k3.y  * r2 * r2;
            let delta_y = params.k1.z * (r2 + 2.0 * y * y) + 2.0 * params.k1.w * x * y + params.k3.z * r2 + params.k3.w * r2 * r2;
            x = (x0 - delta_x) * icdist;
            y = (y0 - delta_y) * icdist;
            i += 1;
        }

        // That is a fixed point with the tangential and thin-prism terms held still, and it only contracts while they
        // and the slope of the radial curve are small: a wide lens with large ones is left hundreds of pixels off in the
        // corners. Newton on the whole map from there (see `polish_inverse` in the core's distortion_models); where
        // neither finds the point, the lens has no ray for it
        let mut pos = vec2(x, y);
        if !(pos.x.abs() < 1e30 && pos.y.abs() < 1e30) { pos = point; } // not finite
        let tol = 1e-4 * (1.0 + point.length());
        let (mut pos, mut err) = Self::polish(pos, point, params);
        if !(err <= tol) {
            // A steep radial curve throws the fixed point between two far-apart values, and Newton from either is too slow
            // to arrive: start again from the radial curve's own inverse, which for radial terms only is the answer
            let r_d = point.length();
            let r = Self::solve_radial(r_d, params);
            if r >= 0.0 && r_d > 1e-12 { (pos, err) = Self::polish(point * (r / r_d), point, params); }
        }
        if !(err <= tol) { return vec2(-99999.0, -99999.0); }
        let (x, y) = (pos.x, pos.y);

        // The solve runs in the z=1 plane; the pipeline carries rays as angle vectors
        let r = (x * x + y * y).sqrt();
        if r < 1e-12 { return vec2(x, y); }
        vec2(x, y) * (r.atan() / r)
    }

    /// Newton on the whole map from `pos` (see `polish_inverse` in the core's distortion_models): the point, and how far
    /// its image still is from `p` - 1e30 where the map has turned over at the point, a ray of the branch the camera
    /// doesn't image with
    fn polish(mut pos: Vec2, p: Vec2, params: &KernelParams) -> (Vec2, f32) {
        let mut e = Self::plane(pos, params) - p;
        let mut err = e.x.abs().max(e.y.abs());
        let tol = 5e-7 * (1.0 + p.length());
        let mut attempt = 0; while attempt < 2 {
            let mut it = 0; while it < 8 {
                if !(err > tol) { break; }
                let j = Self::plane_jacobian(pos, params);
                let det = j.x * j.w - j.y * j.z;
                if !(det > 0.0) { break; }
                let step = vec2(j.w * e.x - j.y * e.y, j.x * e.y - j.z * e.x) / det;
                let mut lambda = 1.0;
                let mut moved = false;
                let mut b = 0; while b < 4 {
                    let c = pos - step * lambda;
                    let ce = Self::plane(c, params) - p;
                    let cerr = ce.x.abs().max(ce.y.abs());
                    if cerr < err { pos = c; e = ce; err = cerr; moved = true; break; }
                    lambda *= 0.5;
                    b += 1;
                }
                if !moved { break; }
                it += 1;
            }
            let jf = Self::plane_jacobian(pos, params);
            if jf.x * jf.w - jf.y * jf.z > 0.0 { if attempt == 1 && !(err <= 1e-6 * (1.0 + p.length())) { err = 1e30; } break; }
            // A point past a fold of the map has no ray: the camera images the other branch. Where that fold is the edge of
            // the image, the point it images can be just short of it: back towards the axis until the map keeps its orientation,
            // and Newton from there; only a point it then reaches counts (see polish_inverse in distortion_models/mod.rs)
            err = 1e30;
            if attempt == 1 { break; }
            let pos0 = pos;
            let mut s = 1e-3;
            let mut onto = false;
            let mut r = 0; while r < 8 {
                let c = pos0 * (1.0 - s);
                let cj = Self::plane_jacobian(c, params);
                if cj.x * cj.w - cj.y * cj.z > 0.0 { pos = c; onto = true; break; }
                s *= 2.0;
                r += 1;
            }
            if !onto { break; }
            e = Self::plane(pos, params) - p;
            err = e.x.abs().max(e.y.abs());
            attempt += 1;
        }
        (pos, err)
    }
    fn radial(r: f32, params: &KernelParams) -> f32 {
        let r2 = r * r;
        r * (1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * params.k2.x))) / (1.0 + r2 * (params.k2.y + r2 * (params.k2.z + r2 * params.k2.w)))
    }
    /// The plane radius whose radial image is `r_d`: bracketed from the axis outwards and bisected (see the core's
    /// `opencv_standard.rs`). -1 where the curve turns back, or runs out, before it reaches `r_d`
    fn solve_radial(r_d: f32, params: &KernelParams) -> f32 {
        let (mut lo, mut hi) = (0.0f32, r_d);
        let mut g_hi = Self::radial(hi, params);
        let mut n = 0; while n < 31 {
            if g_hi >= r_d { break; }
            let g_next = Self::radial(hi * 2.0, params);
            if !(g_next > g_hi) || n == 30 { return -1.0; }
            lo = hi;
            hi *= 2.0;
            g_hi = g_next;
            n += 1;
        }
        let mut i = 0; while i < 40 {
            let mid = 0.5 * (lo + hi);
            if Self::radial(mid, params) < r_d { lo = mid; } else { hi = mid; }
            i += 1;
        }
        0.5 * (lo + hi)
    }
    /// The projection of a z=1 plane point, as [`Self::distort_point`] has it
    fn plane(pos: Vec2, params: &KernelParams) -> Vec2 {
        let (x, y) = (pos.x, pos.y);
        let r2 = x * x + y * y;
        let rad = (1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * params.k2.x))) / (1.0 + r2 * (params.k2.y + r2 * (params.k2.z + r2 * params.k2.w)));
        vec2(x * rad + params.k1.z * 2.0 * x * y + params.k1.w * (r2 + 2.0 * x * x) + params.k3.x * r2 + params.k3.y * r2 * r2,
             y * rad + params.k1.z * (r2 + 2.0 * y * y) + params.k1.w * 2.0 * x * y + params.k3.z * r2 + params.k3.w * r2 * r2)
    }
    /// Its Jacobian: `(∂x/∂u, ∂x/∂v, ∂y/∂u, ∂y/∂v)`
    fn plane_jacobian(pos: Vec2, params: &KernelParams) -> Vec4 {
        let (x, y) = (pos.x, pos.y);
        let r2 = x * x + y * y;
        let cd  = 1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * params.k2.x));
        let cd2 = 1.0 + r2 * (params.k2.y + r2 * (params.k2.z + r2 * params.k2.w));
        let rad = cd / cd2;
        // d(rad)/d(r²)
        let drad = (params.k1.x + r2 * (2.0 * params.k1.y + 3.0 * params.k2.x * r2) - rad * (params.k2.y + r2 * (2.0 * params.k2.z + 3.0 * params.k2.w * r2))) / cd2;
        // d(thin prism)/d(r²)
        let sx = params.k3.x + 2.0 * params.k3.y * r2;
        let sy = params.k3.z + 2.0 * params.k3.w * r2;
        let (p1, p2) = (params.k1.z, params.k1.w);
        vec4(rad + 2.0 * x * x * drad + 2.0 * p1 * y + 6.0 * p2 * x + 2.0 * x * sx,
             2.0 * x * y * drad       + 2.0 * p1 * x + 2.0 * p2 * y + 2.0 * y * sx,
             2.0 * x * y * drad       + 2.0 * p1 * x + 2.0 * p2 * y + 2.0 * x * sy,
             rad + 2.0 * y * y * drad + 6.0 * p1 * y + 2.0 * p2 * x + 2.0 * y * sy)
    }

    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        // A rectilinear model has no image of a ray at or past 90°: send it far outside the frame
        let inv_z = if point.z > 1e-9 { 1.0 / point.z } else { 1e9 };
        let x = point.x * inv_z;
        let y = point.y * inv_z;
        let r2 = x * x + y * y;
        let r4 = r2 * r2;
        let r6 = r4 * r2;
        let a1 = 2.0 * x * y;
        let a2 = r2 + 2.0 * x * x;
        let a3 = r2 + 2.0 * y * y;
        let cdist = 1.0 + params.k1.x * r2 + params.k1.y * r4 + params.k2.x * r6;
        let icdist2 = 1.0 / (1.0 + params.k2.y * r2 + params.k2.z * r4 + params.k2.w * r6);
        let xd0 = x * cdist * icdist2 + params.k1.z * a1 + params.k1.w * a2 + params.k3.x  * r2 + params.k3.y  * r4;
        let yd0 = y * cdist * icdist2 + params.k1.z * a3 + params.k1.w * a1 + params.k3.z * r2 + params.k3.w * r4;

        vec2(xd0, yd0)
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
