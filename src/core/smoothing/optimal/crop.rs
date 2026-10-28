// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! The crop constraint: at the zoom limit, every point of the output frame's outline has to land inside the source
//! picture. Evaluated with the render's own mapping - the output pixel's ray from `Stabilization::output_ray`, rotated
//! by a kernel matrix built the way `FrameTransform::at_timestamp` builds it, imaged by `Stabilization::rotate_and_distort`
//! - so the lens, its correction strength, the output projection and the rolling shutter are all those of the picture.
//! Each frame is held at the time the render reads it at (sync offset and per-frame time offsets included), against the
//! camera's own orientation there, which is what the render corrects - not the horizon-locked, additionally rotated
//! motion the smoothing smooths. Left out, as they need the file metadata the smoothing can't lock: the camera's own
//! IBIS/OIS shifts, mesh correction and lens breathing. Where they move the frame, the zoom limit's iterations
//! (`smoothing_fov_limit_per_frame`) still catch what's left.

use std::collections::HashMap;
use nalgebra::{ Matrix3, Vector2 };
use super::path::{ Constraints, Lin };
use crate::gyro_source::{ Quat64, TimeQuat };
use crate::stabilization::{ ComputeParams, KernelParams, KernelParamsFlags, FrameTransform, Stabilization };
use crate::keyframes::KeyframeType;

/// The lens part of the kernel parameters per lens state, taken where the gyro lock may be held
/// (`ComputeParams::calculate_camera_fovs`): `smooth` runs under it and can't ask the lens itself
#[derive(Clone, Default)]
pub struct CropLenses {
    pub states: Vec<KernelParams>,
    /// Per frame, the index into `states`; empty when the lens is the same for the whole clip
    pub frame_state: Vec<u32>,
}
impl CropLenses {
    /// Adds the lens of `frame`, as `FrameTransform::get_lens_data_at_timestamp` returns it. Every frame in order when
    /// the lens changes over the clip, otherwise just the first
    pub fn push(&mut self, params: &ComputeParams, frame: usize, lens: &crate::zooming::LensData) {
        let (mut camera_matrix, coeffs, field_limit, h_stretch, v_stretch) = (lens.0, lens.1, lens.2, lens.3, lens.4);
        FrameTransform::dequantize_camera_matrix(params, frame, &mut camera_matrix);
        let mut kp = KernelParams {
            f: [camera_matrix[(0, 0)] as f32, camera_matrix[(1, 1)] as f32],
            c: [camera_matrix[(0, 2)] as f32, camera_matrix[(1, 2)] as f32],
            field_limit: field_limit as f32,
            output_projection: params.output_projection,
            input_horizontal_stretch: h_stretch as f32,
            input_vertical_stretch: v_stretch as f32,
            light_refraction_coefficient: params.light_refraction_coefficient as f32,
            ..Default::default()
        };
        for (i, v) in coeffs.iter().enumerate() { kp.k[i] = *v as f32; }
        if let Some(p) = &params.digital_lens_params {
            for (i, v) in p.iter().take(16).enumerate() { kp.digital_lens_params[i] = *v as f32; }
        }
        let same = self.states.last().is_some_and(|l: &KernelParams| { let (lf, lc, lk, ll) = (l.f, l.c, l.k, l.field_limit); lf == kp.f && lc == kp.c && lk == kp.k && ll == kp.field_limit });
        if !same { self.states.push(kp); }
        self.frame_state.push(self.states.len() as u32 - 1);
    }
    pub fn finish(&mut self) {
        if self.states.len() <= 1 { self.frame_state.clear(); }
    }
    /// Of everything the crop model takes from here
    pub fn checksum(&self) -> u64 {
        use std::hash::Hasher;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for kp in &self.states {
            let (f, c, k, dl) = (kp.f, kp.c, kp.k, kp.digital_lens_params);
            let (fl, sh, sv, lr, op) = (kp.field_limit, kp.input_horizontal_stretch, kp.input_vertical_stretch, kp.light_refraction_coefficient, kp.output_projection);
            for v in f.iter().chain(&c).chain(&k).chain(&dl).chain(&[fl, sh, sv, lr]) { h.write_u32(v.to_bits()); }
            h.write_i32(op);
        }
        for s in &self.frame_state { h.write_u32(*s); }
        h.finish()
    }
    fn state_at(&self, frame: usize) -> Option<u32> {
        if self.states.is_empty() { return None; }
        Some(self.frame_state.get(frame).or(self.frame_state.last()).copied().unwrap_or(0))
    }
}

