// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use glam::{ vec2, Vec2, vec3, Vec3, Vec4 };
use super::drawing::*;
use super::types::*;
use super::lens::*;
use super::background::*;

#[inline(never)]
fn get_mtrx_param(_size_for_rs: f32, matrices: &MatricesType, _sampler: SamplerType, row: i32, idx: usize) -> f32 {
    #[cfg(not(feature = "for_qtrhi"))]
    { matrices[row as usize * 16 + idx] }
    #[cfg(feature = "for_qtrhi")]
    {
        use spirv_std::image::{ ImageWithMethods, sample_with };
        matrices.sample_with(*_sampler, vec2(idx as f32 / 15.0, row as f32 / (_size_for_rs - 1.0)), sample_with::lod(0.0f32)).x
    }
}

/// The pipeline carries a ray as an angle vector: `|.|` is the angle from the axis in radians, the
/// direction is the azimuth. See `stabilization::projection` - nothing diverges at 90° the way a z=1 plane
/// point does
pub fn ray_unproject(n: Vec2, proj: i32) -> Vec2 {
    let r = n.length();
    if r < 1e-12 { return vec2(0.0, 0.0); }
    let theta = if proj == 1 { 2.0 * (r * 0.5).atan() }
           else if proj == 2 { r }
           else if proj == 3 { 2.0 * (r * 0.5).min(1.0).asin() }
           else if proj == 4 { r.min(1.0).asin() }
           else              { r.atan() };
    n * (theta / r)
}
/// Where a ray of angle `theta` lands on the output plane, under the output projection
pub fn ray_radius(theta: f32, proj: i32) -> f32 {
    if proj == 1 { 2.0 * (theta * 0.5).tan() }
    else if proj == 2 { theta }
    else if proj == 3 { 2.0 * (theta * 0.5).sin() }
    else if proj == 4 { theta.sin() }
    else if theta < 1.5707 { theta.tan() }
    else { 1e9 }
}
/// `(1-a)·R(θ) + a·P(θ) - t`: the lens-correction blend (see `stabilization::projection`), minus where
/// this pixel is. `R` carries the refraction, because the render applies it on this side too
pub fn blend_residual(theta: f32, u: Vec2, t: f32, a: f32, params: &KernelParams, distortion_model: u32) -> f32 {
    let mut th = theta;
    if params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0 {
        th = (th.sin() * params.light_refraction_coefficient).clamp(-1.0, 1.0).asin();
    }
    let (s, c) = (th.sin(), th.cos());
    let p = lens_distort(vec3(u.x * s, u.y * s, c), params, distortion_model);
    (1.0 - a) * p.length() + a * ray_radius(theta, params.output_projection) - t
}
/// The ray angle that lands at a given output radius, inverse of [`ray_radius`]
pub fn ray_theta(r: f32, proj: i32) -> f32 {
    if proj == 1 { 2.0 * (r * 0.5).atan() }
    else if proj == 2 { r }
    else if proj == 3 { 2.0 * (r * 0.5).min(1.0).asin() }
    else if proj == 4 { r.min(1.0).asin() }
    else { r.atan() }
}
pub fn ray_to_dir(ray: Vec2) -> Vec3 {
    let theta = ray.length();
    if theta < 1e-12 { return vec3(ray.x, ray.y, 1.0); }
    let s = theta.sin() / theta;
    vec3(ray.x * s, ray.y * s, theta.cos())
}

