// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use crate::gpu::{ Buffers, BufferSource };

use super::{ PixelType, Stabilization, ComputeParams, FrameTransform, KernelParams, distortion_models::DistortionModel, projection };
use nalgebra::{ Vector2, Vector3, Vector4, Matrix3 };
use rayon::{ prelude::ParallelSliceMut, iter::{ ParallelIterator, IndexedParallelIterator } };
use crate::util::map_coord;

fn rotate_point(pos: (f32, f32), angle: f32, origin: (f32, f32), origin2: (f32, f32)) -> (f32, f32) {
    (angle.cos() * (pos.0 - origin.0) - angle.sin() * (pos.1 - origin.1) + origin2.0,
     angle.sin() * (pos.0 - origin.0) + angle.cos() * (pos.1 - origin.1) + origin2.1)
}

pub const INVALID_POINT: (f32, f32) = (-1_000_000.0, -1_000_000.0);

pub fn is_valid_point(p: (f32, f32)) -> bool {
    p.0.is_finite() && p.1.is_finite() && p.0 > INVALID_POINT.0 / 2.0 && p.1 > INVALID_POINT.1 / 2.0
}

pub const COEFFS: [f32; 64+128+256 + 9*4 + 4] = [
    // Bilinear
    // offset 0
    1.000000, 0.000000, 0.968750, 0.031250, 0.937500, 0.062500, 0.906250, 0.093750, 0.875000, 0.125000, 0.843750, 0.156250,
    0.812500, 0.187500, 0.781250, 0.218750, 0.750000, 0.250000, 0.718750, 0.281250, 0.687500, 0.312500, 0.656250, 0.343750,
    0.625000, 0.375000, 0.593750, 0.406250, 0.562500, 0.437500, 0.531250, 0.468750, 0.500000, 0.500000, 0.468750, 0.531250,
    0.437500, 0.562500, 0.406250, 0.593750, 0.375000, 0.625000, 0.343750, 0.656250, 0.312500, 0.687500, 0.281250, 0.718750,
    0.250000, 0.750000, 0.218750, 0.781250, 0.187500, 0.812500, 0.156250, 0.843750, 0.125000, 0.875000, 0.093750, 0.906250,
    0.062500, 0.937500, 0.031250, 0.968750,

    // Bicubic
    // offset 64
     0.000000, 1.000000, 0.000000,  0.000000, -0.021996, 0.997841, 0.024864, -0.000710, -0.041199, 0.991516, 0.052429, -0.002747,
    -0.057747, 0.981255, 0.082466, -0.005974, -0.071777, 0.967285, 0.114746, -0.010254, -0.083427, 0.949837, 0.149040, -0.015450,
    -0.092834, 0.929138, 0.185120, -0.021423, -0.100136, 0.905418, 0.222755, -0.028038, -0.105469, 0.878906, 0.261719, -0.035156,
    -0.108971, 0.849831, 0.301781, -0.042641, -0.110779, 0.818420, 0.342712, -0.050354, -0.111031, 0.784904, 0.384285, -0.058159,
    -0.109863, 0.749512, 0.426270, -0.065918, -0.107414, 0.712471, 0.468437, -0.073494, -0.103821, 0.674011, 0.510559, -0.080750,
    -0.099220, 0.634361, 0.552406, -0.087547, -0.093750, 0.593750, 0.593750, -0.093750, -0.087547, 0.552406, 0.634361, -0.099220,
    -0.080750, 0.510559, 0.674011, -0.103821, -0.073494, 0.468437, 0.712471, -0.107414, -0.065918, 0.426270, 0.749512, -0.109863,
    -0.058159, 0.384285, 0.784904, -0.111031, -0.050354, 0.342712, 0.818420, -0.110779, -0.042641, 0.301781, 0.849831, -0.108971,
    -0.035156, 0.261719, 0.878906, -0.105469, -0.028038, 0.222755, 0.905418, -0.100136, -0.021423, 0.185120, 0.929138, -0.092834,
    -0.015450, 0.149040, 0.949837, -0.083427, -0.010254, 0.114746, 0.967285, -0.071777, -0.005974, 0.082466, 0.981255, -0.057747,
    -0.002747, 0.052429, 0.991516, -0.041199, -0.000710, 0.024864, 0.997841, -0.021996,

    // Lanczos4
    // offset 192
     0.000000,  0.000000,  0.000000,  1.000000,  0.000000,  0.000000,  0.000000,  0.000000, -0.002981,  0.009625, -0.027053,  0.998265,
     0.029187, -0.010246,  0.003264, -0.000062, -0.005661,  0.018562, -0.051889,  0.993077,  0.060407, -0.021035,  0.006789, -0.000250,
    -0.008027,  0.026758, -0.074449,  0.984478,  0.093543, -0.032281,  0.010545, -0.000567, -0.010071,  0.034167, -0.094690,  0.972534,
     0.128459, -0.043886,  0.014499, -0.001012, -0.011792,  0.040757, -0.112589,  0.957333,  0.165004, -0.055744,  0.018613, -0.001582,
    -0.013191,  0.046507, -0.128145,  0.938985,  0.203012, -0.067742,  0.022845, -0.002271, -0.014275,  0.051405, -0.141372,  0.917621,
     0.242303, -0.079757,  0.027146, -0.003071, -0.015054,  0.055449, -0.152304,  0.893389,  0.282684, -0.091661,  0.031468, -0.003971,
    -0.015544,  0.058648, -0.160990,  0.866453,  0.323952, -0.103318,  0.035754, -0.004956, -0.015761,  0.061020, -0.167496,  0.836995,
     0.365895, -0.114591,  0.039949, -0.006011, -0.015727,  0.062590, -0.171900,  0.805208,  0.408290, -0.125335,  0.043992, -0.007117,
    -0.015463,  0.063390, -0.174295,  0.771299,  0.450908, -0.135406,  0.047823, -0.008254, -0.014995,  0.063460, -0.174786,  0.735484,
     0.493515, -0.144657,  0.051378, -0.009399, -0.014349,  0.062844, -0.173485,  0.697987,  0.535873, -0.152938,  0.054595, -0.010527,
    -0.013551,  0.061594, -0.170517,  0.659039,  0.577742, -0.160105,  0.057411, -0.011613, -0.012630,  0.059764, -0.166011,  0.618877,
     0.618877, -0.166011,  0.059764, -0.012630, -0.011613,  0.057411, -0.160105,  0.577742,  0.659039, -0.170517,  0.061594, -0.013551,
    -0.010527,  0.054595, -0.152938,  0.535873,  0.697987, -0.173485,  0.062844, -0.014349, -0.009399,  0.051378, -0.144657,  0.493515,
     0.735484, -0.174786,  0.063460, -0.014995, -0.008254,  0.047823, -0.135406,  0.450908,  0.771299, -0.174295,  0.063390, -0.015463,
    -0.007117,  0.043992, -0.125336,  0.408290,  0.805208, -0.171900,  0.062590, -0.015727, -0.006011,  0.039949, -0.114591,  0.365895,
     0.836995, -0.167496,  0.061020, -0.015761, -0.004956,  0.035754, -0.103318,  0.323952,  0.866453, -0.160990,  0.058648, -0.015544,
    -0.003971,  0.031468, -0.091661,  0.282684,  0.893389, -0.152304,  0.055449, -0.015054, -0.003071,  0.027146, -0.079757,  0.242303,
     0.917621, -0.141372,  0.051405, -0.014275, -0.002271,  0.022845, -0.067742,  0.203012,  0.938985, -0.128145,  0.046507, -0.013191,
    -0.001582,  0.018613, -0.055744,  0.165004,  0.957333, -0.112589,  0.040757, -0.011792, -0.001012,  0.014499, -0.043886,  0.128459,
     0.972534, -0.094690,  0.034167, -0.010071, -0.000567,  0.010545, -0.032281,  0.093543,  0.984478, -0.074449,  0.026758, -0.008027,
    -0.000250,  0.006789, -0.021035,  0.060407,  0.993077, -0.051889,  0.018562, -0.005661, -0.000062,  0.003264, -0.010246,  0.029187,
     0.998265, -0.027053,  0.009625, -0.002981,

    // Colors
    // offset 448
    0.0,   0.0,   0.0,     0.0, // None
    255.0, 0.0,   0.0,   255.0, // Red
    0.0,   255.0, 0.0,   255.0, // Green
    0.0,   0.0,   255.0, 255.0, // Blue
    254.0, 251.0, 71.0,  255.0, // Yellow
    200.0, 200.0, 0.0,   255.0, // Yellow2
    255.0, 0.0,   255.0, 255.0, // Magenta
    0.0,   128.0, 255.0, 255.0, // Blue2
    0.0,   200.0, 200.0, 255.0, // Blue3

    // Alphas
    // offset 484
    1.0, 0.75, 0.50, 0.25,
];