/// A key that's the same for lens states differing by less than matters to the crop: a tenth of a percent of the focal
/// length, half a pixel of the principal point, the coefficients to about four significant digits. A lens whose focal
/// length moves every frame then still shares its fits and ray sets over the many frames it barely moves
fn lens_key(kp: &KernelParams) -> [u64; 4] {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let (f, c, k, fl) = (kp.f, kp.c, kp.k, kp.field_limit);
    for v in f { h.write_i64(((v.max(1e-9) as f64).ln() * 1000.0).round() as i64); }
    for v in c { h.write_i64((v as f64 * 2.0).round() as i64); }
    // An f32 keeps 23 bits of mantissa: without the lowest 11 it's about four significant digits
    for v in k.iter().chain([fl].iter()) { h.write_u32(v.to_bits() & !0x7ff); }
    let (sh, sv, lr) = (kp.input_horizontal_stretch, kp.input_vertical_stretch, kp.light_refraction_coefficient);
    [h.finish(), sh.to_bits() as u64, sv.to_bits() as u64, lr.to_bits() as u64]
}

/// Output points along each edge, between the corners
const EDGE_POINTS: usize = 7;
/// Points of the output's outline
const OUTLINE: usize = 4 * (EDGE_POINTS + 1);
/// Rows the rolling shutter rotation is sampled at, from the first to the last
const RS_SAMPLES: usize = 5;
/// The largest zoom the constraint assumes when there's no zoom limit, relative to the source
const DEFAULT_ZOOM_LIMIT: f64 = 2.0;

/// The output rays of one zoom of one lens state
struct RaySet {
    kp: KernelParams,
    /// By their point of the outline: an output pixel the projection has no ray for is left out, the others keep their
    /// place (the corners are found by it)
    rays: Vec<(usize, (f32, f32))>,
    /// Radians: the widest ray angle the source frame shows (its furthest edge point, or the lens's field limit)
    max_angle: f64,
}

/// One frame the crop holds
struct FrameGeom {
    /// Time (ms) the render reads the frame's orientations at, in the motion's timeline
    time: f64,
    set: u32,
    image_rotation: Matrix3<f64>,
    /// `R(t)⁻¹·R(t_row)` of the camera's own motion at `RS_SAMPLES` rows; empty without a rolling shutter
    rs: Vec<Quat64>,
    /// The camera's own orientation, what the render corrects
    target: Quat64,
    /// The motion handed to the smoothing, the virtual camera of no smoothing at all
    unsmoothed: Quat64,
}

pub struct CropModel<'a> {
    params: &'a ComputeParams,
    width: f64,
    height: f64,
    horizontal_rs: bool,
    sets: Vec<RaySet>,
    frames: Vec<FrameGeom>,
}

/// The orientation at `t_us`, interpolated between the samples around it and held past the ends
pub(crate) fn quat_at(quats: &TimeQuat, t_us: i64) -> Option<Quat64> {
    let a = quats.range(..=t_us).next_back();
    let b = quats.range(t_us..).next();
    match (a, b) {
        (Some(a), Some(b)) if b.0 > a.0 => Some(a.1.slerp(b.1, (t_us - a.0) as f64 / (b.0 - a.0) as f64)),
        (Some(a), _) => Some(*a.1),
        (None, Some(b)) => Some(*b.1),
        _ => None
    }
}

