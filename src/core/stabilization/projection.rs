// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! The two ends of the pipeline's ray space: how a ray is carried between the lenses, and what
//! projection the *output* image uses.
//!
//! ## The ray encoding
//!
//! Everything between `undistort_point` (source image → ray) and `distort_point` (ray → source image)
//! travels as an **angle vector**: a 2D vector whose length is the ray's angle from the optical axis in
//! radians and whose direction is its azimuth. The 3D direction is `(sin θ·û, cos θ)`.
//!
//! It used to be `tan θ·û`, the point where the ray pierces the z=1 plane, which is what a 3x3 matrix
//! multiplies directly - but it goes to infinity at 90°, so a lens that sees past 90° from its own axis
//! (an X4/X5 covers 100°) had no representable ray for the outer part of its own frame at any lens
//! correction strength. In angle form nothing diverges before 180°, and the pieces that used to work
//! around the asymptote - the 89° linear continuations in the GoPro and Sony models, `r_limit` as a
//! `tan`, the `z > 0` test - are gone.
//!
//! ## The lens-correction blend
//!
//! At strength `a` a ray of angle θ lands at `(1-a)·R(θ) + a·P(θ)` of the output plane, where `R` is
//! where the source lens images it and `P` where the output projection wants it. It blends the two
//! *positions*, not the two angles: what a viewer sees is the position, and `P = tan θ` is so much
//! flatter than `R` near the axis and so much steeper near 90° that blending angles instead leaves the
//! slider doing 86% of its work in its last 10% (on a DJI fisheye the frame corner had moved 13.6% of
//! the way at 62% correction, against 62% here).
//!
//! In this direction it is a closed form, which is what the zoom search, the sync and the STMap export
//! use. The render needs the other direction - an output position back to a ray - and solves it: both
//! `R` and `P` rise with θ, so the mix does too, and the root is bracketed by the two angles the two
//! projections would each have given on their own (`undistort_point` and `unproject`), which is a tight
//! enough bracket for a few steps of regula falsi.
//!
//! The old blend lerped positions in the z=1 plane with a `1/(1-amount)` scale factor on the focal
//! length to keep them finite, which is the `FIXME: this is close but wrong` this replaces.
//!
//! ## The output projection
//!
//! `θ = P⁻¹(ρ)` maps the output image's normalized radius to the ray angle it stands for. Rectilinear
//! (`ρ = tan θ`) is what Gyroflow has always rendered and stays the default; the others exist because a
//! rectilinear image of a 200° lens does not: `tan θ` runs away, so a fully corrected fisheye can only
//! ever be a truncated infinity, and the zoom that fits the frame around it turns the periphery into
//! smear. All of them agree with `ρ ≈ θ` near the axis, so the focal length keeps its meaning whichever
//! one is picked.
//!
//! Nothing in the UI selects one yet. [`OutputProjection::for_lens`] picks it: rectilinear for every lens
//! rectilinear can hold, which is the picture Gyroflow has always drawn, and stereographic for the ones it
//! cannot - a lens that sees past 90° from its own axis has border rays with no rectilinear position at
//! all, at any correction strength above zero, so its picture has no edge for the zoom to fit and its
//! periphery is the asymptote.

#[repr(i32)]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputProjection {
    /// `ρ = tan θ`: straight lines stay straight, and the image is unbounded as θ → 90°
    #[default]
    Rectilinear = 0,
    /// `ρ = 2·tan(θ/2)`: conformal, finite to 180°, the usual choice for a corrected fisheye
    Stereographic = 1,
    /// `ρ = θ`: the angle itself, what most fisheye lenses are close to
    Equidistant = 2,
    /// `ρ = 2·sin(θ/2)`: equal solid angle per unit area
    EquisolidAngle = 3,
    /// `ρ = sin θ`: the sphere seen from infinitely far away
    Orthographic = 4,
}

