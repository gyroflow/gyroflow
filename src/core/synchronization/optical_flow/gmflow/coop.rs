// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 yasumorishima <fwyasu11@gmail.com>
//
// A matmul and a convolution on the matrix units of Apple GPUs from WGSL. wgpu 30 on Metal offers cooperative matrices
// (Features::EXPERIMENTAL_COOPERATIVE_MATRIX, `enable wgpu_cooperative_matrix;`), which naga writes as simdgroup_matrix
// (8 x 8, f32); cubecl's WGSL compiler does not use them (no cmma configurations on WGSL), so the kernels are WGSL text
// generated here (mod wgsl) that cubecl launches as is (CubeKernel::source) on burn's buffers. Measured on the macOS CI
// GPU (macOS 15): the model's matmuls 2.5 to 2.9 times as fast as kernels::gemm, its convolutions run once each 121 ms
// with production's kernels against 70 ms.

pub mod wgsl {
    // c[batch] = a[batch] @ b[batch], a [m, k] row-major, b [k, n] either a row-major matrix or the columns of a convolution
    // gathered from x [batch, C, H, W] on the fly (F.unfold order: k = (channel, ky, kx), n = (oy, ox)), c [m, n] row-major.
    // Each workgroup computes a [rows, cols] block of c: sgr x sgc simdgroups of 32 threads, each fr x fc fragments of 8 x 8;
    // a [rows, bk] tile of a and a [bk, cols] tile of b go through workgroup memory, bk columns of a at a time, zero outside
    // of the operands. Any m, n, k: a workgroup over an edge of c writes its block to workgroup memory and copies the part
    // inside c (whether it does depends on the workgroup only, so the cooperative stores stay in uniform control flow).

    use std::fmt::Write;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct CoopTile {
        pub bk: usize,
        pub sgr: usize,
        pub sgc: usize,
        pub fr: usize,
        pub fc: usize,
    }

    impl CoopTile {
        pub fn threads(&self) -> usize {
            32 * self.sgr * self.sgc
        }
        /// Rows of c per workgroup
        pub fn rows(&self) -> usize {
            8 * self.fr * self.sgr
        }
        /// Columns of c per workgroup
        pub fn cols(&self) -> usize {
            8 * self.fc * self.sgc
        }
    }

    /// The tiling the measurements chose: 64 x 32, k step 16, 128 threads, its 4 simdgroups one above the other (8 x
    /// 32 fragments each). At the shapes of the model's kernels on the macOS 15 CI GPU it was the fastest of 8 tilings
    /// or within 7 % of it at every one, against the 2 x 2 simdgroups of 4 x 4 fragments (64 x 64) chosen first on the
    /// large matmuls: the transformer's projections at 1/8 0.52 -> 0.30 ms, the 3x3 and 1x5
    /// convolutions 16 and 17 % faster, the FFN's 5 and 8 %, the projections at 1/4 the same
    pub const COOP_DEFAULT: CoopTile = CoopTile { bk: 16, sgr: 4, sgc: 1, fr: 2, fc: 4 };

    /// The windows of split-window attention on an h x w map of tokens [B, h*w, C] (unimatch split_feature, as
    /// kernels::window_token): batch index bw = image * splits^2 + window, position p in the window row-major, the map
    /// rolled by minus half a window first when shifted. The token of (bw, p) is the row of q, k, v and of the output
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Win {
        pub h: usize,
        pub w: usize,
        pub splits: usize,
        pub shift: bool,
    }

    impl Win {
        /// Positions in a window
        pub fn len(&self) -> usize {
            (self.h / self.splits) * (self.w / self.splits)
        }
    }

    /// Where b comes from
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum CoopB {
        /// A row-major [k, n] matrix, `stride` elements from one batch to the next (0: shared by every batch)
        Matrix { stride: usize },
        /// The columns of a convolution of x [batch, c, h, w] (kernel kh x kw, stride, zero padding ph, pw): k = c kh kw,
        /// n = oh ow
        Conv { c: usize, h: usize, w: usize, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize },
        /// The keys of window bw transposed, [C, L] from k [B, h*w, C]: b[kk][p] = k[token(bw, p)][kk] (k = C, n = L)
        WindowT { win: Win },
        /// The values of window bw, [L, C] from v [B, h*w, C]: b[p][j] = v[token(bw, p)][j] (k = L, n = C)
        WindowRows { win: Win },
    }

