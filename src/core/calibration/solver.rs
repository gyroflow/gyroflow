// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! A chessboard calibration of our own: bundle adjustment of the lens intrinsics and every view's board pose, with
//! the ray carried as a direction and the lens model's own projection (`distort`, the very function the render
//! draws with, here in f64), so a fisheye's board corner past 90° from the axis is just another corner. This is
//! what lets a lens wider than 180° be calibrated at all, and what makes any model with a `Model` impl calibratable.
//!
//! `cv::fisheye::calibrate` can't: it projects a board point at `θ = atan(|X/Z|)` and starts the poses from points
//! of the z=1 plane, so a corner near 90° has Z ≈ 0 and one past it Z < 0, which lands it on the opposite side of
//! the image. It gives out before 180° too: on a HERO13 with the Max Lens Mod 2.0, whose boards reach 89°, it fit
//! f = 292 px (RMS 840 px) on all 109 views and threw on a quarter of the random 10-view subsets the OpenCV path
//! picks from. No warp of the image fixes that, not even a perfect one, because the angle is measured on the 3D
//! side; a rotation of the rays does, because it's rigid (see `initial_pose`).
//!
//! - The intrinsics start from the model's `start` lens (equidistant for a fisheye, a pinhole for a rectilinear
//!   model) centred in the frame. Its focal length isn't known, so several are tried - the frame's half-diagonal
//!   seeing 30° to 165° - and the one that fits best after a few iterations is kept.
//! - Each view's pose starts from its corners' rays, rotated so their mean is the optical axis: there every ray has
//!   z > 0 and a planar homography gives the board's pose, which is rotated back.
//! - Levenberg-Marquardt over both, with the poses eliminated view by view (Schur complement), so a step costs one
//!   solve the size of the intrinsics however many views there are. The Jacobian is a central difference.
//! - Views that fit far worse than the rest - a board bent by the rolling shutter of a fast move - are dropped and
//!   the fit repeated until the set settles. On the Max Lens Mod clip they had pulled the principal point ~45 px.
//!
//! On a simulated 220° lens with boards out to 112° and 0.5 px of noise it lands within 0.6 px of the true lens
//! over the whole 0..110° field, where `cv::fisheye::calibrate` is off by hundreds.

use nalgebra::{ DMatrix, DVector, Matrix3, Matrix6, MatrixXx6, Rotation3, SMatrix, SVector, Vector3, Vector6 };
use rayon::prelude::*;
use std::f64::consts::PI;
use crate::stabilization::distortion_models::{ OpenCVFisheye, OpenCVStandard, GenericPolynomial };

/// A lens model the solver can fit. The projection is the model's own; what this adds is where a fit of it starts
/// and which of its coefficients it moves. The focal length and the principal point are the solver's
pub trait Model: Sync {
    /// The lens the fit starts from, before it knows anything about this one
    fn start(&self) -> Start;
    /// The coefficients of that lens, as many as `project` reads
    fn initial(&self) -> Vec<f64>;
    /// Which coefficients the fit moves; the others keep their `initial` value
    fn free(&self) -> Vec<bool>;
    /// Unit ray direction → normalized image point (`× f + c` is the pixel)
    fn project(&self, k: &[f64], ray: &Vector3<f64>) -> (f64, f64);
}

/// The two lenses a fit can start from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// `ρ = θ`: what most fisheyes are close to, and it reaches 180°
    Equidistant,
    /// `ρ = tan θ`: a rectilinear lens, up to 90°
    Pinhole,
}
impl Start {
    /// Normalized image radius of a ray angle, `None` where this lens has no image of it
    fn radius(&self, theta: f64) -> Option<f64> {
        match self {
            Self::Equidistant => (theta < PI).then_some(theta),
            Self::Pinhole => (theta < 89f64.to_radians()).then(|| theta.tan()),
        }
    }
    /// Normalized image point → unit ray
    fn ray(&self, x: f64, y: f64) -> Option<Vector3<f64>> {
        match self {
            Self::Equidistant => {
                let t = x.hypot(y);
                if t >= PI { return None; }
                let s = if t > 1e-12 { t.sin() / t } else { 1.0 };
                Some(Vector3::new(x * s, y * s, t.cos()))
            }
            Self::Pinhole => Some(Vector3::new(x, y, 1.0).normalize()),
        }
    }
}