impl OutputProjection {
    /// The projection to draw a lens with when nothing has chosen one. Rectilinear, unless the lens sees so
    /// far that rectilinear has no image of the lens's own frame: past 90° `tan` has no value, so a 200°
    /// body's border rays land nowhere at any correction strength above zero - the picture is then unbounded
    /// and smeared at the edge, and the zoom has no outline to fit. A projection finite to 180° holds all of
    /// it. The line is drawn at 85° from the axis: `tan 85° = 11.4`, a rectilinear image already eleven
    /// times its own paraxial scale at that ray, which no lens that reaches it was ever usable at anyway.
    ///
    /// Measured on the lens data the render draws the clip's first frame with - the camera matrix scaled to
    /// the video, the lens's own field limit - and at the frame's corners as the source lens saw them: a
    /// SuperView frame is the lens's 4:3 frame stretched to 16:9, so its corner read as the lens's own lies
    /// far outside the lens's real frame and puts an ordinary GoPro past 85°
    pub fn for_lens(params: &super::ComputeParams) -> Self {
        const FAR: f64 = 85.0 * std::f64::consts::PI / 180.0;
        let (w, h) = (params.width as f64, params.height as f64);
        if !(w > 0.0) || !(h > 0.0) { return Self::Rectilinear; }
        let (camera_matrix, coeffs, field_limit, sh, sv, _, _) = super::FrameTransform::get_lens_data_at_timestamp(params, 0.0, false);
        let (fx, fy, cx, cy) = (camera_matrix[(0, 0)], camera_matrix[(1, 1)], camera_matrix[(0, 2)], camera_matrix[(1, 2)]);
        if !(fx > 0.0) || !(fy > 0.0) { return Self::Rectilinear; }
        let model = &params.distortion_model;
        let mut kp = super::KernelParams::default();
        kp.width = params.width as i32; kp.height = params.height as i32;
        kp.output_width = params.width as i32; kp.output_height = params.height as i32;
        for (i, v) in coeffs.iter().enumerate().take(24) { kp.k[i] = *v as f32; }
        if let Some(p) = &params.digital_lens_params { for (i, v) in p.iter().take(16).enumerate() { kp.digital_lens_params[i] = *v as f32; } }
        let radius_at = |t: f64| -> f64 {
            let p = model.distort_point(t.sin() as f32, 0.0, t.cos() as f32, &kp);
            (p.0 as f64).hypot(p.1 as f64)
        };
        // The frame corner furthest from the principal point, in the lens's normalized units - through the
        // digital lens first where there is one, since the source lens never saw the warped frame
        let r_corner = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].iter()
            .map(|&(x, y)| {
                let (x, y) = match &params.digital_lens {
                    Some(d) => d.undistort_point((x as f32, y as f32), &kp).map_or((x, y), |p| (p.0 as f64, p.1 as f64)),
                    None => (x, y)
                };
                ((x * sh - cx) / fx).hypot((y * sv - cy) / fy)
            })
            .fold(0.0f64, f64::max);
        // The end of the lens: a fold of its calibration, or its own projection's end, whichever comes first
        let mut end = std::f64::consts::PI;
        if field_limit > 0.0 { end = end.min(field_limit); }
        if let Some(l) = model.radial_distortion_limit(&coeffs) { if l > 0.0 { end = end.min(l); } }
        if let Some(l) = model.field_limit(&coeffs) { if l > 0.0 { end = end.min(l); } }
        // The ray the frame corner needs: the first angle whose image reaches it. A lens that never gets there
        // (an image circle inside the frame) has its field where its image stops growing. Scanned rather than
        // bisected from the end, because the end itself is degenerate - at 180° a ray has no off-axis part
        // for a model to image, and every model answers 0 there
        let steps = 256;
        let (mut field, mut lo, mut r_best, mut t_best) = (None, 0.0f64, 0.0f64, 0.0f64);
        for i in 1..=steps {
            let t = i as f64 / steps as f64 * end;
            let r = radius_at(t);
            if r >= r_corner {
                let (mut a, mut b) = (lo, t);
                for _ in 0..30 { let m = 0.5 * (a + b); if radius_at(m) < r_corner { a = m; } else { b = m; } }
                field = Some(0.5 * (a + b));
                break;
            }
            if r > r_best { r_best = r; t_best = t; }
            lo = t;
        }
        let field = field.unwrap_or(t_best);
        if field > FAR { Self::Stereographic } else { Self::Rectilinear }
    }

    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Stereographic,
            2 => Self::Equidistant,
            3 => Self::EquisolidAngle,
            4 => Self::Orthographic,
            _ => Self::Rectilinear,
        }
    }
    /// Ray angle of a normalized output radius
    pub fn theta_of_radius(&self, r: f64) -> f64 {
        match self {
            Self::Rectilinear    => r.atan(),
            Self::Stereographic  => 2.0 * (r / 2.0).atan(),
            Self::Equidistant    => r,
            Self::EquisolidAngle => 2.0 * (r / 2.0).clamp(-1.0, 1.0).asin(),
            Self::Orthographic   => r.clamp(-1.0, 1.0).asin(),
        }
    }
    /// Normalized output radius of a ray angle
    pub fn radius_of_theta(&self, t: f64) -> f64 {
        match self {
            // Just under 90°, where `tan` flips sign: the caller gets a large finite radius instead of a
            // wrapped one, so a ray this projection cannot show stays outside the frame instead of folding
            // back into it
            Self::Rectilinear    => if t < 1.5707 { t.tan() } else { 1e9 },
            Self::Stereographic  => 2.0 * (t / 2.0).tan(),
            Self::Equidistant    => t,
            Self::EquisolidAngle => 2.0 * (t / 2.0).sin(),
            Self::Orthographic   => t.sin(),
        }
    }
    /// The largest ray angle this projection has an image for at all. Past it there is no position to put
    /// a ray at - `tan` runs away at 90°, `sin` turns over there - and anything sweeping the field has to
    /// stop, because the sentinel [`Self::radius_of_theta`] returns out there is flat and would read as a
    /// perfectly good, perfectly cheap piece of picture
    pub fn max_theta(&self) -> f64 {
        match self {
            Self::Rectilinear | Self::Orthographic => std::f64::consts::FRAC_PI_2 - 1e-3,
            _ => std::f64::consts::PI - 1e-3,
        }
    }

    /// `dρ/dθ` - how much output radius one radian of field buys at `t`
    pub fn radius_derivative(&self, t: f64) -> f64 {
        match self {
            Self::Rectilinear    => if t < 1.5707 { 1.0 / (t.cos() * t.cos()) } else { 1e9 },
            // `ρ = 2·tan(θ/2)` differentiates to `sec²(θ/2)`, ie. `2/(1 + cos θ)` - not `1/(1 + cos θ)`
            Self::Stereographic  => 1.0 / (t / 2.0).cos().powi(2).max(1e-9),
            Self::Equidistant    => 1.0,
            Self::EquisolidAngle => (t / 2.0).cos(),
            Self::Orthographic   => t.cos(),
        }
    }
}

