// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use crate::stabilization::KernelParams;

#[derive(Default, Clone)]
pub struct Insta360 { }

impl Insta360 {
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let mut px = point.0;
        let mut py = point.1;

        for _ in 0..200 {
            let dp = self.distort_point(px, py, 1.0, params);
            px -= dp.0 - point.0;
            py -= dp.1 - point.1;
        }

        Some((px, py))
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

    pub fn id() -> &'static str { "insta360" }
    pub fn name() -> &'static str { "Insta360" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("insta360.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("insta360.wgsl") }
}