pub fn rotate_and_distort(ray: Vec2, idx: i32, params: &KernelParams, matrices: &MatricesType, sampler: SamplerType, distortion_model: u32, digital_distortion_model: u32, flags: u32) -> Vec2 {
    let size_for_rs = if (flags & 16) == 16 { params.width as f32 } else { params.height as f32 };
    let d = ray_to_dir(ray);
    let mut point_3d = vec3(
        (d.x * get_mtrx_param(size_for_rs, matrices, sampler, idx, 0)) + (d.y * get_mtrx_param(size_for_rs, matrices, sampler, idx, 1)) + (d.z * get_mtrx_param(size_for_rs, matrices, sampler, idx, 2)) + params.translation3d.x,
        (d.x * get_mtrx_param(size_for_rs, matrices, sampler, idx, 3)) + (d.y * get_mtrx_param(size_for_rs, matrices, sampler, idx, 4)) + (d.z * get_mtrx_param(size_for_rs, matrices, sampler, idx, 5)) + params.translation3d.y,
        (d.x * get_mtrx_param(size_for_rs, matrices, sampler, idx, 6)) + (d.y * get_mtrx_param(size_for_rs, matrices, sampler, idx, 7)) + (d.z * get_mtrx_param(size_for_rs, matrices, sampler, idx, 8)) + params.translation3d.z
    );
    {
        let rxy = vec2(point_3d.x, point_3d.y).length();
        // The ray's angle in the source camera: past the lens's own field, or past a fold of its
        // calibration, it has no image at all
        let theta = rxy.atan2(point_3d.z);
        if params.field_limit > 0.0 && theta > params.field_limit {
            return vec2(-99999.0, -99999.0);
        }

        // Refraction (underwater): Snell's law on the angle itself, exact for any field. Past the critical angle - or behind the flat port - no ray enters the housing at all: Snell's window ends there
        if params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0 && rxy > 1e-12 {
            let sin_d = theta.sin() * params.light_refraction_coefficient;
            if sin_d >= 1.0 || point_3d.z <= 0.0 { return vec2(-99999.0, -99999.0); }
            let theta_d = sin_d.asin();
            let s = theta_d.sin() / rxy;
            point_3d = vec3(point_3d.x * s, point_3d.y * s, theta_d.cos());
        }

        let bz = get_mtrx_param(size_for_rs, matrices, sampler, idx, 14);
        let breathing = if bz > 0.0 { bz } else { 1.0 }; // focus breathing: this row's source magnification
        let mut uv = params.f * (breathing * lens_distort(point_3d, params, distortion_model)) + params.c;

        if (flags & 2) == 2 { // Has digital lens
            uv = digital_lens_distort(vec3(uv.x, uv.y, 1.0), params, digital_distortion_model);
        }

        if params.input_horizontal_stretch > 0.001 { uv.x /= params.input_horizontal_stretch; }
        if params.input_vertical_stretch   > 0.001 { uv.y /= params.input_vertical_stretch; }

        uv
    }
}

