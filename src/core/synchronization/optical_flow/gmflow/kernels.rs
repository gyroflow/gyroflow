// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 yasumorishima <fwyasu11@gmail.com>
//
// CubeCL kernels for the parts of gmflow (model.rs) that burn would otherwise run as many small kernels over large
// intermediate tensors: the local correlations (the 9x9 window of matching at 1/4 and of each refinement step), the 3x3
// propagation and the convex upsampling. Each computes in one launch, from the inputs, what model.rs computes with plain
// tensor operations (kept there as the reference the tests compare these to).

use burn::backend::cubecl::dtype_to_storage_type;
use burn::backend::fusion::custom::TensorSpec;
use burn::backend::{ Dispatch, backend_extension, tensor::{ FloatTensor, IntTensor } };
// burn::tensor::Int is the kind of integer tensors; Int in kernels is the CubeCL trait
// Tensor in the kernels is the CubeCL one: the burn tensors of the functions model.rs calls are BurnTensor
use burn::tensor::{ DType, Int as IntKind, Shape, Tensor as BurnTensor };
use burn_cubecl::{ CubeBackend, kernel::into_contiguous, tensor::CubeTensor };
use cubecl::{ calculate_cube_count_elemwise, cube, prelude::*, CubeDim };

/// The 9x9 window of matching at 1/4 and of the refinement
pub const CORR_RADIUS: usize = 4;
/// The 3x3 window of the propagation at 1/4
pub const PROP_RADIUS: usize = 1;
pub const UPSAMPLE: usize = 4;

/// local_correlation_with_flow: out[b, k, y, x] = f0[b, :, y, x] . bilinear(f1[b], (x, y) + flow[b, :, y, x] + offset_k)
/// / sqrt(C), zeros outside of the map (each of the 4 taps on its own, as F.grid_sample with zeros padding).
/// The offsets are integers, so the (2r+1)^2 sample points of a position share the fractional part of its flow: their
/// taps are the (2r+2)^2 integer points from floor((x, y) + flow) - r, and each output is the bilinear weighting of the
/// 4 products of f0 with its taps. Two kernels: the products with the (2r+2)^2 integer points (0 outside of the map),
/// then the weighting: (2r+2)^2 products of C channels per position instead of 4 (2r+1)^2.
#[cube(launch)]
fn corr_taps_kernel<F: Float>(
    f0: &Tensor<F>,      // [B, C, H, W]
    f1: &Tensor<F>,      // [B, C, H, W]
    flow: &Tensor<F>,    // [B, 2, H, W]
    taps: &mut Tensor<F>, // [B, (2r+2)^2, H, W]
    #[comptime] radius: usize,
    #[define(F)] _dtype: ElemType,
) {
    // One thread per position and row of the window: the 2r+2 products of the row share each value of f0 and read
    // neighbouring values of f1, as independent sums
    let idx = ABSOLUTE_POS as usize;
    let (b_n, h, w) = (taps.shape(0), taps.shape(2), taps.shape(3));
    let m = comptime!(2 * radius + 2);
    if idx >= b_n * m * h * w {
        terminate!();
    }
    let hw = h * w;
    let p = idx % hw;
    let ey = (idx / hw) % m;
    let b = idx / (hw * m);
    let x = p % w;
    let y = p / w;
    let r = comptime!(radius as i32);
    let wi = w as i32;
    let x0 = x as i32 + i32::cast_from(flow[b * 2 * hw + p].floor()) - r;
    let sy = y as i32 + i32::cast_from(flow[b * 2 * hw + hw + p].floor()) - r + ey as i32;
    let c_n = f0.shape(1);
    let mut acc = Array::<F>::new(m);
    #[unroll]
    for e in 0..m {
        acc[e] = F::new(0.0_f32);
    }
    if sy >= 0 && sy < h as i32 {
        let row = sy as usize * w;
        for c in 0..c_n {
            let base = (b * c_n + c) * hw;
            let a = f0[base + p];
            #[unroll]
            for e in 0..m {
                // Clamped address: a point outside of the map is set to 0 below
                let sx = clamp(x0 + e as i32, 0, wi - 1) as usize;
                acc[e] += a * f1[base + row + sx];
            }
        }
    }
    let scale = F::sqrt(F::cast_from(c_n as u32));
    #[unroll]
    for e in 0..m {
        let sx = x0 + e as i32;
        let v = if sx >= 0 && sx < wi { acc[e] / scale } else { F::new(0.0_f32) };
        taps[(b * m * m + ey * m + e) * hw + p] = v;
    }
}

#[cube(launch)]
fn corr_weight_kernel<F: Float>(
    taps: &Tensor<F>,    // [B, (2r+2)^2, H, W]
    flow: &Tensor<F>,    // [B, 2, H, W]
    out: &mut Tensor<F>, // [B, (2r+1)^2, H, W]
    #[comptime] radius: usize,
    #[define(F)] _dtype: ElemType,
) {
    let idx = ABSOLUTE_POS as usize;
    let (b_n, k_n, h, w) = (out.shape(0), out.shape(1), out.shape(2), out.shape(3));
    if idx >= b_n * k_n * h * w {
        terminate!();
    }
    let hw = h * w;
    let p = idx % hw;
    let k = (idx / hw) % k_n;
    let b = idx / (hw * k_n);
    let n = comptime!(2 * radius + 1);
    let m = comptime!(2 * radius + 2);
    let fx = flow[b * 2 * hw + p];
    let fy = flow[b * 2 * hw + hw + p];
    let ax = fx - fx.floor();
    let ay = fy - fy.floor();
    let one = F::new(1.0_f32);
    // Tap (i, j) of output (dy, dx) = (k / n, k % n) is integer point (dy + i, dx + j) of the (2r+2)^2 window
    let t = (b * m * m + (k / n) * m + k % n) * hw + p;
    let t00 = taps[t];
    let t01 = taps[t + hw];
    let t10 = taps[t + m * hw];
    let t11 = taps[t + (m + 1) * hw];
    out[idx] = (one - ay) * ((one - ax) * t00 + ax * t01) + ay * ((one - ax) * t10 + ax * t11);
}

/// local_correlation_softmax at integer offsets: the correlation of each position with the (2r+1)^2 window around it
/// (-1e9 outside of the map), softmax, and the expected sample position minus the position: flow [B, 2, H, W]. One thread
/// per position, online softmax.
#[cube(launch)]
fn local_corr_softmax_kernel<F: Float>(
    f0: &Tensor<F>,      // [B, C, H, W]
    f1: &Tensor<F>,      // [B, C, H, W]
    out: &mut Tensor<F>, // [B, 2, H, W]
    #[comptime] radius: usize,
    #[define(F)] _dtype: ElemType,
) {
    let idx = ABSOLUTE_POS as usize;
    let (b_n, c_n, h, w) = (f0.shape(0), f0.shape(1), f0.shape(2), f0.shape(3));
    if idx >= b_n * h * w {
        terminate!();
    }
    let hw = h * w;
    let p = idx % hw;
    let b = idx / hw;
    let x = p % w;
    let y = p / w;
    let n = comptime!(2 * radius + 1);
    let r = comptime!(radius as i32);
    let scale = F::sqrt(F::cast_from(c_n as u32));
    let mut m = F::new(-1e30_f32);
    let mut s = F::new(0.0_f32);
    let mut ax = F::new(0.0_f32);
    let mut ay = F::new(0.0_f32);
    for k in 0..n * n {
        let sx = x as i32 + (k % n) as i32 - r;
        let sy = y as i32 + (k / n) as i32 - r;
        let valid = sx >= 0 && sx < w as i32 && sy >= 0 && sy < h as i32;
        let mut score = F::new(-1e9_f32);
        if valid {
            let q = sy as usize * w + sx as usize;
            let mut acc = F::new(0.0_f32);
            for c in 0..c_n {
                let base = (b * c_n + c) * hw;
                acc += f0[base + p] * f1[base + q];
            }
            score = acc / scale;
        }
        let fsx = F::cast_from(sx);
        let fsy = F::cast_from(sy);
        if score > m {
            let carry = F::exp(m - score);
            s = s * carry + F::new(1.0_f32);
            ax = ax * carry + fsx;
            ay = ay * carry + fsy;
            m = score;
        } else {
            let e = F::exp(score - m);
            s += e;
            ax += e * fsx;
            ay += e * fsy;
        }
    }
    out[b * 2 * hw + p] = ax / s - F::cast_from(x as u32);
    out[b * 2 * hw + hw + p] = ay / s - F::cast_from(y as u32);
}

/// SelfAttnPropagation.forward_local_window_attn: softmax over the (2r+1)^2 window of q . k / sqrt(C), the flow of the
/// window weighted by it. Outside of the map the key and the flow are zeros (F.unfold padding): score 0, still counted.
#[cube(launch)]
fn local_propagation_kernel<F: Float>(
    q: &Tensor<F>,       // [B, C, H, W]
    k: &Tensor<F>,       // [B, C, H, W]
    flow: &Tensor<F>,    // [B, 2, H, W]
    out: &mut Tensor<F>, // [B, 2, H, W]
    #[comptime] radius: usize,
    #[define(F)] _dtype: ElemType,
) {
    let idx = ABSOLUTE_POS as usize;
    let (b_n, c_n, h, w) = (q.shape(0), q.shape(1), q.shape(2), q.shape(3));
    if idx >= b_n * h * w {
        terminate!();
    }
    let hw = h * w;
    let p = idx % hw;
    let b = idx / hw;
    let x = p % w;
    let y = p / w;
    let n = comptime!(2 * radius + 1);
    let r = comptime!(radius as i32);
    let scale = F::sqrt(F::cast_from(c_n as u32));
    let mut m = F::new(-1e30_f32);
    let mut s = F::new(0.0_f32);
    let mut ax = F::new(0.0_f32);
    let mut ay = F::new(0.0_f32);
    for j in 0..n * n {
        let sx = x as i32 + (j % n) as i32 - r;
        let sy = y as i32 + (j / n) as i32 - r;
        let valid = sx >= 0 && sx < w as i32 && sy >= 0 && sy < h as i32;
        let mut score = F::new(0.0_f32);
        let mut vx = F::new(0.0_f32);
        let mut vy = F::new(0.0_f32);
        if valid {
            let o = sy as usize * w + sx as usize;
            let mut acc = F::new(0.0_f32);
            for c in 0..c_n {
                let base = (b * c_n + c) * hw;
                acc += q[base + p] * k[base + o];
            }
            score = acc / scale;
            vx = flow[b * 2 * hw + o];
            vy = flow[b * 2 * hw + hw + o];
        }
        if score > m {
            let carry = F::exp(m - score);
            s = s * carry + F::new(1.0_f32);
            ax = ax * carry + vx;
            ay = ay * carry + vy;
            m = score;
        } else {
            let e = F::exp(score - m);
            s += e;
            ax += e * vx;
            ay += e * vy;
        }
    }
    out[b * 2 * hw + p] = ax / s;
    out[b * 2 * hw + hw + p] = ay / s;
}

/// upsample_flow_with_mask: out[b, c, f*h+ky, f*w+kx] = sum over the 3x3 window j of softmax_j(mask[b, j*f*f + ky*f+kx, h, w])
/// * f * flow[b, c, (h, w) + offset_j] (zeros outside). One thread per output position, both channels.
#[cube(launch)]
fn convex_upsample_kernel<F: Float>(
    flow: &Tensor<F>,    // [B, 2, H, W]
    mask: &Tensor<F>,    // [B, 9 * f * f, H, W]
    out: &mut Tensor<F>, // [B, 2, f*H, f*W]
    #[comptime] factor: usize,
    #[define(F)] _dtype: ElemType,
) {
    let idx = ABSOLUTE_POS as usize;
    let (b_n, h, w) = (flow.shape(0), flow.shape(2), flow.shape(3));
    let ow = w * factor;
    let oh = h * factor;
    if idx >= b_n * oh * ow {
        terminate!();
    }
    let ohw = oh * ow;
    let op = idx % ohw;
    let b = idx / ohw;
    let ox = op % ow;
    let oy = op / ow;
    let x = ox / factor;
    let y = oy / factor;
    let sub = (oy % factor) * factor + ox % factor;
    let hw = h * w;
    let p = y * w + x;
    let ff = comptime!(factor * factor);
    let mbase = b * 9 * ff * hw + sub * hw + p;
    let mut m = F::new(-1e30_f32);
    for j in 0..9usize {
        m = F::max(m, mask[mbase + j * ff * hw]);
    }
    let mult = F::cast_from(factor as u32);
    let mut s = F::new(0.0_f32);
    let mut ax = F::new(0.0_f32);
    let mut ay = F::new(0.0_f32);
    for j in 0..9usize {
        let e = F::exp(mask[mbase + j * ff * hw] - m);
        s += e;
        let sx = x as i32 + (j % 3) as i32 - 1;
        let sy = y as i32 + (j / 3) as i32 - 1;
        if sx >= 0 && sx < w as i32 && sy >= 0 && sy < h as i32 {
            let o = sy as usize * w + sx as usize;
            ax += e * (mult * flow[b * 2 * hw + o]);
            ay += e * (mult * flow[b * 2 * hw + hw + o]);
        }
    }
    out[b * 2 * ohw + op] = ax / s;
    out[b * 2 * ohw + ohw + op] = ay / s;
}

// ---------------------------------------------------------------- launches

fn corr_shape(f0: &Shape) -> Shape {
    let n = 2 * CORR_RADIUS + 1;
    Shape::new([f0[0], n * n, f0[2], f0[3]])
}

fn flow_shape(f0: &Shape) -> Shape {
    Shape::new([f0[0], 2, f0[2], f0[3]])
}

fn upsampled_shape(flow: &Shape) -> Shape {
    Shape::new([flow[0], 2, flow[2] * UPSAMPLE, flow[3] * UPSAMPLE])
}

/// An uninitialised contiguous output on the device of `like`, and the launch grid for one thread per `threads`
fn output_for(like: &CubeTensor, shape: Shape, threads: usize) -> (CubeTensor, cubecl::CubeCount, CubeDim) {
    let dtype = like.dtype;
    let buffer = like.client.empty(shape.num_elements() * dtype.size());
    let out = CubeTensor::new_contiguous(like.client.clone(), like.device.clone(), shape, buffer, dtype);
    let cube_dim = CubeDim::new(&like.client, threads);
    let cube_count = calculate_cube_count_elemwise(&like.client, threads, cube_dim);
    (out, cube_count, cube_dim)
}