// const COLORS: [Vector4<f32>; 9] = [
//     Vector4::new(0.0,   0.0,   0.0,     0.0), // None
//     Vector4::new(255.0, 0.0,   0.0,   255.0), // Red
//     Vector4::new(0.0,   255.0, 0.0,   255.0), // Green
//     Vector4::new(0.0,   0.0,   255.0, 255.0), // Blue
//     Vector4::new(254.0, 251.0, 71.0,  255.0), // Yellow
//     Vector4::new(200.0, 200.0, 0.0,   255.0), // Yellow2
//     Vector4::new(255.0, 0.0,   255.0, 255.0), // Magenta
//     Vector4::new(0.0,   128.0, 255.0, 255.0), // Blue2
//     Vector4::new(0.0,   200.0, 200.0, 255.0)  // Blue3
// ];
// const ALPHAS: [f32; 4] = [ 1.0, 0.75, 0.50, 0.25 ];

impl Stabilization {
    pub fn undistort_image_cpu_spirv<T: PixelType>(buffers: &mut Buffers, params: &KernelParams, distortion_model: &DistortionModel, digital_lens: Option<&DistortionModel>, matrices: &[[f32; 16]], drawing: &[u8]) -> bool {
        if let BufferSource::Cpu { buffer: input } = &mut buffers.input.data {
            if let BufferSource::Cpu { buffer: output } = &mut buffers.output.data {
                if buffers.output.size.2 <= 0 {
                    log::error!("buffers.output_size: {:?}", buffers.output.size);
                    return false;
                }

                output.par_chunks_mut(buffers.output.size.2).enumerate().for_each(|(y, row_bytes)| { // Parallel iterator over buffer rows
                    row_bytes.chunks_mut(params.bytes_per_pixel as usize).enumerate().for_each(|(x, pix_chunk)| { // iterator over row pixels
                        let matrices2: &[f32] = unsafe { std::slice::from_raw_parts(matrices.as_ptr() as *const f32, matrices.len() * 16 ) };
                        let params2: stabilize_spirv::KernelParams  = unsafe { std::mem::transmute(*params) };
                        let drawing2: &[u32]  = unsafe { std::slice::from_raw_parts(drawing.as_ptr() as *const u32, drawing.len() / 4 ) };

                        let color = stabilize_spirv::undistort(
                            stabilize_spirv::glam::vec2(x as f32, y as f32),
                            &params2,
                            matrices2,
                            &COEFFS,
                            &[],
                            drawing2,
                            &(input, T::to_float_glam),
                            0.0,
                            params.interpolation as _,
                            params.distortion_model as u32,
                            params.digital_lens as u32,
                            params.flags as u32
                        );

                        let pix_out: &mut T = bytemuck::from_bytes_mut(pix_chunk); // treat this byte chunk as `T`
                        *pix_out = PixelType::from_float_glam(color);
                    });
                });
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// One output pixel -> the source pixel it samples: the render itself, so anything that needs to know
    /// what the picture does (the STMap export, the tests) can ask the same code the frame is drawn with
    /// rather than a copy of it that drifts. `out_c`/`out_f` are the output plane the projection reads,
    /// `(pixel - out_c) / out_f`, built by the caller because they are the same for every pixel
    pub fn undistort_coord(mut out_pos: Vector2<f32>, params: &KernelParams, matrices: &[[f32; 16]], distortion_model: &DistortionModel, digital_lens: Option<&DistortionModel>, mesh_data: &[f64], out_c: &Vector2<f32>, out_f: &Vector2<f32>) -> Option<Vector2<f32>> {
        out_pos.x = map_coord(out_pos.x, params.output_rect[0] as f32, (params.output_rect[0] + params.output_rect[2]) as f32, 0.0, params.output_width  as f32);
        out_pos.y = map_coord(out_pos.y, params.output_rect[1] as f32, (params.output_rect[1] + params.output_rect[3]) as f32, 0.0, params.output_height as f32);
        out_pos.x += params.translation2d[0];
        out_pos.y += params.translation2d[1];

        ///////////////////////////////////////////////////////////////////
        // Output pixel -> ray. A ray of angle θ lands at `(1-a)·R(θ) + a·P(θ)` of the output plane -
        // where the source lens images it, mixed with where the output projection wants it (see
        // `super::projection`). This is that inverted: both maps rise with θ, so the mix does too, and
        // the root is bracketed by the angles the two projections would each have given on their own.
        let a = params.lens_correction_amount;
        let n_raw = (out_pos - out_c).component_div(out_f);
        let mut n = n_raw;
        if (params.flags & 2) == 2 && a < 1.0 { // Has digial lens
            if let Some(digital) = digital_lens {
                // Apply the digital warp in the UN-zoomed (fov=1) frame so it's FOV-independent,
                let uz = ((out_pos.x - out_c.x) * params.fov + out_c.x, (out_pos.y - out_c.y) * params.fov + out_c.y);
                if let Some(pt) = digital.undistort_point(uz, params) {
                    n = Vector2::new((pt.0 - out_c.x) / params.fov, (pt.1 - out_c.y) / params.fov).component_div(out_f);
                }
            }
        }
        let proj_ray = projection::unproject((n_raw.x, n_raw.y), params.output_projection);
        let mut ray = proj_ray;
        if a < 1.0 {
            // With a digital warp the two legs read different planes - the source lens sees the frame the
            // warp came from - so the target the blend has to reach is mixed the same way as the legs are,
            // which leaves both ends exact and costs nothing when there is no warp (`n` is then `n_raw`)
            let nb = n * (1.0 - a) + n_raw * a;
            let t = (nb.x * nb.x + nb.y * nb.y).sqrt();
            if t > 1e-9 {
                let u = (nb.x / t, nb.y / t);
                // The two ends of the bracket: what the output projection alone says (the blend is at or
                // below `t` there) and what the source lens alone says (at or above)
                let lo0 = (proj_ray.0 * proj_ray.0 + proj_ray.1 * proj_ray.1).sqrt();
                let src = distortion_model.undistort_point((n.x, n.y), params);
                // Under water the lens's angles are the housing's; the bracket is in the water's, the inverse of
                // Snell's law away - and no ray past the flat port's 90° ever enters the housing
                let refr = params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0;
                let water = |th: f32| -> f32 {
                    if refr { (th.min(std::f32::consts::FRAC_PI_2).sin() / params.light_refraction_coefficient).clamp(-1.0, 1.0).asin() } else { th }
                };
                let (mut hi0, mut edge) = match src {
                    Some(s) => (water((s.0 * s.0 + s.1 * s.1).sqrt()), false),
                    // Past the edge of its own image the model has no inverse, but the lens still reaches
                    // to its field limit and the blend may well land inside `t` before then
                    None if params.field_limit > 0.0 => (water(params.field_limit), true),
                    None => return None
                };
                // ... and the root cannot be past the angle at which the output projection alone would already have
                // used up `t` (`a·P(θ) <= t` at the root), and pinning the bracket there keeps both residuals
                // of the order of `t`. Without it a lens that sees past 90° hands `P`'s "no image" sentinel to
                // the solve, and a secant cannot move against 1e9: the blend then silently stalled at the
                // fully corrected angle outside a circle at `R(90°)`
                if a > 0.0 { hi0 = hi0.min(projection::theta_of_radius(t / a, params.output_projection)); }
                if a <= 0.0 {
                    // Exactly the source lens's own ray, tangential terms and all - with the refraction
                    // `rotate_and_distort` applies on the way out undone here, or the round trip isn't one
                    let mut s = src?;
                    if refr {
                        let th = (s.0 * s.0 + s.1 * s.1).sqrt();
                        if th > 1e-12 { let k = water(th) / th; s = (s.0 * k, s.1 * k); }
                    }
                    ray = s;
                } else {
                    // `(1-a)·R(θ) + a·P(θ) - t`, with the refraction the render applies on the way out
                    let g = |theta: f32| -> f32 {
                        let mut th = theta;
                        if params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0 {
                            th = (th.sin() * params.light_refraction_coefficient).clamp(-1.0, 1.0).asin();
                        }
                        let (s, c) = (th.sin(), th.cos());
                        let p = distortion_model.distort_point(u.0 * s, u.1 * s, c, params);
                        let r = (p.0 * p.0 + p.1 * p.1).sqrt();
                        (1.0 - a) * r + a * projection::radius_of_theta(theta, params.output_projection) - t
                    };
                    let (mut lo, mut hi) = (lo0.min(hi0), lo0.max(hi0));
                    // The lens ends at its field limit - under water, at Snell's window - and the projection's own
                    // angle may well lie past it; the bracket ends there too, and past it is background
                    let cap = water(if params.field_limit > 0.0 { params.field_limit } else { std::f32::consts::PI });
                    if hi > cap { hi = cap; lo = lo.min(cap); edge = true; }
                    let (mut glo, mut ghi) = (g(lo), g(hi));
                    let mut theta = hi;
                    // Near the axis every projection agrees, so both ends of the bracket are a difference
                    // of near-equal numbers and the sign of `g` there is float noise - GPU transcendentals
                    // make that band tens of pixels wide. An end of the bracket that IS the source lens's
                    // own answer always has a root beside it, so the end itself is the answer; only when the
                    // end is the field limit - or Snell's window - has the lens really run out, and the pixel is background
                    if ghi < 0.0 {
                        if edge { return None; }
                    } else {
                        // The projection's own angle overshoots - refraction magnifies the source leg - so bracket from the axis, where the residual is -t
                        if glo >= 0.0 { lo = 0.0; glo = -t; }
                        // Regula falsi, Illinois variant: the bracket is tight (the two projections'
                        // own answers), and halving the stale end keeps it from crawling in from one side
                        let mut side = 0i32;
                        for _ in 0..6 {
                            let d = ghi - glo;
                            theta = if d.abs() > 1e-12 { (lo - glo * (hi - lo) / d).clamp(lo, hi) } else { 0.5 * (lo + hi) };
                            let gt = g(theta);
                            if gt < 0.0 {
                                lo = theta; glo = gt;
                                if side == -1 { ghi *= 0.5; }
                                side = -1;
                            } else {
                                hi = theta; ghi = gt;
                                if side == 1 { glo *= 0.5; }
                                side = 1;
                            }
                        }
                    }
                    // The source lens's own ray is off its radius by the tangential terms; that offset belongs to the
                    // source leg, so it fades out with it. It is measured against `n`, the point that leg reads: a
                    // digital warp moves `n` off `u`, and that is no offset of the lens's
                    let dir = match src {
                        Some(s) => {
                            let sl = (s.0 * s.0 + s.1 * s.1).sqrt();
                            let nl = (n.x * n.x + n.y * n.y).sqrt();
                            if sl > 1e-12 && nl > 1e-12 {
                                let d = (u.0 + (1.0 - a) * (s.0 / sl - n.x / nl), u.1 + (1.0 - a) * (s.1 / sl - n.y / nl));
                                let dl = (d.0 * d.0 + d.1 * d.1).sqrt();
                                if dl > 1e-12 { (d.0 / dl, d.1 / dl) } else { u }
                            } else { u }
                        },
                        None => u
                    };
                    ray = (dir.0 * theta, dir.1 * theta);
                }
            }
        }
        ///////////////////////////////////////////////////////////////////

        ///////////////////////////////////////////////////////////////////
        // Calculate source `y` for rolling shutter
        let mut sy = if (params.flags & 16) == 16 { // Horizontal RS
            (out_pos.x.round() as i32).min(params.width).max(0) as usize
        } else {
            (out_pos.y.round() as i32).min(params.height).max(0) as usize
        };
        if params.matrix_count > 1 {
            let idx = params.matrix_count as usize / 2;
            if let Some(pt) = Stabilization::rotate_and_distort(ray, idx, params, matrices, distortion_model, digital_lens, mesh_data) {
                if (params.flags & 16) == 16 { // Horizontal RS
                    sy = (pt.0.round() as i32).min(params.width).max(0) as usize;
                } else {
                    sy = (pt.1.round() as i32).min(params.height).max(0) as usize;
                }
            }
        }
        ///////////////////////////////////////////////////////////////////

        let idx = sy.min(params.matrix_count as usize - 1);
        let mut uv = Stabilization::rotate_and_distort(ray, idx, params, matrices, distortion_model, digital_lens, mesh_data)?;
        let mut frame_size = (params.width as f32, params.height as f32);
        if params.input_rotation != 0.0 {
            let rotation = params.input_rotation * (std::f32::consts::PI / 180.0);
            let size = frame_size;
            frame_size = rotate_point(size, rotation, (0.0, 0.0), (0.0, 0.0));
            frame_size = (frame_size.0.abs().round(), frame_size.1.abs().round());
            uv = rotate_point(uv, rotation, (size.0 / 2.0, size.1 / 2.0), (frame_size.0 / 2.0, frame_size.1 / 2.0));
        }

        let width_f = params.width as f32;
        let height_f = params.height as f32;
        if params.background_mode == 1 { // Edge repeat
            uv = (
                uv.0.max(3.0).min(width_f  - 3.0),
                uv.1.max(3.0).min(height_f - 3.0),
            );
        } else if params.background_mode == 2 { // Edge mirror
            let rx = uv.0.round();
            let ry = uv.1.round();
            let width3 = width_f - 3.0;
            let height3 = height_f - 3.0;
            if rx > width3  { uv.0 = width3  - (rx - width3); }
            if rx < 3.0     { uv.0 = 3.0 + width_f - (width3  + rx); }
            if ry > height3 { uv.1 = height3 - (ry - height3); }
            if ry < 3.0     { uv.1 = 3.0 + height_f - (height3 + ry); }
        }
        if params.background_mode != 3 {
            uv = (
                map_coord(uv.0, 0.0, frame_size.0, params.source_rect[0] as f32, (params.source_rect[0] + params.source_rect[2]) as f32),
                map_coord(uv.1, 0.0, frame_size.1, params.source_rect[1] as f32, (params.source_rect[1] + params.source_rect[3]) as f32)
            );
        }
        Some(Vector2::new(uv.0, uv.1))
    }

    /// Ray -> source pixel. `ray` is an angle vector (`θ·û`, see [`super::projection`]) in the output
    /// camera's frame; the matrix row rotates it into the source camera, and the lens images it.
    pub fn rotate_and_distort(ray: (f32, f32), idx: usize, params: &KernelParams, matrices: &[[f32; 16]], distortion_model: &DistortionModel, digital_lens: Option<&DistortionModel>, mesh_data: &[f64]) -> Option<(f32, f32)> {
        let matrices = matrices[idx];
        let d = projection::ray_to_dir(ray);
        let mut _x = (d.0 * matrices[0]) + (d.1 * matrices[1]) + (d.2 * matrices[2]) + params.translation3d[0];
        let mut _y = (d.0 * matrices[3]) + (d.1 * matrices[4]) + (d.2 * matrices[5]) + params.translation3d[1];
        let mut _w = (d.0 * matrices[6]) + (d.1 * matrices[7]) + (d.2 * matrices[8]) + params.translation3d[2];
        {
            let rxy = (_x * _x + _y * _y).sqrt();
            // The ray's angle in the source camera. Past the lens's own field, or past a fold of its
            // calibration, it has no image at all
            let theta = rxy.atan2(_w);
            if params.field_limit > 0.0 && theta > params.field_limit { return None; }

            // Refraction (underwater): Snell's law on the angle itself, exact for any field. Past the critical angle - or behind the flat port - no ray enters the housing at all: Snell's window ends there
            if params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0 && rxy > 1e-12 {
                let sin_d = theta.sin() * params.light_refraction_coefficient;
                if sin_d >= 1.0 || _w <= 0.0 { return None; }
                let theta_d = sin_d.asin();
                let (s, c) = (theta_d.sin() / rxy, theta_d.cos());
                _x *= s; _y *= s; _w = c;
            }

            let mut uv = distortion_model.distort_point(_x, _y, _w, &params);
            // Focus breathing: this row's magnification of the source image (`gyro_source::sony::breathing`)
            if matrices[14] > 0.0 { uv = (uv.0 * matrices[14], uv.1 * matrices[14]); }
            uv = (uv.0 * params.f[0], uv.1 * params.f[1]);

            if matrices[9] != 0.0 || matrices[10] != 0.0 || matrices[11] != 0.0 || matrices[12] != 0.0 || matrices[13] != 0.0 {
                // The camera applies the sensor roll before the sensor/lens shift, so undo the shift first and then the roll
                let ang_rad = matrices[11];
                let cos_a = (-ang_rad).cos();
                let sin_a = (-ang_rad).sin();
                let shifted = (uv.0 - matrices[9] + matrices[12], uv.1 - matrices[10] + matrices[13]);
                uv = (
                    cos_a * shifted.0 - sin_a * shifted.1,
                    sin_a * shifted.0 + cos_a * shifted.1
                );
            }

            uv = (uv.0 + params.c[0], uv.1 + params.c[1]);

            if (params.flags & 512) == 512 && !mesh_data.is_empty() && mesh_data[0] > 10.0 {
                let mesh_size = (mesh_data[3], mesh_data[4]);
                let origin    = (mesh_data[5] as f32, mesh_data[6] as f32);
                let crop_size = (mesh_data[7] as f32, mesh_data[8] as f32);

                if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; } // framebuffer inverted

                uv.0 = map_coord(uv.0, 0.0, params.width  as f32, origin.0, origin.0 + crop_size.0);
                uv.1 = map_coord(uv.1, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

                let q = (uv.0 as f64, uv.1 as f64);
                let mut new_pos = crate::gyro_source::interpolate_mesh(q.0, q.1, (mesh_size.0, mesh_size.1), mesh_data);
                // The 9x9 inverse mesh is only approximate for large warps, refine against the camera's forward mesh (fwd(p) = q).
                // The block is only there when the mesh needs it (`sony::MESH_REFINE_THRESHOLD_PX`), and a first correction
                // that is already tiny leaves nothing for a second one (`sony::MESH_REFINE_SKIP_PX`)
                let o = mesh_data[0] as usize;
                let fwd = o + 4 + 2 * (mesh_data[o].max(0.0) as usize);
                if mesh_data.len() > fwd + 9 && mesh_data[fwd] > 10.0 {
                    let skip_sq = crate::gyro_source::MESH_REFINE_SKIP_PX * crate::gyro_source::MESH_REFINE_SKIP_PX;
                    for _ in 0..2 {
                        let f = crate::gyro_source::interpolate_mesh(new_pos.x, new_pos.y, (mesh_size.0, mesh_size.1), &mesh_data[fwd..]);
                        let (dx, dy) = (q.0 - f.x, q.1 - f.y);
                        new_pos.x += dx;
                        new_pos.y += dy;
                        if dx * dx + dy * dy < skip_sq { break; }
                    }
                }

                uv.0 = map_coord(new_pos.x as f32, origin.0, origin.0 + crop_size.0, 0.0, params.width  as f32);
                uv.1 = map_coord(new_pos.y as f32, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);

                if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; } // framebuffer inverted
            }

            // FocalPlaneDistortion
            if (params.flags & 1024) == 1024 && !mesh_data.is_empty() && mesh_data[0] > 0.0 && mesh_data[mesh_data[0] as usize] > 0.0 {
                let o = mesh_data[0] as usize; // offset to focal plane distortion data

                let mesh_size = (mesh_data[3], mesh_data[4]);
                let origin    = (mesh_data[5] as f32, mesh_data[6] as f32);
                let crop_size = (mesh_data[7] as f32, mesh_data[8] as f32);
                let stblz_grid = if mesh_data[o + 2] > 0.0 { mesh_data[o + 2] } else { mesh_size.1 / 8.0 }; // band height comes with the table

                if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; } // framebuffer inverted

                uv.0 = map_coord(uv.0, 0.0, params.width  as f32, origin.0, origin.0 + crop_size.0);
                uv.1 = map_coord(uv.1, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

                let idx = (uv.1 as f64 / stblz_grid).floor().max(0.0).min(7.0) as usize;
                let delta = uv.1 as f64 - stblz_grid * idx as f64;
                uv.0 -= (mesh_data[o + 4 + idx * 2 + 0] * delta) as f32;
                uv.1 -= (mesh_data[o + 4 + idx * 2 + 1] * delta) as f32;
                for j in 0..idx {
                    uv.0 -= (mesh_data[o + 4 + j * 2 + 0] * stblz_grid) as f32;
                    uv.1 -= (mesh_data[o + 4 + j * 2 + 1] * stblz_grid) as f32;
                }

                uv.0 = map_coord(uv.0, origin.0, origin.0 + crop_size.0, 0.0, params.width  as f32);
                uv.1 = map_coord(uv.1, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);

                if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; } // framebuffer inverted
            }

            if (params.flags & 2) == 2 { // Has digital lens
                if let Some(digital) = digital_lens {
                    uv = digital.distort_point(uv.0, uv.1, 1.0, params);
                }
            }

            if params.input_horizontal_stretch > 0.001 { uv.0 /= params.input_horizontal_stretch; }
            if params.input_vertical_stretch   > 0.001 { uv.1 /= params.input_vertical_stretch; }

            // Local optical stabilization is measured in raw decoded-frame pixels, so it is the final
            // source-side warp. reserved2 points at its transient mesh block appended after camera metadata.
            if (params.flags & 4096) == 4096 {
                let base = params.reserved2.max(0.0) as usize;
                if let Some(optical) = mesh_data.get(base..).filter(|m| m.len() > 9 && m[0] > 10.0) {
                    let mesh_size = (optical[3], optical[4]);
                    let origin = (optical[5] as f32, optical[6] as f32);
                    let crop_size = (optical[7] as f32, optical[8] as f32);
                    if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; }
                    uv.0 = map_coord(uv.0, 0.0, params.width as f32, origin.0, origin.0 + crop_size.0);
                    uv.1 = map_coord(uv.1, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

                    let q = (uv.0 as f64, uv.1 as f64);
                    let mut new_pos = crate::gyro_source::interpolate_mesh(q.0, q.1, mesh_size, optical);
                    let o = optical[0] as usize;
                    let fwd = o + 4 + 2 * optical.get(o).copied().unwrap_or(0.0).max(0.0) as usize;
                    if optical.len() > fwd + 9 && optical[fwd] > 10.0 {
                        for _ in 0..2 {
                            let f = crate::gyro_source::interpolate_mesh(new_pos.x, new_pos.y, mesh_size, &optical[fwd..]);
                            let (dx, dy) = (q.0 - f.x, q.1 - f.y);
                            new_pos.x += dx;
                            new_pos.y += dy;
                            if dx * dx + dy * dy < 0.0625 { break; }
                        }
                    }
                    uv.0 = map_coord(new_pos.x as f32, origin.0, origin.0 + crop_size.0, 0.0, params.width as f32);
                    uv.1 = map_coord(new_pos.y as f32, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);
                    if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; }
                }
            }

            Some(uv)
        }
    }

