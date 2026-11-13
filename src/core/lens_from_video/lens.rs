// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Anton Belkin

//! The lens the bundle adjustment fits: the OpenCV fisheye model as `distortion_models::OpenCVFisheye` draws it (fx,
//! fy, cx, cy, k1..k4), in f64, with the refraction of a flat port under water in front of it the way the render
//! applies `light_refraction_coefficient`: the parameters stay those of the lens in air.

use nalgebra::{ Matrix3, Rotation3, Vector3 };
use crate::LensProfile;

/// A distortion-free lens written in the fisheye model: tan θ = θ (1 + θ²/3 + 2θ⁴/15 + 17θ⁶/315 + 62θ⁸/2835)
pub const PINHOLE_K: [f64; 4] = [1.0 / 3.0, 2.0 / 15.0, 17.0 / 315.0, 62.0 / 2835.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub k: [f64; 4],
    /// Refraction index of what's in front of the flat port: 1 in air, 1.33 under water. A ray at angle θ to the axis
    /// in the water enters the housing at asin(n sin θ)
    pub refraction: f64,
}

impl Lens {
    pub fn new(fx: f64, fy: f64, cx: f64, cy: f64, k: [f64; 4]) -> Self { Self { fx, fy, cx, cy, k, refraction: 1.0 } }

    pub fn with_refraction(mut self, n: f64) -> Self { self.refraction = if n > 0.0 { n } else { 1.0 }; self }

    /// The lens of a profile, in the frame it was calibrated for (`calib_dimension`), as the render reads it: only
    /// profiles in the OpenCV fisheye model, with no digital lens or stretch on top, are lenses this module works with
    pub fn from_profile(profile: &LensProfile) -> Option<Self> {
        if profile.digital_lens.is_some() || profile.distortion_model.as_deref().is_some_and(|x| x != "opencv_fisheye") { return None; }
        if profile.calib_dimension.w == 0 || profile.calib_dimension.h == 0 || profile.has_interpolations() { return None; }
        if (profile.input_horizontal_stretch > 0.01 && profile.input_horizontal_stretch != 1.0) || (profile.input_vertical_stretch > 0.01 && profile.input_vertical_stretch != 1.0) { return None; }
        let d = &profile.fisheye_params.distortion_coeffs;
        if d.len() != 4 || profile.fisheye_params.camera_matrix.len() != 3 { return None; }
        let m = profile.get_camera_matrix((profile.calib_dimension.w, profile.calib_dimension.h));
        // Without coefficients the render draws a pinhole (`OpenCVFisheye::distort_point`)
        let k = if d.iter().all(|x| *x == 0.0) { PINHOLE_K } else { [d[0], d[1], d[2], d[3]] };
        let lens = Self::new(m[(0, 0)], m[(1, 1)], m[(0, 2)], m[(1, 2)], k);
        lens.is_finite().then_some(lens)
    }

    pub fn is_finite(&self) -> bool {
        self.fx.is_finite() && self.fy.is_finite() && self.cx.is_finite() && self.cy.is_finite() && self.k.iter().all(|x| x.is_finite()) && self.fx > 0.0 && self.fy > 0.0
    }

    pub fn camera_matrix(&self) -> Matrix3<f64> {
        Matrix3::new(self.fx, 0.0, self.cx,  0.0, self.fy, self.cy,  0.0, 0.0, 1.0)
    }

    /// The same lens for the frame scaled by `s`
    pub fn scaled(&self, s: f64) -> Self { Self { fx: self.fx * s, fy: self.fy * s, cx: self.cx * s, cy: self.cy * s, ..*self } }

    fn radius(&self, t: f64) -> f64 {
        let t2 = t * t;
        t * (1.0 + t2 * (self.k[0] + t2 * (self.k[1] + t2 * (self.k[2] + t2 * self.k[3]))))
    }

    /// Point in the camera's space → pixel
    #[inline]
    pub fn project(&self, p: &Vector3<f64>) -> [f64; 2] {
        let r = (p.x * p.x + p.y * p.y).sqrt() + 1e-12;
        let mut t = r.atan2(p.z);
        if self.refraction != 1.0 { t = (self.refraction * t.sin()).clamp(-1.0, 1.0).asin(); }
        let td = self.radius(t);
        [self.fx * td * p.x / r + self.cx, self.fy * td * p.y / r + self.cy]
    }