/// Kannala-Brandt, `θ_d = θ(1 + k1θ² + k2θ⁴ + k3θ⁶ + k4θ⁸)`, all four fitted
impl Model for OpenCVFisheye {
    fn start(&self) -> Start { Start::Equidistant }
    /// All zero is the equidistant lens in `distort` (the "uncalibrated means pinhole" convention is the render's)
    fn initial(&self) -> Vec<f64> { vec![0.0; 4] }
    fn free(&self) -> Vec<bool> { vec![true; 4] }
    fn project(&self, k: &[f64], ray: &Vector3<f64>) -> (f64, f64) { OpenCVFisheye::distort(ray.x, ray.y, ray.z, k) }
}

/// A pinhole with OpenCV's radial and tangential distortion. Fitted: k1, k2, p1, p2, k3 - what `cv::calibrateCamera`
/// fits by default; the rational k4..k6 and the thin prism s1..s4 stay at 0. It has no image of a ray at or past 90°,
/// so it's for rectilinear lenses
impl Model for OpenCVStandard {
    fn start(&self) -> Start { Start::Pinhole }
    fn initial(&self) -> Vec<f64> { vec![0.0; 12] }
    fn free(&self) -> Vec<bool> { (0..12).map(|i| i < 5).collect() }
    fn project(&self, k: &[f64], ray: &Vector3<f64>) -> (f64, f64) { OpenCVStandard::distort(ray.x, ray.y, ray.z, k) }
}

/// `θ_d = θ + k1θ² + k2θ³ + k3θ⁴ + k4θ⁵ + k5θ⁶`. The linear term stays at 1 - with the focal length free it is the
/// same unknown twice. Odd powers alone would be `opencv_fisheye` again; the even ones are what it adds. It stops at
/// θ⁶ so it has one coefficient more than `opencv_fisheye`, not seven: past the boards a polynomial only extrapolates
impl Model for GenericPolynomial {
    fn start(&self) -> Start { Start::Equidistant }
    fn initial(&self) -> Vec<f64> { (0..12).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect() }
    fn free(&self) -> Vec<bool> { (0..12).map(|i| (1..=5).contains(&i)).collect() }
    fn project(&self, k: &[f64], ray: &Vector3<f64>) -> (f64, f64) { GenericPolynomial::distort(ray.x, ray.y, ray.z, k) }
}

#[derive(Debug, Clone)]
pub struct Calibration {
    /// Focal length and principal point, in pixels
    pub f: (f64, f64),
    pub c: (f64, f64),
    /// All of the model's coefficients, as `Model::project` reads them
    pub k: Vec<f64>,
    /// How many of `k` a profile needs to store: up to the last one the fit moves, the ones after it stay 0 in every
    /// model here
    pub fitted: usize,
    /// Reprojection error over the views kept, in pixels
    pub rms: f64,
    /// The views kept, as indices into the views given
    pub kept: Vec<usize>,
    /// Each view's own reprojection error with the final intrinsics, NaN for one that never got a pose
    pub view_rms: Vec<f64>,
    /// The largest angle from the optical axis any corner of a kept view reaches, in radians
    pub max_corner_angle: f64,
}

/// Board rotation (axis × angle) and translation, in the camera's frame
type Pose = [f64; 6];

/// Views whose own error is above this many times the median are dropped, unless it is under `TRIM_FLOOR_PX`
const TRIM_RATIO: f64 = 2.0;
const TRIM_FLOOR_PX: f64 = 0.5;
const TRIM_ROUNDS: usize = 6;