    /// The sizes of one launch; `stride_a` 0 for an a shared by every batch. `a_win`: the rows of a are the tokens of
    /// window bw of q [B, h*w, C] (a[p][kk] = q[token(bw, p)][kk] / sqrt(k): the scale of the attention scores, applied
    /// to q as it is read); `c_win`: row p of c goes to row token(bw, p) of [B, h*w, n]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct CoopShape {
        pub m: usize,
        pub n: usize,
        pub k: usize,
        pub stride_a: usize,
        pub b: CoopB,
        pub a_win: Option<Win>,
        pub c_win: Option<Win>,
    }

    impl CoopShape {
        /// The windows of the launch, if any (one layout for a, b and c)
        pub fn win(&self) -> Option<Win> {
            match self.b {
                CoopB::WindowT { win } | CoopB::WindowRows { win } => Some(win),
                _ => self.a_win.or(self.c_win),
            }
        }
    }

    impl CoopB {
        /// Output size of the convolution, (oh, ow)
        pub fn conv_out(c: usize, h: usize, w: usize, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> (usize, usize) {
            let _ = c;
            ((h + 2 * ph - kh) / stride + 1, (w + 2 * pw - kw) / stride + 1)
        }
    }

    /// Why `tile` cannot run `shape`, or None
    pub fn unsupported(shape: CoopShape, tile: CoopTile) -> Option<String> {
        let t = tile.threads();
        let (wm, wn, bk) = (tile.rows(), tile.cols(), tile.bk);
        if bk == 0 || bk % 8 != 0 {
            return Some(format!("k step {bk}: a positive multiple of 8"));
        }
        // Every thread loads the same number of elements of each tile, and copies the same number at the edges
        if (wm * bk) % t != 0 || (bk * wn) % t != 0 || (wm * wn) % t != 0 {
            return Some(format!("the tiles of {tile:?} do not split evenly over {t} threads"));
        }
        // The gathered columns: each thread keeps one column of the tile (t a multiple of the columns)
        if matches!(shape.b, CoopB::Conv { .. } | CoopB::WindowT { .. }) && t % wn != 0 {
            return Some(format!("{tile:?}: {t} threads are not a multiple of its {wn} columns"));
        }
        if let CoopB::Conv { c, kh, kw, .. } = shape.b {
            if c * kh * kw != shape.k {
                return Some(format!("k {} is not c kh kw = {}", shape.k, c * kh * kw));
            }
        }
        // One window layout for a, b and c, whose sizes match it
        if let Some(win) = shape.win() {
            // Before win.len(), which divides by splits
            if win.splits == 0 || win.h % win.splits != 0 || win.w % win.splits != 0 {
                return Some(format!("windows {win:?}: the map does not split into them"));
            }
            let same = |o: Option<Win>| o.is_none() || o == Some(win);
            let b_ok = match shape.b {
                CoopB::WindowT { win: bw } => bw == win && shape.n == win.len(),
                CoopB::WindowRows { win: bw } => bw == win && shape.k == win.len(),
                _ => true,
            };
            if !same(shape.a_win) || !same(shape.c_win) || !b_ok {
                return Some(format!("windows {win:?} do not fit {shape:?}"));
            }
            if (shape.a_win.is_some() || shape.c_win.is_some()) && shape.m != win.len() {
                return Some(format!("{} rows for windows of {}", shape.m, win.len()));
            }
        }
        // The two tiles (padded rows), an 8 x 8 fragment per simdgroup for the edges and the 64 zeros, in Metal's 32 KiB
        let words = wm * (bk + 4) + bk * (wn + 4) + 64 * tile.sgr * tile.sgc + 64;
        if 4 * words > 32 << 10 {
            return Some(format!("{tile:?} needs {} bytes of workgroup memory, over 32 KiB", 4 * words));
        }
        if shape.m == 0 || shape.n == 0 || shape.k == 0 {
            return Some("an empty matmul".to_string());
        }
        None
    }

    /// Whether the vec4 loads apply: a row of a, or of b, starts at a multiple of 4 and holds whole vec4s
    fn vec_a(shape: CoopShape) -> bool {
        shape.k % 4 == 0 && (shape.a_win.is_some() || shape.stride_a % 4 == 0)
    }
    fn vec_b(shape: CoopShape) -> bool {
        match shape.b {
            CoopB::Matrix { stride } => shape.n % 4 == 0 && stride % 4 == 0,
            CoopB::WindowRows { .. } => shape.n % 4 == 0,
            CoopB::Conv { .. } | CoopB::WindowT { .. } => false,
        }
    }

    /// What a launch does besides c = a @ b. `a_gelu`: the GELU of a taken as each element of a is read (a row-major
    /// matrix only): the FFN's second matmul on the first's output, so that the GELU is not a pass of its own over that
    /// [tokens, 1024] tensor; burn's GELU, x (1 + erf(x / sqrt 2)) / 2, erf by its polyfill (Abramowitz and Stegun
    /// 7.1.26, cubecl-core polyfills.rs), which WGSL has no builtin for. `bias_rows`: a bias per row of c added (binding
    /// 3; the output channels of a convolution), `relu`: max(., 0) after it, both as c is written: every block of c then
    /// goes through workgroup memory, as the blocks over an edge do. `a_split`: if not 0, a is two matrices side by
    /// side, its first a_split columns in a [m, a_split] and the others in a2 [m, k - a_split] (the binding after the
    /// bias, or 3): the FFN's first matmul on the concatenation of its input and the message, so that the concatenation
    /// is not a pass of its own; a multiple of the k step, so that each tile of a comes from one of them. `b_split`: if
    /// not 0, the x of a convolution is two maps of the same size, its first b_split channels in x [N, b_split, H, W]
    /// and the others in x2 [N, C - b_split, H, W] (the binding after the bias and a2): the refinement's convolutions on
    /// a concatenation of channels, chosen per element of the gathered columns. `kv_swap`: if not 0, b's windows (the
    /// keys and values of window attention) are those of the other half of the batch, kv_swap windows away: the
    /// transformer's cross-attention, whose target is its input with the two images swapped, not copied so
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Opts {
        pub a_gelu: bool,
        pub bias_rows: bool,
        pub relu: bool,
        pub a_split: usize,
        pub b_split: usize,
        pub kv_swap: usize,
    }

    /// The kernel, entry point `main`, bindings a, b (or x), c, then those of `opts` (bias, a2, x2) in that order
    pub fn source_with(shape: CoopShape, tile: CoopTile, opts: Opts) -> String {
        if let Some(why) = unsupported(shape, tile) {
            panic!("coop gemm: {why}");
        }
        let Opts { a_gelu, bias_rows, relu, a_split, b_split, kv_swap } = opts;
        assert!(kv_swap == 0 || matches!(shape.b, CoopB::WindowT { .. } | CoopB::WindowRows { .. }), "coop gemm: kv_swap for windows of b only");
        assert!(!a_gelu || shape.a_win.is_none(), "coop gemm: the GELU of a is for a row-major a only");
        let split = a_split != 0;
        assert!(!split || (a_split % tile.bk == 0 && a_split < shape.k && shape.stride_a == 0 && shape.a_win.is_none() && !a_gelu),
            "coop gemm: a split at {a_split} for {shape:?} and k steps of {}", tile.bk);
        let xsplit = b_split != 0;
        assert!(!xsplit || matches!(shape.b, CoopB::Conv { c, .. } if b_split < c), "coop gemm: x split at {b_split} for {shape:?}");
        assert!(!(bias_rows || relu) || shape.c_win.is_none(), "coop gemm: no bias or relu with scattered rows of c");
        let CoopShape { m, n, k, stride_a, b, a_win, c_win } = shape;
        // q scaled as it is read (the attention scores / sqrt(C)); Debug of an f32 is a valid WGSL float literal
        let scale = format!("{:?}", 1.0f32 / (k as f32).sqrt());
        let CoopTile { bk, sgc, fr, fc, .. } = tile;
        let (t, wm, wn) = (tile.threads(), tile.rows(), tile.cols());
        let (va, vb) = (vec_a(shape), vec_b(shape));
        // Tests that the sizes make always true are left out
        let full_m = m % wm == 0;
        let full_n = n % wn == 0;
        let full_k = k % bk == 0;
        let mut s = String::new();
        let w = &mut s;
        // The writes into a String cannot fail
        macro_rules! emit { ($($arg:tt)*) => { writeln!(w, $($arg)*).unwrap() } }
        emit!("enable wgpu_cooperative_matrix;");
        emit!("");
        // Every binding read_write although a and b are only read: the layout comes from the shader (a precompiled kernel
        // has no representation to take it from), and cubecl places tensors in shared pages, so a, b and c can be slices
        // of one wgpu buffer, which wgpu refuses to bind both read-only and read-write (cubecl declares its own
        // kernels' bindings all read-write for that reason, backend/wgsl.rs)
        let ty = |v: bool| if v { "array<vec4<f32>>" } else { "array<f32>" };
        emit!("@group(0) @binding(0) var<storage, read_write> a: {};", ty(va));
        emit!("@group(0) @binding(1) var<storage, read_write> b: {};", ty(vb));
        emit!("@group(0) @binding(2) var<storage, read_write> c: array<f32>;");
        if bias_rows {
            emit!("@group(0) @binding(3) var<storage, read_write> bias: array<f32>;");
        }
        if split {
            emit!("@group(0) @binding({}) var<storage, read_write> a2: {};", 3 + bias_rows as usize, ty(va));
        }
        if xsplit {
            emit!("@group(0) @binding({}) var<storage, read_write> x2: array<f32>;", 3 + bias_rows as usize + split as usize);
        }
        emit!("");
        // The accumulators start from an 8 x 8 block of zeros: loaded, not constructed (what Metal makes of a
        // simdgroup_matrix written as `T {{}}` is not specified)
        emit!("var<workgroup> zeros: array<f32, 64>;");
        let (lda, ldb) = (bk + 4, wn + 4);
        emit!("var<workgroup> tile_a: array<f32, {}>;", wm * lda);
        emit!("var<workgroup> tile_b: array<f32, {}>;", bk * ldb);
        // Scattered rows of c are not one stride apart: always the copy through workgroup memory
        let epi = bias_rows || relu;
        let edge = epi || c_win.is_some() || !(full_m && full_n);
        if edge {
            emit!("var<workgroup> frag: array<f32, {}>;", 64 * tile.sgr * tile.sgc);
        }
        emit!("");
        if let Some(Win { h, w: ww, splits, shift }) = shape.win() {
            let (wh, wwn) = (h / splits, ww / splits);
            emit!("fn token(bw: u32, p: u32) -> u32 {{");
            emit!("    let img = bw / {}u;", splits * splits);
            emit!("    let wi = bw % {}u;", splits * splits);
            emit!("    var y = (wi / {splits}u) * {wh}u + p / {wwn}u;");
            emit!("    var x = (wi % {splits}u) * {wwn}u + p % {wwn}u;");
            if shift {
                emit!("    y = (y + {}u) % {h}u;", wh / 2);
                emit!("    x = (x + {}u) % {ww}u;", wwn / 2);
            }
            emit!("    return (img * {h}u + y) * {ww}u + x;");
            emit!("}}");
            emit!("");
        }
        if a_gelu {
            emit!("fn gelu(x: f32) -> f32 {{");
            emit!("    let z = abs(x) / 1.4142135;");
            emit!("    let t = 1.0 / (1.0 + 0.3275911 * z);");
            emit!("    let p = ((((1.0614054 * t - 1.453152) * t + 1.4214137) * t - 0.28449674) * t + 0.2548296) * t;");
            emit!("    let e = 1.0 - p * exp(-z * z);");
            emit!("    return x * (1.0 + select(e, -e, x < 0.0)) / 2.0;");
            emit!("}}");
            emit!("");
            emit!("fn gelu4(v: vec4<f32>) -> vec4<f32> {{");
            emit!("    return vec4<f32>(gelu(v.x), gelu(v.y), gelu(v.z), gelu(v.w));");
            emit!("}}");
            emit!("");
        }
        emit!("@compute @workgroup_size({t}, 1, 1)");
        emit!("fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {{");
        // Workgroup memory starts uninitialised (cubecl turns wgpu's zeroing off)
        emit!("    for (var z = li; z < 64u; z += {t}u) {{ zeros[z] = 0.0; }}");
        emit!("    workgroupBarrier();");
        // Apple GPUs run 32 threads per simdgroup, numbered in order of the thread index
        emit!("    let sg = li / 32u;");
        emit!("    let sg_row = (sg / {sgc}u) * {}u;", 8 * fr);
        emit!("    let sg_col = (sg % {sgc}u) * {}u;", 8 * fc);
        emit!("    let m0 = wg.y * {wm}u;");
        emit!("    let n0 = wg.x * {wn}u;");
        // The windows of b: those of the other image with kv_swap (2 kv_swap windows in all)
        if kv_swap != 0 {
            emit!("    let bw_kv = select(wg.z + {kv_swap}u, wg.z - {kv_swap}u, wg.z >= {kv_swap}u);");
        } else if shape.win().is_some() {
            emit!("    let bw_kv = wg.z;");
        }
        for i in 0..fr {
            for j in 0..fc {
                emit!("    var c_{i}_{j} = coopLoadT<coop_mat8x8<f32, C>>(&zeros[0], 8u);");
            }
        }
        // The two parts of a split a: rows of a_split and of k - a_split (a batch of 1, stride_a 0)
        let (k1, k2) = (a_split, k - a_split);
        if split {
            emit!("    let a_wg = m0 * {k1}u;");
            emit!("    let a2_wg = m0 * {k2}u;");
        } else if a_win.is_none() {
            emit!("    let a_wg = wg.z * {stride_a}u + m0 * {k}u;");
        }
        // The gathered columns: this thread's column of the tile is li % wn at every k step; the top left of its window in
        // x, unsigned (above or left of the map the subtraction wraps around and adding ky or kx wraps back, so one compare
        // per axis tests 0 <= index < size)
        match b {
            CoopB::Matrix { stride } => emit!("    let b_wg = wg.z * {stride}u + n0;"),
            CoopB::WindowRows { .. } => {}
            // The key of this thread's column: one token for every k step
            CoopB::WindowT { .. } => {
                emit!("    let bcol = li % {wn}u;");
                emit!("    let p = n0 + bcol;");
                emit!("    let p_in = p < {n}u;");
                emit!("    var bt = 0u;");
                emit!("    if p_in {{ bt = token(bw_kv, p) * {k}u; }}");
            }
            CoopB::Conv { c: cc, h, w: ww, kh, kw, stride, ph, pw } => {
                let (_, ow) = CoopB::conv_out(cc, h, ww, kh, kw, stride, ph, pw);
                if xsplit {
                    emit!("    let x_wg = wg.z * {}u;", b_split * h * ww);
                    emit!("    let x2_wg = wg.z * {}u;", (cc - b_split) * h * ww);
                } else {
                    emit!("    let x_wg = wg.z * {}u;", cc * h * ww);
                }
                emit!("    let bcol = li % {wn}u;");
                emit!("    let p = n0 + bcol;");
                emit!("    let iy0 = (p / {ow}u) * {stride}u - {ph}u;");
                emit!("    let ix0 = (p % {ow}u) * {stride}u - {pw}u;");
                emit!("    let p_in = p < {n}u;");
            }
        }
        emit!("    for (var k0 = 0u; k0 < {k}u; k0 += {bk}u) {{");
        // The tile of a, [wm, bk]
        if va {
            for it in 0..(wm * bk / 4).div_ceil(t) {
                let guard_t = (it + 1) * t > wm * bk / 4;
                emit!("        {{");
                emit!("            let i = li + {}u;", it * t);
                if guard_t { emit!("            if i < {}u {{", wm * bk / 4); }
                emit!("            let r = i / {}u;", bk / 4);
                emit!("            let q = i % {}u;", bk / 4);
                emit!("            var v = vec4<f32>(0.0);");
                let mut conds = Vec::new();
                if !full_m { conds.push(format!("m0 + r < {m}u")); }
                if !full_k { conds.push(format!("k0 + q * 4u < {k}u")); }
                let load = if a_win.is_some() {
                    format!("v = a[(token(wg.z, m0 + r) * {k}u + k0) / 4u + q] * {scale};")
                } else if split {
                    format!("if k0 < {k1}u {{ v = a[(a_wg + r * {k1}u + k0) / 4u + q]; }} else {{ v = a2[(a2_wg + r * {k2}u + k0 - {k1}u) / 4u + q]; }}")
                } else if a_gelu {
                    format!("v = gelu4(a[(a_wg + r * {k}u + k0) / 4u + q]);")
                } else {
                    format!("v = a[(a_wg + r * {k}u + k0) / 4u + q];")
                };
                emit_guarded(w, "            ", &conds, &load);
                emit!("            let o = r * {lda}u + q * 4u;");
                emit!("            tile_a[o] = v.x; tile_a[o + 1u] = v.y; tile_a[o + 2u] = v.z; tile_a[o + 3u] = v.w;");
                if guard_t { emit!("            }}"); }
                emit!("        }}");
            }
        } else {
            for it in 0..wm * bk / t {
                emit!("        {{");
                emit!("            let i = li + {}u;", it * t);
                emit!("            let r = i / {bk}u;");
                emit!("            let q = i % {bk}u;");
                emit!("            var v = 0.0;");
                let mut conds = Vec::new();
                if !full_m { conds.push(format!("m0 + r < {m}u")); }
                if !full_k { conds.push(format!("k0 + q < {k}u")); }
                let load = if a_win.is_some() {
                    format!("v = a[token(wg.z, m0 + r) * {k}u + k0 + q] * {scale};")
                } else if split {
                    format!("if k0 < {k1}u {{ v = a[a_wg + r * {k1}u + k0 + q]; }} else {{ v = a2[a2_wg + r * {k2}u + k0 - {k1}u + q]; }}")
                } else if a_gelu {
                    format!("v = gelu(a[a_wg + r * {k}u + k0 + q]);")
                } else {
                    format!("v = a[a_wg + r * {k}u + k0 + q];")
                };
                emit_guarded(w, "            ", &conds, &load);
                emit!("            tile_a[r * {lda}u + q] = v;");
                emit!("        }}");
            }
        }
        // The tile of b, [bk, wn]
        match b {
            CoopB::WindowT { .. } => {
                let step = t / wn;
                for it in 0..bk * wn / t {
                    emit!("        {{");
                    emit!("            let r = li / {wn}u + {}u;", it * step);
                    emit!("            var v = 0.0;");
                    let mut conds = Vec::new();
                    if !full_k { conds.push(format!("k0 + r < {k}u")); }
                    if !full_n { conds.push("p_in".to_string()); }
                    emit_guarded(w, "            ", &conds, "v = b[bt + k0 + r];");
                    emit!("            tile_b[r * {ldb}u + bcol] = v;");
                    emit!("        }}");
                }
            }
            CoopB::WindowRows { .. } if vb => {
                for it in 0..(bk * wn / 4).div_ceil(t) {
                    let guard_t = (it + 1) * t > bk * wn / 4;
                    emit!("        {{");
                    emit!("            let i = li + {}u;", it * t);
                    if guard_t { emit!("            if i < {}u {{", bk * wn / 4); }
                    emit!("            let r = i / {}u;", wn / 4);
                    emit!("            let q = i % {}u;", wn / 4);
                    emit!("            var v = vec4<f32>(0.0);");
                    let mut conds = Vec::new();
                    if !full_k { conds.push(format!("k0 + r < {k}u")); }
                    if !full_n { conds.push(format!("n0 + q * 4u < {n}u")); }
                    let load = format!("v = b[(token(bw_kv, k0 + r) * {n}u + n0) / 4u + q];");
                    emit_guarded(w, "            ", &conds, &load);
                    emit!("            let o = r * {ldb}u + q * 4u;");
                    emit!("            tile_b[o] = v.x; tile_b[o + 1u] = v.y; tile_b[o + 2u] = v.z; tile_b[o + 3u] = v.w;");
                    if guard_t { emit!("            }}"); }
                    emit!("        }}");
                }
            }
            CoopB::WindowRows { .. } => {
                for it in 0..bk * wn / t {
                    emit!("        {{");
                    emit!("            let i = li + {}u;", it * t);
                    emit!("            let r = i / {wn}u;");
                    emit!("            let q = i % {wn}u;");
                    emit!("            var v = 0.0;");
                    let mut conds = Vec::new();
                    if !full_k { conds.push(format!("k0 + r < {k}u")); }
                    if !full_n { conds.push(format!("n0 + q < {n}u")); }
                    let load = format!("v = b[token(bw_kv, k0 + r) * {n}u + n0 + q];");
                    emit_guarded(w, "            ", &conds, &load);
                    emit!("            tile_b[r * {ldb}u + q] = v;");
                    emit!("        }}");
                }
            }
            CoopB::Matrix { .. } if vb => {
                for it in 0..(bk * wn / 4).div_ceil(t) {
                    let guard_t = (it + 1) * t > bk * wn / 4;
                    emit!("        {{");
                    emit!("            let i = li + {}u;", it * t);
                    if guard_t { emit!("            if i < {}u {{", bk * wn / 4); }
                    emit!("            let r = i / {}u;", wn / 4);
                    emit!("            let q = i % {}u;", wn / 4);
                    emit!("            var v = vec4<f32>(0.0);");
                    let mut conds = Vec::new();
                    if !full_k { conds.push(format!("k0 + r < {k}u")); }
                    if !full_n { conds.push(format!("n0 + q * 4u < {n}u")); }
                    let load = format!("v = b[(b_wg + (k0 + r) * {n}u) / 4u + q];");
                    emit_guarded(w, "            ", &conds, &load);
                    emit!("            let o = r * {ldb}u + q * 4u;");
                    emit!("            tile_b[o] = v.x; tile_b[o + 1u] = v.y; tile_b[o + 2u] = v.z; tile_b[o + 3u] = v.w;");
                    if guard_t { emit!("            }}"); }
                    emit!("        }}");
                }
            }
            CoopB::Matrix { .. } => {
                for it in 0..bk * wn / t {
                    emit!("        {{");
                    emit!("            let i = li + {}u;", it * t);
                    emit!("            let r = i / {wn}u;");
                    emit!("            let q = i % {wn}u;");
                    emit!("            var v = 0.0;");
                    let mut conds = Vec::new();
                    if !full_k { conds.push(format!("k0 + r < {k}u")); }
                    if !full_n { conds.push(format!("n0 + q < {n}u")); }
                    let load = format!("v = b[b_wg + (k0 + r) * {n}u + q];");
                    emit_guarded(w, "            ", &conds, &load);
                    emit!("            tile_b[r * {ldb}u + q] = v;");
                    emit!("        }}");
                }
            }
            CoopB::Conv { h, w: ww, kh, kw, .. } => {
                let khw = kh * kw;
                // Rows of the tile this thread loads: li / wn, then every t / wn rows
                let step = t / wn;
                for it in 0..bk * wn / t {
                    emit!("        {{");
                    emit!("            let r = li / {wn}u + {}u;", it * step);
                    emit!("            let kk = k0 + r;");
                    emit!("            var v = 0.0;");
                    let mut conds = Vec::new();
                    if !full_k { conds.push(format!("kk < {k}u")); }
                    if !full_n { conds.push("p_in".to_string()); }
                    let mut body = String::new();
                    body.push_str(&format!("let ch = kk / {khw}u; let rr = kk % {khw}u; "));
                    body.push_str(&format!("let iy = iy0 + rr / {kw}u; let ix = ix0 + rr % {kw}u; "));
                    let read = if xsplit {
                        format!("if ch < {b_split}u {{ v = b[x_wg + ch * {hw}u + iy * {ww}u + ix]; }} else {{ v = x2[x2_wg + (ch - {b_split}u) * {hw}u + iy * {ww}u + ix]; }}", hw = h * ww)
                    } else {
                        format!("v = b[x_wg + ch * {}u + iy * {ww}u + ix];", h * ww)
                    };
                    body.push_str(&format!("if iy < {h}u {{ if ix < {ww}u {{ {read} }} }}"));
                    emit_guarded(w, "            ", &conds, &body);
                    emit!("            tile_b[r * {ldb}u + bcol] = v;");
                    emit!("        }}");
                }
            }
        }
        emit!("        workgroupBarrier();");
        for st in 0..bk / 8 {
            for i in 0..fr {
                emit!("        let a_{st}_{i} = coopLoadT<coop_mat8x8<f32, A>>(&tile_a[(sg_row + {}u) * {lda}u + {}u], {lda}u);", i * 8, st * 8);
            }
            for j in 0..fc {
                emit!("        let b_{st}_{j} = coopLoadT<coop_mat8x8<f32, B>>(&tile_b[{}u + sg_col + {}u], {ldb}u);", st * 8 * ldb, j * 8);
            }
            for i in 0..fr {
                for j in 0..fc {
                    emit!("        c_{i}_{j} = coopMultiplyAdd(a_{st}_{i}, b_{st}_{j}, c_{i}_{j});");
                }
            }
        }
        emit!("        workgroupBarrier();");
        emit!("    }}");
        emit!("    let c_wg = wg.z * {}u + m0 * {n}u + n0;", m * n);
        let store_direct = |w: &mut String, indent: &str| {
            for i in 0..fr {
                for j in 0..fc {
                    writeln!(w, "{indent}coopStoreT(c_{i}_{j}, &c[c_wg + (sg_row + {}u) * {n}u + sg_col + {}u], {n}u);", i * 8, j * 8).unwrap();
                }
            }
        };
        if !edge {
            store_direct(w, "    ");
        } else {
            // Inside c whole or not: the same answer for every thread of the workgroup. Joined by `&`, not `&&`, which naga
            // lowers to a local variable and so to a branch its uniformity analysis takes as non-uniform (naga does not
            // check barriers against that analysis, so validation would not show it)
            if c_win.is_some() || epi {
                // Rows scattered to their tokens, or a bias or relu to apply: the copy below for every workgroup
                emit!("    {{");
            } else {
                let mut inside = Vec::new();
                if !full_m { inside.push(format!("(m0 + {wm}u <= {m}u)")); }
                if !full_n { inside.push(format!("(n0 + {wn}u <= {n}u)")); }
                emit!("    if {} {{", inside.join(" & "));
                store_direct(w, "        ");
                emit!("    }} else {{");
            }
            // Over an edge: each fragment into this simdgroup's 8 x 8 of workgroup memory, then its 32 threads copy the 64
            // values (2 each) that are inside c. Every simdgroup stores the same number of fragments, so the barriers are
            // reached by the whole workgroup
            emit!("        let lane = li % 32u;");
            emit!("        let f0 = sg * 64u;");
            for i in 0..fr {
                for j in 0..fc {
                    emit!("        coopStoreT(c_{i}_{j}, &frag[f0], 8u);");
                    emit!("        workgroupBarrier();");
                    for e in 0..2 {
                        emit!("        {{");
                        emit!("            let e = lane + {}u;", e * 32);
                        emit!("            let r = sg_row + {}u + e / 8u;", i * 8);
                        emit!("            let q = sg_col + {}u + e % 8u;", j * 8);
                        if c_win.is_some() {
                            emit!("            if m0 + r < {m}u {{ if n0 + q < {n}u {{ c[token(wg.z, m0 + r) * {n}u + n0 + q] = frag[f0 + e]; }} }}");
                        } else {
                            let mut val = "frag[f0 + e]".to_string();
                            if bias_rows { val = format!("({val} + bias[m0 + r])"); }
                            if relu { val = format!("max({val}, 0.0)"); }
                            emit!("            if m0 + r < {m}u {{ if n0 + q < {n}u {{ c[c_wg + r * {n}u + q] = {val}; }} }}");
                        }
                        emit!("        }}");
                    }
                    emit!("        workgroupBarrier();");
                }
            }
            emit!("    }}");
        }
        emit!("}}");
        s
    }

    /// `body` inside nested ifs of `conds` (none: as is)
    fn emit_guarded(w: &mut String, indent: &str, conds: &[String], body: &str) {
        if conds.is_empty() {
            writeln!(w, "{indent}{body}").unwrap();
            return;
        }
        let open: String = conds.iter().map(|c| format!("if {c} {{ ")).collect();
        let close: String = conds.iter().map(|_| " }").collect();
        writeln!(w, "{indent}{open}{body}{close}").unwrap();
    }
}