    /// Pixel → unit direction in the camera's space (in the water, with a refraction index)
    pub fn unproject(&self, uv: [f64; 2]) -> Vector3<f64> {
        let x = (uv[0] - self.cx) / self.fx;
        let y = (uv[1] - self.cy) / self.fy;
        let td = (x * x + y * y).sqrt() + 1e-12;
        let k = &self.k;
        let mut t = td;
        for _ in 0..10 {
            let t2 = t * t;
            let f = self.radius(t) - td;
            let df = 1.0 + t2 * (3.0 * k[0] + t2 * (5.0 * k[1] + t2 * (7.0 * k[2] + t2 * 9.0 * k[3])));
            if df.abs() < 1e-9 { break; }
            let next = t - f / df;
            // A coefficient curve that folds can send the Newton off; the angle of the equidistant lens is then as
            // good as it gets, and a NaN here would spread through every fit this bearing takes part in
            if !next.is_finite() { t = td; break; }
            t = next;
        }
        // In the water the ray is at `asin(sin t / n)` to the axis; `t` past 90° has no ray behind a flat port
        if self.refraction != 1.0 { t = (t.sin() / self.refraction).clamp(-1.0, 1.0).asin(); }
        let s = t.sin() / td;
        Vector3::new(x * s, y * s, t.cos())
    }

    /// How far apart two lenses draw the same rays, in pixels over a grid of the frame: the median, the largest, and
    /// the largest over the corners of the frame (the outer fifth of its diagonal), where a wrong lens shows first
    pub fn shift(&self, other: &Lens, size: (usize, usize)) -> LensShift {
        Self::shift_with(self, other, size, &Rotation3::identity())
    }

    /// The same after the best common tilt of the camera: a small rotation of the camera looks like a move of the
    /// lens' centre, and no scene tells the two apart
    pub fn shift_after_tilt(&self, other: &Lens, size: (usize, usize)) -> LensShift {
        // Only the points both lenses have a ray for: one that folds somewhere would otherwise make the whole cost
        // not a number, and the fit would give up at no tilt at all
        let (grid, bearings): (Vec<[f64; 2]>, Vec<Vector3<f64>>) = grid(size, 80).into_iter()
            .map(|g| (g, self.unproject(g)))
            .filter(|(g, b)| { let p = other.project(b); b.iter().all(|x| x.is_finite()) && p[0].is_finite() && p[1].is_finite() && g[0].is_finite() })
            .unzip();
        if grid.len() < 8 { return self.shift(other, size); }
        let cost = |w: &Vector3<f64>| -> (Vec<f64>, f64) {
            let rot = Rotation3::new(*w);
            let r: Vec<f64> = bearings.iter().zip(&grid).flat_map(|(b, g)| { let p = other.project(&(rot * b)); [p[0] - g[0], p[1] - g[1]] }).collect();
            let c = if r.iter().all(|x| x.is_finite()) { r.iter().map(|x| x * x).sum() } else { f64::INFINITY };
            (r, c)
        };
        // Levenberg-Marquardt over the three angles, numeric Jacobian: damped, because one of the three is nearly a
        // null direction for a lens with little distortion, and an undamped step would be rejected and the tilt lost
        let mut w = Vector3::zeros();
        let (mut r, mut c) = cost(&w);
        if !c.is_finite() { return self.shift(other, size); }
        let mut lambda = 1e-6;
        for _ in 0..20 {
            let mut jtj = Matrix3::<f64>::zeros();
            let mut jtr = Vector3::<f64>::zeros();
            let cols: Vec<Vec<f64>> = (0..3).map(|i| {
                let mut d = w; d[i] += 1e-6;
                cost(&d).0.iter().zip(&r).map(|(a, b)| (a - b) / 1e-6).collect()
            }).collect();
            for n in 0..r.len() {
                for i in 0..3 {
                    jtr[i] += cols[i][n] * r[n];
                    for j in 0..3 { jtj[(i, j)] += cols[i][n] * cols[j][n]; }
                }
            }
            let mut stepped = false;
            while lambda < 1e12 {
                let mut h = jtj;
                for i in 0..3 { h[(i, i)] += lambda * jtj[(i, i)].max(1e-12) + 1e-18; }
                let Some(step) = h.try_inverse().map(|inv| inv * jtr) else { lambda *= 10.0; continue };
                let (r2, c2) = cost(&(w - step));
                if c2 < c {
                    stepped = c - c2 > 1e-9 * c;
                    w -= step; r = r2; c = c2; lambda = (lambda / 4.0).max(1e-9);
                    break;
                }
                lambda *= 10.0;
            }
            if !stepped { break; }
        }
        Self::shift_with(self, other, size, &Rotation3::new(w))
    }