/// Calibrates `model` on chessboard views: `board` holds the corners' positions on the board (any unit, z = 0) and
/// each view their detected pixels, in the same order. `protected` views are never dropped as outliers (frames the
/// user added by hand). `None` when there's too little to fit
pub fn calibrate<M: Model + ?Sized>(model: &M, board: &[(f64, f64)], views: &[Vec<(f64, f64)>], size: (usize, usize), protected: &[usize]) -> Option<Calibration> {
    let usable: Vec<usize> = (0..views.len()).filter(|&i| views[i].len() == board.len()).collect();
    if usable.len() < 3 || board.len() < 4 || size.0 == 0 || size.1 == 0 { return None; }
    let (w, h) = (size.0 as f64, size.1 as f64);
    let half_diag = (w * 0.5).hypot(h * 0.5);

    // The focal length: a few guesses, a few iterations each, and the best one goes on. A guess so short that the
    // outer boards land past 180° poses fewer views, and an rms over fewer views isn't comparable, so only the
    // starts that posed nearly all of them compete
    // (a pinhole has no image of the wider guesses, so it tries fewer)
    let starts: Vec<(f64, Vec<f64>, Vec<usize>, Vec<Pose>)> = (30..=165).step_by(15).filter_map(|deg| {
        let f = half_diag / model.start().radius((deg as f64).to_radians())?;
        let mut intr = vec![f, f, w * 0.5, h * 0.5];
        intr.extend(model.initial());
        let initial: Vec<Option<Pose>> = usable.par_iter().map(|&v| initial_pose(model, &intr, board, &views[v])).collect();
        let (idx, mut poses): (Vec<usize>, Vec<Pose>) = usable.iter().zip(initial).filter_map(|(&v, p)| Some((v, p?))).unzip();
        if idx.len() < 3 { return None; }
        let cost = refine(model, board, views, &mut intr, &idx, &mut poses, false, 15);
        let rms = (cost / (idx.len() * board.len()) as f64).sqrt();
        ::log::debug!("calibration solver: start at {deg}° half-diagonal (f = {f:.1}): {} views, rms {rms:.3} px", idx.len());
        rms.is_finite().then_some((rms, intr, idx, poses))
    }).collect();
    let most = starts.iter().map(|s| s.2.len()).max()?;
    let (_, mut intr, mut posed, poses) = starts.into_iter().filter(|s| s.2.len() * 10 >= most * 9).min_by(|a, b| a.0.total_cmp(&b.0))?;

    // The fit, then the views it can't explain dropped, until the set settles
    let mut pose_of: Vec<Option<Pose>> = vec![None; views.len()];
    for (v, p) in posed.iter().zip(poses) { pose_of[*v] = Some(p); }
    let mut kept = posed.clone();
    let mut view_rms = vec![f64::NAN; views.len()];
    for round in 0..TRIM_ROUNDS {
        let mut poses: Vec<Pose> = kept.iter().map(|&v| pose_of[v].unwrap()).collect();
        refine(model, board, views, &mut intr, &kept, &mut poses, false, 200);
        for (v, p) in kept.iter().zip(poses) { pose_of[*v] = Some(p); }

        // The views the start's lens couldn't pose, tried again from this one, which is much closer to the real lens:
        // they're judged with the rest below, and one added by hand is kept like any other `protected` view
        if round == 0 {
            let unposed: Vec<usize> = usable.iter().copied().filter(|&v| pose_of[v].is_none()).collect();
            let retried: Vec<Option<Pose>> = unposed.par_iter().map(|&v| initial_pose(model, &intr, board, &views[v])).collect();
            for (v, p) in unposed.into_iter().zip(retried) {
                if let Some(p) = p { pose_of[v] = Some(p); posed.push(v); }
            }
            posed.sort_unstable();
        }

        // Every other view's pose against these intrinsics, so its error is judged by the same lens
        let others: Vec<usize> = posed.iter().copied().filter(|v| !kept.contains(v)).collect();
        let refit: Vec<Pose> = others.par_iter().map(|&v| {
            let mut i = intr.clone();
            let mut p = [pose_of[v].unwrap()];
            refine(model, board, views, &mut i, &[v], &mut p, true, 30);
            p[0]
        }).collect();
        for (v, p) in others.iter().zip(refit) { pose_of[*v] = Some(p); }
        for &v in &posed {
            view_rms[v] = (view_cost(model, &intr, &pose_of[v].unwrap(), board, &views[v]) / board.len() as f64).sqrt();
        }

        let mut sorted: Vec<f64> = posed.iter().map(|&v| view_rms[v]).filter(|x| x.is_finite()).collect();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
        let limit = (TRIM_RATIO * median).max(TRIM_FLOOR_PX);
        // A view added by hand stays whatever its error, unless it has none at all: a NaN would take the whole fit down
        let next: Vec<usize> = posed.iter().copied().filter(|&v| view_rms[v] <= limit || (protected.contains(&v) && view_rms[v].is_finite())).collect();
        ::log::debug!("calibration solver: round {round}: {} views, f = ({:.2}, {:.2}), c = ({:.2}, {:.2}), median view rms {median:.3} px; {} under {limit:.3} px",
            kept.len(), intr[0], intr[1], intr[2], intr[3], next.len());
        // The last round's fit stands as it is: trimming again would leave it unfitted
        if next == kept || next.len() < 3 || round == TRIM_ROUNDS - 1 { break; }
        kept = next;
    }

    let cost: f64 = kept.iter().map(|&v| view_cost(model, &intr, &pose_of[v].unwrap(), board, &views[v])).sum();
    let rms = (cost / (kept.len() * board.len()) as f64).sqrt();
    let max_corner_angle = kept.iter().flat_map(|&v| {
        let p = pose_of[v].unwrap();
        let (r, t) = (rotation(&p), Vector3::new(p[3], p[4], p[5]));
        board.iter().map(move |b| { let x = r * Vector3::new(b.0, b.1, 0.0) + t; x.x.hypot(x.y).atan2(x.z) })
    }).fold(0.0, f64::max);
    if !rms.is_finite() { return None; }

    Some(Calibration {
        f: (intr[0], intr[1]),
        c: (intr[2], intr[3]),
        k: intr[4..].to_vec(),
        fitted: model.free().iter().rposition(|f| *f).map_or(0, |i| i + 1),
        rms,
        kept,
        view_rms,
        max_corner_angle,
    })
}