fn launch_corr_with_flow(f0: CubeTensor, f1: CubeTensor, flow: CubeTensor) -> CubeTensor {
    let (f0, f1, flow) = (into_contiguous(f0), into_contiguous(f1), into_contiguous(flow));
    let s = f0.meta.shape().clone();
    let m = 2 * CORR_RADIUS + 2;
    let taps_shape = Shape::new([s[0], m * m, s[2], s[3]]);
    let rows = s[0] * m * s[2] * s[3];
    let dtype = f0.dtype;
    let (taps, count, dim) = output_for(&f0, taps_shape, rows);
    corr_taps_kernel::launch(&taps.client, count, dim, f0.into_tensor_arg(), f1.into_tensor_arg(), flow.clone().into_tensor_arg(), taps.clone().into_tensor_arg(), CORR_RADIUS, dtype_to_storage_type(dtype));
    let shape = corr_shape(&s);
    let n = shape.num_elements();
    let (out, count, dim) = output_for(&taps, shape, n);
    corr_weight_kernel::launch(&out.client, count, dim, taps.into_tensor_arg(), flow.into_tensor_arg(), out.clone().into_tensor_arg(), CORR_RADIUS, dtype_to_storage_type(dtype));
    out
}

fn launch_local_corr_softmax(f0: CubeTensor, f1: CubeTensor) -> CubeTensor {
    let (f0, f1) = (into_contiguous(f0), into_contiguous(f1));
    let s = f0.meta.shape().clone();
    let n = s[0] * s[2] * s[3];
    let dtype = f0.dtype;
    let (out, count, dim) = output_for(&f0, flow_shape(&s), n);
    local_corr_softmax_kernel::launch(&out.client, count, dim, f0.into_tensor_arg(), f1.into_tensor_arg(), out.clone().into_tensor_arg(), CORR_RADIUS, dtype_to_storage_type(dtype));
    out
}

fn launch_local_propagation(q: CubeTensor, k: CubeTensor, flow: CubeTensor) -> CubeTensor {
    let (q, k, flow) = (into_contiguous(q), into_contiguous(k), into_contiguous(flow));
    let s = flow.meta.shape().clone();
    let n = s[0] * s[2] * s[3];
    let dtype = flow.dtype;
    let (out, count, dim) = output_for(&flow, s, n);
    local_propagation_kernel::launch(&out.client, count, dim, q.into_tensor_arg(), k.into_tensor_arg(), flow.into_tensor_arg(), out.clone().into_tensor_arg(), PROP_RADIUS, dtype_to_storage_type(dtype));
    out
}

/// The flow at the next finer level: flow [B, C, H, W] -> 2 * bilinear(flow) [B, C, 2H, 2W], corners aligned (unimatch:
/// F.interpolate(flow, scale_factor=2, mode='bilinear', align_corners=True) * 2). Not burn's interpolation: its autotune
/// picks between two cubek strategies, and every CI run that picked minimize_latency for this key (tuned at batch 1, the key
/// has no batch) came out wrong in the bidirectional pass (batch 2) from this stage on, differing from run to run as a part
/// of the output left unwritten would; every run that picked maximize_throughput passed. One thread per output value; the
/// source position o * (n - 1) / (2n - 1) taken as an exact fraction
#[cube(launch)]
fn flow_double_kernel<F: Float>(
    flow: &Tensor<F>,    // [B, C, H, W]
    out: &mut Tensor<F>, // [B, C, 2H, 2W]
    #[define(F)] _dtype: ElemType,
) {
    let idx = ABSOLUTE_POS as usize;
    let (bc_n, h, w) = (flow.shape(0) * flow.shape(1), flow.shape(2), flow.shape(3));
    let oh = h * 2usize;
    let ow = w * 2usize;
    if idx >= bc_n * oh * ow {
        terminate!();
    }
    let ox = idx % ow;
    let oy = (idx / ow) % oh;
    let bc = idx / (oh * ow);
    let ny = oy * (h - 1usize);
    let y0 = ny / (oh - 1usize);
    let ty = F::cast_from((ny - y0 * (oh - 1usize)) as u32) / F::cast_from((oh - 1usize) as u32);
    let nx = ox * (w - 1usize);
    let x0 = nx / (ow - 1usize);
    let tx = F::cast_from((nx - x0 * (ow - 1usize)) as u32) / F::cast_from((ow - 1usize) as u32);
    let y1 = if y0 + 1usize < h { y0 + 1usize } else { y0 };
    let x1 = if x0 + 1usize < w { x0 + 1usize } else { x0 };
    let base = bc * h * w;
    let one = F::new(1.0_f32);
    let top = flow[base + y0 * w + x0] * (one - tx) + flow[base + y0 * w + x1] * tx;
    let bottom = flow[base + y1 * w + x0] * (one - tx) + flow[base + y1 * w + x1] * tx;
    out[idx] = (top * (one - ty) + bottom * ty) * F::new(2.0_f32);
}

fn doubled_shape(flow: &Shape) -> Shape {
    Shape::new([flow[0], flow[1], flow[2] * 2, flow[3] * 2])
}

fn launch_flow_double(flow: CubeTensor) -> CubeTensor {
    let flow = into_contiguous(flow);
    let shape = doubled_shape(flow.meta.shape());
    let n = shape.num_elements();
    let dtype = flow.dtype;
    let (out, count, dim) = output_for(&flow, shape, n);
    flow_double_kernel::launch(&out.client, count, dim, flow.into_tensor_arg(), out.clone().into_tensor_arg(), dtype_to_storage_type(dtype));
    out
}

fn launch_convex_upsample(flow: CubeTensor, mask: CubeTensor) -> CubeTensor {
    let (flow, mask) = (into_contiguous(flow), into_contiguous(mask));
    let shape = upsampled_shape(flow.meta.shape());
    let n = shape[0] * shape[2] * shape[3];
    let dtype = flow.dtype;
    let (out, count, dim) = output_for(&flow, shape, n);
    convex_upsample_kernel::launch(&out.client, count, dim, flow.into_tensor_arg(), mask.into_tensor_arg(), out.clone().into_tensor_arg(), UPSAMPLE, dtype_to_storage_type(dtype));
    out
}

// ---------------------------------------------------------------- convolution as im2col + matmul

/// The columns of a convolution (F.unfold): out[n, (c * KH + ky) * KW + kx, oy * OW + ox] = x[n, c, oy * s - ph + ky,
/// ox * s - pw + kx], zero outside. Multiplied by the weights [Cout, C*KH*KW], they give the convolution. One thread per
/// output value.
#[cube(launch)]
fn im2col_kernel<F: Float>(
    x: &Tensor<F>,       // [N, C, H, W]
    out: &mut Tensor<F>, // [N, C*KH*KW, OH*OW]
    #[comptime] kh: usize,
    #[comptime] kw: usize,
    #[comptime] stride: usize,
    #[comptime] ph: usize,
    #[comptime] pw: usize,
    #[define(F)] _dtype: ElemType,
) {
    let idx = ABSOLUTE_POS as usize;
    let (n_n, k_n, l_n) = (out.shape(0), out.shape(1), out.shape(2));
    if idx >= n_n * k_n * l_n {
        terminate!();
    }
    let (c_n, h, w) = (x.shape(1), x.shape(2), x.shape(3));
    let ow = (w + 2 * pw - kw) / stride + 1;
    let l = idx % l_n;
    let k = (idx / l_n) % k_n;
    let n = idx / (l_n * k_n);
    let kx = k % kw;
    let ky = (k / kw) % kh;
    let c = k / (kw * kh);
    let iy = ((l / ow) * stride + ky) as i32 - ph as i32;
    let ix = ((l % ow) * stride + kx) as i32 - pw as i32;
    let mut v = F::new(0.0_f32);
    if iy >= 0 && iy < h as i32 && ix >= 0 && ix < w as i32 {
        v = x[((n * c_n + c) * h + iy as usize) * w + ix as usize];
    }
    out[idx] = v;
}

