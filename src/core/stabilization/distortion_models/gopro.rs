// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// GoPro native radial lens model — uses the camera's in-camera GPMF POLY calibration:
//   `world_radians = POLY(p)`, p = r1·(normalized image radius). The raw POLY coefficients
//   r0..r6 are carried in `params.k[0..7]`. undistort = direct POLY eval (radius → angle);
//   distort = Newton-invert POLY (angle → radius).
//
// The Superview/Hyperview digital warp (MAPX/MAPY) is a separate pixel-space stage handled
// by the `gopro_warp` digital lens, so it composes correctly with the lens-correction and
// zoom paths (which renormalize around different focal lengths).

use crate::stabilization::KernelParams;

#[derive(Default, Clone)]
pub struct GoPro;

impl GoPro {
    #[inline] fn poly_eval(p: f32, k: &[f32; 12]) -> f32 {
        k[0] + p * (k[1] + p * (k[2] + p * (k[3] + p * (k[4] + p * (k[5] + p * k[6])))))
    }
    #[inline] fn poly_deriv(p: f32, k: &[f32; 12]) -> f32 {
        k[1] + p * (2.0 * k[2] + p * (3.0 * k[3] + p * (4.0 * k[4] + p * (5.0 * k[5] + p * (6.0 * k[6])))))
    }
    // Solve POLY(p) = theta for p (angle -> normalized radius parameter).
    #[inline] fn poly_invert(theta: f32, k: &[f32; 12]) -> f32 {
        let mut p = (theta - k[0]) / k[1]; // paraxial guess (k0 ≈ 0)
        for _ in 0..10 {
            let d = Self::poly_deriv(p, k);
            if d.abs() < 1e-12 { break; }
            let fix = (Self::poly_eval(p, k) - theta) / d;
            p -= fix;
            if fix.abs() < 1e-7 { break; }
        }
        p
    }

    /// `point` range: normalized (recorded pixel - c) / f
    /// From image to ray (direction in the normalized image plane, |.| = tan θ)
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        if params.k[1] == 0.0 { return Some(point); }
        let r_norm = (point.0 * point.0 + point.1 * point.1).sqrt();
        if r_norm < 1e-9 { return Some(point); }
        let p = r_norm / params.k[1];
        let theta = Self::poly_eval(p, &params.k);
        // tan() wraps/flips sign at θ≥90° (the over-FOV concentric-ring fold): the wrapped rays come out
        // small, slip under r_limit in rotate_and_distort, and get sampled. Clamp the angle just under 90°
        // and continue the radius linearly (C1) past it so over-FOV rays stay large & monotonic — r_limit
        // (= tan(ZFOV/2)) then clips them to background. distort_point uses the exact inverse continuation.
        const TMAX: f32 = 1.5533; // ~89°, just under tan()'s 90° asymptote
        let tt = TMAX.tan();
        let rr = if theta < TMAX { theta.tan() } else { tt + (theta - TMAX) * (1.0 + tt * tt) };
        let scale = rr / r_norm;
        Some((point.0 * scale, point.1 * scale))
    }

    /// `(x, y, z)` is the ray; returns normalized coord (× f + c → image pixel).
    /// From ray to image.
    pub fn distort_point(&self, x: f32, y: f32, z: f32, params: &KernelParams) -> (f32, f32) {
        let pos = (x / z, y / z);
        if params.k[1] == 0.0 { return pos; }
        let r = (pos.0 * pos.0 + pos.1 * pos.1).sqrt();
        // Inverse of undistort_point's angle clamp: past tan(89°) recover θ from the linear continuation
        // instead of atan() (which saturates at 90° and would fold every over-FOV ray back onto the frame).
        const TMAX: f32 = 1.5533; // ~89°
        let tt = TMAX.tan();
        let theta = if r < tt { r.atan() } else { TMAX + (r - tt) / (1.0 + tt * tt) };
        let p = Self::poly_invert(theta, &params.k);
        let r_norm = params.k[1] * p;
        let scale = if r < 1e-9 { 1.0 } else { r_norm / r };
        (pos.0 * scale, pos.1 * scale)
    }

    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    pub fn distortion_derivative(&self, theta: f64, k: &[f64]) -> Option<f64> {
        // d(r_norm)/dθ where r_norm = k1·p and θ = POLY(p): sign tracks POLY'(p). The POLY
        // radial map generally doesn't fold within [0, π/2], so this usually yields no limit
        // — the FOV clamp (r_limit) is instead baked as tan(ZFOV/2) by telemetry-parser.
        if k.len() < 2 || k[1] == 0.0 { return None; }
        let eval  = |p: f64| -> f64 { let mut acc = 0.0; let mut pw = 1.0; for i in 0..k.len() { acc += k[i] * pw; pw *= p; } acc };
        let deriv = |p: f64| -> f64 { let mut acc = 0.0; let mut pw = 1.0; for i in 1..k.len() { acc += (i as f64) * k[i] * pw; pw *= p; } acc };
        let mut p = (theta - k[0]) / k[1];
        for _ in 0..10 {
            let d = deriv(p);
            if d.abs() < 1e-12 { break; }
            let fix = (eval(p) - theta) / d;
            p -= fix;
            if fix.abs() < 1e-9 { break; }
        }
        Some(k[1] * deriv(p))
    }

    pub fn id() -> &'static str { "gopro" }
    pub fn name() -> &'static str { "GoPro" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("gopro.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("gopro.wgsl") }
}