pub fn undistort(uv: Vec2, params: &KernelParams, matrices: &MatricesType, coeffs: &[f32], _mesh_data: &[f32], drawing: &DrawingType, input: &ImageType, sampler: SamplerType, interpolation: u32, distortion_model: u32, digital_distortion_model: u32, flags: u32) -> Vec4 {
    let bg = params.background * params.max_pixel_value;

    if (params.flags & 4) == 4 { // Fill with background
        return bg;
    }

    let mut out_pos = if (flags & 64) == 64 { // Uses output rect
        vec2(
            map_coord(uv.x, params.output_rect.x as f32, (params.output_rect.x + params.output_rect.z) as f32, 0.0, params.output_width  as f32),
            map_coord(uv.y, params.output_rect.y as f32, (params.output_rect.y + params.output_rect.w) as f32, 0.0, params.output_height as f32)
        )
    } else {
        vec2(uv.x, uv.y)
    };

    #[cfg(not(feature = "for_qtrhi"))]
    if out_pos.x < 0.0 || out_pos.y < 0.0 || out_pos.x > params.output_width as f32 || out_pos.y > params.output_height as f32 { return bg; }

    let org_out_pos = out_pos;
    out_pos = out_pos + params.translation2d;

    let mut lens_undistort_failed = false;

    ///////////////////////////////////////////////////////////////////
    // Output pixel -> ray. A ray of angle θ lands at `(1-a)·R(θ) + a·P(θ)` of the output plane - where the
    // source lens images it, mixed with where the output projection wants it (`stabilization::projection`).
    // This is that inverted: both maps rise with θ, so the mix does too, and the root is bracketed by the
    // angles the two projections would each have given on their own
    let stretch = if params.input_horizontal_stretch > 0.01 { 1.0 / params.input_horizontal_stretch } else { 1.0 };
    let out_c = vec2(params.output_width as f32 / 2.0, params.output_height as f32 / 2.0);
    let out_f = params.f * stretch / params.fov;
    let a = params.lens_correction_amount;
    let n_raw = (out_pos - out_c) / out_f;
    let mut n = n_raw;
    if (flags & 2) == 2 && a < 1.0 { // Has digial lens
        // Apply the digital warp in the UN-zoomed (fov=1) frame so it's FOV-independent, the same way every
        // other backend and the point path do it: the warp is a frame-relative pixel map, so reading it on
        // post-zoom pixels makes the corrected shape - and the zoom fitted around that shape - depend on the
        // zoom itself. Un-zoom -> warp -> re-zoom
        let uz = (out_pos - out_c) * params.fov + out_c;
        let pt = digital_lens_undistort(uz, params, digital_distortion_model);
        if pt.x > -99998.0 {
            n = ((pt - out_c) / params.fov) / out_f;
        }
    }
    let mut ray = ray_unproject(n_raw, params.output_projection);
    if a < 1.0 {
        // With a digital warp the two legs read different planes, so the target is mixed the same way they
        // are; without one `n` is `n_raw` and this is just the pixel itself
        let nb = n * (1.0 - a) + n_raw * a;
        let t = nb.length();
        if t > 1e-9 {
            let u = nb / t;
            let lo = ray.length();
            let src = lens_undistort(n, params, distortion_model);
            let has_src = src.x > -99998.0;
            lens_undistort_failed = !has_src && !(params.field_limit > 0.0);
            if !lens_undistort_failed {
                // Past the edge of its own image the model has no inverse, but the lens still reaches to
                // its field limit and the blend may well land inside `t` before then
                // Under water the lens's angles are the housing's; the bracket is in the water's, the inverse of
                // Snell's law away - and no ray past the flat port's 90° ever enters the housing
                let refr = params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0;
                let mut hi = if has_src { src.length() } else { params.field_limit };
                if refr { hi = (hi.min(1.5707963).sin() / params.light_refraction_coefficient).clamp(-1.0, 1.0).asin(); }
                let mut edge = !has_src;
                // ... and the root cannot be past the angle at which the output projection alone would already have
                // used up `t` (`a·P(θ) <= t` at the root), and pinning the bracket there keeps both residuals
                // of the order of `t`. Without it a lens that sees past 90° hands `P`'s "no image" sentinel to
                // the solve, and a secant cannot move against 1e9: the blend then silently stalled at the
                // fully corrected angle outside a circle at `R(90°)`
                if a > 0.0 { hi = hi.min(ray_theta(t / a, params.output_projection)); }
                if a <= 0.0 {
                    lens_undistort_failed = !has_src;
                    // Exactly the source lens's own ray, tangential terms and all - with the refraction
                    // `rotate_and_distort` applies on the way out undone here, or the round trip isn't one
                    ray = src;
                    if refr {
                        let th = src.length();
                        if th > 1e-12 { ray = src * ((th.min(1.5707963).sin() / params.light_refraction_coefficient).clamp(-1.0, 1.0).asin() / th); }
                    }
                } else {
                    let (mut l, mut h) = (lo.min(hi), lo.max(hi));
                    // The lens ends at its field limit - under water, at Snell's window - and the projection's own
                    // angle may well lie past it; the bracket ends there too, and past it is background
                    let mut cap = if params.field_limit > 0.0 { params.field_limit } else { 3.1415927 };
                    if refr { cap = (cap.min(1.5707963).sin() / params.light_refraction_coefficient).clamp(-1.0, 1.0).asin(); }
                    if h > cap { h = cap; l = l.min(cap); edge = true; }
                    let mut gl = blend_residual(l, u, t, a, params, distortion_model);
                    let mut gh = blend_residual(h, u, t, a, params, distortion_model);
                    // Near the axis every projection agrees, so both ends of the bracket are a difference of
                    // near-equal numbers and the sign of `g` there is float noise. An end that IS the source
                    // lens's own answer always has a root beside it, so the end itself is the answer; only
                    // when the end is the field limit - or Snell's window - has the lens really run out
                    lens_undistort_failed = gh < 0.0 && edge;
                    if !lens_undistort_failed {
                        let mut theta = h;
                        if gh >= 0.0 {
                            // The projection's own angle overshoots - refraction magnifies the source leg - so bracket from the axis, where the residual is -t
                            if gl >= 0.0 { l = 0.0; gl = -t; }
                            // Regula falsi, Illinois variant: the bracket is tight (the two projections'
                            // own answers), and halving the stale end keeps it from crawling in one-sided
                            let mut side = 0i32;
                            let mut i = 0; while i < 6 {
                                let d = gh - gl;
                                theta = if d.abs() > 1e-12 { (l - gl * (h - l) / d).clamp(l, h) } else { 0.5 * (l + h) };
                                let gt = blend_residual(theta, u, t, a, params, distortion_model);
                                if gt < 0.0 {
                                    l = theta; gl = gt;
                                    if side == -1 { gh *= 0.5; }
                                    side = -1;
                                } else {
                                    h = theta; gh = gt;
                                    if side == 1 { gl *= 0.5; }
                                    side = 1;
                                }
                                i += 1;
                            }
                        }
                        // The source lens's own ray is off its radius by the tangential terms; that offset belongs to the
                        // source leg, so it fades out with it. It is measured against `n`, the point that leg reads: a
                        // digital warp moves `n` off `u`, and that is no offset of the lens's
                        let mut dir = u;
                        if has_src {
                            let (sl, nl) = (src.length(), n.length());
                            if sl > 1e-12 && nl > 1e-12 {
                                let dd = u + (src / sl - n / nl) * (1.0 - a);
                                let dl = dd.length();
                                if dl > 1e-12 { dir = dd / dl; }
                            }
                        }
                        ray = dir * theta;
                    }
                }
            }
        }
    }
    ///////////////////////////////////////////////////////////////////

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    let mut sy = if (flags & 16) == 16 { // Horizontal RS
        (fast_round(out_pos.x) as f32).min(params.width as f32).max(0.0)
    } else {
        (fast_round(out_pos.y) as f32).min(params.height as f32).max(0.0)
    };
    if params.matrix_count > 1 {
        let idx = params.matrix_count / 2;
        let pt = rotate_and_distort(ray, idx, params, matrices, sampler, distortion_model, digital_distortion_model, flags);
        if pt.x > -99998.0 {
            if (flags & 16) == 16 { // Horizontal RS
                sy = (fast_round(pt.x) as f32).min(params.width as f32).max(0.0);
            } else {
                sy = (fast_round(pt.y) as f32).min(params.height as f32).max(0.0);
            }
        }
    }
    ///////////////////////////////////////////////////////////////////

    let mut pixel = bg;

    let idx = sy.min(params.matrix_count as f32 - 1.0) as i32;
    let uv = if lens_undistort_failed {
        vec2(-99999.0, -99999.0)
    } else {
        rotate_and_distort(ray, idx, params, matrices, sampler, distortion_model, digital_distortion_model, flags)
    };
    if uv.x > -99998.0 {
        pixel = sample_with_background_at(uv, coeffs, input, params, sampler, interpolation, flags);
    }
    pixel = process_final_pixel(pixel, uv, org_out_pos, params, coeffs, drawing, sampler, flags);

    pixel
}
