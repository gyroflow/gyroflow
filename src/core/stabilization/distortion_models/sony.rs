// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)
//
// Sony native lens model.
// The camera records its lens curve (RTMD tag 0xe421) as the ray angle at N (normally 10) equally spaced image
// radii and we evaluate it as a natural cubic spline radius → angle, continued linearly with the end slope outside the knots.
//
// params.k layout (24 floats, built by `Sony::coefficients_from_lens_curve`):
//   k[0]      N    number of spline segments (1..=10); 0 = no lens data (identity)
//   k[1]      h    knot spacing in normalized image-plane units (image radius / focal length)
//   k[2..13]  y_i  ray angle (radians) at radius i·h, i = 0..=10 (y_0 = 0, unused knots 0)
//   k[13]     r_lin end of the linear region near the optical axis (normalized radius), 0 = none; it takes the slot
//             of c_0, which is always 0 for a natural spline
//   k[14..24] c_i  half of the spline's second derivative at knot i, i = 1..=10 (c_N = 0)
// Segment i (dx = r − i·h):  θ(r) = y_i + b_i·dx + c_i·dx² + d_i·dx³  with
//   b_i = (y_{i+1} − y_i)/h − h·(c_{i+1} + 2·c_i)/3,   d_i = (c_{i+1} − c_i)/(3·h)
// Linear region: for r < r_lin the curve is the straight line
// θ = 3° · r / r_lin, where r_lin is the radius the angle → radius spline gives for 3° (extrapolated with its end
// slope when the knots end below 3°, which is the case for every long lens: the whole frame is then on that line).
// The knots are quantized to 1/200°; near the axis of a long lens that is a few percent of the angle, and a curve
// that follows them there has a radially varying magnification error that shows up as warping once the
// stabilization moves the image.

use crate::stabilization::KernelParams;
use crate::gyro_source::splines::NaturalSpline;

#[derive(Default, Clone)]
pub struct Sony { }

pub const MAX_SEGMENTS: usize = 10;
pub const COEFF_COUNT: usize = 24;
const Y: usize = 2;  // y_i at k[Y + i]
const C: usize = 13; // c_i at k[C + i] (i >= 1)
const RLIN: usize = 13; // r_lin, end of the linear region
/// Angle at the end of the linear region (3°)
pub const THETA0: f64 = 3.0 * std::f64::consts::PI / 180.0;
// Angles are clamped just under tan()'s 90° asymptote and the radius continued linearly past it, so over-FOV rays
// stay large and monotonic instead of folding back into the frame; r_limit then clips them (same as gopro.rs)
const TMAX: f32 = 1.5533; // ~89°
/// Smallest dθ/dr still counted as rising; at or below it the curve is taken as folded
const MIN_SLOPE: f32 = 1e-9;

