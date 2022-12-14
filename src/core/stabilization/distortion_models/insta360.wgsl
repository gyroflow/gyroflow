// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

fn distort_point(px: f32, py: f32, pz: f32) -> vec2<f32> {
    let k1 = params.k1.x;
    let k2 = params.k1.y;
    let k3 = params.k1.z;
    let p1 = params.k1.w;

    let p2 = params.k2.x;
    let xi = params.k2.y;

    var p = vec3<f32>(px, py, pz);
    p /= length(p);

    let x = p.x / (p.z + xi);
    let y = p.y / (p.z + xi);

    let r2 = x*x + y*y;
    let r4 = r2 * r2;
    let r6 = r4 * r2;

    return vec2<f32>(
        x * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p1*x*y + p2*(r2 + 2.0*x*x),
        y * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p2*x*y + p1*(r2 + 2.0*y*y)
    );
}

fn undistort_point(p: vec2<f32>) -> vec2<f32> {
    var pp = p;

    for (var i: i32 = 0; i < 200; i = i + 1) {
        pp -= distort_point(pp.x, pp.y, 1.0) - p;
    }

    return pp;
}
