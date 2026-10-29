// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

// See `gpu_klt.rs`: pyramidal Lucas-Kanade the way OpenCV's calcOpticalFlowPyrLK runs it (21x21 window, Scharr
// derivatives, its exit rules), cornerMinEigenVal (Sobel 3, block 7) and the local maxima of that. Each frame's pyramid
// (pyrDown, rounded to whole grey levels like OpenCV's 8-bit levels) and its derivatives are the mip levels of two
// textures, for two frames: LK reads them through the texture cache (Adreno caches only textures, and LK on storage
// buffers took 1.8 times as long). Level 0 is made from the 8-bit frame with its corner strengths and their local
// maxima in 4 passes, each level above from the one before in one, in tiles of 16 x 16

struct Params {
    w: u32, h: u32,        // the level written
    sw: u32, sh: u32,      // the level read (`down`)
    stride: u32,           // bytes per row of `raw` (`to_img`)
    cap: u32,              // room in `cands`
    eig_at: u32,           // in `work`, in f32s: the corner strengths and the noise samples
    noise_at: u32,
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
    // Per level: its width and height
    lv: array<vec4<u32>, 8>,
}

// <formats> The levels' images and derivatives as f32s, or (`gpu_klt.rs` swaps them in where the GPU can write those)
// r8uint and rg16sint: the images are whole grey levels and the derivatives whole numbers within +-4080, as exact in a
// quarter and half the bytes, and LK reads more of them from the cache
alias ImageIn = texture_2d<f32>;
alias ImageOut = texture_storage_2d<r32float, write>;
alias GradOut = texture_storage_2d<rg32float, write>;
fn img_load(t: ImageIn, p: vec2<i32>, lvl: i32) -> f32 { return textureLoad(t, p, lvl).x; }
fn img_store(p: vec2<i32>, v: f32) { textureStore(dst_img, p, vec4<f32>(v, 0.0, 0.0, 0.0)); }
fn grad_store(p: vec2<i32>, v: vec2<f32>) { textureStore(dst_grad, p, vec4<f32>(v, 0.0, 0.0)); }
// ... the derivatives as LK reads them
alias GradIn = texture_2d<f32>;
fn grad_load(t: GradIn, p: vec2<i32>, lvl: i32) -> vec2<f32> { return textureLoad(t, p, lvl).xy; }
// </formats>

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> raw: array<u32>;
@group(0) @binding(2) var src_img: ImageIn;
@group(0) @binding(3) var dst_img: ImageOut;
@group(0) @binding(4) var dst_grad: GradOut;
@group(0) @binding(5) var<storage, read_write> work: array<f32>;
@group(0) @binding(6) var<storage, read_write> cands: Candidates;
@group(0) @binding(7) var<uniform> L: LkParams;
@group(0) @binding(8) var<storage, read> pts_in: array<vec4<f32>>;
@group(0) @binding(9) var<storage, read_write> pts_out: array<vec4<f32>>;
// LK's frames: I's image and derivatives, J's image, all their levels
@group(0) @binding(10) var img_i: ImageIn;
@group(0) @binding(11) var grad_i: GradIn;
@group(0) @binding(12) var img_j: ImageIn;

// OpenCV's BORDER_REFLECT_101: gfedcb|abcdefgh|gfedcba
fn reflect101(p: i32, n: i32) -> i32 {
    if (n == 1) { return 0; }
    var q = p;
    if (q < 0) { q = -q; }
    if (q >= n) { q = 2 * n - 2 - q; }
    return clamp(q, 0, n - 1);
}

// [1 4 6 4 1] at -2..2
fn k5(d: i32) -> f32 { return select(select(6.0, 4.0, d * d == 1), 1.0, d * d == 4); }

// A pixel of the 8-bit frame
fn raw_px(x: i32, y: i32) -> f32 {
    let b = u32(y) * P.stride + u32(x);
    return f32((raw[b >> 2u] >> ((b & 3u) * 8u)) & 0xffu);
}

// calcSharrDeriv at a pixel from its 3 x 3 (a b c / d e f / g h k, reflected at the edge): dI/dx, dI/dy with the
// 3/10/3 kernel, unscaled (32x the gradient, as OpenCV's int16 ones)
fn scharr(a: f32, b: f32, c: f32, d: f32, f: f32, g: f32, h: f32, k: f32) -> vec2<f32> {
    let t00 = (a + g) * 3.0 + d * 10.0;
    let t02 = (c + k) * 3.0 + f * 10.0;
    return vec2<f32>(t02 - t00, ((k - c) + (g - a)) * 3.0 + (h - b) * 10.0);
}