use self::wgsl::{ CoopB, CoopShape, CoopTile };
use burn::backend::{ Dispatch, backend_extension, tensor::FloatTensor };
use burn::tensor::{ DType, Shape, Tensor as BurnTensor };
use burn_cubecl::{ CubeBackend, kernel::into_contiguous, tensor::CubeTensor };
use cubecl::ir::{ UIntKind, metadata::Info };
// BufferIOAttr comes with the prelude (cubecl_runtime::kernel::*)
use cubecl::prelude::*;
use cubecl::server::KernelArguments;

/// One compiled kernel per shape and tiling: the text is a function of them alone (wgsl::source), so they are the id,
/// which is what the compilation caches are keyed by, and the text is made only when cubecl compiles it. WGSL pipelines
/// are cached in memory only (cubecl keeps SPIR-V on disk), so each process compiles its own
struct CoopKernel {
    shape: CoopShape,
    tile: CoopTile,
    opts: wgsl::Opts,
}

impl KernelMetadata for CoopKernel {
    fn id(&self) -> KernelId {
        KernelId::new::<Self>().info((self.shape, self.tile, self.opts))
    }

    fn address_type(&self) -> ElemType {
        UIntKind::U32.into()
    }
}

impl CubeKernel for CoopKernel {
    /// Not compiled (source() is): only the settings of the launch
    fn define(&self) -> KernelDefinition {
        let settings = KernelSettings::new(CubeDim::new_1d(self.tile.threads() as u32).0, ExecutionMode::Checked, AddressType::U32);
        KernelDefinition { body: Scope::root(settings.clone()), settings, info: Info::default() }
    }

