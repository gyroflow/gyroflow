// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

// See `gpu_klt.rs`: pyramidal Lucas-Kanade the way OpenCV's calcOpticalFlowPyrLK runs it (21x21 window, Scharr
// derivatives, its exit rules), cornerMinEigenVal (Sobel 3, block 7) and the local maxima of that. Everything is in
// storage buffers, at offsets in f32s the host computes: each frame's pyramid (pyrDown, rounded to whole grey levels
// like OpenCV's 8-bit levels) and its derivatives, for two frames

struct Params {
    w: u32, h: u32,        // the level written
    sw: u32, sh: u32,      // the level read (pyr_down)
    src: u32, dst: u32,    // offsets in `pyr` read and written; in `work`: min_eig's output, what `corners` reads
    stride: u32, aux: u32, // bytes per row of `raw` (to_float), room in `cands` (corners); offset of the covariances
                           // in `work` (cov, min_eig), of the noise samples (corners)
}

struct Candidates {
    count: atomic<u32>,
    pad: u32,
    // x | y << 16, the strength's bits
    items: array<vec2<u32>>,
}

struct LkParams {
    n_points: u32, n_levels: u32, max_iters: u32, pad0: u32,
    eps2: f32, min_eig: f32, pad1: f32, pad2: f32,
    // Per level: x the image of I, y its derivatives, z the image of J, w width << 16 | height
    lv: array<vec4<u32>, 8>,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read_write> pyr: array<f32>;
@group(0) @binding(2) var<storage, read> pyr_ro: array<f32>;
@group(0) @binding(3) var<storage, read> raw: array<u32>;
@group(0) @binding(4) var<uniform> L: LkParams;
@group(0) @binding(5) var<storage, read> pts_in: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> pts_out: array<vec4<f32>>;
@group(0) @binding(7) var<storage, read_write> work: array<f32>;
@group(0) @binding(8) var<storage, read_write> cands: Candidates;

// OpenCV's BORDER_REFLECT_101: gfedcb|abcdefgh|gfedcba
fn reflect101(p: i32, n: i32) -> i32 {
    if (n == 1) { return 0; }
    var q = p;
    if (q < 0) { q = -q; }
    if (q >= n) { q = 2 * n - 2 - q; }
    return clamp(q, 0, n - 1);
}

fn at(off: u32, w: i32, x: i32, y: i32) -> u32 { return off + u32(y * w + x); }

// [1 4 6 4 1] at -2..2
fn k5(d: i32) -> f32 { return select(select(6.0, 4.0, d * d == 1), 1.0, d * d == 4); }

// The 8-bit frame into level 0
@compute @workgroup_size(8, 8, 1)
fn to_float(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.w || id.y >= P.h) { return; }
    let b = id.y * P.stride + id.x;
    pyr[P.dst + id.y * P.w + id.x] = f32((raw[b >> 2u] >> ((b & 3u) * 8u)) & 0xffu);
}

// pyrDown: [1 4 6 4 1]^2 / 256 at every second pixel, rounded as the 8-bit one rounds
@compute @workgroup_size(8, 8, 1)
fn pyr_down(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.w || id.y >= P.h) { return; }
    let sw = i32(P.sw);
    var sum = 0.0;
    for (var j = -2; j <= 2; j++) {
        let y = reflect101(2 * i32(id.y) + j, i32(P.sh));
        var row = 0.0;
        for (var i = -2; i <= 2; i++) {
            row += k5(i) * pyr[at(P.src, sw, reflect101(2 * i32(id.x) + i, sw), y)];
        }
        sum += k5(j) * row;
    }
    pyr[P.dst + id.y * P.w + id.x] = floor((sum + 128.0) / 256.0);
}

// calcSharrDeriv: dI/dx, dI/dy with the 3/10/3 kernel, unscaled (32x the gradient, as OpenCV's int16 ones)
@compute @workgroup_size(8, 8, 1)
fn scharr(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.w || id.y >= P.h) { return; }
    let w = i32(P.w);
    let y = i32(id.y);
    let ym = reflect101(y - 1, i32(P.h));
    let yp = reflect101(y + 1, i32(P.h));
    var t0: array<f32, 3>;
    var t1: array<f32, 3>;
    for (var k = 0; k < 3; k++) {
        let x = reflect101(i32(id.x) + k - 1, w);
        let a = pyr[at(P.src, w, x, ym)];
        let b = pyr[at(P.src, w, x, y)];
        let c = pyr[at(P.src, w, x, yp)];
        t0[k] = (a + c) * 3.0 + b * 10.0;
        t1[k] = c - a;
    }
    let o = P.dst + 2u * (id.y * P.w + id.x);
    pyr[o] = t0[2] - t0[0];
    pyr[o + 1u] = (t1[2] + t1[0]) * 3.0 + t1[1] * 10.0;
}