impl Sony {
    /// Number of spline segments in `k`, 0 when there is no (valid) lens curve
    #[inline] fn segments(k: &[f32]) -> usize {
        let n = k[0];
        if n >= 1.0 && n <= MAX_SEGMENTS as f32 && k[1] > 0.0 && k[Y] == 0.0 { n as usize } else { 0 }
    }
    /// Cubic coefficients (y_i, b_i, c_i, d_i) of segment `i`
    #[inline] fn segment(k: &[f32], i: usize) -> (f32, f32, f32, f32) {
        let h = k[1];
        let (y0, y1, c0, c1) = (k[Y + i], k[Y + i + 1], if i == 0 { 0.0 } else { k[C + i] }, k[C + i + 1]);
        (y0, (y1 - y0) / h - h * (c1 + 2.0 * c0) / 3.0, c0, (c1 - c0) / (3.0 * h))
    }
    /// Ray angle θ and dθ/dr at normalized image radius `r`
    #[inline] fn angle_at(k: &[f32], n: usize, r: f32) -> (f32, f32) {
        let h = k[1];
        let r_lin = k[RLIN];
        if r_lin > 0.0 && r < r_lin { let slope = THETA0 as f32 / r_lin; return (slope * r, slope); }
        let i = ((r / h) as i32).clamp(0, n as i32 - 1) as usize;
        let (y, b, c, d) = Self::segment(k, i);
        let dx = (r - i as f32 * h).clamp(0.0, h);
        let slope = b + (2.0 * c + 3.0 * d * dx) * dx;
        let theta = y + (b + (c + d * dx) * dx) * dx;
        // Outside the knots the curve continues linearly with the end slope
        (theta + slope * (r - i as f32 * h - dx), slope)
    }
    /// Normalized image radius for ray angle `theta`, inverse of `angle_at`
    #[inline] fn radius_at(k: &[f32], n: usize, theta: f32) -> f32 {
        let h = k[1];
        if theta <= 0.0 { return 0.0; }
        let r_lin = k[RLIN];
        if r_lin > 0.0 && theta < THETA0 as f32 { return theta * r_lin / THETA0 as f32; }
        let mut i = 0;
        while i + 1 < n && theta >= k[Y + i + 1] { i += 1; }
        let (y, b, c, d) = Self::segment(k, i);
        let y1 = k[Y + i + 1];
        if theta >= y1 { // past the last knot: linear continuation
            let slope = b + (2.0 * c + 3.0 * d * h) * h;
            return n as f32 * h + if slope > 1e-9 { (theta - y1) / slope } else { 0.0 };
        }
        // Newton on the segment's cubic, starting from the chord
        let mut dx = h * (theta - y) / (y1 - y).max(1e-12);
        for _ in 0..8 {
            let f = y + (b + (c + d * dx) * dx) * dx - theta;
            let fp = b + (2.0 * c + 3.0 * d * dx) * dx;
            if fp.abs() < 1e-12 { break; }
            let fix = f / fp;
            dx = (dx - fix).clamp(0.0, h);
            if fix.abs() < 1e-7 { break; }
        }
        i as f32 * h + dx
    }

    /// `point`: normalized (recorded pixel − c) / f. Image → ray direction in the normalized image plane (|.| = tan θ)
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let n = Self::segments(&params.k);
        if n == 0 { return Some(point); }
        let r = (point.0 * point.0 + point.1 * point.1).sqrt();
        if r < 1e-9 { return Some(point); }
        let theta = Self::angle_at(&params.k, n, r).0;
        if theta <= 0.0 { return None; }
        let tt = TMAX.tan();
        let rr = if theta < TMAX { theta.tan() } else { tt + (theta - TMAX) * (1.0 + tt * tt) };
        if params.r_limit > 0.0 && rr > params.r_limit { return None; }
        let scale = rr / r;
        Some((point.0 * scale, point.1 * scale))
    }

    /// `(x, y, z)` is the ray; returns the normalized image coordinate (× f + c → recorded pixel)
    pub fn distort_point(&self, x: f32, y: f32, z: f32, params: &KernelParams) -> (f32, f32) {
        let pos = (x / z, y / z);
        let n = Self::segments(&params.k);
        if n == 0 { return pos; }
        let r = (pos.0 * pos.0 + pos.1 * pos.1).sqrt();
        if r < 1e-9 { return pos; }
        let tt = TMAX.tan();
        let theta = if r < tt { r.atan() } else { TMAX + (r - tt) / (1.0 + tt * tt) };
        let scale = Self::radius_at(&params.k, n, theta) / r;
        (pos.0 * scale, pos.1 * scale)
    }

    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    /// The kernel's single-precision copy of the coefficient block, `None` when it is too short
    #[inline] fn k32(k: &[f64]) -> Option<[f32; COEFF_COUNT]> {
        if k.len() < COEFF_COUNT { return None; }
        let mut kf = [0.0f32; COEFF_COUNT];
        for (dst, src) in kf.iter_mut().zip(k) { *dst = *src as f32; }
        Some(kf)
    }

    /// d(image radius)/dθ, ≤ 0 where the curve folds. Each call is a Newton solve for the radius at `theta`, so
    /// the fold itself is found by `radial_distortion_limit` from the coefficients instead of from samples of this
    pub fn distortion_derivative(&self, theta: f64, k: &[f64]) -> Option<f64> {
        let kf = Self::k32(k)?;
        let n = Self::segments(&kf);
        if n == 0 { return None; }
        let r = Self::radius_at(&kf, n, theta as f32);
        let slope = Self::angle_at(&kf, n, r).1;
        Some(if slope > MIN_SLOPE { 1.0 / slope as f64 } else { -1.0 })
    }

    /// Where the curve stops rising: the smallest normalized radius with dθ/dr ≤ `MIN_SLOPE` and the angle there,
    /// `None` when it rises all the way. Solved from the coefficients: within a segment the slope is the quadratic
    /// b + 2c·dx + 3d·dx², so its first non-positive point is a root; the linear region below `r_lin` always
    /// rises, and past the knots the slope stays the end slope
    fn fold(k: &[f32], n: usize) -> Option<(f32, f32)> {
        let h = k[1];
        let r_lin = k[RLIN].max(0.0);
        for i in 0..n {
            let (y, b, c, d) = Self::segment(k, i);
            // The line replaces the spline below r_lin
            let lo = (r_lin - i as f32 * h).max(0.0);
            if lo > h { continue; }
            let slope_at = |dx: f32| b + (2.0 * c + 3.0 * d * dx) * dx;
            let dx = if slope_at(lo) <= MIN_SLOPE { Some(lo) } else { first_root(3.0 * d, 2.0 * c, b - MIN_SLOPE, lo, h) };
            if let Some(dx) = dx {
                return Some((i as f32 * h + dx, y + (b + (c + d * dx) * dx) * dx));
            }
        }
        // Only reached with the whole spline under the linear region (r_lin beyond the last knot): the end slope
        // takes over where the line ends
        let (_, b, c, d) = Self::segment(k, n - 1);
        if b + (2.0 * c + 3.0 * d * h) * h <= MIN_SLOPE {
            let r = (n as f32 * h).max(r_lin);
            return Some((r, Self::angle_at(k, n, r).0));
        }
        None
    }

    /// Largest usable ray radius `tan θ`, `None` when the curve rises all the way to 90°. Same meaning as the
    /// sampled search in `DistortionModel::radial_distortion_limit`, exact and a few dozen operations instead of
    /// 256 Newton solves; the renderer asks for it per frame when the file records a lens curve per frame
    pub fn radial_distortion_limit(&self, k: &[f64]) -> Option<f64> {
        let kf = Self::k32(k)?;
        let n = Self::segments(&kf);
        if n == 0 { return None; }
        let theta = Self::fold(&kf, n)?.1 as f64;
        (theta < std::f64::consts::FRAC_PI_2 - 0.001).then(|| theta.tan())
    }

    /// Builds the `params.k` block from the camera's lens curve: `angles[i]` is the ray angle (radians) at image
    /// radius (i + 1)·`radius_step`, the radius in normalized units (divided by the focal length).
    /// The natural cubic spline of `splines::NaturalSpline` (Thomas algorithm, natural boundaries), the same one Sony's
    /// `SplineCurveInterpolator::calc_coefficient` computes; the linear region comes from the inverse spline built by the
    /// same solver, so the line and the curve meet at `r_lin` without a kink
    pub fn coefficients_from_lens_curve(angles: &[f64], radius_step: f64) -> Vec<f64> {
        let n = angles.len().min(MAX_SEGMENTS);
        let mut k = vec![0.0; COEFF_COUNT];
        if n == 0 || !(radius_step > 0.0) { return k; }
        let h = radius_step;
        let mut y = vec![0.0; n + 1];
        y[1..].copy_from_slice(&angles[..n]);
        let radii: Vec<f64> = (0..=n).map(|i| i as f64 * h).collect();
        let Some(spline) = NaturalSpline::new(&radii, &y) else { return k; };
        k[0] = n as f64;
        k[1] = h;
        k[Y..Y + n + 1].copy_from_slice(&y);
        k[C..C + n + 1].copy_from_slice(spline.c()); // c_0 is 0, its slot holds r_lin
        // Linear region: r_lin = radius(3°) from the angle → radius spline (which exists when the angles rise),
        // extrapolated with its end slope
        if let Some(r_lin) = NaturalSpline::new(&y, &radii).map(|inverse| inverse.at(THETA0)) {
            if r_lin > 0.0 { k[RLIN] = r_lin; }
        }
        k
    }

    /// Converts the 6-term polynomial `r(θ) = Σ p_i·θ^(i+1)` that older Gyroflow versions fitted to the lens curve
    /// (still present per frame in project files with embedded metadata) into the spline block.
    /// `radius_max` is the normalized radius the spline has to cover (image half-diagonal / focal length).
    pub fn coefficients_from_legacy_polynomial(poly: &[f64], radius_max: f64) -> Vec<f64> {
        if poly.len() < 2 || poly[0] <= 0.0 || !(radius_max > 0.0) { return vec![0.0; COEFF_COUNT]; }
        let eval  = |t: f64| poly.iter().enumerate().map(|(i, p)| p * t.powi(i as i32 + 1)).sum::<f64>();
        let deriv = |t: f64| poly.iter().enumerate().map(|(i, p)| (i as f64 + 1.0) * p * t.powi(i as i32)).sum::<f64>();
        let h = radius_max / MAX_SEGMENTS as f64;
        let mut theta = 0.0;
        let angles: Vec<f64> = (1..=MAX_SEGMENTS).map(|i| {
            let r = i as f64 * h;
            for _ in 0..20 {
                let d = deriv(theta);
                if d.abs() < 1e-12 { break; }
                let fix = (eval(theta) - r) / d;
                theta -= fix;
                if fix.abs() < 1e-12 { break; }
            }
            theta
        }).collect();
        Self::coefficients_from_lens_curve(&angles, h)
    }

    pub fn id() -> &'static str { "sony" }
    pub fn name() -> &'static str { "Sony" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("sony.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("sony.wgsl") }
}