    fn source(&self) -> Option<PrecompiledSource> {
        Some(PrecompiledSource { source: wgsl::source_with(self.shape, self.tile, self.opts), entrypoint_name: "main".to_string(), lang: "wgsl" })
    }
}

/// c = a @ b on `batch` workgroup layers into a new [batch, m, n] (with c_win, the rows scattered to tokens: shaped
/// `out` then); a and b declared read only to cubecl, so that a kernel that fails to compile marks only its output, not
/// inputs other launches share
#[allow(clippy::too_many_arguments)]
fn launch(a: &CubeTensor, b: &CubeTensor, shape: CoopShape, batch: usize, tile: CoopTile, out: Option<Shape>, opts: wgsl::Opts, bias: Option<&CubeTensor>, a2: Option<&CubeTensor>, x2: Option<&CubeTensor>) -> CubeTensor {
    assert_eq!(a.dtype, DType::F32, "coop: f32 only");
    // Every index of the kernel is a u32 (the windows': as many tokens as rows of all the windows)
    let b_elems = match shape.b {
        CoopB::Matrix { stride } => stride.max(shape.k * shape.n),
        CoopB::Conv { c, h, w, .. } => c * h * w,
        CoopB::WindowT { .. } | CoopB::WindowRows { .. } => shape.k * shape.n,
    };
    let largest = batch * (shape.m * shape.n).max(shape.stride_a.max(shape.m * shape.k)).max(b_elems);
    assert!(largest < 1 << 32, "coop: {batch} x {shape:?} needs indices over u32");
    // Here, on the caller's thread, rather than when the kernel compiles on the device's
    if let Some(why) = wgsl::unsupported(shape, tile) {
        panic!("coop: {why}");
    }
    let out_shape = out.unwrap_or_else(|| Shape::new([batch, shape.m, shape.n]));
    assert_eq!(out_shape.num_elements(), batch * shape.m * shape.n, "coop: output {out_shape:?} for {batch} x {shape:?}");
    let buffer = a.client.empty(out_shape.num_elements() * a.dtype.size());
    let out = CubeTensor::new_contiguous(a.client.clone(), a.device.clone(), out_shape, buffer, a.dtype);
    let count = CubeCount::Static(shape.n.div_ceil(tile.cols()) as u32, shape.m.div_ceil(tile.rows()) as u32, batch as u32);
    assert_eq!(opts.bias_rows, bias.is_some(), "coop: a bias binding exactly with bias_rows");
    let mut args = KernelArguments::new()
        .with_buffer_io(a.handle.clone().binding(), BufferIOAttr::ReadOnly)
        .with_buffer_io(b.handle.clone().binding(), BufferIOAttr::ReadOnly)
        .with_buffer_io(out.handle.clone().binding(), BufferIOAttr::WriteOnly);
    if let Some(bias) = bias {
        assert!(bias.meta.shape().num_elements() == shape.m && bias.dtype == DType::F32, "coop: bias {:?} for {} rows", bias.meta.shape(), shape.m);
        args = args.with_buffer_io(bias.handle.clone().binding(), BufferIOAttr::ReadOnly);
    }
    assert_eq!(opts.a_split != 0, a2.is_some(), "coop: a second part of a exactly with a_split");
    if let Some(a2) = a2 {
        assert!(a2.meta.shape().num_elements() == shape.m * (shape.k - opts.a_split) && a2.dtype == DType::F32, "coop: a2 {:?} for {shape:?} split at {}", a2.meta.shape(), opts.a_split);
        args = args.with_buffer_io(a2.handle.clone().binding(), BufferIOAttr::ReadOnly);
    }
    assert_eq!(opts.b_split != 0, x2.is_some(), "coop: a second part of x exactly with b_split");
    if let (Some(x2), CoopB::Conv { c, h, w, .. }) = (x2, shape.b) {
        assert!(x2.meta.shape().num_elements() == batch * (c - opts.b_split) * h * w && x2.dtype == DType::F32, "coop: x2 {:?} for {batch} x {shape:?} split at {}", x2.meta.shape(), opts.b_split);
        args = args.with_buffer_io(x2.handle.clone().binding(), BufferIOAttr::ReadOnly);
    }
    out.client.launch(Box::new(CoopKernel { shape, tile, opts }), count, args);
    out
}