fn rotation(p: &Pose) -> Rotation3<f64> { Rotation3::new(Vector3::new(p[0], p[1], p[2])) }

/// The pixel errors of one view, x and y interleaved
fn residuals<M: Model + ?Sized>(model: &M, intr: &[f64], pose: &Pose, board: &[(f64, f64)], pts: &[(f64, f64)], out: &mut Vec<f64>) {
    out.clear();
    let (r, t) = (rotation(pose), Vector3::new(pose[3], pose[4], pose[5]));
    for (b, p) in board.iter().zip(pts) {
        // A unit direction, like the render's rays (the models' near-axis branch assumes one)
        let (u, v) = model.project(&intr[4..], &(r * Vector3::new(b.0, b.1, 0.0) + t).normalize());
        out.push(intr[0] * u + intr[2] - p.0);
        out.push(intr[1] * v + intr[3] - p.1);
    }
}

fn view_cost<M: Model + ?Sized>(model: &M, intr: &[f64], pose: &Pose, board: &[(f64, f64)], pts: &[(f64, f64)]) -> f64 {
    let mut r = Vec::with_capacity(board.len() * 2);
    residuals(model, intr, pose, board, pts, &mut r);
    r.iter().map(|x| x * x).sum()
}

/// One view's share of the normal equations
struct Block {
    jii: DMatrix<f64>,
    jip: MatrixXx6<f64>,
    jpp: Matrix6<f64>,
    gi: DVector<f64>,
    gp: Vector6<f64>,
}

/// `free`: which intrinsics get a column (a fixed one's stays 0), `None` for none at all
fn linearize<M: Model + ?Sized>(model: &M, intr: &[f64], pose: &Pose, board: &[(f64, f64)], pts: &[(f64, f64)], free: Option<&[bool]>) -> Block {
    let mut r0 = Vec::with_capacity(board.len() * 2);
    residuals(model, intr, pose, board, pts, &mut r0);
    let m = r0.len();
    let free = free.unwrap_or(&[]);
    let ni = free.len();
    let (mut rp, mut rm) = (Vec::with_capacity(m), Vec::with_capacity(m));

    let mut jp = MatrixXx6::<f64>::zeros(m);
    for j in 0..6 {
        let h = 1e-6 * pose[j].abs().max(1.0);
        let (mut a, mut b) = (*pose, *pose);
        a[j] += h; b[j] -= h;
        residuals(model, intr, &a, board, pts, &mut rp);
        residuals(model, intr, &b, board, pts, &mut rm);
        for i in 0..m { jp[(i, j)] = (rp[i] - rm[i]) / (2.0 * h); }
    }
    let mut ji = DMatrix::<f64>::zeros(m, ni);
    for j in (0..ni).filter(|&j| free[j]) {
        let h = 1e-6 * intr[j].abs().max(1.0);
        let (mut a, mut b) = (intr.to_vec(), intr.to_vec());
        a[j] += h; b[j] -= h;
        residuals(model, &a, pose, board, pts, &mut rp);
        residuals(model, &b, pose, board, pts, &mut rm);
        for i in 0..m { ji[(i, j)] = (rp[i] - rm[i]) / (2.0 * h); }
    }
    let r = DVector::from_vec(r0);
    Block {
        jpp: jp.transpose() * &jp,
        gp: jp.transpose() * &r,
        jip: ji.transpose() * &jp,
        jii: ji.transpose() * &ji,
        gi: ji.transpose() * &r,
    }
}