impl<'a> CropModel<'a> {
    /// The constraint for `frames`: (frame, time in ms the render reads its orientations at, in the motion's timeline).
    /// `input` is the motion handed to the smoothing, `original` the camera's own before the horizon lock and the
    /// additional rotation: the render corrects that one, and its rotation within the frame is what the rolling shutter
    /// follows. `None` when there's nothing to constrain with
    pub fn new(params: &'a ComputeParams, frames: &[(usize, f64)], input: &TimeQuat, original: &TimeQuat) -> Option<Self> {
        let lenses = params.smoothing_crop_lenses.as_ref()?;
        if lenses.states.is_empty() || params.width == 0 || params.height == 0 || frames.is_empty() { return None; }
        let (width, height) = (params.width as f64, params.height as f64);
        let out_aspect = params.output_height.max(1) as f64 / params.output_width.max(1) as f64;
        let (ow, oh) = (params.width, ((params.width as f64 * out_aspect).round() as usize).max(1));
        let limits = crate::zooming::zoom_limit_fovs(params);
        let scaling = params.width as f64 / params.output_width.max(1) as f64;
        // `calculate_fovs` takes the safety pad off the applied zoom; the frame has to fit with it
        let inset = 2.0 * params.fov_algorithm_margin as f64 / (params.width as f64).min(oh as f64);
        let readout = params.frame_readout_time.abs() * if params.frame_readout_direction.is_inverted() { -1.0 } else { 1.0 };
        let horizontal_rs = params.frame_readout_direction.is_horizontal();
        let mut model = Self { params, width, height, horizontal_rs, sets: Vec::new(), frames: Vec::with_capacity(frames.len()) };

        // Per frame its lens - one state for every state close enough to it, see `CropLenses::key` - and the settings
        // that shape the output
        let mut repr: HashMap<[u64; 4], u32> = HashMap::new();
        struct Setup { frame: usize, time: f64, state: u32, amount: f64, center: (f64, f64), video_rotation: f64 }
        let setups: Vec<Setup> = frames.iter().filter_map(|&(frame, time)| {
            let state = lenses.state_at(frame)?;
            let state = *repr.entry(lens_key(&lenses.states[state as usize])).or_insert(state);
            let ts = crate::timestamp_at_frame(frame as i32, params.scaled_fps);
            let kf = |t: KeyframeType, def: f64| params.keyframes.value_at_video_timestamp(&t, ts).unwrap_or(def);
            Some(Setup {
                frame, time, state,
                amount: kf(KeyframeType::LensCorrectionStrength, params.lens_correction_amount),
                center: (kf(KeyframeType::ZoomingCenterX, params.adaptive_zoom_center_offset.0), kf(KeyframeType::ZoomingCenterY, params.adaptive_zoom_center_offset.1)),
                video_rotation: kf(KeyframeType::VideoRotation, params.video_rotation),
            })
        }).collect();

        // The widest fov that fits without any correction, by lens and output: a bisection each, so on threads of their own
        let base_key = |s: &Setup| [s.state as u64, ((s.amount * 1e4).round() as i64) as u64, ((s.center.0 * 1e4).round() as i64) as u64, ((s.center.1 * 1e4).round() as i64) as u64];
        let mut fit_index: HashMap<[u64; 4], usize> = HashMap::new();
        let mut fit_jobs: Vec<usize> = Vec::new();
        for (k, s) in setups.iter().enumerate() {
            fit_index.entry(base_key(s)).or_insert_with(|| { fit_jobs.push(k); fit_jobs.len() - 1 });
        }
        let fits = super::path::parallel_map(fit_jobs.len(), |j| {
            let s = &setups[fit_jobs[j]];
            model.widest_fit(&lenses.states[s.state as usize], s.amount, s.center, (ow, oh))
        });

        let mut set_index: HashMap<[u64; 5], u32> = HashMap::new();
        for s in &setups {
            let fit = fits[fit_index[&base_key(s)]];
            // A lens no output frame fits even uncorrected: nothing for the crop to hold
            if !(fit > 0.0) { continue; }
            // The zoom limit (the internal one without a limit), tightened where the zoom limit's iterations say the
            // frame needed more zoom than allowed, and never tighter than what lets the frame fit uncorrected
            let mut fov = limits.get(s.frame).copied().flatten().unwrap_or(1.0 / (DEFAULT_ZOOM_LIMIT * scaling.max(1.0)));
            if let Some(f) = params.smoothing_fov_limit_per_frame.get(s.frame) { if *f > 0.0 { fov /= f; } }
            // 1% wider than the limit: the zoom fits its frame into a polygon of the outline, which lands a few tenths of
            // a percent inside these points, and a frame only a hair over the limit would run the limit's iterations
            fov = (fov + inset) * 1.01;
            fov = fov.min(fit * 0.95);
            // Quantized, so a curve of limits doesn't make a ray set per frame
            let fov = (fov * 2000.0).round() / 2000.0;

            let b = base_key(s);
            let key = [b[0], b[1], b[2], b[3], fov.to_bits()];
            let set = match set_index.get(&key) {
                Some(i) => *i,
                None => {
                    let i = model.sets.len() as u32;
                    model.sets.push(model.ray_set(&lenses.states[s.state as usize], s.amount, s.center, fov, (ow, oh)));
                    set_index.insert(key, i);
                    i
                }
            };
            let image_rotation = Matrix3::new_rotation(s.video_rotation * (std::f64::consts::PI / 180.0));
            let at = |q: &TimeQuat, t: f64| quat_at(q, (t * 1000.0).round() as i64);
            let (Some(target), Some(unsmoothed)) = (at(original, s.time), at(input, s.time)) else { continue; };
            let mut rs = Vec::new();
            if readout != 0.0 {
                let ti = target.inverse();
                for k in 0..RS_SAMPLES {
                    let tr = s.time - readout / 2.0 + readout * k as f64 / (RS_SAMPLES - 1) as f64;
                    rs.push(at(original, tr).map(|q| ti * q).unwrap_or_else(Quat64::identity));
                }
            }
            model.frames.push(FrameGeom { time: s.time, set, image_rotation, rs, target, unsmoothed });
        }
        if model.frames.is_empty() { return None; }
        Some(model)
    }