fn coop_shape(a: &Shape, b: &Shape) -> Shape {
    Shape::new([a[0].max(b[0]), a[1], b[2]])
}

fn launch_gemm(a: CubeTensor, b: CubeTensor, tile: CoopTile, opts: wgsl::Opts, bias: Option<CubeTensor>) -> CubeTensor {
    let bias = bias.map(into_contiguous);
    let (a, b) = (into_contiguous(a), into_contiguous(b));
    let (sa, sb) = (a.meta.shape().clone(), b.meta.shape().clone());
    assert!(sa[2] == sb[1], "coop gemm: inner dimensions {} and {}", sa[2], sb[1]);
    assert!(sa[0] == sb[0] || sa[0] == 1 || sb[0] == 1, "coop gemm: batches {} and {}", sa[0], sb[0]);
    let (m, k, n) = (sa[1], sa[2], sb[2]);
    let shape = CoopShape {
        m,
        n,
        k,
        stride_a: if sa[0] == 1 { 0 } else { m * k },
        b: CoopB::Matrix { stride: if sb[0] == 1 { 0 } else { k * n } },
        a_win: None,
        c_win: None,
    };
    launch(&a, &b, shape, sa[0].max(sb[0]), tile, None, opts, bias.as_ref(), None, None)
}

/// cat([a, a2], 2) @ b without the concatenation (wgsl::Opts::a_split): a [1, m, k1], a2 [1, m, k2], b [batch, k1 + k2, n]
fn launch_gemm_cat_a(a: CubeTensor, a2: CubeTensor, b: CubeTensor, tile: CoopTile) -> CubeTensor {
    let (a, a2, b) = (into_contiguous(a), into_contiguous(a2), into_contiguous(b));
    let (sa, sa2, sb) = (a.meta.shape().clone(), a2.meta.shape().clone(), b.meta.shape().clone());
    assert!(sa[0] == 1 && sa2[0] == 1 && sa[1] == sa2[1], "coop gemm_cat_a: a {sa:?} and a2 {sa2:?}");
    let (m, k1, k, n) = (sa[1], sa[2], sa[2] + sa2[2], sb[2]);
    assert!(sb[1] == k, "coop gemm_cat_a: b {sb:?} for {k1} + {} columns of a", sa2[2]);
    let shape = CoopShape { m, n, k, stride_a: 0, b: CoopB::Matrix { stride: if sb[0] == 1 { 0 } else { k * n } }, a_win: None, c_win: None };
    launch(&a, &b, shape, sb[0], tile, None, wgsl::Opts { a_split: k1, ..Default::default() }, None, Some(&a2), None)
}