// ── Level 0, its corner strengths and their local maxima ─────────────────────────────────────────────────────────
// A pass a step, through textures: one kernel with them all in tiles of workgroup memory (16 KB a workgroup of 256)
// took 43 ms on Adreno 650, where two of those fit at a time; these take ~1 ms together

@group(0) @binding(15) var rows_out: texture_storage_2d<rgba32float, write>;
@group(0) @binding(16) var rows_in: texture_2d<f32>;
@group(0) @binding(17) var eig_out: texture_storage_2d<r32float, write>;
@group(0) @binding(18) var eig_in: texture_2d<f32>;

// The 8-bit frame into level 0
@compute @workgroup_size(8, 8, 1)
fn to_img(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.w || id.y >= P.h) { return; }
    img_store(vec2<i32>(id.xy), raw_px(i32(id.x), i32(id.y)));
}

// Column x of level 0 at rows y - 1, y, y + 1, past the edge the image reflected (BORDER_REFLECT_101)
fn column(x: i32, y: i32, h: i32) -> vec3<f32> {
    return vec3<f32>(img_load(src_img, vec2<i32>(x, reflect101(y - 1, h)), 0), img_load(src_img, vec2<i32>(x, y), 0), img_load(src_img, vec2<i32>(x, reflect101(y + 1, h)), 0));
}
// cornerMinEigenVal's Sobel 3 products (scaled as OpenCV scales them for an 8-bit image and block 7) between the
// columns left of, at and right of a pixel: xx, xy, yy
fn products(l: vec3<f32>, m: vec3<f32>, r: vec3<f32>) -> vec3<f32> {
    let s = 1.0 / (4.0 * 7.0 * 255.0);
    let dx = ((r.x + 2.0 * r.y + r.z) - (l.x + 2.0 * l.y + l.z)) * s;
    let dy = ((l.z + 2.0 * m.z + r.z) - (l.x + 2.0 * m.x + r.x)) * s;
    return vec3<f32>(dx * dx, dx * dy, dy * dy);
}
// ... at column x (in the frame) of row y
fn products_at(x: i32, y: i32, w: i32, h: i32) -> vec3<f32> {
    return products(column(reflect101(x - 1, w), y, h), column(x, y, h), column(reflect101(x + 1, w), y, h));
}

// Level 0's derivatives, and the products' 7 x 7 sums along the rows: past the edge the products of the position
// reflected to. The products are made here and not kept: a texture of them was 16 bytes a pixel more (and the pass
// writing it), at the sync's processing resolution up to the whole frame. Inside the frame from the 3 x 9 around the
// pixel, 27 loads; at its edge each product from its own 3 x 3, reflected
@compute @workgroup_size(8, 8, 1)
fn box_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.w || id.y >= P.h) { return; }
    let w = i32(P.w);
    let h = i32(P.h);
    let x = i32(id.x);
    let y = i32(id.y);
    var s = vec3<f32>(0.0);
    var at: array<vec3<f32>, 3>;
    if (x >= 4 && x + 4 < w) {
        let c0 = column(x - 4, y, h); let c1 = column(x - 3, y, h); let c2 = column(x - 2, y, h);
        let c3 = column(x - 1, y, h); let c4 = column(x, y, h);     let c5 = column(x + 1, y, h);
        let c6 = column(x + 2, y, h); let c7 = column(x + 3, y, h); let c8 = column(x + 4, y, h);
        s += products(c0, c1, c2); s += products(c1, c2, c3); s += products(c2, c3, c4); s += products(c3, c4, c5);
        s += products(c4, c5, c6); s += products(c5, c6, c7); s += products(c6, c7, c8);
        at = array<vec3<f32>, 3>(c3, c4, c5);
    } else {
        for (var i = -3; i <= 3; i++) { s += products_at(reflect101(x + i, w), y, w, h); }
        at = array<vec3<f32>, 3>(column(reflect101(x - 1, w), y, h), column(x, y, h), column(reflect101(x + 1, w), y, h));
    }
    let l = at[0];
    let m = at[1];
    let r = at[2];
    grad_store(vec2<i32>(x, y), scharr(l.x, m.x, r.x, l.y, r.y, l.z, m.z, r.z));
    textureStore(rows_out, vec2<i32>(id.xy), vec4<f32>(s, 0.0));
}