    fn kernel_params(&self, lens: &KernelParams, amount: f64, center: (f64, f64), fov: f64, (ow, oh): (usize, usize)) -> KernelParams {
        let mut kp = *lens;
        kp.width = self.params.width as i32;
        kp.height = self.params.height as i32;
        kp.output_width = ow as i32;
        kp.output_height = oh as i32;
        kp.source_rect = [0, 0, kp.width, kp.height];
        kp.output_rect = [0, 0, kp.output_width, kp.output_height];
        kp.matrix_count = 1;
        kp.fov = fov as f32;
        kp.lens_correction_amount = amount as f32;
        kp.translation2d = [(center.0 * self.width / fov) as f32, (center.1 * self.height / fov) as f32];
        let mut flags = KernelParamsFlags::empty();
        flags.set(KernelParamsFlags::HAS_DIGITAL_LENS, self.params.digital_lens.is_some());
        flags.set(KernelParamsFlags::HORIZONTAL_RS, self.horizontal_rs);
        kp.flags = flags.bits();
        kp
    }

    /// The outline of a frame, corners included, clockwise from the top left
    fn outline((ow, oh): (usize, usize)) -> Vec<(f32, f32)> {
        let (w, h) = (ow as f32, oh as f32);
        let n = EDGE_POINTS + 1;
        let mut pts = Vec::with_capacity(4 * n);
        for i in 0..n { pts.push((w * i as f32 / n as f32, 0.0)); }
        for i in 0..n { pts.push((w, h * i as f32 / n as f32)); }
        for i in 0..n { pts.push((w * (n - i) as f32 / n as f32, h)); }
        for i in 0..n { pts.push((0.0, h * (n - i) as f32 / n as f32)); }
        pts
    }