/// Smallest root of `a·x² + b·x + c` within `[lo, hi]`, `None` when there is none. The two roots come from the
/// cancellation-free form `q = -(b + sign(b)·√D) / 2`, `x = q / a` and `x = c / q`, which stays exact when `a`
/// is tiny, the case of a nearly straight spline segment
fn first_root(a: f32, b: f32, c: f32, lo: f32, hi: f32) -> Option<f32> {
    let roots = if a == 0.0 {
        [(b != 0.0).then(|| -c / b), None]
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 { return None; }
        let q = -0.5 * (b + b.signum() * disc.sqrt());
        [Some(q / a), (q != 0.0).then(|| c / q)]
    };
    roots.into_iter().flatten().filter(|x| *x >= lo && *x <= hi).reduce(f32::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stabilization::distortion_models::DistortionModel;

    const H: f64 = 0.1;

    /// Ground truth for the fold: the first radius on a fine grid where the curve stops rising
    fn limit_by_scan(k: &[f64]) -> Option<f64> {
        let kf = Sony::k32(k).unwrap();
        let n = Sony::segments(&kf);
        (1..200_000).map(|i| i as f32 * 1e-5).find(|&r| Sony::angle_at(&kf, n, r).1 <= MIN_SLOPE).map(|r| Sony::angle_at(&kf, n, r).0.tan() as f64)
    }

    #[test]
    fn a_rising_curve_has_no_limit() {
        // Rectilinear: θ = atan(r). The block gets a linear region (r_lin > 0) and rises everywhere
        let angles: Vec<f64> = (1..=MAX_SEGMENTS).map(|i| (i as f64 * H).atan()).collect();
        let k = Sony::coefficients_from_lens_curve(&angles, H);
        assert!(k[RLIN] > 0.0);
        let model = Sony::default();
        assert_eq!(model.radial_distortion_limit(&k), None);
        assert_eq!(limit_by_scan(&k), None);
        assert_eq!(DistortionModel::from_name("sony").radial_distortion_limit(&k), None);
        // No curve, no limit
        assert_eq!(model.radial_distortion_limit(&vec![0.0; COEFF_COUNT]), None);
        assert_eq!(model.radial_distortion_limit(&[1.0, 2.0]), None);
    }

    #[test]
    fn a_folding_curve_is_limited_where_it_folds() {
        // Rises to the ninth knot and dips at the tenth: the spline peaks near the last knot
        let angles = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.78, 0.83, 0.82];
        let k = Sony::coefficients_from_lens_curve(&angles, H);
        assert_eq!(k[RLIN], 0.0, "a dipping curve has no monotone inverse to place the linear region with");
        let model = Sony::default();
        let limit = model.radial_distortion_limit(&k).expect("the curve folds below 90°");
        let scanned = limit_by_scan(&k).expect("the scan sees the fold too");
        assert!((limit - scanned).abs() < 1e-4, "analytic {limit} vs scanned {scanned}");
        assert!(limit > 0.82f64.tan() && limit < 0.9f64.tan(), "{limit}");
        // The generic dispatcher routes to the same answer
        assert_eq!(DistortionModel::from_name("sony").radial_distortion_limit(&k), Some(limit));
        // The sampled derivative agrees where it can: rising well inside the knots, folded past them. Right at the
        // fold it can't tell, `radius_at` picks the segment by knot value and the dipping knots send an angle
        // between the peak and the last knot to the linear continuation, which is why the fold is solved, not sampled
        assert!(model.distortion_derivative(0.5, &k).unwrap() > 0.0);
        assert!(model.distortion_derivative(0.9, &k).unwrap() <= 0.0);
    }

    #[test]
    fn a_fold_past_the_linear_region_is_found_at_its_end() {
        // Hand-built: the line covers the whole spline (r_lin beyond the last knot) and the end slope is negative,
        // so the curve folds exactly where the line hands over
        let angles = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.78, 0.83, 0.82];
        let mut k = Sony::coefficients_from_lens_curve(&angles, H);
        k[RLIN] = 1.5;
        let kf = Sony::k32(&k).unwrap();
        let expected = Sony::angle_at(&kf, MAX_SEGMENTS, 1.5).0.tan() as f64;
        assert!(expected > 0.0);
        let limit = Sony::default().radial_distortion_limit(&k).unwrap();
        assert!((limit - expected).abs() < 1e-5, "{limit} vs {expected}");
        assert!((limit_by_scan(&k).unwrap() - expected).abs() < 1e-4);
    }

    #[test]
    fn first_root_picks_the_smallest_root_in_range() {
        // (x - 1)(x - 3) = x² - 4x + 3
        assert_eq!(first_root(1.0, -4.0, 3.0, 0.0, 10.0), Some(1.0));
        assert_eq!(first_root(1.0, -4.0, 3.0, 2.0, 10.0), Some(3.0));
        assert_eq!(first_root(1.0, -4.0, 3.0, 4.0, 10.0), None);
        // Opens downwards: -(x - 1)(x - 3)
        assert_eq!(first_root(-1.0, 4.0, -3.0, 2.0, 10.0), Some(3.0));
        // Linear and degenerate
        assert_eq!(first_root(0.0, 2.0, -1.0, 0.0, 1.0), Some(0.5));
        assert_eq!(first_root(0.0, 0.0, 1.0, 0.0, 1.0), None);
        assert_eq!(first_root(1.0, 0.0, 1.0, 0.0, 1.0), None);
        // A nearly linear quadratic keeps its finite root exact
        let x = first_root(1e-12, 2.0, -1.0, 0.0, 1.0).unwrap();
        assert!((x - 0.5).abs() < 1e-6, "{x}");
    }
}