/// unimatch single_head_split_window_attention, as kernels::window_attention does it (no window layout or roll in
/// memory) with its two matmuls on cooperative matrices: q @ k^T / sqrt(C) of each window [Bw, L, L], the masked softmax
/// of kernels.rs in place, then the probabilities @ v written back to the rows of the positions. q, k, v [B, h*w, C]
#[allow(clippy::too_many_arguments)]
fn launch_window_attention(q: CubeTensor, k: CubeTensor, v: CubeTensor, h: usize, w: usize, splits: usize, shift: bool, kv_swap: bool, tile: CoopTile) -> CubeTensor {
    let (q, k, v) = (into_contiguous(q), into_contiguous(k), into_contiguous(v));
    let s = q.meta.shape().clone();
    assert!(s[1] == h * w && h % splits == 0 && w % splits == 0 && s[2] > 0, "coop window_attention: {s:?} for {h}x{w} in {splits}x{splits} windows");
    assert!(k.meta.shape() == &s && v.meta.shape() == &s, "coop window_attention: q {s:?}, k {:?}, v {:?}", k.meta.shape(), v.meta.shape());
    let win = wgsl::Win { h, w, splits, shift };
    let (c, l, bw) = (s[2], win.len(), s[0] * splits * splits);
    // k and v of the other image: the windows half the batch away
    assert!(!kv_swap || s[0] % 2 == 0, "coop window_attention: k and v swapped by halves of a batch of {}", s[0]);
    let opts = wgsl::Opts { kv_swap: if kv_swap { bw / 2 } else { 0 }, ..Default::default() };
    let scores = CoopShape { m: l, n: l, k: c, stride_a: 0, b: CoopB::WindowT { win }, a_win: Some(win), c_win: None };
    let scores = launch(&q, &k, scores, bw, tile, None, opts, None, None, None);
    super::kernels::launch_window_softmax(&scores, h, w, splits, shift);
    let apply = CoopShape { m: l, n: c, k: l, stride_a: l * l, b: CoopB::WindowRows { win }, a_win: None, c_win: Some(win) };
    launch(&scores, &v, apply, bw, tile, Some(s), opts, None, None, None)
}