// cornerMinEigenVal, 1/2: Sobel 3 derivatives, scaled as OpenCV scales them for an 8-bit image and block 7, and their
// products
@compute @workgroup_size(8, 8, 1)
fn cov(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.w || id.y >= P.h) { return; }
    let w = i32(P.w);
    let x = i32(id.x);
    let y = i32(id.y);
    let xm = reflect101(x - 1, w);
    let xp = reflect101(x + 1, w);
    let ym = reflect101(y - 1, i32(P.h));
    let yp = reflect101(y + 1, i32(P.h));
    let a = pyr[at(P.src, w, xm, ym)]; let b = pyr[at(P.src, w, x, ym)]; let c = pyr[at(P.src, w, xp, ym)];
    let d = pyr[at(P.src, w, xm, y)];                                    let f = pyr[at(P.src, w, xp, y)];
    let g = pyr[at(P.src, w, xm, yp)]; let e = pyr[at(P.src, w, x, yp)]; let k = pyr[at(P.src, w, xp, yp)];
    let s = 1.0 / (4.0 * 7.0 * 255.0);
    let dx = ((c + 2.0 * f + k) - (a + 2.0 * d + g)) * s;
    let dy = ((g + 2.0 * e + k) - (a + 2.0 * b + c)) * s;
    let o = P.aux + 3u * (id.y * P.w + id.x);
    work[o] = dx * dx;
    work[o + 1u] = dx * dy;
    work[o + 2u] = dy * dy;
}

// cornerMinEigenVal, 2/2: the products summed over 7x7, and the smaller eigenvalue of that
@compute @workgroup_size(8, 8, 1)
fn min_eig(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.w || id.y >= P.h) { return; }
    let w = i32(P.w);
    var s = vec3<f32>(0.0);
    for (var j = -3; j <= 3; j++) {
        let y = reflect101(i32(id.y) + j, i32(P.h));
        for (var i = -3; i <= 3; i++) {
            let o = P.aux + 3u * u32(y * w + reflect101(i32(id.x) + i, w));
            s += vec3<f32>(work[o], work[o + 1u], work[o + 2u]);
        }
    }
    let a = s.x * 0.5;
    let b = s.y;
    let c = s.z * 0.5;
    work[P.dst + id.y * P.w + id.x] = (a + c) - sqrt((a - c) * (a - c) + b * b);
}

// What good_features_to_track picks from: the pixels inside the outermost ones whose strength is above zero and none
// of its 8 neighbours', in no order (the host sorts them); and every 8th pixel's strength in both directions, the
// noise of flat areas is a percentile of
@compute @workgroup_size(8, 8, 1)
fn corners(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.w || id.y >= P.h) { return; }
    let w = i32(P.w);
    let x = i32(id.x);
    let y = i32(id.y);
    let v = work[at(P.dst, w, x, y)];
    if ((id.x & 7u) == 0u && (id.y & 7u) == 0u) {
        work[P.aux + (id.y >> 3u) * ((P.w + 7u) >> 3u) + (id.x >> 3u)] = v;
    }
    if (x == 0 || y == 0 || x == w - 1 || y == i32(P.h) - 1 || v <= 0.0) { return; }
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            if (work[at(P.dst, w, x + i, y + j)] > v) { return; }
        }
    }
    let n = atomicAdd(&cands.count, 1u);
    if (n < P.stride) { cands.items[n] = vec2<u32>(id.x | (id.y << 16u), bitcast<u32>(v)); }
}

// ── Lucas-Kanade: one workgroup of 64 per point, each thread 7 of the window's 441 pixels ──────────────────────

const WIN: i32 = 21;
const HALF_WIN: f32 = 10.0;
const NPIX: u32 = 441u;
const PER_THREAD: u32 = 7u;
const FLT_SCALE: f32 = 1.0 / 1048576.0;

var<workgroup> wg_terms: array<vec4<f32>, 64>;

// The workgroup's sum, to every thread, uniform (so a decision on it keeps the barriers in uniform control flow)
fn wg_sum(lidx: u32, v: vec4<f32>) -> vec4<f32> {
    wg_terms[lidx] = v;
    workgroupBarrier();
    for (var s = 32u; s > 0u; s = s >> 1u) {
        if (lidx < s) { wg_terms[lidx] = wg_terms[lidx] + wg_terms[lidx + s]; }
        workgroupBarrier();
    }
    return workgroupUniformLoad(&wg_terms[0]);
}