/// Levenberg-Marquardt over the intrinsics (unless `fix_intr`) and the poses of views `idx` (`poses[i]` is view
/// `idx[i]`'s). Returns the sum of squared pixel errors
fn refine<M: Model + ?Sized>(model: &M, board: &[(f64, f64)], views: &[Vec<(f64, f64)>], intr: &mut Vec<f64>, idx: &[usize], poses: &mut [Pose], fix_intr: bool, max_iter: usize) -> f64 {
    // With the intrinsics fixed the blocks have no columns for them (`linearize`), and the step none either
    let ni = if fix_intr { 0 } else { intr.len() };
    let free: Vec<bool> = [true; 4].into_iter().chain(model.free()).collect();
    let cost_of = |intr: &[f64], poses: &[Pose]| -> f64 {
        idx.par_iter().zip(poses.par_iter()).map(|(&v, p)| view_cost(model, intr, p, board, &views[v])).sum()
    };
    let mut cost = cost_of(intr, poses);
    if !cost.is_finite() { return cost; }
    let mut lambda = 1e-3;
    for _ in 0..max_iter {
        let blocks: Vec<Block> = idx.par_iter().zip(poses.par_iter()).map(|(&v, p)| linearize(model, intr, p, board, &views[v], (!fix_intr).then_some(&free[..]))).collect();
        let (mut jii, mut gi) = (DMatrix::<f64>::zeros(ni, ni), DVector::<f64>::zeros(ni));
        if !fix_intr {
            for b in &blocks { jii += &b.jii; gi += &b.gi; }
        }
        let mut stepped = false;
        let mut converged = false;
        while lambda < 1e12 {
            // Marquardt's damping, each pose block inverted on its own
            let inv: Option<Vec<Matrix6<f64>>> = blocks.iter().map(|b| {
                let mut a = b.jpp;
                for d in 0..6 { a[(d, d)] += lambda * a[(d, d)].max(1e-12); }
                a.try_inverse()
            }).collect();
            let Some(inv) = inv else { lambda *= 10.0; continue; };

            let di = if fix_intr {
                DVector::<f64>::zeros(ni)
            } else {
                let mut s = jii.clone();
                // A fixed coefficient's row and column are all 0, its gradient too: a 1 on the diagonal makes its step 0
                for d in 0..ni { s[(d, d)] = if free[d] { s[(d, d)] + lambda * s[(d, d)].max(1e-12) } else { 1.0 }; }
                let mut rhs = -&gi;
                for (b, inv) in blocks.iter().zip(&inv) {
                    let w = &b.jip * inv;
                    s -= &w * b.jip.transpose();
                    rhs += &w * b.gp;
                }
                match s.cholesky() {
                    Some(c) => c.solve(&rhs),
                    None => { lambda *= 10.0; continue; }
                }
            };
            let new_intr: Vec<f64> = if fix_intr { intr.clone() } else { intr.iter().zip(di.iter()).map(|(a, d)| a + d).collect() };
            let new_poses: Vec<Pose> = blocks.iter().zip(&inv).zip(poses.iter()).map(|((b, inv), p)| {
                let dp: Vector6<f64> = inv * -(b.gp + b.jip.transpose() * &di);
                std::array::from_fn(|k| p[k] + dp[k])
            }).collect();
            let new_cost = cost_of(&new_intr, &new_poses);
            if new_cost.is_finite() && new_cost < cost {
                converged = cost - new_cost < 1e-10 * cost;
                *intr = new_intr;
                poses.copy_from_slice(&new_poses);
                cost = new_cost;
                lambda = (lambda / 10.0).max(1e-12);
                stepped = true;
                break;
            }
            lambda *= 10.0;
        }
        if !stepped || converged { break; }
    }
    cost
}