    fn shift_with(&self, other: &Lens, size: (usize, usize), rot: &Rotation3<f64>) -> LensShift {
        let (cx, cy) = (size.0 as f64 / 2.0, size.1 as f64 / 2.0);
        let corner_from = 0.8 * (cx * cx + cy * cy).sqrt();
        let mut all = Vec::new();
        let mut corner: f64 = 0.0;
        for g in grid(size, 80) {
            let p = other.project(&(rot * self.unproject(g)));
            let d = ((p[0] - g[0]).powi(2) + (p[1] - g[1]).powi(2)).sqrt();
            if !d.is_finite() { continue; }
            if ((g[0] - cx).powi(2) + (g[1] - cy).powi(2)).sqrt() >= corner_from { corner = corner.max(d); }
            all.push(d);
        }
        if all.is_empty() { return LensShift { median: f64::INFINITY, max: f64::INFINITY, corner: f64::INFINITY }; }
        all.sort_by(|a, b| a.total_cmp(b));
        LensShift { median: all[all.len() / 2], max: *all.last().unwrap(), corner }
    }

    /// A Gyroflow lens profile of this lens (in air), for a video of this size
    pub fn to_profile(&self, size: (usize, usize)) -> LensProfile {
        use crate::lens_profile::{ CameraParams, Dimensions };
        let mut p = LensProfile::default();
        p.calib_dimension = Dimensions { w: size.0, h: size.1 };
        p.orig_dimension  = Dimensions { w: size.0, h: size.1 };
        p.input_horizontal_stretch = 1.0;
        p.input_vertical_stretch   = 1.0;
        // The centre found is the lens' own: without this the render puts it in the middle of the frame
        p.asymmetrical = true;
        p.fisheye_params = CameraParams {
            camera_matrix: vec![[self.fx, 0.0, self.cx], [0.0, self.fy, self.cy], [0.0, 0.0, 1.0]],
            distortion_coeffs: self.k.to_vec(),
            ..Default::default()
        };
        p
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct LensShift {
    pub median: f64,
    pub max: f64,
    pub corner: f64,
}

fn grid(size: (usize, usize), step: usize) -> Vec<[f64; 2]> {
    // Finer for a small frame: a few hundred points either way
    let step = step.min((size.0.max(size.1) / 24).max(1));
    (0..size.1).step_by(step).flat_map(|y| (0..size.0).step_by(step).map(move |x| [x as f64, y as f64])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stabilization::distortion_models::OpenCVFisheye;

    fn hero() -> Lens { Lens::new(1766.95, 1773.07, 1311.74, 774.68, [0.05352, -0.00625, 0.02390, -0.01311]) }

    #[test]
    fn unproject_inverts_project() {
        for n in [1.0, 1.33] {
            let lens = hero().with_refraction(n);
            for uv in [[20.0, 30.0], [1352.0, 760.0], [2690.0, 1500.0], [400.0, 1300.0]] {
                let p = lens.project(&(lens.unproject(uv) * 3.7));
                assert!((p[0] - uv[0]).abs() < 1e-6 && (p[1] - uv[1]).abs() < 1e-6, "{uv:?} -> {p:?} (n = {n})");
            }
        }
    }

    /// The model the render draws with, and its refraction: the lens found here is the one it gets
    #[test]
    fn as_the_render_draws() {
        let lens = hero();
        for ray in [Vector3::new(0.3, -0.2, 1.0), Vector3::new(-0.9, 0.5, 1.0), Vector3::new(0.05, 0.6, 1.0)] {
            let d = OpenCVFisheye::distort::<f64>(ray.x, ray.y, ray.z, &lens.k);
            let p = lens.project(&ray);
            assert!((p[0] - (d.0 * lens.fx + lens.cx)).abs() < 1e-6 && (p[1] - (d.1 * lens.fy + lens.cy)).abs() < 1e-6, "{ray:?}: {p:?} vs {d:?}");
        }
        // Under water: sin of the angle in the housing = n x sin of the angle in the water (`cpu_undistort`)
        let wet = lens.with_refraction(1.33);
        let ray = Vector3::new(0.4, 0.0, 1.0);
        let t_air = (1.33 * (0.4f64).atan().sin()).asin();
        let expected = lens.project(&Vector3::new(t_air.tan(), 0.0, 1.0));
        let p = wet.project(&ray);
        assert!((p[0] - expected[0]).abs() < 1e-6, "{p:?} vs {expected:?}");
    }

    #[test]
    fn shift_sees_a_wrong_focal_length() {
        let a = hero();
        let size = (2704, 1520);
        assert!(a.shift(&a, size).max < 1e-6);
        let mut b = a; b.fx *= 1.03; b.fy *= 1.03;
        let s = a.shift(&b, size);
        assert!(s.median > 15.0 && s.corner > s.median && s.max >= s.corner, "{s:?}");
        // A move of the centre is mostly a tilt of the camera
        let mut c = a; c.cx += 20.0;
        assert!(a.shift(&c, size).median > 19.0);
        assert!(a.shift_after_tilt(&c, size).median < 3.0);
    }
}