// Window pixels outside the image: the image reflected, the derivatives zero, as OpenCV pads them
fn img_at(off: u32, w: i32, h: i32, x: i32, y: i32) -> f32 {
    return pyr_ro[at(off, w, reflect101(x, w), reflect101(y, h))];
}
fn grad_at(off: u32, w: i32, h: i32, x: i32, y: i32) -> vec2<f32> {
    if (x < 0 || y < 0 || x >= w || y >= h) { return vec2<f32>(0.0); }
    let o = off + 2u * u32(y * w + x);
    return vec2<f32>(pyr_ro[o], pyr_ro[o + 1u]);
}
fn img_bilinear(off: u32, w: i32, h: i32, x: i32, y: i32, k: vec4<f32>) -> f32 {
    return img_at(off, w, h, x, y) * k.x + img_at(off, w, h, x + 1, y) * k.y + img_at(off, w, h, x, y + 1) * k.z + img_at(off, w, h, x + 1, y + 1) * k.w;
}
fn bilinear_weights(f: vec2<f32>) -> vec4<f32> {
    return vec4<f32>((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
}

// pts_in: x, y, status (0 skips it); pts_out: x, y, status, iterations over all levels
@compute @workgroup_size(64, 1, 1)
fn lk(@builtin(local_invocation_index) lidx: u32, @builtin(workgroup_id) wid: vec3<u32>) {
    let pid = wid.x;
    if (pid >= L.n_points) { return; }
    let start = pts_in[pid];
    if (start.z < 0.5) {
        if (lidx == 0u) { pts_out[pid] = vec4<f32>(start.xy, 0.0, 0.0); }
        return;
    }
    var status = true;
    var next = vec2<f32>(0.0);
    var iters = 0u;
    var t_img: array<f32, 7>;
    var t_grad: array<vec2<f32>, 7>;
    for (var li = 0u; li < L.n_levels; li++) {
        let level = L.n_levels - 1u - li;
        let lv = L.lv[level];
        let w = i32(lv.w >> 16u);
        let h = i32(lv.w & 0xffffu);
        let scale = 1.0 / f32(1u << level);
        // OpenCV's nextPts: where the point is, at this level; the window's corner is it less half the window
        if (li == 0u) { next = start.xy * scale; } else { next = next * 2.0; }
        let prev_pt = start.xy * scale - vec2<f32>(HALF_WIN);
        let ip = vec2<i32>(floor(prev_pt));
        if (ip.x < -WIN || ip.x >= w || ip.y < -WIN || ip.y >= h) {
            if (level == 0u) { status = false; }
            continue;
        }
        let kp = bilinear_weights(prev_pt - vec2<f32>(ip));
        var acc = vec4<f32>(0.0);
        for (var k = 0u; k < PER_THREAD; k++) {
            let i = lidx + k * 64u;
            t_img[k] = 0.0;
            t_grad[k] = vec2<f32>(0.0);
            if (i < NPIX) {
                let x = ip.x + i32(i % 21u);
                let y = ip.y + i32(i / 21u);
                t_img[k] = img_bilinear(lv.x, w, h, x, y, kp);
                let g = grad_at(lv.y, w, h, x, y) * kp.x + grad_at(lv.y, w, h, x + 1, y) * kp.y + grad_at(lv.y, w, h, x, y + 1) * kp.z + grad_at(lv.y, w, h, x + 1, y + 1) * kp.w;
                t_grad[k] = g;
                acc += vec4<f32>(g.x * g.x, g.x * g.y, g.y * g.y, 0.0);
            }
        }
        let a = wg_sum(lidx, acc) * FLT_SCALE;
        let det = a.x * a.z - a.y * a.y;
        let min_eig = (a.z + a.x - sqrt((a.x - a.z) * (a.x - a.z) + 4.0 * a.y * a.y)) / (2.0 * f32(NPIX));
        if (min_eig < L.min_eig || det < 1.1920929e-7) {
            if (level == 0u) { status = false; }
            continue;
        }
        let inv_det = 1.0 / det;
        var nt = next - vec2<f32>(HALF_WIN);
        var prev_delta = vec2<f32>(0.0);
        for (var j = 0u; j < L.max_iters; j++) {
            let inx = vec2<i32>(floor(nt));
            if (inx.x < -WIN || inx.x >= w || inx.y < -WIN || inx.y >= h) {
                if (level == 0u) { status = false; }
                break;
            }
            let kn = bilinear_weights(nt - vec2<f32>(inx));
            var b = vec4<f32>(0.0);
            for (var k = 0u; k < PER_THREAD; k++) {
                let i = lidx + k * 64u;
                if (i < NPIX) {
                    let diff = (img_bilinear(lv.z, w, h, inx.x + i32(i % 21u), inx.y + i32(i / 21u), kn) - t_img[k]) * 32.0;
                    b += vec4<f32>(diff * t_grad[k].x, diff * t_grad[k].y, 0.0, 0.0);
                }
            }
            let s = wg_sum(lidx, b) * FLT_SCALE;
            let delta = vec2<f32>((a.y * s.y - a.z * s.x) * inv_det, (a.y * s.x - a.x * s.y) * inv_det);
            nt = nt + delta;
            next = nt + vec2<f32>(HALF_WIN);
            iters += 1u;
            if (dot(delta, delta) <= L.eps2) { break; }
            if (j > 0u && abs(delta.x + prev_delta.x) < 0.01 && abs(delta.y + prev_delta.y) < 0.01) {
                next = next - delta * 0.5;
                break;
            }
            prev_delta = delta;
        }
    }
    if (lidx == 0u) { pts_out[pid] = vec4<f32>(next, select(0.0, 1.0, status), f32(iters)); }
}