    // Adapted from OpenCV: initUndistortRectifyMap + remap
    // https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L465-L567
    // https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/imgproc/src/opencl/remap.cl#L390-L498
    pub fn undistort_image_cpu<const I: i32, T: PixelType>(buffers: &mut Buffers, params: &KernelParams, distortion_model: &DistortionModel, digital_lens: Option<&DistortionModel>, matrices: &[[f32; 16]], drawing: &[u8], mesh_data: &[f32]) -> bool {
        // #[cold]
        // fn draw_pixel(pix: &mut Vector4<f32>, x: i32, y: i32, is_input: bool, width: i32, params: &KernelParams, drawing: &[u8]) {
        //     if drawing.is_empty() || (params.flags & 8) == 0 { return; }
        //     let pos = ((y as f32 / params.canvas_scale).floor() * (width as f32) + (x as f32 / params.canvas_scale).floor()).round() as usize;
        //     if let Some(&data) = drawing.get(pos) {
        //         if data > 0 {
        //             let color = (data & 0xF8) >> 3;
        //             let alpha = (data & 0x06) >> 1;
        //             let stage = data & 1;
        //             if ((stage == 0 && is_input) || (stage == 1 && !is_input)) && color < 9 {
        //                 let colorf = COLORS[color as usize];
        //                 let alphaf = ALPHAS[alpha as usize];
        //                 *pix = colorf * alphaf + *pix * (1.0 - alphaf);
        //                 pix.w = 255.0;
        //             }
        //         }
        //     }
        // }

        // From 0-255(JPEG/Full) to 16-235(MPEG/Limited)
        #[cold]
        fn remap_colorrange(px: &mut Vector4<f32>, is_y: bool) {
            if is_y { *px *= 0.85882352; } // (235 - 16) / 255
            else    { *px *= 0.87843137; } // (240 - 16) / 255
            px[0] += 16.0;
            px[1] += 16.0;
        }

        ////////////////////////////// EWA (Elliptical Weighted Average) CubicBC sampling //////////////////////////////
        // Keys Cubic Filter Family https://imagemagick.org/Usage/filter/#robidoux
        // https://github.com/ImageMagick/ImageMagick/blob/main/MagickCore/resize.c

        // Gives a bounding box in the source image containing pixels that cover a circle of radius 2 completely in both the source and destination images
        fn affine_bbox(jac: &Vector4<f32>) -> Vector2<f32> {
            const MAX_SUPPORT: f32 = 64.0;
            return Vector2::new(
                (2.0 * ((jac.x + jac.y).abs().max((jac.x - jac.y).abs()).max(1.0))).min(MAX_SUPPORT),
                (2.0 * ((jac.z + jac.w).abs().max((jac.z - jac.w).abs()).max(1.0))).min(MAX_SUPPORT)
            );
        }
        // Computes minimum area ellipse which covers a unit circle in both the source and destination image
        fn clamped_ellipse(jac: &Vector4<f32>) -> Vector3<f32> {
            // find ellipse
            let f0 = (jac.x * jac.w - jac.y * jac.z).abs();
            let f = (f0 * f0).max(0.1);
            let a = (jac.z * jac.z + jac.w * jac.w) / f;
            let b = -2.0 * (jac.x * jac.z + jac.y * jac.w) / f;
            let c = (jac.x * jac.x + jac.y * jac.y) / f;
            // find the angle to rotate ellipse
            let v = Vector2::<f32>::new(c - a, -b);
            let lv = v.norm();
            let v0 = if lv > 0.01 { v.x / lv } else { 1.0 };
            // let v1 = if lv > 0.01 { v.y / lv } else { 1.0 };
            let cc = ((1.0 + v0).max(0.0) / 2.0).sqrt();
            let mut s = ((1.0 - v0).max(0.0) / 2.0).sqrt();
            // rotate the ellipse to align it with axes
            let mut a0 = a * cc * cc - b * cc * s + c * s * s;
            let mut c0 = a * s * s + b * cc * s + c * cc * cc;
            let bt1 = b * (cc * cc - s * s);
            let bt2 = 2.0 * (a - c) * cc * s;
            let mut b0 = bt1 + bt2;
            let b0v2 = bt1 - bt2;
            if b0.abs() > b0v2.abs() {
                s = -s;
                b0 = b0v2;
            }
            // clamp A,C
            a0 = a0.min(1.0);
            c0 = c0.min(1.0);
            let sn = -s;
            // rotate it back
            Vector3::new(
                a0 * cc * cc - b0 * cc * sn + c0 * sn * sn,
                2.0 * a0 * cc * sn + b0 * cc * cc - b0 * sn * sn - 2.0 * c0 * cc * sn,
                a0 * sn * sn + b0 * cc * sn + c0 * cc * cc
            )
        }
        fn bc2(x: f32, params: &KernelParams) -> f32 {
            let x = x.abs();
            unsafe {
                let x2 = x * x;
                if x < 1.0 {
                    return params.ewa_coeffs_p.get_unchecked(0) + params.ewa_coeffs_p.get_unchecked(1) * x + params.ewa_coeffs_p.get_unchecked(2) * x2 + params.ewa_coeffs_p.get_unchecked(3) * x2 * x;
                } else if x < 2.0 {
                    return params.ewa_coeffs_q.get_unchecked(0) + params.ewa_coeffs_q.get_unchecked(1) * x + params.ewa_coeffs_q.get_unchecked(2) * x2 + params.ewa_coeffs_q.get_unchecked(3) * x2 * x;
                }
            }
            0.0
        }
        ////////////////////////////// EWA (Elliptical Weighted Average) CubicBC sampling //////////////////////////////

        fn sample_input_at<const I: i32, T: PixelType>(uv: Vector2<f32>, jac: &Vector4<f32>, input: &[u8], params: &KernelParams, bg: &Vector4<f32>, _drawing: &[u8]) -> Vector4<f32> {
            let mut sum = Vector4::from_element(0.0);
            if I > 8 {
                // find how many pixels we need around that pixel in each direction
                let trans_size = affine_bbox(jac);
                let bounds = (
                    (uv.x - trans_size.x).floor() as i32,
                    (uv.x + trans_size.x).ceil() as i32,
                    (uv.y - trans_size.y).floor() as i32,
                    (uv.y + trans_size.y).ceil() as i32
                );
                let mut sum_div = 0.0;
                let mut src_index = bounds.2 * params.stride;

                // See: Andreas Gustafsson. "Interactive Image Warping", section 3.6 http://www.gson.org/thesis/warping-thesis.pdf
                let abc = clamped_ellipse(jac);
                for in_y in bounds.2..=bounds.3 {
                    let in_fy = in_y as f32 - uv.y;
                    let in_fy2 = in_fy * abc.y;
                    let in_fy3 = in_fy * in_fy * abc.z;
                    for in_x in bounds.0..=bounds.1 {
                        let in_fx = in_x as f32 - uv.x;
                        let dr = in_fx * in_fx * abc.x + in_fx * in_fy2 + in_fy3;
                        let k = bc2(dr.sqrt(), params); // cylindrical filtering
                        if k == 0.0 {
                            continue;
                        }
                        let pixel = if in_y >= params.source_rect[1] && in_y < params.source_rect[1] + params.source_rect[3] && in_x >= params.source_rect[0] && in_x < params.source_rect[0] + params.source_rect[2] {
                            let px1: &T = bytemuck::from_bytes(&input[src_index as usize + (params.bytes_per_pixel * in_x) as usize..src_index as usize + (params.bytes_per_pixel * (in_x + 1)) as usize]);
                            let src_px = PixelType::to_float(*px1);
                            // draw_pixel(&mut src_px, sx + xp, sy + yp, true, params.width, params, drawing);
                            src_px
                        } else {
                            *bg
                        };
                        sum += k * pixel;
                        sum_div += k;
                    }
                    src_index += params.stride;
                }
                sum /= sum_div;
            } else {
                const INTER_BITS: usize = 5;
                const INTER_TAB_SIZE: usize = 1 << INTER_BITS;
                let shift: i32 = (I >> 2) + 1;
                let offset: f32 = [0.0, 1.0, 3.0][I as usize >> 2];
                let ind: usize = [0, 64, 64 + 128][I as usize >> 2];

                let u = uv.x - offset;
                let v = uv.y - offset;

                let sx0 = (u * INTER_TAB_SIZE as f32).round() as i32;
                let sy0 = (v * INTER_TAB_SIZE as f32).round() as i32;

                let sx = sx0 >> INTER_BITS;
                let sy = sy0 >> INTER_BITS;

                let coeffs_x = &COEFFS[ind + ((sx0 as usize & (INTER_TAB_SIZE - 1)) << shift)..];
                let coeffs_y = &COEFFS[ind + ((sy0 as usize & (INTER_TAB_SIZE - 1)) << shift)..];

                let mut src_index = sy as isize * params.stride as isize + sx as isize * params.bytes_per_pixel as isize;

                for yp in 0..I {
                    if sy + yp >= params.source_rect[1] && sy + yp < params.source_rect[1] + params.source_rect[3] {
                        let mut xsum = Vector4::<f32>::from_element(0.0);
                        for xp in 0..I {
                            let pixel = if sx + xp >= params.source_rect[0] && sx + xp < params.source_rect[0] + params.source_rect[2] {
                                let px1: &T = bytemuck::from_bytes(&input[src_index as usize + (params.bytes_per_pixel * xp) as usize..src_index as usize + (params.bytes_per_pixel * (xp + 1)) as usize]);
                                let src_px = PixelType::to_float(*px1);
                                // draw_pixel(&mut src_px, sx + xp, sy + yp, true, params.width, params, drawing);
                                src_px
                            } else {
                                *bg
                            };
                            xsum += pixel * coeffs_x[xp as usize];
                        }

                        sum += xsum * coeffs_y[yp as usize];
                    } else {
                        sum += bg * coeffs_y[yp as usize];
                    }
                    src_index += params.stride as isize;
                }
            }
            Vector4::new(
                sum.x.min(params.pixel_value_limit),
                sum.y.min(params.pixel_value_limit),
                sum.z.min(params.pixel_value_limit),
                sum.w.min(params.pixel_value_limit),
            )
        }


        if let BufferSource::Cpu { buffer: input } = &mut buffers.input.data {
            if let BufferSource::Cpu { buffer: output } = &mut buffers.output.data {
                let bg = Vector4::<f32>::new(params.background[0], params.background[1], params.background[2], params.background[3]) * params.max_pixel_value;
                let bg_t: T = PixelType::from_float(bg);

                // The output plane: `(pixel - out_c) / out_f` is the normalized coordinate the output
                // projection reads. It no longer depends on the lens correction amount - the blend happens
                // in ray space - so the `1/(1-amount)` focal fudge that used to sit here is gone, and this
                // is now exactly the camera matrix `FrameTransform::get_new_k` builds for the point path
                let stretch = if params.input_horizontal_stretch > 0.01 { 1.0 / params.input_horizontal_stretch } else { 1.0 };
                let out_c = Vector2::new(params.output_width as f32 / 2.0, params.output_height as f32 / 2.0);
                let out_f = Vector2::new(params.f[0] * stretch / params.fov, params.f[1] * stretch / params.fov);

                // let drawing_enabled = !drawing.is_empty() && (params.flags & 8) == 8;
                let fill_bg = (params.flags & 4) == 4;
                let fix_range = (params.flags & 1) == 1;
                let is_y = params.plane_index == 0;
                if buffers.output.size.2 <= 0 {
                    log::error!("buffers.output_size: {:?}", buffers.output.size);
                    return false;
                }

                let mesh_data = mesh_data.iter().map(|x| *x as f64).collect::<Vec<f64>>();

                assert_eq!(params.bytes_per_pixel as usize, std::mem::size_of::<T>());

                output.par_chunks_mut(buffers.output.size.2).enumerate().for_each(|(y, row_bytes)| { // Parallel iterator over buffer rows
                    row_bytes.chunks_mut(params.bytes_per_pixel as usize).enumerate().for_each(|(x, pix_chunk)| { // iterator over row pixels

                        let out_pos = (
                            map_coord(x as f32, params.output_rect[0] as f32, (params.output_rect[0] + params.output_rect[2]) as f32, 0.0, params.output_width  as f32),
                            map_coord(y as f32, params.output_rect[1] as f32, (params.output_rect[1] + params.output_rect[3]) as f32, 0.0, params.output_height as f32)
                        );

                        if out_pos.0 >= 0.0 && out_pos.1 >= 0.0 && (out_pos.0 as i32) < params.output_width && (out_pos.1 as i32) < params.output_height {

                            // let p = out_pos;
                            let mut pixel = bg;

                            let pix_out = bytemuck::from_bytes_mut(pix_chunk); // treat this byte chunk as `T`

                            if fill_bg {
                                *pix_out = bg_t;
                                return;
                            }

                            let position = Vector2::new(x as f32, y as f32);

                            if let Some(mut uv) = Self::undistort_coord(position, params, matrices, distortion_model, digital_lens, &mesh_data, &out_c, &out_f) {
                                let mut jac = Vector4::new(1.0, 0.0, 0.0, 1.0);
                                if I > 8 {
                                    let eps = 0.01;
                                    let nx = Self::undistort_coord(position + Vector2::new(eps, 0.0), params, matrices, distortion_model, digital_lens, &mesh_data, &out_c, &out_f);
                                    let ny = Self::undistort_coord(position + Vector2::new(0.0, eps), params, matrices, distortion_model, digital_lens, &mesh_data, &out_c, &out_f);
                                    if let (Some(nx), Some(ny)) = (nx, ny) {
                                        let xyx = nx - uv;
                                        let xyy = ny - uv;
                                        jac = Vector4::new(xyx.x / eps, xyy.x / eps, xyx.y / eps, xyy.y / eps);
                                    }
                                }

                                let width_f = params.width as f32;
                                let height_f = params.height as f32;
                                if params.background_mode == 3 { // Margin with feather
                                    let widthf  = width_f - 1.0;
                                    let heightf = height_f - 1.0;

                                    let feather = (params.background_margin_feather * heightf).max(0.0001);
                                    let mut pt2 = uv;
                                    let mut alpha = 1.0;
                                    if (uv.x > widthf - feather) || (uv.x < feather) || (uv.y > heightf - feather) || (uv.y < feather) {
                                        alpha = ((widthf - uv.x).min(heightf - uv.y).min(uv.x).min(uv.y) / feather).min(1.0).max(0.0);
                                        let size_f = Vector2::new(width_f, height_f);
                                        let half = Vector2::from_element(0.5);
                                        pt2.component_div_assign(&size_f);
                                        pt2 = ((pt2 - half) * (1.0 - params.background_margin)) + half;
                                        pt2.component_mul_assign(&size_f);
                                    }

                                    let mut frame_size = (params.width as f32, params.height as f32);
                                    if params.input_rotation != 0.0 {
                                        let rotation = params.input_rotation * (std::f32::consts::PI / 180.0);
                                        let size = frame_size;
                                        frame_size = rotate_point(size, rotation, (0.0, 0.0), (0.0, 0.0));
                                        frame_size = (frame_size.0.abs().round(), frame_size.1.abs().round());
                                    }
                                    uv  = Vector2::new(map_coord(uv.x,  0.0, frame_size.0, params.source_rect[0] as f32, (params.source_rect[0] + params.source_rect[2]) as f32),
                                                       map_coord(uv.y,  0.0, frame_size.1, params.source_rect[1] as f32, (params.source_rect[1] + params.source_rect[3]) as f32));
                                    pt2 = Vector2::new(map_coord(pt2.x, 0.0, frame_size.0, params.source_rect[0] as f32, (params.source_rect[0] + params.source_rect[2]) as f32),
                                                       map_coord(pt2.y, 0.0, frame_size.1, params.source_rect[1] as f32, (params.source_rect[1] + params.source_rect[3]) as f32));

                                    let c1 = sample_input_at::<I, T>(uv, &jac, input, params, &bg, drawing);
                                    let c2 = sample_input_at::<I, T>(pt2, &jac, input, params, &bg, drawing); // FIXME: jac should be adjusted for pt2
                                    pixel = c1 * alpha + c2 * (1.0 - alpha);
                                    // draw_pixel(&mut pixel, p.0 as i32, p.1 as i32, false, params.output_width, params, drawing);
                                    if fix_range {
                                        remap_colorrange(&mut pixel, is_y)
                                    }
                                    *pix_out = PixelType::from_float(pixel);
                                    return;
                                }

                                pixel = sample_input_at::<I, T>(uv, &jac, input, params, &bg, drawing);
                            }
                            // draw_pixel(&mut pixel, p.0 as i32, p.1 as i32, false, params.output_width, params, drawing);

                            if fix_range {
                                remap_colorrange(&mut pixel, is_y)
                            }
                            *pix_out = PixelType::from_float(pixel);
                        }
                    });
                });
                true
            } else {
                false
            }
        } else {
            false
        }
    }
}