fn conv_shape(w: &Shape, x: &Shape, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> Shape {
    let (oh, ow) = CoopB::conv_out(x[1], x[2], x[3], kh, kw, stride, ph, pw);
    Shape::new([x[0], w[1], oh * ow])
}

#[allow(clippy::too_many_arguments)]
fn launch_conv(w: CubeTensor, x: CubeTensor, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, tile: CoopTile, opts: wgsl::Opts, bias: Option<CubeTensor>, x2: Option<CubeTensor>) -> CubeTensor {
    let (w, x) = (into_contiguous(w), into_contiguous(x));
    let bias = bias.map(into_contiguous);
    let x2 = x2.map(into_contiguous);
    let (sw, sx) = (w.meta.shape().clone(), x.meta.shape().clone());
    // With x2: the channels of x then those of x2, maps of the same size
    let c2 = match &x2 {
        Some(x2) => {
            let s2 = x2.meta.shape();
            assert!(s2[0] == sx[0] && s2[2] == sx[2] && s2[3] == sx[3], "coop conv: x {sx:?} and x2 {s2:?}");
            s2[1]
        }
        None => 0,
    };
    let opts = wgsl::Opts { b_split: if x2.is_some() { sx[1] } else { 0 }, ..opts };
    let (c, h, wd) = (sx[1] + c2, sx[2], sx[3]);
    assert!(sw[0] == 1 && sw[2] == c * kh * kw, "coop conv: weight {sw:?} for x {sx:?} and a {kh}x{kw} kernel");
    let (oh, ow) = CoopB::conv_out(c, h, wd, kh, kw, stride, ph, pw);
    let shape = CoopShape { m: sw[1], n: oh * ow, k: sw[2], stride_a: 0, b: CoopB::Conv { c, h, w: wd, kh, kw, stride, ph, pw }, a_win: None, c_win: None };
    launch(&w, &x, shape, sx[0], tile, None, opts, bias.as_ref(), None, x2.as_ref())
}

#[backend_extension(Cube, Fusion)]
pub trait CoopKernels: burn::backend::Backend {
    #[fusion(dtype = a, shape = coop_shape(a, b))]
    fn coop_gemm(a: FloatTensor<Self>, b: FloatTensor<Self>, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self>;
    #[fusion(dtype = a, shape = coop_shape(a, b))]
    #[allow(clippy::too_many_arguments)]
    fn coop_gemm_cat_a(a: FloatTensor<Self>, a2: FloatTensor<Self>, b: FloatTensor<Self>, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self>;
    #[fusion(dtype = a, shape = coop_shape(a, b))]
    fn coop_gemm_gelu_a(a: FloatTensor<Self>, b: FloatTensor<Self>, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self>;
    #[fusion(dtype = a, shape = coop_shape(a, b))]
    #[allow(clippy::too_many_arguments)]
    fn coop_gemm_epi(a: FloatTensor<Self>, b: FloatTensor<Self>, bias: FloatTensor<Self>, relu: bool, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self>;
    #[fusion(dtype = x, shape = conv_shape(w, x, *kh, *kw, *stride, *ph, *pw))]
    #[allow(clippy::too_many_arguments)]
    fn coop_conv_epi(w: FloatTensor<Self>, x: FloatTensor<Self>, bias: FloatTensor<Self>, relu: bool, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self>;
    #[fusion(dtype = x, shape = conv_shape(w, x, *kh, *kw, *stride, *ph, *pw))]
    #[allow(clippy::too_many_arguments)]
    fn coop_conv_cat(w: FloatTensor<Self>, x: FloatTensor<Self>, x2: FloatTensor<Self>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self>;
    #[fusion(dtype = x, shape = conv_shape(w, x, *kh, *kw, *stride, *ph, *pw))]
    #[allow(clippy::too_many_arguments)]
    fn coop_conv_cat_epi(w: FloatTensor<Self>, x: FloatTensor<Self>, x2: FloatTensor<Self>, bias: FloatTensor<Self>, relu: bool, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self>;
    #[fusion(dtype = x, shape = conv_shape(w, x, *kh, *kw, *stride, *ph, *pw))]
    #[allow(clippy::too_many_arguments)]
    fn coop_conv(w: FloatTensor<Self>, x: FloatTensor<Self>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self>;
    #[fusion(dtype = q, shape = q)]
    #[allow(clippy::too_many_arguments)]
    fn coop_window_attention(q: FloatTensor<Self>, k: FloatTensor<Self>, v: FloatTensor<Self>, h: usize, w: usize, splits: usize, shift: bool, kv_swap: bool, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self>;
}

impl CoopKernels for CubeBackend {
    fn coop_gemm(a: FloatTensor<Self>, b: FloatTensor<Self>, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self> {
        launch_gemm(a, b, CoopTile { bk, sgr, sgc, fr, fc }, wgsl::Opts::default(), None)
    }
    #[allow(clippy::too_many_arguments)]
    fn coop_gemm_cat_a(a: FloatTensor<Self>, a2: FloatTensor<Self>, b: FloatTensor<Self>, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self> {
        launch_gemm_cat_a(a, a2, b, CoopTile { bk, sgr, sgc, fr, fc })
    }
    fn coop_gemm_gelu_a(a: FloatTensor<Self>, b: FloatTensor<Self>, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self> {
        launch_gemm(a, b, CoopTile { bk, sgr, sgc, fr, fc }, wgsl::Opts { a_gelu: true, ..Default::default() }, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn coop_gemm_epi(a: FloatTensor<Self>, b: FloatTensor<Self>, bias: FloatTensor<Self>, relu: bool, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self> {
        launch_gemm(a, b, CoopTile { bk, sgr, sgc, fr, fc }, wgsl::Opts { bias_rows: true, relu, ..Default::default() }, Some(bias))
    }
    #[allow(clippy::too_many_arguments)]
    fn coop_conv_epi(w: FloatTensor<Self>, x: FloatTensor<Self>, bias: FloatTensor<Self>, relu: bool, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self> {
        launch_conv(w, x, kh, kw, stride, ph, pw, CoopTile { bk, sgr, sgc, fr, fc }, wgsl::Opts { bias_rows: true, relu, ..Default::default() }, Some(bias), None)
    }
    #[allow(clippy::too_many_arguments)]
    fn coop_conv_cat(w: FloatTensor<Self>, x: FloatTensor<Self>, x2: FloatTensor<Self>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self> {
        launch_conv(w, x, kh, kw, stride, ph, pw, CoopTile { bk, sgr, sgc, fr, fc }, wgsl::Opts::default(), None, Some(x2))
    }
    #[allow(clippy::too_many_arguments)]
    fn coop_conv_cat_epi(w: FloatTensor<Self>, x: FloatTensor<Self>, x2: FloatTensor<Self>, bias: FloatTensor<Self>, relu: bool, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self> {
        launch_conv(w, x, kh, kw, stride, ph, pw, CoopTile { bk, sgr, sgc, fr, fc }, wgsl::Opts { bias_rows: true, relu, ..Default::default() }, Some(bias), Some(x2))
    }
    fn coop_conv(w: FloatTensor<Self>, x: FloatTensor<Self>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self> {
        launch_conv(w, x, kh, kw, stride, ph, pw, CoopTile { bk, sgr, sgc, fr, fc }, wgsl::Opts::default(), None, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn coop_window_attention(q: FloatTensor<Self>, k: FloatTensor<Self>, v: FloatTensor<Self>, h: usize, w: usize, splits: usize, shift: bool, kv_swap: bool, bk: usize, sgr: usize, sgc: usize, fr: usize, fc: usize) -> FloatTensor<Self> {
        launch_window_attention(q, k, v, h, w, splits, shift, kv_swap, CoopTile { bk, sgr, sgc, fr, fc })
    }
}

/// kernels::window_attention with its two matmuls on cooperative matrices: q, k, v [B, h*w, C] of an h x w map in
/// splits x splits windows -> [B, h*w, C]
#[allow(clippy::too_many_arguments)]
pub fn window_attention(q: BurnTensor<3>, k: BurnTensor<3>, v: BurnTensor<3>, h: usize, w: usize, splits: usize, shift: bool, tile: CoopTile) -> BurnTensor<3> {
    let CoopTile { bk, sgr, sgc, fr, fc } = tile;
    BurnTensor::from_dispatch(Dispatch::coop_window_attention(q.into_dispatch(), k.into_dispatch(), v.into_dispatch(), h, w, splits, shift, false, bk, sgr, sgc, fr, fc))
}

/// window_attention(q, swap_halves(k), swap_halves(v), ..) without the swaps: the windows of k and v read from the other
/// half of the batch (wgsl::Opts::kv_swap). The batch even
#[allow(clippy::too_many_arguments)]
pub fn window_attention_kv_swapped(q: BurnTensor<3>, k: BurnTensor<3>, v: BurnTensor<3>, h: usize, w: usize, splits: usize, shift: bool, tile: CoopTile) -> BurnTensor<3> {
    let CoopTile { bk, sgr, sgc, fr, fc } = tile;
    BurnTensor::from_dispatch(Dispatch::coop_window_attention(q.into_dispatch(), k.into_dispatch(), v.into_dispatch(), h, w, splits, shift, true, bk, sgr, sgc, fr, fc))
}

/// [batch, m, k] @ [batch, k, n] -> [batch, m, n] (an operand of batch 1 shared), f32, any sizes
pub fn gemm(a: BurnTensor<3>, b: BurnTensor<3>, tile: CoopTile) -> BurnTensor<3> {
    let CoopTile { bk, sgr, sgc, fr, fc } = tile;
    BurnTensor::from_dispatch(Dispatch::coop_gemm(a.into_dispatch(), b.into_dispatch(), bk, sgr, sgc, fr, fc))
}

/// cat([a, a2], 2) @ b, each tile of a read from the part it lies in (wgsl::Opts::a_split): a [1, m, k1] (k1 a multiple
/// of the tile's k step), a2 [1, m, k2], b [batch, k1 + k2, n] -> [batch, m, n]
pub fn gemm_cat_a(a: BurnTensor<3>, a2: BurnTensor<3>, b: BurnTensor<3>, tile: CoopTile) -> BurnTensor<3> {
    let CoopTile { bk, sgr, sgc, fr, fc } = tile;
    BurnTensor::from_dispatch(Dispatch::coop_gemm_cat_a(a.into_dispatch(), a2.into_dispatch(), b.into_dispatch(), bk, sgr, sgc, fr, fc))
}

/// gelu(a) @ b, the GELU taken as a is read (wgsl::source_with): [batch, m, k] @ [batch, k, n] -> [batch, m, n]
pub fn gemm_gelu_a(a: BurnTensor<3>, b: BurnTensor<3>, tile: CoopTile) -> BurnTensor<3> {
    let CoopTile { bk, sgr, sgc, fr, fc } = tile;
    BurnTensor::from_dispatch(Dispatch::coop_gemm_gelu_a(a.into_dispatch(), b.into_dispatch(), bk, sgr, sgc, fr, fc))
}

/// w [1, Co, C*KH*KW] @ the KH x KW columns of x [N, C, H, W] (stride, symmetric zero padding) -> [N, Co, OH*OW], the
/// columns gathered in the kernel
#[allow(clippy::too_many_arguments)]
pub fn conv_gemm(w: BurnTensor<3>, x: BurnTensor<4>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, tile: CoopTile) -> BurnTensor<3> {
    let CoopTile { bk, sgr, sgc, fr, fc } = tile;
    BurnTensor::from_dispatch(Dispatch::coop_conv(w.into_dispatch(), x.into_dispatch(), kh, kw, stride, ph, pw, bk, sgr, sgc, fr, fc))
}

/// conv_gemm of cat([x, x2], 1) without the concatenation (wgsl::Opts::b_split): x [N, C1, H, W], x2 [N, C2, H, W],
/// w [1, Co, (C1 + C2)*KH*KW] -> [N, Co, OH*OW]
#[allow(clippy::too_many_arguments)]
pub fn conv_gemm_cat(w: BurnTensor<3>, x: BurnTensor<4>, x2: BurnTensor<4>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize, tile: CoopTile) -> BurnTensor<3> {
    let CoopTile { bk, sgr, sgc, fr, fc } = tile;
    BurnTensor::from_dispatch(Dispatch::coop_conv_cat(w.into_dispatch(), x.into_dispatch(), x2.into_dispatch(), kh, kw, stride, ph, pw, bk, sgr, sgc, fr, fc))
}

/// conv2d_act of cat([x, x2], 1), the columns gathered from both (wgsl::Opts::b_split): x [N, C1, H, W], x2 [N, C2, H, W],
/// weight [Cout, C1 + C2, KH, KW] -> [N, Cout, OH, OW]
#[allow(clippy::too_many_arguments)]
pub fn conv2d_act_cat(x: BurnTensor<4>, x2: BurnTensor<4>, weight: BurnTensor<4>, bias: Option<BurnTensor<1>>, stride: usize, padding: [usize; 2], relu: bool, tile: CoopTile) -> BurnTensor<4> {
    let [n, c1, h, w] = x.dims();
    let c2 = x2.dims()[1];
    let [cout, cin, kh, kw] = weight.dims();
    assert_eq!(c1 + c2, cin, "coop conv2d_act_cat: channels");
    let (ph, pw) = (padding[0], padding[1]);
    let (oh, ow) = CoopB::conv_out(cin, h, w, kh, kw, stride, ph, pw);
    let wm = weight.reshape([1, cout, cin * kh * kw]);
    let CoopTile { bk, sgr, sgc, fr, fc } = tile;
    let out: BurnTensor<3> = match (bias, relu) {
        (None, false) => conv_gemm_cat(wm, x, x2, kh, kw, stride, ph, pw, tile),
        (bias, relu) => {
            let bias = bias.unwrap_or_else(|| BurnTensor::<1>::zeros([cout], &x.device()));
            BurnTensor::from_dispatch(Dispatch::coop_conv_cat_epi(wm.into_dispatch(), x.into_dispatch(), x2.into_dispatch(), bias.into_dispatch(), relu, kh, kw, stride, ph, pw, bk, sgr, sgc, fr, fc))
        }
    };
    out.reshape([n, cout, oh, ow])
}

/// A 2D convolution (symmetric zero padding, no dilation or groups): x [N, C, H, W], weight [Cout, C, KH, KW] ->
/// [N, Cout, OH, OW]; 1x1 of stride 1 as a plain matmul of the weight and x, the others gathered
pub fn conv2d(x: BurnTensor<4>, weight: BurnTensor<4>, bias: Option<BurnTensor<1>>, stride: usize, padding: [usize; 2], tile: CoopTile) -> BurnTensor<4> {
    conv2d_act(x, weight, bias, stride, padding, false, tile)
}

/// conv2d, the bias (if any) added and with `relu` max(., 0) taken as the kernel writes its output (wgsl::Opts), not as
/// a pass of their own
#[allow(clippy::too_many_arguments)]
pub fn conv2d_act(x: BurnTensor<4>, weight: BurnTensor<4>, bias: Option<BurnTensor<1>>, stride: usize, padding: [usize; 2], relu: bool, tile: CoopTile) -> BurnTensor<4> {
    let [n, c, h, w] = x.dims();
    let [cout, cin, kh, kw] = weight.dims();
    assert_eq!(c, cin, "coop conv2d: channels");
    let (ph, pw) = (padding[0], padding[1]);
    let (oh, ow) = CoopB::conv_out(c, h, w, kh, kw, stride, ph, pw);
    let wm = weight.reshape([1, cout, c * kh * kw]);
    let one = kh == 1 && kw == 1 && stride == 1 && ph == 0 && pw == 0;
    let CoopTile { bk, sgr, sgc, fr, fc } = tile;
    let out: BurnTensor<3> = match (bias, relu) {
        (None, false) if one => gemm(wm, x.reshape([n, c, h * w]), tile),
        (None, false) => conv_gemm(wm, x, kh, kw, stride, ph, pw, tile),
        (bias, relu) => {
            // A relu without a bias: a bias of zeros (one kernel text with both)
            let bias = bias.unwrap_or_else(|| BurnTensor::<1>::zeros([cout], &x.device()));
            BurnTensor::from_dispatch(if one {
                Dispatch::coop_gemm_epi(wm.into_dispatch(), x.reshape([n, c, h * w]).into_dispatch(), bias.into_dispatch(), relu, bk, sgr, sgc, fr, fc)
            } else {
                Dispatch::coop_conv_epi(wm.into_dispatch(), x.into_dispatch(), bias.into_dispatch(), relu, kh, kw, stride, ph, pw, bk, sgr, sgc, fr, fc)
            })
        }
    };
    out.reshape([n, cout, oh, ow])
}