/// Output plane point → ray (angle vector)
#[inline]
pub fn unproject(n: (f32, f32), proj: i32) -> (f32, f32) {
    let r = (n.0 * n.0 + n.1 * n.1).sqrt();
    if r < 1e-12 { return (0.0, 0.0); }
    let theta = match proj {
        1 => 2.0 * (r * 0.5).atan(),
        2 => r,
        3 => 2.0 * (r * 0.5).min(1.0).asin(),
        4 => r.min(1.0).asin(),
        _ => r.atan(),
    };
    let s = theta / r;
    (n.0 * s, n.1 * s)
}

/// Ray (angle vector) → output plane point
#[inline]
pub fn project(ray: (f32, f32), proj: i32) -> (f32, f32) {
    let theta = (ray.0 * ray.0 + ray.1 * ray.1).sqrt();
    if theta < 1e-12 { return (0.0, 0.0); }
    let r = match proj {
        1 => 2.0 * (theta * 0.5).tan(),
        2 => theta,
        3 => 2.0 * (theta * 0.5).sin(),
        4 => theta.sin(),
        _ => if theta < 1.5707 { theta.tan() } else { 1e9 },
    };
    let s = r / theta;
    (ray.0 * s, ray.1 * s)
}

/// The output radius a ray angle lands at, scalar form of [`project`]
#[inline]
pub fn radius_of_theta(theta: f32, proj: i32) -> f32 {
    match proj {
        1 => 2.0 * (theta * 0.5).tan(),
        2 => theta,
        3 => 2.0 * (theta * 0.5).sin(),
        4 => theta.sin(),
        // Just under 90°, where `tan` flips sign: a large finite radius keeps a ray this projection has no
        // image for outside the frame instead of folding it back into the picture
        _ => if theta < 1.5707 { theta.tan() } else { 1e9 },
    }
}

/// The ray angle that lands at a given output radius, scalar inverse of [`radius_of_theta`]
#[inline]
pub fn theta_of_radius(r: f32, proj: i32) -> f32 {
    match proj {
        1 => 2.0 * (r * 0.5).atan(),
        2 => r,
        3 => 2.0 * (r * 0.5).min(1.0).asin(),
        4 => r.min(1.0).asin(),
        _ => r.atan(),
    }
}

/// Ray (angle vector) → 3D unit direction
#[inline]
pub fn ray_to_dir(ray: (f32, f32)) -> (f32, f32, f32) {
    let theta = (ray.0 * ray.0 + ray.1 * ray.1).sqrt();
    if theta < 1e-12 { return (ray.0, ray.1, 1.0); }
    let s = theta.sin() / theta;
    (ray.0 * s, ray.1 * s, theta.cos())
}

/// 3D direction → ray (angle vector). The direction needs no normalization
#[inline]
pub fn dir_to_ray(d: (f32, f32, f32)) -> (f32, f32) {
    let rxy = (d.0 * d.0 + d.1 * d.1).sqrt();
    if rxy < 1e-12 { return (0.0, 0.0); }
    let theta = rxy.atan2(d.2);
    let s = theta / rxy;
    (d.0 * s, d.1 * s)
}