/// Source pixels -> their positions in the output frame: the picture's own outline, which is what the zoom
/// search fits the frame into, what the sync matches features in and what the STMap export writes.
///
/// `clamp_to_field` is what the two kinds of caller disagree about, and the reason it is theirs to say:
/// where the lens has no image of a point, the zoom search still wants a boundary point in that direction
/// (a frame corner outside the image circle is still a corner it has to fit around), and the sync wants
/// nothing at all - a stand-in ray at the edge of the field is a displacement nobody measured, and on a
/// fisheye whose image circle sits inside the frame it lands *inside* the frame, passes the callers' own
/// "is it in view" test and feeds the offset search a match that was never there
pub fn undistort_points_with_rolling_shutter(distorted: &[(f32, f32)], timestamp_ms: f64, frame: Option<usize>, params: &ComputeParams, lens_correction_amount: f64, use_fovs: bool, clamp_to_field: bool) -> Vec<(f32, f32)> {
    if distorted.is_empty() { return Vec::new(); }
    let (camera_matrix, distortion_coeffs, new_k, rotations, is, mesh, optical_mesh, fov, field_limit, breathing) = FrameTransform::at_timestamp_for_points(params, distorted, timestamp_ms, frame, use_fovs);

    undistort_points(distorted, camera_matrix, &distortion_coeffs, rotations[0], Some(new_k), Some(rotations), params, lens_correction_amount, fov, timestamp_ms, is, mesh, optical_mesh, field_limit, breathing, clamp_to_field)
}
/// Optical flow features -> the bearings they arrived on: unit ray directions in the camera's own frame,
/// `None` where the lens has no image of the feature.
///
/// Bearings, and deliberately not positions on an image plane: the output projection describes the picture
/// Gyroflow *draws*, not the camera that recorded it, and [`OutputProjection::for_lens`] picks a
/// stereographic one for every lens whose frame reaches past 85° from the axis. Pose estimation and the
/// rolling-shutter sync read a 2D answer as `(x, y, 1)`, so under that projection a 60° ray would reach
/// them as a 49.1° one and every rotation they solve for would come out short.
///
/// [`OutputProjection::for_lens`]: super::projection::OutputProjection::for_lens
pub fn undistort_points_for_optical_flow(distorted: &[(f32, f32)], timestamp_us: i64, params: &ComputeParams, points_dims: (u32, u32)) -> Vec<Option<(f32, f32, f32)>> {
    if distorted.is_empty() { return Vec::new(); }
    let timestamp_ms = timestamp_us as f64 / 1000.0;
    let img_dim_ratio = points_dims.0 as f64 / params.width.max(1) as f64;//FrameTransform::get_ratio(params);

    let (camera_matrix, distortion_coeffs, _, _, _, _, _) = FrameTransform::get_lens_data_at_timestamp(params, timestamp_ms, false);

    // The features were tracked on a frame of their own size, so the lens is read at that scale - and so is
    // the digital lens, which works in whole pixels of the frame it is handed
    let mut kernel_params = point_kernel_params(params, camera_matrix * img_dim_ratio, distortion_coeffs, 0.0, timestamp_ms);
    kernel_params.width  = points_dims.0 as i32; kernel_params.output_width  = points_dims.0 as i32;
    kernel_params.height = points_dims.1 as i32; kernel_params.output_height = points_dims.1 as i32;

    // Optical flow matches real image features in the camera's own frame: no rotation into an output camera,
    // no field clamp, no zoom - just the lens, undone
    undistort_points_to_rays(distorted, &kernel_params, Matrix3::identity(), None, params, None, None, None, None, false)
        .into_iter()
        .map(|ray| ray.map(projection::ray_to_dir).filter(|d| d.0.is_finite() && d.1.is_finite() && d.2.is_finite()))
        .collect()
}
/// [`undistort_points_for_optical_flow`] with everything else the render undoes on the source side of `frame`: its
/// mesh correction, the sensor and lens shifts of in-body and optical stabilization, and lens breathing (when it's
/// on). What the picture moved by beyond these is the camera's own rotation, which is what `optical_motion`
/// measures - a sensor shift taken for one would be corrected twice, once as the shift and once as the rotation.
///
/// The per-frame data is in pixels of the frame itself, so the features are taken there first.
pub fn undistort_points_for_optical_motion(distorted: &[(f32, f32)], timestamp_ms: f64, frame: usize, params: &ComputeParams, points_dims: (u32, u32)) -> Vec<Option<(f32, f32, f32)>> {
    if distorted.is_empty() { return Vec::new(); }
    let scale = (params.width as f32 / points_dims.0.max(1) as f32, params.height as f32 / points_dims.1.max(1) as f32);
    let full: Vec<(f32, f32)> = distorted.iter().map(|p| (p.0 * scale.0, p.1 * scale.1)).collect();

    // `use_fovs` is what brings lens breathing in, the magnification `at_timestamp` applies on the source side; the
    // zoom it's otherwise about isn't read here
    let (camera_matrix, distortion_coeffs, _, _, shifts, mesh, optical_mesh, _, _, breathing) = FrameTransform::at_timestamp_for_points(params, &full, timestamp_ms, Some(frame), true);
    // Without rolling shutter correction the shift is looked up once, for the whole frame
    let shifts = shifts.map(|s| if s.len() == 1 { vec![s[0]; full.len()] } else { s });

    let kernel_params = point_kernel_params(params, camera_matrix, distortion_coeffs, 0.0, timestamp_ms);
    undistort_points_to_rays(&full, &kernel_params, Matrix3::identity(), None, params, shifts.as_deref(), mesh.as_deref(), optical_mesh.as_deref(), breathing.as_deref(), false)
        .into_iter()
        .map(|ray| ray.map(projection::ray_to_dir).filter(|d| d.0.is_finite() && d.1.is_finite() && d.2.is_finite()))
        .collect()
}