fn im2col_shape(x: &Shape, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> Shape {
    let oh = (x[2] + 2 * ph - kh) / stride + 1;
    let ow = (x[3] + 2 * pw - kw) / stride + 1;
    Shape::new([x[0], x[1] * kh * kw, oh * ow])
}

fn launch_im2col(x: CubeTensor, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> CubeTensor {
    let x = into_contiguous(x);
    let shape = im2col_shape(x.meta.shape(), kh, kw, stride, ph, pw);
    let n = shape.num_elements();
    let dtype = x.dtype;
    let (out, count, dim) = output_for(&x, shape, n);
    im2col_kernel::launch(&out.client, count, dim, x.into_tensor_arg(), out.clone().into_tensor_arg(), kh, kw, stride, ph, pw, dtype_to_storage_type(dtype));
    out
}

// ---------------------------------------------------------------- matmul for GPUs without f32 matrix units

/// The tiling of gemm_kernel (gemm_tiled; gemm itself uses gemm4 where it can): rows and columns of the block of the output a cube computes, its k step, and outputs per thread
/// per side (block / per threads per side, outputs strided by that so that neighbouring threads read neighbouring
/// values). Measured on the macOS CI GPU (conv / GRU / FFN shapes, GFLOPS), WGSL: 64 / 8 / 4 458 / 480 / 524, 64 / 16 / 4
/// 257 / 273 / 296, 32 / 16 / 2 340 / 405 / 472, 32 / 32 / 2 286 / 298 / 328, 128 / 8 / 8 143 / 188 / 192 (64 accumulators a
/// thread); MSL in the same order, 64 / 8 / 4 fastest at 578 / 576 / 633
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GemmTile {
    pub block: usize,
    pub kstep: usize,
    pub per: usize,
}

impl GemmTile {
    pub const fn threads(&self) -> usize {
        self.block / self.per
    }
}

pub const GEMM_DEFAULT: GemmTile = GemmTile { block: 64, kstep: 8, per: 4 };
/// The tiling of gemm4 that gemm uses. Measured on the macOS CI GPU (conv / GRU / FFN shapes, GFLOPS), WGSL: 4x4 569 / 577 /
/// 641 (gemm_kernel 64 / 8 / 4: 457 / 498 / 530), 8x4 321 / 405 / 449, 4x8 431 / 472 / 486, 8x8 244 / 329 / 332; MSL 4x4
/// 599 / 611 / 671, 4x8 fastest at 653 / 662 / 681 (MSL keeps burn's matmuls on its matrix units)
pub const GEMM4_DEFAULT: Gemm4Tile = Gemm4Tile { pm: 4, pn: 4, kstep: 8 };
/// gemm4_kernel and conv_gemm4_kernel launched unchecked: cubecl then does not clamp each buffer index (both kernels test
/// their own indices). On WGSL it also turns off naga's loop bounding (cubecl-wgpu sets force_loop_bounding only when
/// checked), which the WGSL text does not show. Off; the tests set it to measure and check both
pub static GEMM4_UNCHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Threads per side of every tiling used (the workgroup is GEMM_THREADS squared)
pub const GEMM_THREADS: usize = 16;

/// out[b] = a[b] @ b[b] (+ bias per row): a [Ba, M, K], b [Bb, K, N], out [B, M, N], a batch of 1 shared by all. Where the
/// device has no f32 matrix units, burn's own matmuls use 4 x 4 register tiles indexed at run time (cubek-matmul
/// instruction/register.rs, UNROLL = false), which reach 8-16% of the arithmetic peak on WGSL: here the register tile is
/// indexed at compile time (unrolled), with the tiles of a and b staged in shared memory, a transposed (k-major) and its
/// rows padded to 4 more than the block (every block used is a multiple of 32, so with 32 banks a row is 4 banks on from
/// the one above: the 32 stores of a 32-wide warp, k = idx % kstep and row = idx / kstep, fall in 32 banks for a k step of
/// exactly 8, in 16 pairs (2-way) for 16, 4-way for 32, and 2-way again for 4 or 2). Fixed order of the sums: the same result every run.
#[cube(launch)]
fn gemm_kernel<F: Float>(
    a: &Tensor<F>,       // [Ba, M, K]
    b: &Tensor<F>,       // [Bb, K, N]
    bias: &Tensor<F>,    // [M], read only with has_bias
    out: &mut Tensor<F>, // [B, M, N]
    #[comptime] has_bias: bool,
    #[comptime] block: usize,
    #[comptime] kstep: usize,
    #[comptime] per: usize,
    #[define(F)] _dtype: ElemType,
) {
    let (m_n, k_n, n_n) = (a.shape(1), a.shape(2), b.shape(2));
    let batch = CUBE_POS_Z as usize;
    let a_off = if a.shape(0) == 1usize { 0usize } else { batch * m_n * k_n };
    let b_off = if b.shape(0) == 1usize { 0usize } else { batch * k_n * n_n };
    let bm = block;
    let bk = kstep;
    let th = comptime!(block / per);
    let a_row = comptime!(block + 4);
    let loads = comptime!(kstep * per * per / block);
    let m0 = CUBE_POS_Y as usize * bm;
    let n0 = CUBE_POS_X as usize * bm;
    let tx = UNIT_POS_X as usize;
    let ty = UNIT_POS_Y as usize;
    let t = ty * th + tx;
    let zero = F::new(0.0_f32);
    let mut a_s = Shared::<[F]>::new_slice(comptime!(kstep * (block + 4)));
    let mut b_s = Shared::<[F]>::new_slice(comptime!(kstep * block));
    let mut acc = Array::<F>::new(comptime!(per * per));
    let mut bv = Array::<F>::new(per);
    #[unroll]
    for i in 0..per * per {
        acc[i] = zero;
    }
    let k_tiles = (k_n + bk - 1usize) / bk;
    for kt in 0..k_tiles {
        let k0 = kt * bk;
        // Every thread loads the same number of values of each tile; out of range: 0
        #[unroll]
        for e in 0..loads {
            let idx = t + e * th * th;
            let am = idx / bk;
            let ak = idx % bk;
            let mut va = zero;
            if m0 + am < m_n && k0 + ak < k_n {
                va = a[a_off + (m0 + am) * k_n + k0 + ak];
            }
            a_s[ak * a_row + am] = va;
            let bkk = idx / bm;
            let bn = idx % bm;
            let mut vb = zero;
            if k0 + bkk < k_n && n0 + bn < n_n {
                vb = b[b_off + (k0 + bkk) * n_n + n0 + bn];
            }
            b_s[bkk * bm + bn] = vb;
        }
        sync_cube();
        #[unroll]
        for kk in 0..bk {
            #[unroll]
            for j in 0..per {
                bv[j] = b_s[kk * bm + tx + j * th];
            }
            #[unroll]
            for i in 0..per {
                let av = a_s[kk * a_row + ty + i * th];
                #[unroll]
                for j in 0..per {
                    acc[i * per + j] += av * bv[j];
                }
            }
        }
        sync_cube();
    }
    #[unroll]
    for i in 0..per {
        let row = m0 + ty + i * th;
        if row < m_n {
            let add = if has_bias { bias[row] } else { zero };
            #[unroll]
            for j in 0..per {
                let col = n0 + tx + j * th;
                if col < n_n {
                    out[(batch * m_n + row) * n_n + col] = acc[i * per + j] + add;
                }
            }
        }
    }
}

fn gemm_shape(a: &Shape, b: &Shape) -> Shape {
    Shape::new([a[0].max(b[0]), a[1], b[2]])
}

fn launch_gemm(a: CubeTensor, b: CubeTensor, bias: Option<CubeTensor>, tile: GemmTile) -> CubeTensor {
    let (a, b) = (into_contiguous(a), into_contiguous(b));
    let (sa, sb) = (a.meta.shape().clone(), b.meta.shape().clone());
    assert!(sa[2] == sb[1], "gemm: inner dimensions {} and {}", sa[2], sb[1]);
    assert!(sa[0] == sb[0] || sa[0] == 1 || sb[0] == 1, "gemm: batches {} and {}", sa[0], sb[0]);
    let shape = gemm_shape(&sa, &sb);
    let (batch, m, n) = (shape[0], shape[1], shape[2]);
    let dtype = a.dtype;
    let buffer = a.client.empty(shape.num_elements() * dtype.size());
    let out = CubeTensor::new_contiguous(a.client.clone(), a.device.clone(), shape, buffer, dtype);
    let count = cubecl::CubeCount::Static(n.div_ceil(tile.block) as u32, m.div_ceil(tile.block) as u32, batch as u32);
    let dim = CubeDim::new_2d(tile.threads() as u32, tile.threads() as u32);
    let has_bias = bias.is_some();
    // Without a bias the kernel reads none: any tensor stands in
    let bias = into_contiguous(bias.unwrap_or_else(|| a.clone()));
    gemm_kernel::launch(&out.client, count, dim, a.into_tensor_arg(), b.into_tensor_arg(), bias.into_tensor_arg(), out.clone().into_tensor_arg(), has_bias, tile.block, tile.kstep, tile.per, dtype_to_storage_type(dtype));
    out
}

/// The tiling of gemm4: rows and columns of outputs per thread (PN a multiple of 4) and the k step; the cube is
/// GEMM_THREADS x GEMM_THREADS threads, its block 16 PM x 16 PN
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gemm4Tile {
    pub pm: usize,
    pub pn: usize,
    pub kstep: usize,
}

/// gemm with each thread's outputs contiguous (PM rows x PN columns) and b and out read and written 4 columns at a time
/// (Vector<F, 4>): per k step a thread reads PM values of a from shared memory (the same for the 16 threads of a row of
/// the cube) and PN / 4 vectors of b, for PM * PN multiply-adds, where gemm_kernel reads 8 scalars for 16. Each output
/// is summed over k in the same order as gemm_kernel. The columns of b and out [.., N] must be a multiple of 4
#[cube(launch, launch_unchecked)]
fn gemm4_kernel<F: Float, N: Size>(
    a: &Tensor<F>,                  // [Ba, M, K]
    b: &Tensor<Vector<F, N>>,       // [Bb, K, Nn]
    bias: &Tensor<F>,               // [M], read only with has_bias
    out: &mut Tensor<Vector<F, N>>, // [B, M, Nn]
    #[comptime] has_bias: bool,
    #[comptime] pm: usize,
    #[comptime] pn: usize,
    #[comptime] kstep: usize,
    #[define(F)] _dtype: ElemType,
) {
    let (m_n, k_n, n_n) = (a.shape(1), a.shape(2), b.shape(2));
    let nv = n_n / b.vector_size();
    let batch = CUBE_POS_Z as usize;
    let a_off = if a.shape(0) == 1usize { 0usize } else { batch * m_n * k_n };
    let b_off = if b.shape(0) == 1usize { 0usize } else { batch * k_n * nv };
    let bm = comptime!(16 * pm);
    let bnv = comptime!(4 * pn);
    let pv = comptime!(pn / 4);
    let a_row = comptime!(16 * pm + 4);
    let loads_a = comptime!(16 * pm * kstep / 256);
    let loads_b = comptime!((kstep * 4 * pn + 255) / 256);
    let tile_b = comptime!(kstep * 4 * pn);
    let m0 = CUBE_POS_Y as usize * bm;
    let n0 = CUBE_POS_X as usize * bnv;
    let tx = UNIT_POS_X as usize;
    let ty = UNIT_POS_Y as usize;
    let t = ty * 16usize + tx;
    let zero = F::new(0.0_f32);
    let zv = Vector::<F, N>::broadcast(zero);
    let mut a_s = Shared::<[F]>::new_slice(comptime!(kstep * (16 * pm + 4)));
    let mut b_s = Shared::<[Vector<F, N>]>::new_slice(comptime!(kstep * 4 * pn));
    let mut acc = Array::<Vector<F, N>>::new(comptime!(pm * pn / 4));
    let mut bvv = Array::<Vector<F, N>>::new(pv);
    #[unroll]
    for i in 0..pm * pv {
        acc[i] = zv;
    }
    let k_tiles = (k_n + kstep - 1usize) / kstep;
    for kt in 0..k_tiles {
        let k0 = kt * kstep;
        #[unroll]
        for e in 0..loads_a {
            let idx = t + e * 256usize;
            let am = idx / kstep;
            let ak = idx % kstep;
            let mut va = zero;
            if m0 + am < m_n && k0 + ak < k_n {
                va = a[a_off + (m0 + am) * k_n + k0 + ak];
            }
            a_s[ak * a_row + am] = va;
        }
        #[unroll]
        for e in 0..loads_b {
            let idx = t + e * 256usize;
            if idx < tile_b {
                let bkk = idx / bnv;
                let bc = idx % bnv;
                let mut vb = zv;
                if k0 + bkk < k_n && n0 + bc < nv {
                    vb = b[b_off + (k0 + bkk) * nv + n0 + bc];
                }
                b_s[bkk * bnv + bc] = vb;
            }
        }
        sync_cube();
        #[unroll]
        for kk in 0..kstep {
            #[unroll]
            for v in 0..pv {
                bvv[v] = b_s[kk * bnv + tx * pv + v];
            }
            #[unroll]
            for i in 0..pm {
                let av = Vector::<F, N>::broadcast(a_s[kk * a_row + ty * pm + i]);
                #[unroll]
                for v in 0..pv {
                    acc[i * pv + v] += av * bvv[v];
                }
            }
        }
        sync_cube();
    }
    #[unroll]
    for i in 0..pm {
        let row = m0 + ty * pm + i;
        if row < m_n {
            let add = if has_bias { bias[row] } else { zero };
            let addv = Vector::<F, N>::broadcast(add);
            #[unroll]
            for v in 0..pv {
                let col = n0 + tx * pv + v;
                if col < nv {
                    out[(batch * m_n + row) * nv + col] = acc[i * pv + v] + addv;
                }
            }
        }
    }
}

fn launch_gemm4(a: CubeTensor, b: CubeTensor, bias: Option<CubeTensor>, tile: Gemm4Tile) -> CubeTensor {
    let (a, b) = (into_contiguous(a), into_contiguous(b));
    let (sa, sb) = (a.meta.shape().clone(), b.meta.shape().clone());
    assert!(sa[2] == sb[1], "gemm4: inner dimensions {} and {}", sa[2], sb[1]);
    assert!(sa[0] == sb[0] || sa[0] == 1 || sb[0] == 1, "gemm4: batches {} and {}", sa[0], sb[0]);
    assert!(sb[2] % 4 == 0 && tile.pn % 4 == 0, "gemm4: {} columns, tiling {tile:?}", sb[2]);
    // gemm4_kernel indexes its 16 x 16 threads and 4-wide vectors with literals
    assert_eq!(GEMM_THREADS, 16, "gemm4: threads per side");
    let shape = gemm_shape(&sa, &sb);
    let (batch, m, n) = (shape[0], shape[1], shape[2]);
    let dtype = a.dtype;
    let buffer = a.client.empty(shape.num_elements() * dtype.size());
    let out = CubeTensor::new_contiguous(a.client.clone(), a.device.clone(), shape, buffer, dtype);
    let count = cubecl::CubeCount::Static((n / 4).div_ceil(4 * tile.pn) as u32, m.div_ceil(16 * tile.pm) as u32, batch as u32);
    let dim = CubeDim::new_2d(GEMM_THREADS as u32, GEMM_THREADS as u32);
    if let Some(bias) = &bias {
        assert_eq!(bias.meta.shape().num_elements(), m, "gemm4: a bias per row");
    }
    let has_bias = bias.is_some();
    let bias = into_contiguous(bias.unwrap_or_else(|| a.clone()));
    let (a, b, bias, o) = (a.into_tensor_arg(), b.into_tensor_arg(), bias.into_tensor_arg(), out.clone().into_tensor_arg());
    if GEMM4_UNCHECKED.load(std::sync::atomic::Ordering::Relaxed) {
        // SAFETY: every read and write of gemm4_kernel is inside its own tests of rows, k and columns, the bias read only
        // with has_bias, the batch offsets 0 for a batch of 1 (the asserts above)
        unsafe { gemm4_kernel::launch_unchecked(&out.client, count, dim, 4usize, a, b, bias, o, has_bias, tile.pm, tile.pn, tile.kstep, dtype_to_storage_type(dtype)) };
    } else {
        gemm4_kernel::launch(&out.client, count, dim, 4usize, a, b, bias, o, has_bias, tile.pm, tile.pn, tile.kstep, dtype_to_storage_type(dtype));
    }
    out
}

/// Implicit GEMM convolution: gemm4_kernel with the tile of b gathered from x [N, C, H, W] (the columns of F.unfold made
/// on the fly, zero outside of the map) instead of read from an im2col buffer, which for the 3x3 convolutions at 1/4 and
/// 1/2 is 9 times the input, written and read once per convolution. a: the weight [1, Co, K], K = C KH KW in im2col order;
/// out [N, Co, OH*OW], OH*OW a multiple of 4. Each output is summed over k in the same order as im2col + gemm4. The sizes
/// are compile-time constants, one kernel per shape (the batch stays a launch dimension), so that the kernel reads none
/// of cubecl's metadata: naga binds that buffer in device memory where cubecl's own MSL takes it as constant, and with
/// wgpu-hal patched to bind it as constant this kernel took 17.2 against 27.6 ms on WGSL (the 3x3 256->192 at 1/4, the
/// macOS 15 CI GPU)
#[cube(launch, launch_unchecked)]
fn conv_gemm4_kernel<F: Float, N: Size>(
    a: &Tensor<F>,                  // [1, Co, K]
    x: &Tensor<F>,                  // [N, C, H, W]
    out: &mut Tensor<Vector<F, N>>, // [N, Co, OH*OW]
    #[comptime] kh: usize,
    #[comptime] kw: usize,
    #[comptime] stride: usize,
    #[comptime] ph: usize,
    #[comptime] pw: usize,
    #[comptime] pm: usize,
    #[comptime] pn: usize,
    #[comptime] kstep: usize,
    #[comptime] m_n: usize,
    #[comptime] k_n: usize,
    #[comptime] c_n: usize,
    #[comptime] h: usize,
    #[comptime] w: usize,
    #[comptime] nv: usize,
    #[define(F)] _dtype: ElemType,
) {
    let ow = comptime!((w + 2 * pw - kw) / stride + 1);
    let batch = CUBE_POS_Z as usize;
    let bm = comptime!(16 * pm);
    let bnv = comptime!(4 * pn);
    let pv = comptime!(pn / 4);
    let a_row = comptime!(16 * pm + 4);
    let loads_a = comptime!(16 * pm * kstep / 256);
    let tile_b = comptime!(kstep * 4 * pn);
    let khw = comptime!(kh * kw);
    let m0 = CUBE_POS_Y as usize * bm;
    let n0 = CUBE_POS_X as usize * bnv;
    let tx = UNIT_POS_X as usize;
    let ty = UNIT_POS_Y as usize;
    let t = ty * 16usize + tx;
    let zero = F::new(0.0_f32);
    let zv = Vector::<F, N>::broadcast(zero);
    let mut a_s = Shared::<[F]>::new_slice(comptime!(kstep * (16 * pm + 4)));
    let mut b_s = Shared::<[Vector<F, N>]>::new_slice(comptime!(kstep * 4 * pn));
    let mut acc = Array::<Vector<F, N>>::new(comptime!(pm * pn / 4));
    let mut bvv = Array::<Vector<F, N>>::new(pv);
    #[unroll]
    for i in 0..pm * pv {
        acc[i] = zv;
    }
    // This thread's load of b: row bkk of the tile and the 4 output positions of vector column bc, the same at every k
    // step (tile_b is at most the 256 threads); the top left of their windows in x. Unsigned only: above the map the
    // subtraction wraps around and adding ky or kx wraps back, so one compare per axis tests 0 <= index < size. naga
    // wraps every signed add, subtract and multiply in as_type casts on Metal (this kernel had 16); the MSL of cubecl does not
    let bkk = t / bnv;
    let bc = t % bnv;
    let mut iy0 = Array::<usize>::new(4usize);
    let mut ix0 = Array::<usize>::new(4usize);
    #[unroll]
    for j in 0..4 {
        let p = (n0 + bc) * 4usize + j;
        iy0[j] = (p / ow) * stride - ph;
        ix0[j] = (p % ow) * stride - pw;
    }
    let x_base = batch * c_n * h * w;
    let k_tiles = comptime!(k_n.div_ceil(kstep));
    for kt in 0..k_tiles {
        let k0 = kt * kstep;
        #[unroll]
        for e in 0..loads_a {
            let idx = t + e * 256usize;
            let am = idx / kstep;
            let ak = idx % kstep;
            let mut va = zero;
            if m0 + am < m_n {
                if k0 + ak < k_n {
                    va = a[(m0 + am) * k_n + k0 + ak];
                }
            }
            a_s[ak * a_row + am] = va;
        }
        if t < tile_b {
            let mut vb = zv;
            let k = k0 + bkk;
            // Nested tests, no && (naga turns each && of WGSL into branches on a local bool; removing them alone did
            // not change the time)
            if k < k_n {
                if n0 + bc < nv {
                    let c = k / khw;
                    let r = k % khw;
                    let ky = r / kw;
                    let kx = r % kw;
                    let base = x_base + c * h * w;
                    #[unroll]
                    for j in 0..4 {
                        let iy = iy0[j] + ky;
                        let ix = ix0[j] + kx;
                        if iy < h {
                            if ix < w {
                                vb.insert_dynamic(j, x[base + iy * w + ix]);
                            }
                        }
                    }
                }
            }
            b_s[bkk * bnv + bc] = vb;
        }
        sync_cube();
        #[unroll]
        for kk in 0..kstep {
            #[unroll]
            for v in 0..pv {
                bvv[v] = b_s[kk * bnv + tx * pv + v];
            }
            #[unroll]
            for i in 0..pm {
                let av = Vector::<F, N>::broadcast(a_s[kk * a_row + ty * pm + i]);
                #[unroll]
                for v in 0..pv {
                    acc[i * pv + v] += av * bvv[v];
                }
            }
        }
        sync_cube();
    }
    #[unroll]
    for i in 0..pm {
        let row = m0 + ty * pm + i;
        if row < m_n {
            #[unroll]
            for v in 0..pv {
                let col = n0 + tx * pv + v;
                if col < nv {
                    out[(batch * m_n + row) * nv + col] = acc[i * pv + v];
                }
            }
        }
    }
}

fn conv_gemm_shape(w: &Shape, x: &Shape, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> Shape {
    let oh = (x[2] + 2 * ph - kh) / stride + 1;
    let ow = (x[3] + 2 * pw - kw) / stride + 1;
    Shape::new([x[0], w[1], oh * ow])
}

#[allow(clippy::too_many_arguments)]
fn launch_conv_gemm(w: CubeTensor, x: CubeTensor, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> CubeTensor {
    let (w, x) = (into_contiguous(w), into_contiguous(x));
    let (sw, sx) = (w.meta.shape().clone(), x.meta.shape().clone());
    assert!(sw[0] == 1 && sw[2] == sx[1] * kh * kw, "conv_gemm: weight {sw:?} for x {sx:?} and a {kh}x{kw} kernel");
    let shape = conv_gemm_shape(&sw, &sx, kh, kw, stride, ph, pw);
    let tile = GEMM4_DEFAULT;
    assert!(shape[2] % 4 == 0 && tile.kstep * 4 * tile.pn <= 256 && GEMM_THREADS == 16, "conv_gemm: {shape:?}, tiling {tile:?}");
    let (batch, m, n) = (shape[0], shape[1], shape[2]);
    // The sizes the kernel takes as constants
    let (k, c, h, wd) = (sw[2], sx[1], sx[2], sx[3]);
    let dtype = x.dtype;
    let buffer = x.client.empty(shape.num_elements() * dtype.size());
    let out = CubeTensor::new_contiguous(x.client.clone(), x.device.clone(), shape, buffer, dtype);
    let count = cubecl::CubeCount::Static((n / 4).div_ceil(4 * tile.pn) as u32, m.div_ceil(16 * tile.pm) as u32, batch as u32);
    let dim = CubeDim::new_2d(GEMM_THREADS as u32, GEMM_THREADS as u32);
    let (w, x, o) = (w.into_tensor_arg(), x.into_tensor_arg(), out.clone().into_tensor_arg());
    if GEMM4_UNCHECKED.load(std::sync::atomic::Ordering::Relaxed) {
        // SAFETY: the weight is read inside the tests of rows and k, x inside k < C KH KW and the map (padding read as
        // zero), the output written inside the tests of rows and columns (the asserts above)
        unsafe { conv_gemm4_kernel::launch_unchecked(&out.client, count, dim, 4usize, w, x, o, kh, kw, stride, ph, pw, tile.pm, tile.pn, tile.kstep, m, k, c, h, wd, n / 4, dtype_to_storage_type(dtype)) };
    } else {
        conv_gemm4_kernel::launch(&out.client, count, dim, 4usize, w, x, o, kh, kw, stride, ph, pw, tile.pm, tile.pn, tile.kstep, m, k, c, h, wd, n / 4, dtype_to_storage_type(dtype));
    }
    out
}

// ---------------------------------------------------------------- split-window attention and small convolutions

/// The row of q, k, v [B, H*W, C] (as [B*H*W, C]) at position p of window bw of split-window attention (unimatch
/// split_feature: windows in row-major order per image, positions row-major in a window), the map rolled by minus half a
/// window first when shifted (torch.roll(x, -s): out[i] = in[i + s]). The output goes back to the same row (merge_splits,
/// then the roll by plus half a window), so neither the window layout nor the roll is made in memory
#[cube]
fn window_token(bw: usize, p: usize, #[comptime] h: usize, #[comptime] w: usize, #[comptime] splits: usize, #[comptime] shift: bool) -> usize {
    let wh = comptime!(h / splits);
    let ww = comptime!(w / splits);
    let kk = comptime!(splits * splits);
    let b = bw / kk;
    let wi = bw % kk;
    let mut y = (wi / splits) * wh + p / ww;
    let mut x = (wi % splits) * ww + p % ww;
    if shift {
        y = (y + comptime!(wh / 2)) % h;
        x = (x + comptime!(ww / 2)) % w;
    }
    (b * h + y) * w + x
}

/// generate_shift_window_attn_mask: the region (0, 1, 2) of coordinate v of a side n split into windows of `win`, at the
/// borders of the windows shifted by half a window
#[cube]
fn shift_region(v: usize, #[comptime] n: usize, #[comptime] win: usize) -> usize {
    let mut r = 2usize;
    if v < comptime!(n - win) {
        r = 0usize;
    } else if v < comptime!(n - win / 2) {
        r = 1usize;
    }
    r
}

/// The label of position p of window wi in the shifted map (the attention mask is -100 between different labels)
#[cube]
fn shift_label(wi: usize, p: usize, #[comptime] h: usize, #[comptime] w: usize, #[comptime] splits: usize) -> usize {
    let wh = comptime!(h / splits);
    let ww = comptime!(w / splits);
    shift_region((wi / splits) * wh + p / ww, h, wh) * 3usize + shift_region((wi % splits) * ww + p % ww, w, ww)
}

/// The two matmuls of split-window attention, one cube per block of one window (the tiling and inner loop of gemm_kernel):
/// with `scores`, q @ k^T / sqrt(C) of each window [Bw, L, L] (k read transposed: the channel index fastest, so that
/// neighbouring threads read neighbouring values of one key; staged with rows padded by 4 as a is); otherwise
/// probabilities [Bw, L, L] @ v of each window, written to the rows of the positions [B*H*W, C]
#[cube(launch)]
fn window_attn_gemm_kernel<F: Float>(
    a: &Tensor<F>,       // scores: q [B, H*W, C]; else: probabilities [Bw, L, L]
    b: &Tensor<F>,       // scores: k [B, H*W, C]; else: v [B, H*W, C]
    out: &mut Tensor<F>, // scores: [Bw, L, L]; else: [B, H*W, C]
    #[comptime] scores: bool,
    #[comptime] shift: bool,
    #[comptime] h: usize,
    #[comptime] w: usize,
    #[comptime] splits: usize,
    #[comptime] block: usize,
    #[comptime] kstep: usize,
    #[comptime] per: usize,
    #[define(F)] _dtype: ElemType,
) {
    let c_n = b.shape(2);
    let l_n = if scores { out.shape(1) } else { a.shape(1) };
    let k_n = if scores { c_n } else { l_n };
    let n_n = if scores { l_n } else { c_n };
    let bw = CUBE_POS_Z as usize;
    let bm = block;
    let bk = kstep;
    let th = comptime!(block / per);
    let row_s = comptime!(block + 4);
    let loads = comptime!(kstep * per * per / block);
    let m0 = CUBE_POS_Y as usize * bm;
    let n0 = CUBE_POS_X as usize * bm;
    let tx = UNIT_POS_X as usize;
    let ty = UNIT_POS_Y as usize;
    let t = ty * th + tx;
    let zero = F::new(0.0_f32);
    let mut a_s = Shared::<[F]>::new_slice(comptime!(kstep * (block + 4)));
    let mut b_s = Shared::<[F]>::new_slice(comptime!(kstep * (block + 4)));
    let mut acc = Array::<F>::new(comptime!(per * per));
    let mut bv = Array::<F>::new(per);
    #[unroll]
    for i in 0..per * per {
        acc[i] = zero;
    }
    let k_tiles = (k_n + bk - 1usize) / bk;
    for kt in 0..k_tiles {
        let k0 = kt * bk;
        #[unroll]
        for e in 0..loads {
            let idx = t + e * th * th;
            let am = idx / bk;
            let ak = idx % bk;
            let mut va = zero;
            if m0 + am < l_n && k0 + ak < k_n {
                let row = if scores { window_token(bw, m0 + am, h, w, splits, shift) } else { bw * l_n + m0 + am };
                va = a[row * k_n + k0 + ak];
            }
            a_s[ak * row_s + am] = va;
            let mut vb = zero;
            if scores {
                let bn = idx / bk;
                let bkk = idx % bk;
                if k0 + bkk < k_n && n0 + bn < n_n {
                    vb = b[window_token(bw, n0 + bn, h, w, splits, shift) * c_n + k0 + bkk];
                }
                b_s[bkk * row_s + bn] = vb;
            } else {
                let bkk = idx / bm;
                let bn = idx % bm;
                if k0 + bkk < k_n && n0 + bn < n_n {
                    vb = b[window_token(bw, k0 + bkk, h, w, splits, shift) * c_n + n0 + bn];
                }
                b_s[bkk * row_s + bn] = vb;
            }
        }
        sync_cube();
        #[unroll]
        for kk in 0..bk {
            #[unroll]
            for j in 0..per {
                bv[j] = b_s[kk * row_s + tx + j * th];
            }
            #[unroll]
            for i in 0..per {
                let av = a_s[kk * row_s + ty + i * th];
                #[unroll]
                for j in 0..per {
                    acc[i * per + j] += av * bv[j];
                }
            }
        }
        sync_cube();
    }
    let scale = F::sqrt(F::cast_from(c_n as u32));
    #[unroll]
    for i in 0..per {
        let row = m0 + ty + i * th;
        if row < l_n {
            // The output row: of the window's scores, or of the position (both n_n wide)
            let orow = if scores { bw * l_n + row } else { window_token(bw, row, h, w, splits, shift) };
            #[unroll]
            for j in 0..per {
                let col = n0 + tx + j * th;
                if col < n_n {
                    let v = if scores { acc[i * per + j] / scale } else { acc[i * per + j] };
                    out[orow * n_n + col] = v;
                }
            }
        }
    }
}

/// Threads of a cube of window_softmax_kernel (its reductions halve 64 in 6 steps)
pub const ATTN_SOFTMAX_THREADS: usize = 64;

/// The softmax of the attention scores [Bw, L, L] over the keys, in place, with the -100 between positions of different
/// regions of the shifted map added first (as unimatch adds the mask to the scaled scores). One cube per row; the row
/// maximum and sum by a tree in shared memory (fixed order: the same result every run; no plane operations, which
/// lavapipe and WGSL on Metal may not have)
#[cube(launch)]
fn window_softmax_kernel<F: Float>(
    s: &mut Tensor<F>, // [Bw, L, L]
    #[comptime] shift: bool,
    #[comptime] h: usize,
    #[comptime] w: usize,
    #[comptime] splits: usize,
    #[define(F)] _dtype: ElemType,
) {
    let l_n = s.shape(2);
    let row = CUBE_POS_Y as usize * l_n + CUBE_POS_X as usize;
    let t = UNIT_POS_X as usize;
    let threads = comptime!(ATTN_SOFTMAX_THREADS);
    let wi = (row / l_n) % comptime!(splits * splits);
    let p = row % l_n;
    let base = row * l_n;
    let label_p = shift_label(wi, p, h, w, splits);
    let per_thread = (l_n + threads - 1usize) / threads;
    let mut red = Shared::<[F]>::new_slice(threads);
    let minus = F::new(-100.0_f32);
    let mut m = F::new(-1e30_f32);
    for e in 0..per_thread {
        let j = t + e * threads;
        if j < l_n {
            let mut v = s[base + j];
            if shift {
                if shift_label(wi, j, h, w, splits) != label_p {
                    v += minus;
                }
            }
            m = F::max(m, v);
        }
    }
    red[t] = m;
    sync_cube();
    let mut half = 32usize;
    for _ in 0..6usize {
        if t < half {
            red[t] = F::max(red[t], red[t + half]);
        }
        sync_cube();
        half = half / 2usize;
    }
    let m = red[0];
    sync_cube();
    let mut sum = F::new(0.0_f32);
    for e in 0..per_thread {
        let j = t + e * threads;
        if j < l_n {
            let mut v = s[base + j];
            if shift {
                if shift_label(wi, j, h, w, splits) != label_p {
                    v += minus;
                }
            }
            let ex = F::exp(v - m);
            s[base + j] = ex;
            sum += ex;
        }
    }
    red[t] = sum;
    sync_cube();
    let mut half = 32usize;
    for _ in 0..6usize {
        if t < half {
            red[t] = red[t] + red[t + half];
        }
        sync_cube();
        half = half / 2usize;
    }
    let total = red[0];
    for e in 0..per_thread {
        let j = t + e * threads;
        if j < l_n {
            s[base + j] = s[base + j] / total;
        }
    }
}

/// The masked softmax of split-window attention over the scores [Bw, L, L] of an h x w map in splits x splits windows,
/// in place (window_softmax_kernel); also between the two matmuls of coop.rs's window attention
pub(crate) fn launch_window_softmax(scores: &CubeTensor, h: usize, w: usize, splits: usize, shift: bool) {
    let s = scores.meta.shape();
    let l = (h / splits) * (w / splits);
    assert!(s[1] == l && s[2] == l && s[0] % (splits * splits) == 0, "window_softmax: scores {s:?} for {h}x{w} in {splits}x{splits} windows");
    assert_eq!(ATTN_SOFTMAX_THREADS, 64, "window_softmax_kernel reduces 64 values");
    let count = cubecl::CubeCount::Static(l as u32, s[0] as u32, 1);
    window_softmax_kernel::launch(&scores.client, count, CubeDim::new_2d(ATTN_SOFTMAX_THREADS as u32, 1), scores.clone().into_tensor_arg(), shift, h, w, splits, dtype_to_storage_type(scores.dtype));
}

fn launch_window_attention(q: CubeTensor, k: CubeTensor, v: CubeTensor, h: usize, w: usize, splits: usize, shift: bool) -> CubeTensor {
    let (q, k, v) = (into_contiguous(q), into_contiguous(k), into_contiguous(v));
    let s = q.meta.shape().clone();
    assert!(s[1] == h * w && h % splits == 0 && w % splits == 0 && s[2] > 0, "window_attention: {s:?} for {h}x{w} in {splits}x{splits} windows");
    assert!(k.meta.shape() == &s && v.meta.shape() == &s, "window_attention: q {s:?}, k {:?}, v {:?}", k.meta.shape(), v.meta.shape());
    assert_eq!(ATTN_SOFTMAX_THREADS, 64, "window_softmax_kernel reduces 64 values");
    let (b_n, c_n) = (s[0], s[2]);
    let l = (h / splits) * (w / splits);
    let bw = b_n * splits * splits;
    let dtype = q.dtype;
    let tile = GEMM_DEFAULT;
    let dim = CubeDim::new_2d(tile.threads() as u32, tile.threads() as u32);
    let scores_shape = Shape::new([bw, l, l]);
    let buffer = q.client.empty(scores_shape.num_elements() * dtype.size());
    let scores = CubeTensor::new_contiguous(q.client.clone(), q.device.clone(), scores_shape, buffer, dtype);
    let count = cubecl::CubeCount::Static(l.div_ceil(tile.block) as u32, l.div_ceil(tile.block) as u32, bw as u32);
    window_attn_gemm_kernel::launch(&scores.client, count, dim, q.clone().into_tensor_arg(), k.into_tensor_arg(), scores.clone().into_tensor_arg(), true, shift, h, w, splits, tile.block, tile.kstep, tile.per, dtype_to_storage_type(dtype));
    launch_window_softmax(&scores, h, w, splits, shift);
    let buffer = q.client.empty(s.num_elements() * dtype.size());
    let out = CubeTensor::new_contiguous(q.client.clone(), q.device.clone(), s, buffer, dtype);
    let count = cubecl::CubeCount::Static(c_n.div_ceil(tile.block) as u32, l.div_ceil(tile.block) as u32, bw as u32);
    window_attn_gemm_kernel::launch(&out.client, count, dim, scores.into_tensor_arg(), v.into_tensor_arg(), out.clone().into_tensor_arg(), false, shift, h, w, splits, tile.block, tile.kstep, tile.per, dtype_to_storage_type(dtype));
    out
}

/// The threads of a cube of softmax_flow_kernel (one cube per row)
pub const SOFTMAX_FLOW_THREADS: usize = 256;

/// softmax(scores, over the last axis) @ v: scores [B, N, L], v [Bv, L, 2] (Bv 1, shared by the batch, or B) -> [B, N, 2],
/// what global matching (the softmax of the correlation times the grid of positions) and global propagation (of the
/// attention scores times the flow) compute with burn's softmax and a matmul of 2 columns, both autotuned at the first
/// pass. One cube per row: the row maximum, then the sum of exp(s - max) and its products with the 2 values of each key,
/// each by a tree in shared memory in a fixed order (the same result every run; no plane operations)
#[cube(launch)]
fn softmax_flow_kernel<F: Float>(
    scores: &Tensor<F>,  // [B, N, L]
    v: &Tensor<F>,       // [Bv, L, 2]
    out: &mut Tensor<F>, // [B, N, 2]
    #[define(F)] _dtype: ElemType,
) {
    let (n_n, l_n) = (scores.shape(1), scores.shape(2));
    let bi = CUBE_POS_Y as usize;
    let row = bi * n_n + CUBE_POS_X as usize;
    let t = UNIT_POS_X as usize;
    let threads = comptime!(SOFTMAX_FLOW_THREADS);
    let base = row * l_n;
    let mut vb = bi;
    if v.shape(0) == 1usize {
        vb = 0usize;
    }
    let vbase = vb * l_n * 2usize;
    let per_thread = (l_n + threads - 1usize) / threads;
    let mut red = Shared::<[F]>::new_slice(comptime!(3 * SOFTMAX_FLOW_THREADS));
    let mut m = F::new(-1e30_f32);
    for e in 0..per_thread {
        let j = t + e * threads;
        if j < l_n {
            m = F::max(m, scores[base + j]);
        }
    }
    red[t] = m;
    sync_cube();
    let mut half = 128usize;
    for _ in 0..8usize {
        if t < half {
            red[t] = F::max(red[t], red[t + half]);
        }
        sync_cube();
        half = half / 2usize;
    }
    let m = red[0];
    sync_cube();
    let zero = F::new(0.0_f32);
    let mut sum = zero;
    let mut a0 = zero;
    let mut a1 = zero;
    for e in 0..per_thread {
        let j = t + e * threads;
        if j < l_n {
            let ex = F::exp(scores[base + j] - m);
            sum += ex;
            a0 += ex * v[vbase + j * 2usize];
            a1 += ex * v[vbase + j * 2usize + 1usize];
        }
    }
    red[t] = sum;
    red[t + threads] = a0;
    red[t + 2usize * threads] = a1;
    sync_cube();
    let mut half = 128usize;
    for _ in 0..8usize {
        if t < half {
            red[t] = red[t] + red[t + half];
            red[t + threads] = red[t + threads] + red[t + threads + half];
            red[t + 2usize * threads] = red[t + 2usize * threads] + red[t + 2usize * threads + half];
        }
        sync_cube();
        half = half / 2usize;
    }
    if t == 0usize {
        out[row * 2usize] = red[threads] / red[0];
        out[row * 2usize + 1usize] = red[2usize * threads] / red[0];
    }
}

fn softmax_flow_shape(scores: &Shape, _v: &Shape) -> Shape {
    Shape::new([scores[0], scores[1], 2])
}

fn launch_softmax_flow(scores: CubeTensor, v: CubeTensor) -> CubeTensor {
    let (scores, v) = (into_contiguous(scores), into_contiguous(v));
    let (s, vs) = (scores.meta.shape().clone(), v.meta.shape().clone());
    assert!(s.num_dims() == 3 && s[0] > 0 && s[1] > 0 && s[2] > 0, "softmax_flow: scores {s:?}");
    assert!(vs.num_dims() == 3 && (vs[0] == 1 || vs[0] == s[0]) && vs[1] == s[2] && vs[2] == 2, "softmax_flow: v {vs:?} for scores {s:?}");
    // The tree of softmax_flow_kernel sums 256 threads in 8 steps; rows as cubes in x, the batch in y
    assert!(SOFTMAX_FLOW_THREADS == 256 && s[1] <= 65535 && s[0] <= 65535, "softmax_flow: {s:?}");
    let dtype = scores.dtype;
    let shape = softmax_flow_shape(&s, &vs);
    let buffer = scores.client.empty(shape.num_elements() * dtype.size());
    let out = CubeTensor::new_contiguous(scores.client.clone(), scores.device.clone(), shape, buffer, dtype);
    let count = cubecl::CubeCount::Static(s[1] as u32, s[0] as u32, 1);
    softmax_flow_kernel::launch(&out.client, count, CubeDim::new_2d(SOFTMAX_FLOW_THREADS as u32, 1), scores.into_tensor_arg(), v.into_tensor_arg(), out.clone().into_tensor_arg(), dtype_to_storage_type(dtype));
    out
}

/// The lanes of a row in layer_norm_kernel, and the threads of its cubes (4 rows each)
pub const LN_LANES: usize = 32;
const LN_THREADS: usize = 128;
/// The epsilon of nn.LayerNorm and of burn's LayerNormConfig by default, which the model's LayerNorms use (burn keeps a
/// LayerNorm's own private)
pub const LN_EPS: f32 = 1e-5;

/// LayerNorm over the last dimension of x [R, C] as burn's layer_norm computes it (gamma * (x - mean) / sqrt(var + eps)
/// plus beta, the variance biased), plus res [R, C] with has_res: one launch for what burn runs as two reductions and an
/// elementwise kernel (and the residual add after them). 32 threads per row (coalesced reads), 4 rows per cube, the values
/// of a thread kept in registers between the passes; the sums by a tree in shared memory in a fixed order (no plane
/// operations, which lavapipe and WGSL on Metal may not have)
#[cube(launch)]
fn layer_norm_kernel<F: Float>(
    x: &Tensor<F>,       // [R, C]
    gamma: &Tensor<F>,   // [C]
    beta: &Tensor<F>,    // [C]
    res: &Tensor<F>,     // [R, C], read only with has_res
    out: &mut Tensor<F>, // [R, C]
    #[comptime] c: usize,
    #[comptime] has_res: bool,
    #[define(F)] _dtype: ElemType,
) {
    let rows = x.shape(0);
    let t = UNIT_POS_X as usize;
    let lane = t % comptime!(LN_LANES);
    let seg = t - lane;
    // Every thread of a cube reaches every barrier: the rows past the end only skip their loads and stores
    let row = ABSOLUTE_POS as usize / comptime!(LN_LANES);
    let live = row < rows;
    let base = row * c;
    let per = comptime!(c.div_ceil(LN_LANES));
    let zero = F::new(0.0_f32);
    let c_f = F::cast_from(x.shape(1) as u32);
    let mut red = Shared::<[F]>::new_slice(comptime!(LN_THREADS));
    let mut v = Array::<F>::new(per);
    let mut sum = zero;
    #[unroll]
    for e in 0..per {
        let j = lane + e * comptime!(LN_LANES);
        let mut a = zero;
        if live && j < c {
            a = x[base + j];
        }
        v[e] = a;
        sum += a;
    }
    red[t] = sum;
    sync_cube();
    let mut half = 16usize;
    for _ in 0..5usize {
        if lane < half {
            red[t] = red[t] + red[t + half];
        }
        sync_cube();
        half = half / 2usize;
    }
    let mean = red[seg] / c_f;
    sync_cube();
    let mut sq = zero;
    #[unroll]
    for e in 0..per {
        let j = lane + e * comptime!(LN_LANES);
        if j < c {
            let dv = v[e] - mean;
            sq += dv * dv;
        }
    }
    red[t] = sq;
    sync_cube();
    let mut half = 16usize;
    for _ in 0..5usize {
        if lane < half {
            red[t] = red[t] + red[t + half];
        }
        sync_cube();
        half = half / 2usize;
    }
    let inv = F::new(1.0_f32) / F::sqrt(red[seg] / c_f + F::new(comptime!(LN_EPS)));
    #[unroll]
    for e in 0..per {
        let j = lane + e * comptime!(LN_LANES);
        if live && j < c {
            let mut y = (v[e] - mean) * inv * gamma[j] + beta[j];
            if has_res {
                y += res[base + j];
            }
            out[base + j] = y;
        }
    }
}

fn launch_layer_norm(x: CubeTensor, gamma: CubeTensor, beta: CubeTensor, res: Option<CubeTensor>) -> CubeTensor {
    let (x, gamma, beta) = (into_contiguous(x), into_contiguous(gamma), into_contiguous(beta));
    let s = x.meta.shape().clone();
    assert!(s.num_dims() == 2 && s[1] > 0, "layer_norm: x {s:?}");
    let (rows, c) = (s[0], s[1]);
    assert!(gamma.meta.shape().num_elements() == c && beta.meta.shape().num_elements() == c, "layer_norm: gamma {:?}, beta {:?} for {c} channels", gamma.meta.shape(), beta.meta.shape());
    // The tree of layer_norm_kernel sums 32 lanes in 5 steps
    assert!(LN_LANES == 32 && LN_THREADS % LN_LANES == 0, "layer_norm_kernel: lanes");
    let has_res = res.is_some();
    let res = match res {
        Some(r) => {
            let r = into_contiguous(r);
            assert!(r.meta.shape() == &s, "layer_norm: residual {:?} for x {s:?}", r.meta.shape());
            r
        }
        None => x.clone(),
    };
    let dtype = x.dtype;
    let buffer = x.client.empty(s.num_elements() * dtype.size());
    let out = CubeTensor::new_contiguous(x.client.clone(), x.device.clone(), s, buffer, dtype);
    let dim = CubeDim::new_2d(LN_THREADS as u32, 1);
    let count = calculate_cube_count_elemwise(&x.client, rows * LN_LANES, dim);
    layer_norm_kernel::launch(&out.client, count, dim, x.into_tensor_arg(), gamma.into_tensor_arg(), beta.into_tensor_arg(), res.into_tensor_arg(), out.clone().into_tensor_arg(), c, has_res, dtype_to_storage_type(dtype));
    out
}

/// The threads of a cube of instance_norm_kernel (one row each)
const IN_THREADS: usize = 256;
/// The epsilon of nn.InstanceNorm2d, which the model's InstanceNorms use (model.rs instance_norm)
pub const IN_EPS: f32 = 1e-5;

/// InstanceNorm without affine parameters over each row of x [R, S] (a channel of an image, S its pixels) as burn's
/// group_norm computes it ((x - mean) / sqrt(var + eps), the variance biased, of the centred values), then relu with
/// relu_y, then with has_res max(res + y, 0) (the add and relu that end a residual block): one launch for what burn runs
/// as two reductions, a subtraction and a division (and the relu and add after them). A cube per row, the sums by a tree
/// in shared memory in a fixed order (no plane operations); the row read three times (sum, centred squares, output), as
/// it does not fit in registers
#[cube(launch)]
fn instance_norm_kernel<F: Float>(
    x: &Tensor<F>,       // [R, S]
    res: &Tensor<F>,     // [R, S], read only with has_res
    out: &mut Tensor<F>, // [R, S]
    #[comptime] relu_y: bool,
    #[comptime] has_res: bool,
    #[define(F)] _dtype: ElemType,
) {
    let (rows, s_n) = (x.shape(0), x.shape(1));
    let t = UNIT_POS_X as usize;
    // Every thread of a cube reaches every barrier: a cube past the last row only skips its loads and stores
    let row = ABSOLUTE_POS as usize / comptime!(IN_THREADS);
    let live = row < rows;
    let base = row * s_n;
    let iters = (s_n + comptime!(IN_THREADS - 1)) / comptime!(IN_THREADS);
    let zero = F::new(0.0_f32);
    let s_f = F::cast_from(s_n as u32);
    let mut red = Shared::<[F]>::new_slice(comptime!(IN_THREADS));
    let mut sum = zero;
    for e in 0..iters {
        let j = t + e * comptime!(IN_THREADS);
        if live && j < s_n {
            sum += x[base + j];
        }
    }
    red[t] = sum;
    sync_cube();
    let mut half = 128usize;
    for _ in 0..8usize {
        if t < half {
            red[t] = red[t] + red[t + half];
        }
        sync_cube();
        half = half / 2usize;
    }
    let mean = red[0] / s_f;
    sync_cube();
    let mut sq = zero;
    for e in 0..iters {
        let j = t + e * comptime!(IN_THREADS);
        if live && j < s_n {
            let dv = x[base + j] - mean;
            sq += dv * dv;
        }
    }
    red[t] = sq;
    sync_cube();
    let mut half = 128usize;
    for _ in 0..8usize {
        if t < half {
            red[t] = red[t] + red[t + half];
        }
        sync_cube();
        half = half / 2usize;
    }
    let sd = F::sqrt(red[0] / s_f + F::new(comptime!(IN_EPS)));
    for e in 0..iters {
        let j = t + e * comptime!(IN_THREADS);
        if live && j < s_n {
            let mut y = (x[base + j] - mean) / sd;
            if relu_y {
                y = F::max(y, zero);
            }
            if has_res {
                y = F::max(res[base + j] + y, zero);
            }
            out[base + j] = y;
        }
    }
}

fn launch_instance_norm(x: CubeTensor, res: Option<CubeTensor>, relu_y: bool) -> CubeTensor {
    let x = into_contiguous(x);
    let s = x.meta.shape().clone();
    assert!(s.num_dims() == 2 && s[0] > 0 && s[1] > 0, "instance_norm: x {s:?}");
    // The tree of instance_norm_kernel sums 256 threads in 8 steps
    assert!(IN_THREADS == 256, "instance_norm_kernel: threads");
    let has_res = res.is_some();
    let res = match res {
        Some(r) => {
            let r = into_contiguous(r);
            assert!(r.meta.shape() == &s, "instance_norm: residual {:?} for x {s:?}", r.meta.shape());
            r
        }
        None => x.clone(),
    };
    let dtype = x.dtype;
    let buffer = x.client.empty(s.num_elements() * dtype.size());
    let out = CubeTensor::new_contiguous(x.client.clone(), x.device.clone(), s.clone(), buffer, dtype);
    let dim = CubeDim::new_2d(IN_THREADS as u32, 1);
    let count = calculate_cube_count_elemwise(&x.client, s[0] * IN_THREADS, dim);
    instance_norm_kernel::launch(&out.client, count, dim, x.into_tensor_arg(), res.into_tensor_arg(), out.clone().into_tensor_arg(), relu_y, has_res, dtype_to_storage_type(dtype));
    out
}

/// Channel ch of slot `slot`'s tensor [N, Ct, L] at batch b, position p (a term of gru_gate_kernel's sums)
#[cube]
#[allow(clippy::too_many_arguments)]
fn gate_term<F: Float>(t0: &Tensor<F>, t1: &Tensor<F>, t2: &Tensor<F>, t3: &Tensor<F>, b: usize, ch: usize, p: usize, l_n: usize, #[comptime] slot: usize) -> F {
    if comptime!(slot == 0) {
        t0[(b * t0.shape(1) + ch) * l_n + p]
    } else {
        if comptime!(slot == 1) {
            t1[(b * t1.shape(1) + ch) * l_n + p]
        } else {
            if comptime!(slot == 2) {
                t2[(b * t2.shape(1) + ch) * l_n + p]
            } else {
                t3[(b * t3.shape(1) + ch) * l_n + p]
            }
        }
    }
}

/// The gates of a step of the factored GRU (model.rs GruFactored::step) from their summed pre-activations, in one launch
/// each instead of burn's narrow, add, sigmoid (exp(-log(exp(-x) + 1)), 6 ops), tanh and blend kernels. The sums read
/// channels of 4 tensor slots [N, Ct, L] from given offsets, in order ((a + b) + c) as model.rs adds them; h [N, C, L].
/// z/r: slots 0, 1 (and 2 with z_three) from zo0..zo2. Reset (update false): sigmoid(r) * h, r C channels after z.
/// Update: (1 - z) * h + z * tanh(q), q from the slots qs0..qs2 at qo0..qo2. The sigmoid as 1 / (1 + exp(-x)), the
/// function burn's computes. 6 tensors: 7 storage bindings with cubecl's metadata, under WebGPU's default 8 per stage
#[cube(launch)]
#[allow(clippy::too_many_arguments)]
fn gru_gate_kernel<F: Float>(
    t0: &Tensor<F>,      // [N, Ct, L]
    t1: &Tensor<F>,
    t2: &Tensor<F>,
    t3: &Tensor<F>,
    h: &Tensor<F>,       // [N, C, L]
    out: &mut Tensor<F>, // [N, C, L]
    #[comptime] z_three: bool,
    #[comptime] zo0: usize,
    #[comptime] zo1: usize,
    #[comptime] zo2: usize,
    #[comptime] qs0: usize,
    #[comptime] qo0: usize,
    #[comptime] qs1: usize,
    #[comptime] qo1: usize,
    #[comptime] qs2: usize,
    #[comptime] qo2: usize,
    #[comptime] update: bool,
    #[define(F)] _dtype: ElemType,
) {
    let idx = ABSOLUTE_POS as usize;
    let (n_n, c_n, l_n) = (h.shape(0), h.shape(1), h.shape(2));
    if idx >= n_n * c_n * l_n {
        terminate!();
    }
    let p = idx % l_n;
    let ch = (idx / l_n) % c_n;
    let b = idx / (c_n * l_n);
    let one = F::new(1.0_f32);
    let zero = F::new(0.0_f32);
    // z at its channel, r C channels further
    let gate = if update { ch } else { c_n + ch };
    let mut s = gate_term::<F>(t0, t1, t2, t3, b, zo0 + gate, p, l_n, 0usize) + gate_term::<F>(t0, t1, t2, t3, b, zo1 + gate, p, l_n, 1usize);
    if z_three {
        s += gate_term::<F>(t0, t1, t2, t3, b, zo2 + gate, p, l_n, 2usize);
    }
    let g = one / (one + F::exp(zero - s));
    let hv = h[idx];
    if update {
        let t = gate_term::<F>(t0, t1, t2, t3, b, qo0 + ch, p, l_n, qs0) + gate_term::<F>(t0, t1, t2, t3, b, qo1 + ch, p, l_n, qs1) + gate_term::<F>(t0, t1, t2, t3, b, qo2 + ch, p, l_n, qs2);
        out[idx] = (one - g) * hv + g * F::tanh(t);
    } else {
        out[idx] = g * hv;
    }
}

/// The comptime layout of a gru_gate_kernel launch: z/r offsets of slots 0..2, the (slot, offset) of the 3 q terms
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GateLayout {
    z_three: bool,
    zo: [usize; 3],
    q: [(usize, usize); 3],
    update: bool,
}

fn launch_gru_gate(t: [CubeTensor; 4], h: CubeTensor, layout: GateLayout) -> CubeTensor {
    let t = t.map(into_contiguous);
    let h = into_contiguous(h);
    let s = h.meta.shape().clone();
    assert!(s.num_dims() == 3, "gru_gate: h {s:?}");
    let (n, c, l) = (s[0], s[1], s[2]);
    // Every term read: the batch and positions of h, and the channels it is read at
    let fits = |slot: usize, off: usize, width: usize| { let ts = t[slot].meta.shape(); ts.num_dims() == 3 && ts[0] == n && ts[2] == l && off + width <= ts[1] };
    let zn = if layout.z_three { 3 } else { 2 };
    for (slot, &off) in layout.zo.iter().enumerate().take(zn) {
        assert!(fits(slot, off, 2 * c), "gru_gate: z/r slot {slot} {:?} from channel {off} for h {s:?}", t[slot].meta.shape());
    }
    if layout.update {
        for &(slot, off) in &layout.q {
            assert!(slot < 4 && fits(slot, off, c), "gru_gate: q slot {slot} from channel {off} for h {s:?}");
        }
    }
    let dtype = h.dtype;
    let (out, count, dim) = output_for(&h, s, n * c * l);
    let [t0, t1, t2, t3] = t;
    let (zo, q) = (layout.zo, layout.q);
    gru_gate_kernel::launch(&out.client, count, dim, t0.into_tensor_arg(), t1.into_tensor_arg(), t2.into_tensor_arg(), t3.into_tensor_arg(), h.into_tensor_arg(), out.clone().into_tensor_arg(), layout.z_three, zo[0], zo[1], zo[2], q[0].0, q[0].1, q[1].0, q[1].1, q[2].0, q[2].1, layout.update, dtype_to_storage_type(dtype));
    out
}

/// A convolution with few output channels (the flow head's last, 256 -> 2, 3x3): one thread per output position computing
/// all its output channels, so each input value of the position is read once (im2col + matmul made a 106 MB column
/// buffer each refinement step for 0.1 GFLOP). Stride 1, symmetric zero padding; the bias added last, as after a matmul
#[cube(launch)]
fn conv_small_kernel<F: Float>(
    x: &Tensor<F>,       // [N, C, H, W]
    weight: &Tensor<F>,  // [CO, C, KH, KW]
    bias: &Tensor<F>,    // [CO]
    out: &mut Tensor<F>, // [N, CO, OH, OW]
    #[comptime] co: usize,
    #[comptime] kh: usize,
    #[comptime] kw: usize,
    #[comptime] ph: usize,
    #[comptime] pw: usize,
    #[define(F)] _dtype: ElemType,
) {
    let idx = ABSOLUTE_POS as usize;
    let (n_n, c_n, h, w) = (x.shape(0), x.shape(1), x.shape(2), x.shape(3));
    let (oh, ow) = (out.shape(2), out.shape(3));
    if idx >= n_n * oh * ow {
        terminate!();
    }
    let ox = idx % ow;
    let oy = (idx / ow) % oh;
    let n = idx / (oh * ow);
    let phi = comptime!(ph as i32);
    let pwi = comptime!(pw as i32);
    let mut acc = Array::<F>::new(co);
    #[unroll]
    for o in 0..co {
        acc[o] = F::new(0.0_f32);
    }
    for c in 0..c_n {
        let base = (n * c_n + c) * h * w;
        for ky in 0..kh {
            let sy = oy as i32 + ky as i32 - phi;
            for kx in 0..kw {
                let sx = ox as i32 + kx as i32 - pwi;
                if sy >= 0 && sy < h as i32 && sx >= 0 && sx < w as i32 {
                    let v = x[base + sy as usize * w + sx as usize];
                    #[unroll]
                    for o in 0..co {
                        acc[o] += v * weight[((o * c_n + c) * kh + ky) * kw + kx];
                    }
                }
            }
        }
    }
    let p = oy * ow + ox;
    #[unroll]
    for o in 0..co {
        out[(n * co + o) * oh * ow + p] = acc[o] + bias[o];
    }
}

fn conv_small_shape(x: &Shape, weight: &Shape, ph: usize, pw: usize) -> Shape {
    Shape::new([x[0], weight[0], x[2] + 2 * ph - weight[2] + 1, x[3] + 2 * pw - weight[3] + 1])
}

fn launch_conv_small(x: CubeTensor, weight: CubeTensor, bias: CubeTensor, ph: usize, pw: usize) -> CubeTensor {
    let (x, weight, bias) = (into_contiguous(x), into_contiguous(weight), into_contiguous(bias));
    let ws = weight.meta.shape().clone();
    let shape = conv_small_shape(x.meta.shape(), &ws, ph, pw);
    let n = shape[0] * shape[2] * shape[3];
    let dtype = x.dtype;
    let (out, count, dim) = output_for(&x, shape, n);
    conv_small_kernel::launch(&out.client, count, dim, x.into_tensor_arg(), weight.into_tensor_arg(), bias.into_tensor_arg(), out.clone().into_tensor_arg(), ws[0], ws[2], ws[3], ph, pw, dtype_to_storage_type(dtype));
    out
}

// ---------------------------------------------------------------- preprocessing on the GPU

/// A frame of 8-bit grey (luma), packed 3 bytes to an i32 word (always positive, so that no conversion can change it), as
/// it is uploaded: [H, words per row]
pub fn pack_frame(width: usize, height: usize, stride: usize, pixels: &[u8]) -> (Vec<i32>, [usize; 2]) {
    let words = width.div_ceil(3);
    let mut out = vec![0i32; height * words];
    for y in 0..height {
        let row = &pixels[y * stride..y * stride + width];
        for (j, chunk) in row.chunks(3).enumerate() {
            let mut v = 0i32;
            for (b, &p) in chunk.iter().enumerate() { v |= (p as i32) << (8 * b); }
            out[y * words + j] = v;
        }
    }
    (out, [height, words])
}

/// What `resize` of the image crate (0.25) does with FilterType::Triangle, then the letterbox: the frame scaled to
/// new_w x new_h, vertical pass then horizontal pass with the weights of the triangle filter widened by the ratio and
/// normalised, rounded to an integer grey level once at the end, placed at (pad_x, pad_y) on a canvas of zeros; the same
/// grey in the 3 channels: [1, 3, out_h, out_w], raw 0-255 as the model takes it. One thread per canvas position.
#[cube(launch)]
fn preprocess_kernel<I: Int, F: Float>(
    frame: &Tensor<I>,   // [H0, ceil(W0 / 3)], 3 bytes per word
    out: &mut Tensor<F>, // [1, 3, out_h, out_w]
    #[comptime] w0: usize,
    #[comptime] new_w: usize,
    #[comptime] new_h: usize,
    #[comptime] pad_x: usize,
    #[comptime] pad_y: usize,
    #[define(I)] _itype: ElemType,
    #[define(F)] _ftype: ElemType,
) {
    let idx = ABSOLUTE_POS as usize;
    let (oh, ow) = (out.shape(2), out.shape(3));
    if idx >= oh * ow {
        terminate!();
    }
    let ox = idx % ow;
    let oy = idx / ow;
    let h0 = frame.shape(0);
    let words = frame.shape(1);
    let zero = F::new(0.0_f32);
    let one = F::new(1.0_f32);
    let half = F::new(0.5_f32);
    let mut value = zero;
    if ox >= pad_x && ox < pad_x + new_w && oy >= pad_y && oy < pad_y + new_h {
        let x = ox - pad_x;
        let y = oy - pad_y;
        // Vertical window and weights (vertical_sample)
        let ry = F::cast_from(h0 as u32) / F::cast_from(new_h as u32);
        let sy = F::max(ry, one);
        let iny = (F::cast_from(y as u32) + half) * ry;
        let top = clamp(i32::cast_from((iny - sy).floor()), 0, h0 as i32 - 1);
        let bottom = clamp(i32::cast_from((iny + sy).ceil()), top + 1, h0 as i32);
        let iny = iny - half;
        let mut sumy = zero;
        for i in top..bottom {
            let d = ((F::cast_from(i) - iny) / sy).abs();
            if d < one { sumy += one - d; }
        }
        // Horizontal window and weights (horizontal_sample)
        let rx = F::cast_from(w0 as u32) / F::cast_from(new_w as u32);
        let sx = F::max(rx, one);
        let inx = (F::cast_from(x as u32) + half) * rx;
        let left = clamp(i32::cast_from((inx - sx).floor()), 0, w0 as i32 - 1);
        let right = clamp(i32::cast_from((inx + sx).ceil()), left + 1, w0 as i32);
        let inx = inx - half;
        let mut sumx = zero;
        for j in left..right {
            let d = ((F::cast_from(j) - inx) / sx).abs();
            if d < one { sumx += one - d; }
        }
        let mut t = zero;
        for j in left..right {
            let dx = ((F::cast_from(j) - inx) / sx).abs();
            let mut wx = zero;
            if dx < one { wx = one - dx; }
            wx = wx / sumx;
            // The vertical pass at column j
            let mut v = zero;
            for i in top..bottom {
                let dy = ((F::cast_from(i) - iny) / sy).abs();
                let mut wy = zero;
                if dy < one { wy = one - dy; }
                wy = wy / sumy;
                let word = u32::cast_from(frame[i as usize * words + j as usize / 3]);
                let byte = (word >> (8 * (j as u32 % 3))) & 255;
                v += F::cast_from(byte) * wy;
            }
            t += v * wx;
        }
        // f32::round of the image crate rounds half away from zero; WGSL round() rounds half to even: t >= 0, so
        // floor(t + 0.5) is what the CPU does
        value = (clamp(t, zero, F::new(255.0_f32)) + half).floor();
    }
    let plane = oh * ow;
    let v = value;
    out[idx] = v;
    out[plane + idx] = v;
    out[2 * plane + idx] = v;
}

fn preprocess_meta(_frame: &TensorSpec, _w0: &usize, _new_w: &usize, _new_h: &usize, _pad_x: &usize, _pad_y: &usize, out_h: &usize, out_w: &usize) -> TensorSpec {
    TensorSpec::new(Shape::new([1, 3, *out_h, *out_w]), DType::F32)
}

#[allow(clippy::too_many_arguments)]
fn launch_preprocess(frame: CubeTensor, w0: usize, new_w: usize, new_h: usize, pad_x: usize, pad_y: usize, out_h: usize, out_w: usize) -> CubeTensor {
    let frame = into_contiguous(frame);
    let shape = Shape::new([1, 3, out_h, out_w]);
    let threads = out_h * out_w;
    let dtype = DType::F32;
    let buffer = frame.client.empty(shape.num_elements() * dtype.size());
    let out = CubeTensor::new_contiguous(frame.client.clone(), frame.device.clone(), shape, buffer, dtype);
    let cube_dim = CubeDim::new(&frame.client, threads);
    let count = calculate_cube_count_elemwise(&frame.client, threads, cube_dim);
    let itype = dtype_to_storage_type(frame.dtype);
    preprocess_kernel::launch(&out.client, count, cube_dim, frame.into_tensor_arg(), out.clone().into_tensor_arg(), w0, new_w, new_h, pad_x, pad_y, itype, dtype_to_storage_type(dtype));
    out
}

/// The letterbox of a frame packed by `pack_frame` (width w0), computed on the GPU: [1, 3, out_h, out_w] raw 0-255
#[allow(clippy::too_many_arguments)]
pub fn preprocess(frame: BurnTensor<2, IntKind>, w0: usize, new_w: usize, new_h: usize, pad_x: usize, pad_y: usize, out_h: usize, out_w: usize) -> BurnTensor<4> {
    BurnTensor::from_dispatch(Dispatch::preprocess(frame.into_dispatch(), w0, new_w, new_h, pad_x, pad_y, out_h, out_w))
}

// ---------------------------------------------------------------- the backend extension

/// The kernels as backend operations: on the CubeCL backends (Vulkan, Metal), and lazily under burn fusion
#[backend_extension(Cube, Fusion)]
pub trait GmflowKernels: burn::backend::Backend {
    #[fusion(dtype = f0, shape = corr_shape(f0))]
    fn corr_with_flow(f0: FloatTensor<Self>, f1: FloatTensor<Self>, flow: FloatTensor<Self>) -> FloatTensor<Self>;
    #[fusion(dtype = f0, shape = flow_shape(f0))]
    fn local_corr_softmax(f0: FloatTensor<Self>, f1: FloatTensor<Self>) -> FloatTensor<Self>;
    #[fusion(dtype = flow, shape = flow)]
    fn local_propagation(q: FloatTensor<Self>, k: FloatTensor<Self>, flow: FloatTensor<Self>) -> FloatTensor<Self>;
    #[fusion(dtype = flow, shape = upsampled_shape(flow))]
    fn convex_upsample(flow: FloatTensor<Self>, mask: FloatTensor<Self>) -> FloatTensor<Self>;
    #[fusion(dtype = flow, shape = doubled_shape(flow))]
    fn flow_double(flow: FloatTensor<Self>) -> FloatTensor<Self>;
    #[fusion(dtype = x, shape = im2col_shape(x, *kh, *kw, *stride, *ph, *pw))]
    fn im2col(x: FloatTensor<Self>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> FloatTensor<Self>;
    #[fusion(dtype = a, shape = gemm_shape(a, b))]
    fn gemm(a: FloatTensor<Self>, b: FloatTensor<Self>, bias: FloatTensor<Self>, has_bias: bool, block: usize, kstep: usize, per: usize) -> FloatTensor<Self>;
    #[fusion(dtype = a, shape = gemm_shape(a, b))]
    fn gemm4(a: FloatTensor<Self>, b: FloatTensor<Self>, bias: FloatTensor<Self>, has_bias: bool, pm: usize, pn: usize, kstep: usize) -> FloatTensor<Self>;
    #[fusion(dtype = x, shape = conv_gemm_shape(w, x, *kh, *kw, *stride, *ph, *pw))]
    #[allow(clippy::too_many_arguments)]
    fn conv_gemm(w: FloatTensor<Self>, x: FloatTensor<Self>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> FloatTensor<Self>;
    #[fusion(dtype = q, shape = q)]
    fn window_attention(q: FloatTensor<Self>, k: FloatTensor<Self>, v: FloatTensor<Self>, h: usize, w: usize, splits: usize, shift: bool) -> FloatTensor<Self>;
    #[fusion(dtype = x, shape = x)]
    fn layer_norm(x: FloatTensor<Self>, gamma: FloatTensor<Self>, beta: FloatTensor<Self>, res: FloatTensor<Self>, has_res: bool) -> FloatTensor<Self>;
    #[fusion(dtype = x, shape = x)]
    fn instance_norm(x: FloatTensor<Self>, res: FloatTensor<Self>, relu_y: bool, has_res: bool) -> FloatTensor<Self>;
    #[fusion(dtype = scores, shape = softmax_flow_shape(scores, v))]
    fn softmax_flow(scores: FloatTensor<Self>, v: FloatTensor<Self>) -> FloatTensor<Self>;
    #[fusion(dtype = h, shape = h)]
    #[allow(clippy::too_many_arguments)]
    fn gru_gate(t0: FloatTensor<Self>, t1: FloatTensor<Self>, t2: FloatTensor<Self>, t3: FloatTensor<Self>, h: FloatTensor<Self>, z_three: bool, zo0: usize, zo1: usize, zo2: usize, qs0: usize, qo0: usize, qs1: usize, qo1: usize, qs2: usize, qo2: usize, update: bool) -> FloatTensor<Self>;
    #[fusion(dtype = x, shape = conv_small_shape(x, weight, *ph, *pw))]
    fn conv_small(x: FloatTensor<Self>, weight: FloatTensor<Self>, bias: FloatTensor<Self>, ph: usize, pw: usize) -> FloatTensor<Self>;
    #[fusion(meta = preprocess_meta)]
    #[allow(clippy::too_many_arguments)]
    fn preprocess(frame: IntTensor<Self>, w0: usize, new_w: usize, new_h: usize, pad_x: usize, pad_y: usize, out_h: usize, out_w: usize) -> FloatTensor<Self>;
}

impl GmflowKernels for CubeBackend {
    fn corr_with_flow(f0: FloatTensor<Self>, f1: FloatTensor<Self>, flow: FloatTensor<Self>) -> FloatTensor<Self> {
        launch_corr_with_flow(f0, f1, flow)
    }
    fn local_corr_softmax(f0: FloatTensor<Self>, f1: FloatTensor<Self>) -> FloatTensor<Self> {
        launch_local_corr_softmax(f0, f1)
    }
    fn local_propagation(q: FloatTensor<Self>, k: FloatTensor<Self>, flow: FloatTensor<Self>) -> FloatTensor<Self> {
        launch_local_propagation(q, k, flow)
    }
    fn convex_upsample(flow: FloatTensor<Self>, mask: FloatTensor<Self>) -> FloatTensor<Self> {
        launch_convex_upsample(flow, mask)
    }
    fn flow_double(flow: FloatTensor<Self>) -> FloatTensor<Self> {
        launch_flow_double(flow)
    }
    fn im2col(x: FloatTensor<Self>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> FloatTensor<Self> {
        launch_im2col(x, kh, kw, stride, ph, pw)
    }
    fn gemm(a: FloatTensor<Self>, b: FloatTensor<Self>, bias: FloatTensor<Self>, has_bias: bool, block: usize, kstep: usize, per: usize) -> FloatTensor<Self> {
        launch_gemm(a, b, has_bias.then_some(bias), GemmTile { block, kstep, per })
    }
    fn gemm4(a: FloatTensor<Self>, b: FloatTensor<Self>, bias: FloatTensor<Self>, has_bias: bool, pm: usize, pn: usize, kstep: usize) -> FloatTensor<Self> {
        launch_gemm4(a, b, has_bias.then_some(bias), Gemm4Tile { pm, pn, kstep })
    }
    fn conv_gemm(w: FloatTensor<Self>, x: FloatTensor<Self>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> FloatTensor<Self> {
        launch_conv_gemm(w, x, kh, kw, stride, ph, pw)
    }
    fn window_attention(q: FloatTensor<Self>, k: FloatTensor<Self>, v: FloatTensor<Self>, h: usize, w: usize, splits: usize, shift: bool) -> FloatTensor<Self> {
        launch_window_attention(q, k, v, h, w, splits, shift)
    }
    fn layer_norm(x: FloatTensor<Self>, gamma: FloatTensor<Self>, beta: FloatTensor<Self>, res: FloatTensor<Self>, has_res: bool) -> FloatTensor<Self> {
        launch_layer_norm(x, gamma, beta, has_res.then_some(res))
    }
    fn instance_norm(x: FloatTensor<Self>, res: FloatTensor<Self>, relu_y: bool, has_res: bool) -> FloatTensor<Self> {
        launch_instance_norm(x, has_res.then_some(res), relu_y)
    }
    fn softmax_flow(scores: FloatTensor<Self>, v: FloatTensor<Self>) -> FloatTensor<Self> {
        launch_softmax_flow(scores, v)
    }
    #[allow(clippy::too_many_arguments)]
    fn gru_gate(t0: FloatTensor<Self>, t1: FloatTensor<Self>, t2: FloatTensor<Self>, t3: FloatTensor<Self>, h: FloatTensor<Self>, z_three: bool, zo0: usize, zo1: usize, zo2: usize, qs0: usize, qo0: usize, qs1: usize, qo1: usize, qs2: usize, qo2: usize, update: bool) -> FloatTensor<Self> {
        let layout = GateLayout { z_three, zo: [zo0, zo1, zo2], q: [(qs0, qo0), (qs1, qo1), (qs2, qo2)], update };
        launch_gru_gate([t0, t1, t2, t3], h, layout)
    }
    fn conv_small(x: FloatTensor<Self>, weight: FloatTensor<Self>, bias: FloatTensor<Self>, ph: usize, pw: usize) -> FloatTensor<Self> {
        launch_conv_small(x, weight, bias, ph, pw)
    }
    fn preprocess(frame: IntTensor<Self>, w0: usize, new_w: usize, new_h: usize, pad_x: usize, pad_y: usize, out_h: usize, out_w: usize) -> FloatTensor<Self> {
        launch_preprocess(frame, w0, new_w, new_h, pad_x, pad_y, out_h, out_w)
    }
}

// ---------------------------------------------------------------- what model.rs calls

/// f0, f1 [B, C, H, W], flow [B, 2, H, W] -> [B, 81, H, W]
pub fn corr_with_flow(f0: BurnTensor<4>, f1: BurnTensor<4>, flow: BurnTensor<4>) -> BurnTensor<4> {
    BurnTensor::from_dispatch(Dispatch::corr_with_flow(f0.into_dispatch(), f1.into_dispatch(), flow.into_dispatch()))
}

/// f0, f1 [B, C, H, W] -> flow [B, 2, H, W]
pub fn local_corr_softmax(f0: BurnTensor<4>, f1: BurnTensor<4>) -> BurnTensor<4> {
    BurnTensor::from_dispatch(Dispatch::local_corr_softmax(f0.into_dispatch(), f1.into_dispatch()))
}

/// q, k [B, C, H, W] (projected), flow [B, 2, H, W] -> flow
pub fn local_propagation(q: BurnTensor<4>, k: BurnTensor<4>, flow: BurnTensor<4>) -> BurnTensor<4> {
    BurnTensor::from_dispatch(Dispatch::local_propagation(q.into_dispatch(), k.into_dispatch(), flow.into_dispatch()))
}

/// flow [B, 2, H, W], mask [B, 144, H, W] -> [B, 2, 4H, 4W]
pub fn convex_upsample(flow: BurnTensor<4>, mask: BurnTensor<4>) -> BurnTensor<4> {
    BurnTensor::from_dispatch(Dispatch::convex_upsample(flow.into_dispatch(), mask.into_dispatch()))
}

/// flow [B, C, H, W] -> [B, C, 2H, 2W]: the flow at the next finer level (flow_double_kernel)
pub fn flow_double(flow: BurnTensor<4>) -> BurnTensor<4> {
    BurnTensor::from_dispatch(Dispatch::flow_double(flow.into_dispatch()))
}

/// The columns of x [N, C, H, W] for a KH x KW convolution (F.unfold): [N, C*KH*KW, OH*OW]
pub fn im2col(x: BurnTensor<4>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> BurnTensor<3> {
    BurnTensor::from_dispatch(Dispatch::im2col(x.into_dispatch(), kh, kw, stride, ph, pw))
}

/// a [Ba, M, K] @ b [Bb, K, N] (+ bias [M] per row) -> [max(Ba, Bb), M, N], a batch of 1 shared by all: the matmul for
/// devices without f32 matrix units. gemm4_kernel with GEMM4_DEFAULT where N is a multiple of 4, gemm_kernel otherwise
pub fn gemm(a: BurnTensor<3>, b: BurnTensor<3>, bias: Option<BurnTensor<1>>) -> BurnTensor<3> {
    gemm4_tiled(a, b, bias, GEMM4_DEFAULT)
}

/// gemm_kernel with a given tiling (the tests measure several)
pub fn gemm_tiled(a: BurnTensor<3>, b: BurnTensor<3>, bias: Option<BurnTensor<1>>, tile: GemmTile) -> BurnTensor<3> {
    let th = tile.threads();
    assert!(tile.block % tile.per == 0 && th == GEMM_THREADS && (tile.block * tile.kstep) % (th * th) == 0, "gemm: tiling {tile:?}");
    assert!(tile.kstep * (2 * tile.block + 4) * 4 <= 16 << 10, "gemm: tiling {tile:?} needs more than 16 KiB of shared memory");
    let has_bias = bias.is_some();
    let bias = match bias { Some(v) => v.into_dispatch(), None => a.clone().into_dispatch() };
    BurnTensor::from_dispatch(Dispatch::gemm(a.into_dispatch(), b.into_dispatch(), bias, has_bias, tile.block, tile.kstep, tile.per))
}

/// gemm by gemm4_kernel with a given tiling; columns not a multiple of 4: gemm (the tests measure and check several)
pub fn gemm4_tiled(a: BurnTensor<3>, b: BurnTensor<3>, bias: Option<BurnTensor<1>>, tile: Gemm4Tile) -> BurnTensor<3> {
    assert!(tile.pn % 4 == 0 && (16 * tile.pm * tile.kstep) % 256 == 0, "gemm4: tiling {tile:?}");
    // f32 (the model's type)
    assert!(tile.kstep * (16 * tile.pm + 4 + 16 * tile.pn) * 4 <= 16 << 10, "gemm4: tiling {tile:?} needs more than 16 KiB of shared memory");
    if b.dims()[2] % 4 != 0 {
        return gemm_tiled(a, b, bias, GEMM_DEFAULT);
    }
    let has_bias = bias.is_some();
    let bias = match bias { Some(v) => v.into_dispatch(), None => a.clone().into_dispatch() };
    BurnTensor::from_dispatch(Dispatch::gemm4(a.into_dispatch(), b.into_dispatch(), bias, has_bias, tile.pm, tile.pn, tile.kstep))
}

/// w [1, Co, C*KH*KW] @ the KH x KW columns of x [N, C, H, W] (stride, symmetric zero padding) -> [N, Co, OH*OW], the
/// columns gathered in the kernel (conv_gemm4_kernel); OH*OW not a multiple of 4: im2col + gemm
pub fn conv_gemm(w: BurnTensor<3>, x: BurnTensor<4>, kh: usize, kw: usize, stride: usize, ph: usize, pw: usize) -> BurnTensor<3> {
    let [_, _, h, ww] = x.dims();
    let (oh, ow) = ((h + 2 * ph - kh) / stride + 1, (ww + 2 * pw - kw) / stride + 1);
    if (oh * ow) % 4 != 0 {
        return gemm(w, im2col(x, kh, kw, stride, ph, pw), None);
    }
    BurnTensor::from_dispatch(Dispatch::conv_gemm(w.into_dispatch(), x.into_dispatch(), kh, kw, stride, ph, pw))
}

/// A 2D convolution (symmetric zero padding, no dilation or groups) by conv_gemm: no column buffer. x [N, C, H, W],
/// weight [Cout, C, KH, KW] -> [N, Cout, OH, OW]
pub fn conv2d_implicit(x: BurnTensor<4>, weight: BurnTensor<4>, bias: Option<BurnTensor<1>>, stride: usize, padding: [usize; 2]) -> BurnTensor<4> {
    let [n, c, h, w] = x.dims();
    let [cout, cin, kh, kw] = weight.dims();
    assert_eq!(c, cin, "conv2d_implicit: channels");
    let (ph, pw) = (padding[0], padding[1]);
    let (oh, ow) = ((h + 2 * ph - kh) / stride + 1, (w + 2 * pw - kw) / stride + 1);
    let out = conv_gemm(weight.reshape([1, cout, c * kh * kw]), x, kh, kw, stride, ph, pw).reshape([n, cout, oh, ow]);
    match bias {
        Some(b) => out + b.reshape([1, cout, 1, 1]),
        None => out,
    }
}

/// unimatch single_head_split_window_attention without the window layout or the roll in memory, in three launches (scores,
/// masked softmax, probabilities @ v): q, k, v [B, H*W, C] of an H x W map in splits x splits windows -> [B, H*W, C]
pub fn window_attention(q: BurnTensor<3>, k: BurnTensor<3>, v: BurnTensor<3>, h: usize, w: usize, splits: usize, shift: bool) -> BurnTensor<3> {
    BurnTensor::from_dispatch(Dispatch::window_attention(q.into_dispatch(), k.into_dispatch(), v.into_dispatch(), h, w, splits, shift))
}

/// LayerNorm over the last dimension of x [B, L, C] with gamma and beta [C] and the model's epsilon (LN_EPS), plus
/// res [B, L, C] if given, in one launch (layer_norm_kernel): what burn's LayerNorm::forward (and an add) computes
pub fn layer_norm(x: BurnTensor<3>, gamma: BurnTensor<1>, beta: BurnTensor<1>, res: Option<BurnTensor<3>>) -> BurnTensor<3> {
    let [b, l, c] = x.dims();
    // The values of a row are kept in registers (C / 32 a thread)
    assert!(c > 0 && c <= 1024, "layer_norm: {c} channels");
    if let Some(r) = &res {
        assert_eq!(r.dims(), [b, l, c], "layer_norm: residual");
    }
    let has_res = res.is_some();
    let x = x.reshape([b * l, c]);
    let res = match res { Some(r) => r.reshape([b * l, c]).into_dispatch(), None => x.clone().into_dispatch() };
    let out: BurnTensor<2> = BurnTensor::from_dispatch(Dispatch::layer_norm(x.into_dispatch(), gamma.into_dispatch(), beta.into_dispatch(), res, has_res));
    out.reshape([b, l, c])
}

/// softmax(scores, 2) @ v: scores [B, N, L], v [1 or B, L, 2] -> [B, N, 2] in one launch (softmax_flow_kernel)
pub fn softmax_flow(scores: BurnTensor<3>, v: BurnTensor<3>) -> BurnTensor<3> {
    BurnTensor::from_dispatch(Dispatch::softmax_flow(scores.into_dispatch(), v.into_dispatch()))
}

/// InstanceNorm without affine parameters of x [N, C, H, W] with the model's epsilon (IN_EPS), then relu with relu_y,
/// then with res [N, C, H, W] relu(res + y), in one launch (instance_norm_kernel): what burn's InstanceNorm::forward and
/// the activations after it in the CNN compute
pub fn instance_norm(x: BurnTensor<4>, relu_y: bool, res: Option<BurnTensor<4>>) -> BurnTensor<4> {
    let [n, c, h, w] = x.dims();
    assert!(n * c > 0 && h * w > 0, "instance_norm: x {:?}", [n, c, h, w]);
    if let Some(r) = &res {
        assert_eq!(r.dims(), [n, c, h, w], "instance_norm: residual");
    }
    let has_res = res.is_some();
    let x = x.reshape([n * c, h * w]);
    let res = match res { Some(r) => r.reshape([n * c, h * w]).into_dispatch(), None => x.clone().into_dispatch() };
    let out: BurnTensor<2> = BurnTensor::from_dispatch(Dispatch::instance_norm(x.into_dispatch(), res, relu_y, has_res));
    out.reshape([n, c, h, w])
}

/// Where a q term of gru_gate is read: the tensor of one of the z/r terms (by index), or the extra tensor
#[derive(Clone, Copy, Debug)]
pub enum QFrom {
    Zr(usize),
    Extra,
}

/// A gate of the factored GRU (gru_gate_kernel). zr: the 2 or 3 terms (t [N, Ct, L], first channel) whose sum holds the
/// pre-activations of z (C channels from the offset) and r (the C after them), C the channels of h [N, C, L]. Without q:
/// sigmoid(r) * h. With q = ((q0, offset), extra, [(from, offset); 2]): (1 - z) * h + z * tanh(q0 + a + b), a and b
/// read from a z/r term's tensor or the extra one, C channels from their offsets. At most 4 tensors in all
pub fn gru_gate(zr: &[(BurnTensor<3>, usize)], q: Option<((BurnTensor<3>, usize), Option<BurnTensor<3>>, [(QFrom, usize); 2])>, h: BurnTensor<3>) -> BurnTensor<3> {
    assert!(zr.len() == 2 || zr.len() == 3, "gru_gate: {} z/r terms", zr.len());
    let zn = zr.len();
    let mut slots: Vec<BurnTensor<3>> = zr.iter().map(|(t, _)| t.clone()).collect();
    let zo = [zr[0].1, zr[1].1, if zn == 3 { zr[2].1 } else { 0 }];
    let update = q.is_some();
    let mut qs = [(0usize, 0usize); 3];
    if let Some(((q0, o0), extra, rest)) = q {
        slots.push(q0);
        qs[0] = (zn, o0);
        let extra_slot = extra.map(|e| { slots.push(e); slots.len() - 1 });
        for (i, (from, off)) in rest.into_iter().enumerate() {
            let slot = match from {
                QFrom::Zr(k) => { assert!(k < zn, "gru_gate: q from z/r term {k} of {zn}"); k }
                QFrom::Extra => extra_slot.expect("gru_gate: a q term from the extra tensor, which is not given"),
            };
            qs[i + 1] = (slot, off);
        }
    }
    assert!(slots.len() <= 4, "gru_gate: {} tensors (at most 4)", slots.len());
    while slots.len() < 4 {
        slots.push(zr[0].0.clone());
    }
    let [t0, t1, t2, t3]: [BurnTensor<3>; 4] = slots.try_into().unwrap_or_else(|_| unreachable!());
    BurnTensor::from_dispatch(Dispatch::gru_gate(
        t0.into_dispatch(), t1.into_dispatch(), t2.into_dispatch(), t3.into_dispatch(), h.into_dispatch(),
        zn == 3, zo[0], zo[1], zo[2], qs[0].0, qs[0].1, qs[1].0, qs[1].1, qs[2].0, qs[2].1, update,
    ))
}

/// A stride-1 convolution with few output channels (conv_small_kernel): x [N, C, H, W], weight [CO, C, KH, KW], bias [CO]
/// -> [N, CO, H + 2 ph - KH + 1, W + 2 pw - KW + 1]
pub fn conv_small(x: BurnTensor<4>, weight: BurnTensor<4>, bias: BurnTensor<1>, ph: usize, pw: usize) -> BurnTensor<4> {
    BurnTensor::from_dispatch(Dispatch::conv_small(x.into_dispatch(), weight.into_dispatch(), bias.into_dispatch(), ph, pw))
}

/// A 2D convolution (symmetric zero padding, no dilation or groups) as im2col + one batched matmul, the columns made in
/// chunks of images so that none is over `max_bytes` (Vulkan guarantees 128 MiB for one buffer). x [N, C, H, W],
/// weight [Cout, C, KH, KW] -> [N, Cout, OH, OW]
/// (burn's matmul, or gemm with own_gemm)
pub fn conv2d_im2col(x: BurnTensor<4>, weight: BurnTensor<4>, bias: Option<BurnTensor<1>>, stride: usize, padding: [usize; 2], max_bytes: usize, own_gemm: bool) -> BurnTensor<4> {
    let [n, c, h, w] = x.dims();
    let [cout, cin, kh, kw] = weight.dims();
    assert_eq!(c, cin, "conv2d_im2col: channels");
    let (ph, pw) = (padding[0], padding[1]);
    let oh = (h + 2 * ph - kh) / stride + 1;
    let ow = (w + 2 * pw - kw) / stride + 1;
    let k = c * kh * kw;
    let per_image = k * oh * ow * 4;
    assert!(per_image <= max_bytes, "conv2d_im2col: the columns of one image ({per_image} bytes) exceed {max_bytes}");
    let chunk = (max_bytes / per_image).max(1);
    let wmat = weight.reshape([1, cout, k]);
    let mut outs = Vec::new();
    let mut start = 0;
    while start < n {
        let len = chunk.min(n - start);
        let part = if len == n { x.clone() } else { x.clone().narrow(0, start, len) };
        let cols: BurnTensor<3> = BurnTensor::from_dispatch(Dispatch::im2col(part.into_dispatch(), kh, kw, stride, ph, pw));
        // Few output channels (the flow head's 2) would leave most of a 64-row block of gemm idle
        outs.push(if own_gemm && cout >= 32 { gemm(wmat.clone(), cols, None) } else { wmat.clone().matmul(cols) }); // [len, Cout, OH*OW]
        start += len;
    }
    let out = if outs.len() == 1 { outs.pop().unwrap() } else { BurnTensor::cat(outs, 0) };
    let out = out.reshape([n, cout, oh, ow]);
    match bias {
        Some(b) => out + b.reshape([1, cout, 1, 1]),
        None => out,
    }
}