    fn ray_set(&self, lens: &KernelParams, amount: f64, center: (f64, f64), fov: f64, out: (usize, usize)) -> RaySet {
        let kp = self.kernel_params(lens, amount, center, fov, out);
        let stretch = if kp.input_horizontal_stretch > 0.01 { 1.0 / kp.input_horizontal_stretch } else { 1.0 };
        let out_c = Vector2::new(kp.output_width as f32 / 2.0, kp.output_height as f32 / 2.0);
        let out_f = Vector2::new(kp.f[0] * stretch / kp.fov, kp.f[1] * stretch / kp.fov);
        let rays = Self::outline(out).into_iter().enumerate().filter_map(|(k, (x, y))| {
            Stabilization::output_ray(Vector2::new(x, y), &kp, &self.params.distortion_model, self.params.digital_lens.as_ref(), &out_c, &out_f).map(|v| (k, v.0))
        }).collect();
        // The source frame's own outline, back to rays: no point of the output can be inside the frame at a wider angle
        let sh = if kp.input_horizontal_stretch > 0.001 { kp.input_horizontal_stretch } else { 1.0 };
        let sv = if kp.input_vertical_stretch > 0.001 { kp.input_vertical_stretch } else { 1.0 };
        let mut max_angle = Self::outline((self.params.width, self.params.height)).into_iter().filter_map(|(x, y)| {
            let n = ((x * sh - kp.c[0]) / kp.f[0], (y * sv - kp.c[1]) / kp.f[1]);
            self.params.distortion_model.undistort_point(n, &kp).map(|r| ((r.0 * r.0 + r.1 * r.1).sqrt()) as f64)
        }).fold(0.0, f64::max);
        if kp.field_limit > 0.0 && (max_angle <= 0.0 || kp.field_limit as f64 <= max_angle) { max_angle = kp.field_limit as f64; }
        RaySet { kp, rays, max_angle }
    }

    /// The angle (radians) of `ray` in the source camera, rotated by `q` (output camera -> source camera)
    fn ray_angle(&self, frame: &FrameGeom, ray: (f32, f32), q: &Quat64) -> f64 {
        let m = Self::matrix(q, &frame.image_rotation);
        let d = crate::stabilization::projection::ray_to_dir(ray);
        let x = d.0 * m[0] + d.1 * m[1] + d.2 * m[2];
        let y = d.0 * m[3] + d.1 * m[4] + d.2 * m[5];
        let w = d.0 * m[6] + d.1 * m[7] + d.2 * m[8];
        ((x * x + y * y).sqrt() as f64).atan2(w as f64)
    }

    /// The widest fov whose output frame lands inside the source without any correction (bisection), `0` if none does.
    /// A fov at which some point of the output has no ray at all doesn't fit: that pixel would be background
    fn widest_fit(&self, lens: &KernelParams, amount: f64, center: (f64, f64), out: (usize, usize)) -> f64 {
        let identity = Self::matrix(&Quat64::identity(), &Matrix3::new_rotation(self.params.video_rotation * (std::f64::consts::PI / 180.0)));
        let fits = |fov: f64| -> bool {
            let set = self.ray_set(lens, amount, center, fov, out);
            set.rays.len() == OUTLINE && set.rays.iter().all(|(_, r)| {
                Stabilization::rotate_and_distort(*r, 0, &set.kp, &[identity], &self.params.distortion_model, self.params.digital_lens.as_ref(), &[])
                    .is_some_and(|uv| uv.0 >= 0.0 && uv.1 >= 0.0 && uv.0 as f64 <= self.width && uv.1 as f64 <= self.height)
            })
        };
        let (mut lo, mut hi) = (0.0, 4.0);
        if !fits(0.01) { return 0.0; }
        // To 2.5e-4 of the fov: it's only ever used with a 5% margin
        for _ in 0..14 {
            let mid = 0.5 * (lo + hi);
            if fits(mid) { lo = mid; } else { hi = mid; }
        }
        lo
    }