/// The block the point path evaluates the lens with: the same one the render fills per frame, minus
/// everything that is about pixels rather than about the lens
fn point_kernel_params(params: &ComputeParams, camera_matrix: Matrix3<f64>, distortion_coeffs: [f64; 24], field_limit: f64, timestamp_ms: f64) -> KernelParams {
    let mut digital_lens_params = [0f32; 16];
    if let Some(p) = &params.digital_lens_params {
        for (i, v) in p.iter().take(16).enumerate() {
            digital_lens_params[i] = *v as f32;
        }
    }

    // TODO more params
    KernelParams {
        width : params.width as i32,
        height: params.height as i32,
        output_width: params.output_width as i32,
        output_height: params.output_height as i32,
        f: [camera_matrix[(0, 0)] as f32, camera_matrix[(1, 1)] as f32],
        c: [camera_matrix[(0, 2)] as f32, camera_matrix[(1, 2)] as f32],
        k: distortion_coeffs.iter().map(|x| *x as f32).collect::<Vec<_>>().try_into().unwrap(),
        digital_lens_params,
        light_refraction_coefficient: params.keyframes.value_at_video_timestamp(&crate::KeyframeType::LightRefractionCoeff, timestamp_ms).unwrap_or(params.light_refraction_coefficient) as f32,
        // The model has to reach exactly as far here as it does in the render, or the lens-correction
        // solve measures a field the picture doesn't have
        field_limit: field_limit as f32,
        output_projection: params.output_projection,

        ..Default::default()
    }
}