/// A view's board pose from its corners' rays through the model's starting lens (`Model::start`, at `intr`'s focal
/// length and principal point, so only for the coefficients the fit starts from). The rays are first rotated so their
/// mean is the optical axis: a rigid change of frame, so the board is still a board in it, and one where every ray
/// of a board has z > 0 - which a board seen past 90° from the real axis has not - so the planar homography (Zhang)
/// works there. The pose is then rotated back
fn initial_pose<M: Model + ?Sized>(model: &M, intr: &[f64], board: &[(f64, f64)], pts: &[(f64, f64)]) -> Option<Pose> {
    let rays: Vec<Vector3<f64>> = pts.iter().map(|p| model.start().ray((p.0 - intr[2]) / intr[0], (p.1 - intr[3]) / intr[1])).collect::<Option<_>>()?;
    let mean = rays.iter().sum::<Vector3<f64>>();
    if mean.norm() < 1e-9 { return None; }
    let to_axis = Rotation3::rotation_between(&mean, &Vector3::z()).unwrap_or_else(|| Rotation3::from_axis_angle(&Vector3::x_axis(), PI));
    let plane: Vec<(f64, f64)> = rays.iter().map(|r| {
        let q = to_axis * r;
        (q.z > 1e-3).then(|| (q.x / q.z, q.y / q.z))
    }).collect::<Option<_>>()?;

    let hm = homography(board, &plane)?;
    let (h1, h2, h3) = (hm.column(0).into_owned(), hm.column(1).into_owned(), hm.column(2).into_owned());
    let mut l = 2.0 / (h1.norm() + h2.norm());
    // The board's origin is one of its corners, so it's in front
    if h3.z * l < 0.0 { l = -l; }
    let (r1, r2) = (h1 * l, h2 * l);
    let svd = Matrix3::from_columns(&[r1, r2, r1.cross(&r2)]).svd(true, true);
    let (mut u, vt) = (svd.u?, svd.v_t?);
    if (u * vt).determinant() < 0.0 { u.column_mut(2).neg_mut(); }
    let r = to_axis.inverse() * Rotation3::from_matrix_unchecked(u * vt);
    let t = to_axis.inverse() * (h3 * l);
    let a = r.scaled_axis();
    let pose = [a.x, a.y, a.z, t.x, t.y, t.z];
    pose.iter().all(|x| x.is_finite()).then_some(pose)
}

/// `dst ~ H·src`, normalized DLT
fn homography(src: &[(f64, f64)], dst: &[(f64, f64)]) -> Option<Matrix3<f64>> {
    let normalize = |p: &[(f64, f64)]| -> Matrix3<f64> {
        let n = p.len() as f64;
        let (mx, my) = p.iter().fold((0.0, 0.0), |a, q| (a.0 + q.0 / n, a.1 + q.1 / n));
        let d = p.iter().map(|q| (q.0 - mx).hypot(q.1 - my)).sum::<f64>() / n;
        let s = if d > 1e-12 { std::f64::consts::SQRT_2 / d } else { 1.0 };
        Matrix3::new(s, 0.0, -s * mx, 0.0, s, -s * my, 0.0, 0.0, 1.0)
    };
    let (ts, td) = (normalize(src), normalize(dst));
    let mut ata = SMatrix::<f64, 9, 9>::zeros();
    for (a, b) in src.iter().zip(dst) {
        let p = ts * Vector3::new(a.0, a.1, 1.0);
        let q = td * Vector3::new(b.0, b.1, 1.0);
        let r1 = SVector::<f64, 9>::from_row_slice(&[-p.x, -p.y, -1.0, 0.0, 0.0, 0.0, q.x * p.x, q.x * p.y, q.x]);
        let r2 = SVector::<f64, 9>::from_row_slice(&[0.0, 0.0, 0.0, -p.x, -p.y, -1.0, q.y * p.x, q.y * p.y, q.y]);
        ata += r1 * r1.transpose() + r2 * r2.transpose();
    }
    let eig = ata.symmetric_eigen();
    let h = eig.eigenvectors.column(eig.eigenvalues.imin()).into_owned();
    let hn = Matrix3::new(h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], h[8]);
    Some(td.try_inverse()? * hn * ts)
}
