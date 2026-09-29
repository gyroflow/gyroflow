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
    #[inline] fn poly_eval(p: f32, k: &[f32; 24]) -> f32 {
        k[0] + p * (k[1] + p * (k[2] + p * (k[3] + p * (k[4] + p * (k[5] + p * k[6])))))
    }
    #[inline] fn poly_deriv(p: f32, k: &[f32; 24]) -> f32 {
        k[1] + p * (2.0 * k[2] + p * (3.0 * k[3] + p * (4.0 * k[4] + p * (5.0 * k[5] + p * (6.0 * k[6])))))
    }
    // Solve POLY(p) = theta for p (angle -> normalized radius parameter).
    #[inline] fn poly_invert(theta: f32, k: &[f32; 24]) -> f32 {
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
    /// From image to ray, as an angle vector (`θ·û`, see `stabilization::projection`)
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        if params.k[1] == 0.0 {
            let r = (point.0 * point.0 + point.1 * point.1).sqrt();
            if r < 1e-12 { return Some(point); }
            let s = r.atan() / r;
            return Some((point.0 * s, point.1 * s));
        }
        let r_norm = (point.0 * point.0 + point.1 * point.1).sqrt();
        if r_norm < 1e-9 { return Some(point); }
        let p = r_norm / params.k[1];
        let theta = Self::poly_eval(p, &params.k);
        // The POLY is a fit over the camera's own frame, and the zoom search and the correction blend both
        // read it past that. Outside its fit range it is free to run negative - a degree-6 polynomial with
        // a negative leading term always does eventually - or past 180°, and neither is an angle this
        // pipeline can carry: a negative `theta` flips `scale`, so the ray comes back pointing at the
        // opposite side of the frame and `undistort_coord` takes *that* azimuth for its tangential
        // correction, and past 180° `ray_to_dir`'s `sin θ` has turned over and the ray arrives from behind
        // the camera. There is no image of this point; `Sony::undistort_point` ends the same way
        if !(theta > 0.0 && theta < std::f32::consts::PI) { return None; }
        let scale = theta / r_norm;
        Some((point.0 * scale, point.1 * scale))
    }

    /// `(x, y, z)` is the ray; returns normalized coord (× f + c → image pixel).
    /// From ray to image.
    pub fn distort_point(&self, x: f32, y: f32, z: f32, params: &KernelParams) -> (f32, f32) {
        // No POLY block: a pinhole, which has no image of a ray at or past 90°
        if params.k[1] == 0.0 { return if z > 1e-9 { (x / z, y / z) } else { (x * 1e9, y * 1e9) }; }
        let r = (x * x + y * y).sqrt();
        if r < 1e-12 { return (0.0, 0.0); }
        // atan2 against the ray's own z: a GoPro's 150°+ field needs the angle, not a z=1 plane radius
        let theta = r.atan2(z);
        let p = Self::poly_invert(theta, &params.k);
        let r_norm = params.k[1] * p;
        let scale = r_norm / r;
        (x * scale, y * scale)
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