// ... down the columns, and the smaller eigenvalue of that: the corner strength, also into `work`
@compute @workgroup_size(8, 8, 1)
fn box_cols(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.w || id.y >= P.h) { return; }
    let h = i32(P.h);
    var s = vec3<f32>(0.0);
    for (var j = -3; j <= 3; j++) { s += textureLoad(rows_in, vec2<i32>(i32(id.x), reflect101(i32(id.y) + j, h)), 0).xyz; }
    let a = s.x * 0.5;
    let c = s.z * 0.5;
    let v = (a + c) - sqrt((a - c) * (a - c) + s.y * s.y);
    textureStore(eig_out, vec2<i32>(id.xy), vec4<f32>(v, 0.0, 0.0, 0.0));
    work[P.eig_at + id.y * P.w + id.x] = v;
}

// What good_features_to_track picks from: the pixels inside the outermost ones whose strength is above zero and none
// of its 8 neighbours', in no order (the host sorts them); and every 8th pixel's strength in both directions, the
// noise of flat areas is a percentile of
@compute @workgroup_size(8, 8, 1)
fn nms(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.w || id.y >= P.h) { return; }
    let w = i32(P.w);
    let h = i32(P.h);
    let x = i32(id.x);
    let y = i32(id.y);
    let v = textureLoad(eig_in, vec2<i32>(x, y), 0).x;
    if ((id.x & 7u) == 0u && (id.y & 7u) == 0u) {
        work[P.noise_at + (id.y >> 3u) * ((P.w + 7u) >> 3u) + (id.x >> 3u)] = v;
    }
    if (x == 0 || y == 0 || x == w - 1 || y == h - 1 || v <= 0.0) { return; }
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            if (textureLoad(eig_in, vec2<i32>(x + i, y + j), 0).x > v) { return; }
        }
    }
    let n = atomicAdd(&cands.count, 1u);
    if (n < P.cap) { cands.items[n] = vec2<u32>(id.x | (id.y << 16u), bitcast<u32>(v)); }
}

// ── The levels above, and their derivatives ──────────────────────────────────────────────────────────────────────

// A workgroup takes T x T pixels
const T: i32 = 16;

var<workgroup> d_img: array<f32, 324>;

// pyrDown of the level above (`src_img`, sw x sh) for T x T pixels and one around them, reflected past the edge
// ([1 4 6 4 1]^2 / 256 at every second pixel, rounded as the 8-bit one rounds), and the derivatives of the T x T
@compute @workgroup_size(16, 16, 1)
fn down(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3<u32>) {
    let w = i32(P.w);
    let h = i32(P.h);
    let sw = i32(P.sw);
    let sh = i32(P.sh);
    let x0 = i32(wid.x) * T;
    let y0 = i32(wid.y) * T;
    for (var t = li; t < 324u; t += 256u) {
        let px = reflect101(x0 - 1 + i32(t % 18u), w);
        let py = reflect101(y0 - 1 + i32(t / 18u), h);
        var sum = 0.0;
        for (var j = -2; j <= 2; j++) {
            let y = reflect101(2 * py + j, sh);
            var row = 0.0;
            for (var i = -2; i <= 2; i++) {
                row += k5(i) * img_load(src_img, vec2<i32>(reflect101(2 * px + i, sw), y), 0);
            }
            sum += k5(j) * row;
        }
        d_img[t] = floor((sum + 128.0) / 256.0);
    }
    workgroupBarrier();
    let x = x0 + i32(lid.x);
    let y = y0 + i32(lid.y);
    if (x >= w || y >= h) { return; }
    let o = (i32(lid.y) + 1) * 18 + i32(lid.x) + 1;
    img_store(vec2<i32>(x, y), d_img[o]);
    let g = scharr(d_img[o - 19], d_img[o - 18], d_img[o - 17], d_img[o - 1], d_img[o + 1], d_img[o + 17], d_img[o + 18], d_img[o + 19]);
    grad_store(vec2<i32>(x, y), g);
}

// ── Lucas-Kanade: one workgroup of 64 per point. Thread t takes a run of 7 pixels in row t / 3 of the 21x21 window,
// from column 7 (t % 3) on, and thread 63 none: the bilinear taps of a run share their pixels, 16 loads for its 7. A
// run is in vectors (4 + 3), not arrays: Adreno's compiler keeps arrays indexed in a loop in memory ─────────────────

const WIN: i32 = 21;
const HALF_WIN: f32 = 10.0;
const NPIX: u32 = 441u;
const FLT_SCALE: f32 = 1.0 / 1048576.0;