    fn matrix(q: &Quat64, image_rotation: &Matrix3<f64>) -> [f32; 16] {
        let m = FrameTransform::kernel_rotation(q, image_rotation, false);
        [m[(0, 0)], m[(0, 1)], m[(0, 2)], m[(1, 0)], m[(1, 1)], m[(1, 2)], m[(2, 0)], m[(2, 1)], m[(2, 2)], 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    }

    /// Where the output ray `ray` lands in the source with the correction `c`
    fn source_point(&self, frame: &FrameGeom, set: &RaySet, ray: (f32, f32), c: &Quat64) -> Option<(f64, f64)> {
        self.source_point_row(frame, set, ray, c).map(|v| v.0)
    }

    /// `source_point`, with the rotation within the readout of the row it lands on (identity without a rolling shutter)
    fn source_point_row(&self, frame: &FrameGeom, set: &RaySet, ray: (f32, f32), c: &Quat64) -> Option<((f64, f64), Quat64)> {
        if frame.rs.is_empty() {
            return self.image_point(frame, set, ray, c).map(|p| (p, Quat64::identity()));
        }
        // Like the kernel: the row the middle of the frame's readout puts it on, then that row's rotation
        let rs_at = |pos: f64| -> Quat64 {
            let span = if self.horizontal_rs { self.width } else { self.height };
            let f = (pos / span).clamp(0.0, 1.0) * (RS_SAMPLES - 1) as f64;
            let k = (f.floor() as usize).min(RS_SAMPLES - 2);
            frame.rs[k].slerp(&frame.rs[k + 1], f - k as f64)
        };
        let p = self.image_point(frame, set, ray, &(c * frame.rs[RS_SAMPLES / 2]))?;
        let rs = rs_at(if self.horizontal_rs { p.0 } else { p.1 });
        self.image_point(frame, set, ray, &(c * rs)).map(|p| (p, rs))
    }

    /// The source point of `ray` rotated by `q` (output camera -> source camera)
    fn image_point(&self, frame: &FrameGeom, set: &RaySet, ray: (f32, f32), q: &Quat64) -> Option<(f64, f64)> {
        let m = Self::matrix(q, &frame.image_rotation);
        Stabilization::rotate_and_distort(ray, 0, &set.kp, &[m], &self.params.distortion_model, self.params.digital_lens.as_ref(), &[]).map(|p| (p.0 as f64, p.1 as f64))
    }

    /// The widest fov (in `zooming::calculate_fovs`' units) whose output frame lands inside the source at frame `j`
    /// with the correction `c`, by bisection: what `minimal_fovs` measures there, for checking the model against it
    pub fn widest_fov_at(&self, j: usize, c: &Quat64) -> f64 {
        let Some(frame) = self.frames.get(j) else { return f64::NAN; };
        let base = &self.sets[frame.set as usize].kp;
        let amount = base.lens_correction_amount as f64;
        let fov0 = base.fov as f64;
        let center = (base.translation2d[0] as f64 * fov0 / self.width, base.translation2d[1] as f64 * fov0 / self.height);
        let out = (base.output_width as usize, base.output_height as usize);
        let fits = |fov: f64| -> bool {
            let set = self.ray_set(base, amount, center, fov, out);
            set.rays.len() == OUTLINE && set.rays.iter().all(|(_, r)| self.source_point(frame, &set, *r, c).is_some_and(|p| p.0 >= 0.0 && p.1 >= 0.0 && p.0 <= self.width && p.1 <= self.height))
        };
        let (mut lo, mut hi) = (0.0, 4.0);
        for _ in 0..30 {
            let mid = 0.5 * (lo + hi);
            if fits(mid) { lo = mid; } else { hi = mid; }
        }
        lo
    }
}

impl Constraints for CropModel<'_> {
    fn len(&self) -> usize { self.frames.len() }
    fn time(&self, j: usize) -> f64 { self.frames[j].time }
    fn target(&self, j: usize) -> Quat64 { self.frames[j].target }
    fn unsmoothed(&self, j: usize) -> Quat64 { self.frames[j].unsmoothed }