/// Source pixels -> the rays they arrived on, in the output camera's frame, as angle vectors (see
/// `stabilization::projection`); `None` where the lens has no image of the point.
///
/// The half of the point path that is about the camera and not about the picture: [`undistort_points`] goes
/// on to project these onto the output plane, the sync reads them as bearings.
fn undistort_points_to_rays(distorted: &[(f32, f32)], kernel_params: &KernelParams, rotation: Matrix3<f64>, rot_per_point: Option<&[Matrix3<f64>]>, params: &ComputeParams, shift_per_point: Option<&[(f32, f32, f32, f32, f32)]>, mesh: Option<&[f64]>, optical_mesh: Option<&[f64]>, breathing_per_point: Option<&[f32]>, clamp_to_field: bool) -> Vec<Option<(f32, f32)>> {
    let f = (kernel_params.f[0], kernel_params.f[1]);
    let c = (kernel_params.c[0], kernel_params.c[1]);
    // Only the picture's own outline gets the edge-of-field stand-in below; a caller measuring real image
    // features (the sync) has to hear "no image here" instead of a ray nothing arrived on
    let field_limit = if clamp_to_field { kernel_params.field_limit } else { 0.0 };

    // Snell's law backwards - the inverse of what `rotate_and_distort` applies going the other way
    let refract = |ray: (f32, f32)| -> (f32, f32) {
        let n = kernel_params.light_refraction_coefficient;
        if n == 1.0 || n <= 0.0 { return ray; }
        let theta = (ray.0 * ray.0 + ray.1 * ray.1).sqrt();
        if theta < 1e-12 { return ray; }
        // A ray from behind the flat port never entered the housing: hold it at the edge of Snell's window
        let s = (theta.min(std::f32::consts::FRAC_PI_2).sin() / n).clamp(-1.0, 1.0).asin() / theta;
        (ray.0 * s, ray.1 * s)
    };

    // Not parallelized here: the only caller that reaches the lens-correction blend
    // (zooming::fov_iterative) already runs this across frames via into_par_iter, and the
    // stmap caller passes one point at a time — so a nested par_iter would only oversubscribe.
    distorted.iter().enumerate().map(|(index, pi)| {
        let mut x = pi.0;
        let mut y = pi.1;

        // Inverse of rotate_and_distort's final optical lookup: raw source pixel -> optically corrected
        // source coordinate, before stretch/digital lens and the camera's own FPD/mesh are undone.
        if let Some(optical) = optical_mesh.filter(|m| m.len() > 9 && m[0] > 10.0) {
            let mesh_size = (optical[3], optical[4]);
            let origin = (optical[5] as f32, optical[6] as f32);
            let crop_size = (optical[7] as f32, optical[8] as f32);
            x = map_coord(x, 0.0, params.width as f32, origin.0, origin.0 + crop_size.0);
            y = map_coord(y, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);
            let p = crate::gyro_source::interpolate_mesh(x as f64, y as f64, mesh_size, optical);
            x = map_coord(p.x as f32, origin.0, origin.0 + crop_size.0, 0.0, params.width as f32);
            y = map_coord(p.y as f32, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);
        }

        if params.lens.input_horizontal_stretch > 0.001 { x *= params.lens.input_horizontal_stretch as f32; }
        if params.lens.input_vertical_stretch   > 0.001 { y *= params.lens.input_vertical_stretch as f32; }

        if let Some(digital) = &params.digital_lens {
            if let Some(pt2) = digital.undistort_point((x, y), &kernel_params) {
                x = pt2.0;
                y = pt2.1;
            }
        }

        if let Some(mesh_data) = mesh.filter(|m| m.len() > 9) {
            // FocalPlaneDistortion
            let o = mesh_data[0] as usize; // offset to focal plane distortion data
            if o > 0 && mesh_data.get(o).map_or(false, |x| *x > 0.0) {

                let mesh_size = (mesh_data[3], mesh_data[4]);
                let origin    = (mesh_data[5] as f32, mesh_data[6] as f32);
                let crop_size = (mesh_data[7] as f32, mesh_data[8] as f32);
                let stblz_grid = if mesh_data[o + 2] > 0.0 { mesh_data[o + 2] } else { mesh_size.1 / 8.0 }; // band height comes with the table

                x = map_coord(x, 0.0, params.width  as f32, origin.0, origin.0 + crop_size.0);
                y = map_coord(y, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

                let idx = (y as f64 / stblz_grid).floor().max(0.0).min(7.0) as usize;
                let delta = y as f64 - stblz_grid * idx as f64;
                x += (mesh_data[o + 4 + idx * 2 + 0] * delta) as f32;
                y += (mesh_data[o + 4 + idx * 2 + 1] * delta) as f32;
                for j in 0..idx {
                    x += (mesh_data[o + 4 + j * 2 + 0] * stblz_grid) as f32;
                    y += (mesh_data[o + 4 + j * 2 + 1] * stblz_grid) as f32;
                }

                x = map_coord(x, origin.0, origin.0 + crop_size.0, 0.0, params.width  as f32);
                y = map_coord(y, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);
            }

            if mesh_data[0] > 10.0 {
                let mesh_size = (mesh_data[3], mesh_data[4]);
                let origin    = (mesh_data[5] as f32, mesh_data[6] as f32);
                let crop_size = (mesh_data[7] as f32, mesh_data[8] as f32);

                x = map_coord(x, 0.0, params.width  as f32, origin.0, origin.0 + crop_size.0);
                y = map_coord(y, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

                let new_pos = crate::gyro_source::interpolate_mesh(x as f64, y as f64, (mesh_size.0, mesh_size.1), &mesh_data);

                x = map_coord(new_pos.x as f32, origin.0, origin.0 + crop_size.0, 0.0, params.width  as f32);
                y = map_coord(new_pos.y as f32, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);
            }
        }
        if let Some(shift) = shift_per_point.and_then(|v| v.get(index)) {
            // Sensor roll around the principal point first, then the sensor/lens shift (the inverse of `rotate_and_distort`)
            let ang_rad = shift.2;
            let cos_a = ang_rad.cos();
            let sin_a = ang_rad.sin();
            let (xr, yr) = (x - c.0, y - c.1);
            x = cos_a * xr - sin_a * yr - shift.3 + shift.0 + c.0;
            y = sin_a * xr + cos_a * yr - shift.4 + shift.1 + c.1;
        }

        let mut pw = ((x - c.0) / f.0, (y - c.1) / f.1); // normalized image point
        // Focus breathing is a magnification of the source image (`at_timestamp` applies it there too), so
        // undoing it is a division, before the lens model is asked for the ray
        if let Some(bz) = breathing_per_point.and_then(|v| v.get(index).or(v.first())).filter(|k| **k > 0.0) {
            pw = (pw.0 / bz, pw.1 / bz);
        }

        let rot = nalgebra::convert::<nalgebra::Matrix3<f64>, nalgebra::Matrix3<f32>>(*rot_per_point.and_then(|v| v.get(index)).unwrap_or(&rotation));

        // Source image -> ray, as an angle vector (see `stabilization::projection`)
        let ray = match params.distortion_model.undistort_point(pw, &kernel_params) {
            Some(pt) => Some(refract(pt)),
            // No image of this point: hold the direction at the edge of the lens's field, so a frame corner
            // outside the image circle still gives the zoom search a boundary point in the right direction
            None if field_limit > 0.0 => {
                let r = (pw.0 * pw.0 + pw.1 * pw.1).sqrt();
                if r > 1e-12 { Some((pw.0 * (field_limit / r), pw.1 * (field_limit / r))) } else { None }
            },
            None => None
        };

        // Rotate into the output camera; the rest of the way - onto its image plane - is the caller's
        ray.map(|ray| {
            let d = projection::ray_to_dir(ray);
            let pr = rot * nalgebra::Vector3::new(d.0, d.1, d.2);
            projection::dir_to_ray((pr[0], pr[1], pr[2]))
        })
    }).collect()
}

// Ported from OpenCV: https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L321
pub fn undistort_points(distorted: &[(f32, f32)], camera_matrix: Matrix3<f64>, distortion_coeffs: &[f64; 24], rotation: Matrix3<f64>, p: Option<Matrix3<f64>>, rot_per_point: Option<Vec<Matrix3<f64>>>, params: &ComputeParams, lens_correction_amount: f64, fov: f64, timestamp_ms: f64, shift_per_point: Option<Vec<(f32, f32, f32, f32, f32)>>, mesh: Option<Vec<f64>>, optical_mesh: Option<Vec<f64>>, field_limit: f64, breathing_per_point: Option<Vec<f32>>, clamp_to_field: bool) -> Vec<(f32, f32)> {
    // The rotations act on ray *directions* now, so the output camera matrix is no longer folded into
    // them: `p` is that matrix, applied to the projected plane point at the very end
    let (out_f, out_c) = match p {
        Some(m) => ((m[(0, 0)] as f32, m[(1, 1)] as f32), (m[(0, 2)] as f32, m[(1, 2)] as f32)),
        None    => ((1.0, 1.0), (0.0, 0.0))
    };

    let kernel_params = point_kernel_params(params, camera_matrix, *distortion_coeffs, field_limit, timestamp_ms);
    let field_limit = kernel_params.field_limit;
    let amount = lens_correction_amount as f32;
    let proj = params.output_projection;

    let refr = kernel_params.light_refraction_coefficient != 1.0 && kernel_params.light_refraction_coefficient > 0.0;
    // Where the blend's source leg ends: the lens's field limit - under water, Snell's window, the inverse
    // of Snell's law away. Below `1.0` the render draws background past it (its bracket ends there), so a
    // ray the rotation carried past it holds at the edge, the way a source point past the field limit does
    let cap = {
        let fl = if field_limit > 0.0 { field_limit } else { std::f32::consts::PI };
        if refr { (fl.min(std::f32::consts::FRAC_PI_2).sin() / kernel_params.light_refraction_coefficient).clamp(-1.0, 1.0).asin() } else { fl }
    };

    // Ray (in the output camera's frame) -> where it lands on the output plane: `(1-a)·R(θ) + a·P(θ)`,
    // the lens-correction blend in the direction where it is a closed form (see `super::projection`).
    // `R` carries the refraction, because the render applies it on this side of the blend too
    let blend_forward = |ray_out: (f32, f32)| -> (f32, f32) {
        let mut theta = (ray_out.0 * ray_out.0 + ray_out.1 * ray_out.1).sqrt();
        if theta < 1e-12 { return (0.0, 0.0); }
        let u = (ray_out.0 / theta, ray_out.1 / theta);
        if amount < 1.0 { theta = theta.min(cap); }
        let rp = projection::radius_of_theta(theta, proj);
        if amount >= 1.0 { return (u.0 * rp, u.1 * rp); }
        let mut th = theta;
        if refr {
            th = (th.sin() * kernel_params.light_refraction_coefficient).clamp(-1.0, 1.0).asin();
        }
        let (s, c) = (th.sin(), th.cos());
        // The whole vector, not just its length: the tangential terms move the source point off its own
        // radius, and at amount 0 this has to come back exactly to the pixel it started from
        let p = params.distortion_model.distort_point(u.0 * s, u.1 * s, c, &kernel_params);
        ((1.0 - amount) * p.0 + amount * u.0 * rp, (1.0 - amount) * p.1 + amount * u.1 * rp)
    };

    let rays = undistort_points_to_rays(distorted, &kernel_params, rotation, rot_per_point.as_deref(), params, shift_per_point.as_deref(), mesh.as_deref(), optical_mesh.as_deref(), breathing_per_point.as_deref(), clamp_to_field);

    rays.into_iter().map(|ray| {
        let Some(ray_out) = ray else { return INVALID_POINT };

        let n = blend_forward(ray_out);
        if !n.0.is_finite() || !n.1.is_finite() { return INVALID_POINT; }
        let mut out = (n.0 * out_f.0 + out_c.0, n.1 * out_f.1 + out_c.1);

        // The digital warp, which the render undoes on its way in (FOV-independently: un-zoom -> warp
        // -> re-zoom). The render mixes its target as `(1-a)·U(out) + a·out`, so that both ends stay
        // exact, and `out` has to be the pixel the render reads this `nb` from - which between the
        // ends is not the warp simply put back on `nb`
        if amount < 1.0 && let Some(digital) = &params.digital_lens {
            let fov = fov as f32;
            let unwarp = |q: (f32, f32)| -> Option<(f32, f32)> {
                let uz = ((q.0 - out_c.0) * fov + out_c.0, (q.1 - out_c.1) * fov + out_c.1);
                digital.undistort_point(uz, &kernel_params).map(|d| ((d.0 - out_c.0) / fov + out_c.0, (d.1 - out_c.1) / fov + out_c.1))
            };
            // The warp put back the same way: the answer at 0, and the start elsewhere
            let nb = out;
            let uz = ((nb.0 - out_c.0) * fov + out_c.0, (nb.1 - out_c.1) * fov + out_c.1);
            let d = digital.distort_point(uz.0, uz.1, 1.0, &kernel_params);
            // A digital warp is a polynomial map inverted by iteration, and a point far enough outside the
            // frame it was fitted to has no answer to give: the warps say so with the off-frame sentinel,
            // and where the iteration simply left for infinity it says NaN. Neither is a position, and a
            // NaN reported as one is worse than no answer at all - it goes into the zoom polygon, where
            // every comparison against it is false and the frame corner quietly stops being a corner
            if d.0 <= -99998.0 || !d.0.is_finite() || !d.1.is_finite() { return INVALID_POINT; }
            out = ((d.0 - out_c.0) / fov + out_c.0, (d.1 - out_c.1) / fov + out_c.1);
            if amount > 0.0 {
                // Newton on `(1-a)·U(out) + a·out - nb`, the warp's Jacobian by differences: the warps
                // are low-order polynomials, so this converges in a few steps from that start. Safeguarded
                // on the way: it keeps the best iterate it has measured rather than the last one it took,
                // so a step that leaves the warp's domain cannot turn a usable answer into a NaN
                let residual = |q: (f32, f32)| -> Option<(f32, f32)> {
                    let u = unwarp(q)?;
                    let r = ((1.0 - amount) * u.0 + amount * q.0 - nb.0, (1.0 - amount) * u.1 + amount * q.1 - nb.1);
                    (r.0.is_finite() && r.1.is_finite()).then_some(r)
                };
                const H: f32 = 0.5;
                let (mut best, mut best_err) = (out, f32::INFINITY);
                for _ in 0..8 {
                    let Some(f0) = residual(out) else { break };
                    let err = f0.0.abs().max(f0.1.abs());
                    if err < best_err { best = out; best_err = err; }
                    if f0.0.abs() < 1e-3 && f0.1.abs() < 1e-3 { break; }
                    let (Some(fx), Some(fy)) = (residual((out.0 + H, out.1)), residual((out.0, out.1 + H))) else { break };
                    let (j00, j10) = ((fx.0 - f0.0) / H, (fx.1 - f0.1) / H);
                    let (j01, j11) = ((fy.0 - f0.0) / H, (fy.1 - f0.1) / H);
                    let det = j00 * j11 - j01 * j10;
                    if det.abs() < 1e-12 { break; }
                    out.0 -= (j11 * f0.0 - j01 * f0.1) / det;
                    out.1 -= (j00 * f0.1 - j10 * f0.0) / det;
                    if !out.0.is_finite() || !out.1.is_finite() { break; }
                }
                out = best;
            }
        }
        if !out.0.is_finite() || !out.1.is_finite() { return INVALID_POINT; }
        out
    }).collect()
}