// The sums over a workgroup, by its workgroup memory. Subgroup sums were 2.9 times as slow on Adreno 650 (where wgpu
// doesn't have them: no quad operations), and so were ones in workgroups of 128 (two waves)
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

// The iterations' sums, of two: through 8 partial sums, behind 2 barriers and not 8
var<workgroup> wg_two: array<vec2<f32>, 64>;
var<workgroup> wg_parts: array<vec2<f32>, 8>;

fn wg_sum2(lidx: u32, v: vec2<f32>) -> vec2<f32> {
    wg_two[lidx] = v;
    workgroupBarrier();
    if (lidx < 8u) {
        var s = wg_two[lidx * 8u];
        for (var k = 1u; k < 8u; k++) { s += wg_two[lidx * 8u + k]; }
        wg_parts[lidx] = s;
    }
    // A barrier, then the same 8 for all (uniform): `wg_two` is written again only after the next sum's barrier
    let q = workgroupUniformLoad(&wg_parts);
    return ((q[0] + q[1]) + (q[2] + q[3])) + ((q[4] + q[5]) + (q[6] + q[7]));
}

fn bilinear_weights(f: vec2<f32>) -> vec4<f32> {
    return vec4<f32>((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
}

// 8 pixels of a row, the taps of a run of 7
struct Row { lo: vec4<f32>, hi: vec4<f32> }
// A run of 7 values
struct Run { lo: vec4<f32>, hi: vec3<f32> }

// The 8 pixels of rows y and y + 1 from x on, of level `lvl` (w x h) of `t`. Past the edge the image reflected, as
// OpenCV pads it; `inside`: all of them in the image, nothing to reflect
fn img_rows(t: ImageIn, lvl: i32, w: i32, h: i32, x: i32, y: i32, inside: bool) -> array<Row, 2> {
    return array<Row, 2>(img_row(t, lvl, w, h, x, y, inside), img_row(t, lvl, w, h, x, y + 1, inside));
}
fn img_row(t: ImageIn, lvl: i32, w: i32, h: i32, x: i32, y: i32, inside: bool) -> Row {
    if (inside) {
        return Row(vec4<f32>(img_load(t, vec2<i32>(x, y), lvl), img_load(t, vec2<i32>(x + 1, y), lvl), img_load(t, vec2<i32>(x + 2, y), lvl), img_load(t, vec2<i32>(x + 3, y), lvl)),
                   vec4<f32>(img_load(t, vec2<i32>(x + 4, y), lvl), img_load(t, vec2<i32>(x + 5, y), lvl), img_load(t, vec2<i32>(x + 6, y), lvl), img_load(t, vec2<i32>(x + 7, y), lvl)));
    }
    let ry = reflect101(y, h);
    return Row(vec4<f32>(img_load(t, vec2<i32>(reflect101(x, w), ry), lvl), img_load(t, vec2<i32>(reflect101(x + 1, w), ry), lvl),
                         img_load(t, vec2<i32>(reflect101(x + 2, w), ry), lvl), img_load(t, vec2<i32>(reflect101(x + 3, w), ry), lvl)),
               vec4<f32>(img_load(t, vec2<i32>(reflect101(x + 4, w), ry), lvl), img_load(t, vec2<i32>(reflect101(x + 5, w), ry), lvl),
                         img_load(t, vec2<i32>(reflect101(x + 6, w), ry), lvl), img_load(t, vec2<i32>(reflect101(x + 7, w), ry), lvl)));
}
// ... of the derivatives, x and y apart: rows y (x, y), y + 1 (x, y). Past the edge zero, as OpenCV pads them
fn grad_rows(lvl: i32, w: i32, h: i32, x: i32, y: i32, inside: bool) -> array<Row, 4> {
    let a = grad_row(lvl, w, h, x, y, inside);
    let b = grad_row(lvl, w, h, x, y + 1, inside);
    return array<Row, 4>(a[0], a[1], b[0], b[1]);
}
fn grad_at(lvl: i32, w: i32, h: i32, x: i32, y: i32) -> vec2<f32> {
    if (x < 0 || y < 0 || x >= w || y >= h) { return vec2<f32>(0.0); }
    return grad_load(grad_i, vec2<i32>(x, y), lvl);
}
fn grad_row(lvl: i32, w: i32, h: i32, x: i32, y: i32, inside: bool) -> array<Row, 2> {
    var g0: vec2<f32>; var g1: vec2<f32>; var g2: vec2<f32>; var g3: vec2<f32>; var g4: vec2<f32>; var g5: vec2<f32>; var g6: vec2<f32>; var g7: vec2<f32>;
    if (inside) {
        g0 = grad_load(grad_i, vec2<i32>(x, y), lvl); g1 = grad_load(grad_i, vec2<i32>(x + 1, y), lvl);
        g2 = grad_load(grad_i, vec2<i32>(x + 2, y), lvl); g3 = grad_load(grad_i, vec2<i32>(x + 3, y), lvl);
        g4 = grad_load(grad_i, vec2<i32>(x + 4, y), lvl); g5 = grad_load(grad_i, vec2<i32>(x + 5, y), lvl);
        g6 = grad_load(grad_i, vec2<i32>(x + 6, y), lvl); g7 = grad_load(grad_i, vec2<i32>(x + 7, y), lvl);
    } else {
        g0 = grad_at(lvl, w, h, x, y); g1 = grad_at(lvl, w, h, x + 1, y); g2 = grad_at(lvl, w, h, x + 2, y); g3 = grad_at(lvl, w, h, x + 3, y);
        g4 = grad_at(lvl, w, h, x + 4, y); g5 = grad_at(lvl, w, h, x + 5, y); g6 = grad_at(lvl, w, h, x + 6, y); g7 = grad_at(lvl, w, h, x + 7, y);
    }
    return array<Row, 2>(Row(vec4<f32>(g0.x, g1.x, g2.x, g3.x), vec4<f32>(g4.x, g5.x, g6.x, g7.x)), Row(vec4<f32>(g0.y, g1.y, g2.y, g3.y), vec4<f32>(g4.y, g5.y, g6.y, g7.y)));
}
// The 7 values bilinear between rows a (y) and b (y + 1): each its pixel and the next, as a * k.x + a' * k.y + b * k.z + b' * k.w
fn bilinear_run(a: Row, b: Row, k: vec4<f32>) -> Run {
    return Run(a.lo * k.x + vec4<f32>(a.lo.yzw, a.hi.x) * k.y + b.lo * k.z + vec4<f32>(b.lo.yzw, b.hi.x) * k.w,
               a.hi.xyz * k.x + a.hi.yzw * k.y + b.hi.xyz * k.z + b.hi.yzw * k.w);
}
fn run_dot(a: Run, b: Run) -> f32 { return dot(a.lo, b.lo) + dot(a.hi, b.hi); }
fn run_inside(w: i32, h: i32, x: i32, y: i32) -> bool { return x >= 0 && y >= 0 && x + 7 < w && y + 1 < h; }

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
    // This thread's run, in the window: its row and its first column
    let has_run = lidx < 63u;
    let row = i32(lidx / 3u);
    let col = i32((lidx % 3u) * 7u);
    let zero = Run(vec4<f32>(0.0), vec3<f32>(0.0));
    var status = true;
    var next = vec2<f32>(0.0);
    var iters = 0u;
    var t_img = zero;
    var t_gx = zero;
    var t_gy = zero;
    for (var li = 0u; li < L.n_levels; li++) {
        let level = L.n_levels - 1u - li;
        let lvl = i32(level);
        let w = i32(L.lv[level].x);
        let h = i32(L.lv[level].y);
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
        if (has_run) {
            let x = ip.x + col;
            let y = ip.y + row;
            let inside = run_inside(w, h, x, y);
            let ir = img_rows(img_i, lvl, w, h, x, y, inside);
            t_img = bilinear_run(ir[0], ir[1], kp);
            let g = grad_rows(lvl, w, h, x, y, inside);
            t_gx = bilinear_run(g[0], g[2], kp);
            t_gy = bilinear_run(g[1], g[3], kp);
            acc = vec4<f32>(run_dot(t_gx, t_gx), run_dot(t_gx, t_gy), run_dot(t_gy, t_gy), 0.0);
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
            var b = vec2<f32>(0.0);
            if (has_run) {
                let x = inx.x + col;
                let y = inx.y + row;
                let inside = run_inside(w, h, x, y);
                let jr2 = img_rows(img_j, lvl, w, h, x, y, inside);
                let jr = bilinear_run(jr2[0], jr2[1], kn);
                let diff = Run((jr.lo - t_img.lo) * 32.0, (jr.hi - t_img.hi) * 32.0);
                b = vec2<f32>(run_dot(diff, t_gx), run_dot(diff, t_gy));
            }
            let s = wg_sum2(lidx, b) * FLT_SCALE;
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