    fn linearize(&self, j: usize, c: &Quat64, near: f64, out: &mut Vec<Lin>) {
        let frame = &self.frames[j];
        let set = &self.sets[frame.set as usize];
        const EPS: f64 = 2e-4;
        let perturbed = [0, 1, 2].map(|a| { let mut v = nalgebra::Vector3::zeros(); v[a] = -EPS; Quat64::from_scaled_axis(v) * c });
        // (point of the outline, its ray, where it lands, the rolling shutter rotation of its row)
        let mut points: Vec<(usize, (f32, f32), (f64, f64), Quat64)> = Vec::with_capacity(set.rays.len());
        // Past the lens's field a point has no image at all - and if it simply dropped out, the constraint would vanish
        // just where it's violated the most, and the path run further off. Its angle still says how far out it is, and
        // which way is back
        let angular = |k: usize, ray: (f32, f32), out: &mut Vec<Lin>| {
            if !(set.max_angle > 0.0) { return; }
            let rs = frame.rs.get(RS_SAMPLES / 2).copied().unwrap_or_else(Quat64::identity);
            let f = set.kp.f[0] as f64;
            let a0 = self.ray_angle(frame, ray, &(c * rs));
            let g = [0, 1, 2].map(|a| -f * (self.ray_angle(frame, ray, &(perturbed[a] * rs)) - a0) / EPS);
            out.push(Lin { s0: (set.max_angle - a0) * f, g, id: (4 * OUTLINE + k) as u32 });
        };
        for &(k, ray) in &set.rays {
            match self.source_point_row(frame, set, ray, c) {
                Some((p, rs)) => points.push((k, ray, p, rs)),
                None => angular(k, ray, out),
            }
        }
        // Per edge of the source, only the points of the outline closest to it: neighbouring points make nearly the
        // same constraint, and a few of them already follow the closest one as it slides along the edge within a step.
        // And always the frame's corners: a roll moves the middle of an edge along the edge, so with a barrel lens,
        // whose tightest points are the edges' middles, nothing else holds the roll, and the linearized problem then
        // swings it by tens of degrees one way and back the next
        let slack_of = |p: &(f64, f64), side: usize| match side { 0 => p.0, 1 => self.width - p.0, 2 => p.1, _ => self.height - p.1 };
        let corner = |k: usize| k % (EDGE_POINTS + 1) == 0;
        let mut chosen: Vec<(usize, usize, f64)> = Vec::with_capacity(4 * (PER_EDGE + 4)); // (index in points, side, slack)
        for side in 0..4 {
            let mut best: Vec<(usize, f64)> = points.iter().enumerate().map(|(i, (_, _, p, _))| (i, slack_of(p, side))).filter(|(_, s)| *s < near).collect();
            best.sort_by(|a, b| a.1.total_cmp(&b.1));
            let mut taken = 0;
            for (i, s0) in best {
                if taken < PER_EDGE || corner(points[i].0) {
                    chosen.push((i, side, s0));
                    if !corner(points[i].0) { taken += 1; }
                }
            }
        }
        let mut jacobians: Vec<(usize, Option<[[f64; 3]; 2]>)> = Vec::with_capacity(chosen.len());
        for &(i, side, s0) in &chosen {
            let (k, ray, p, rs) = points[i];
            let d = match jacobians.iter().find(|x| x.0 == i) {
                Some(x) => x.1,
                None => {
                    // d(source point)/dx, x the update of the path: c' = exp(-x)·c. The rolling shutter row is held:
                    // it moves the point by a fraction of a row's rotation, far below what the linearization needs
                    let mut d = [[0.0; 3]; 2];
                    let mut ok = true;
                    for a in 0..3 {
                        match self.image_point(frame, set, ray, &(perturbed[a] * rs)) {
                            Some(q) => { d[0][a] = (q.0 - p.0) / EPS; d[1][a] = (q.1 - p.1) / EPS; },
                            None => { ok = false; break; }
                        }
                    }
                    let d = ok.then_some(d);
                    jacobians.push((i, d));
                    d
                }
            };
            let Some(d) = d else {
                // The point is inside the field, but a step from it isn't
                if !out.iter().any(|l| l.id == (4 * OUTLINE + k) as u32) { angular(k, ray, out); }
                continue;
            };
            let (g, sign) = match side { 0 => (d[0], 1.0), 1 => (d[0], -1.0), 2 => (d[1], 1.0), _ => (d[1], -1.0) };
            out.push(Lin { s0, g: [g[0] * sign, g[1] * sign, g[2] * sign], id: (k * 4 + side) as u32 });
        }
    }
}

/// Outline points kept per edge of the source in a linearization
const PER_EDGE: usize = 3;
