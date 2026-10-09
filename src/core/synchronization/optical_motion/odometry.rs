// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! The camera's rotation between two frames from the image alone, for files without motion data.
//!
//! A rotation-only fit takes the parallax of the camera's own translation for rotation: a drone flying forward over
//! the ground seems to pitch, by as much as the ground passes by - several pixels per frame, changing with height,
//! speed and terrain. Fitting only the distant points avoids that where there are any, and jumps between whichever
//! happen to agree where there aren't. So the rotation is estimated together with the translation and every tracked
//! point's depth: a point's bearing in the second frame is `b ∝ m·(a − ρ·c)`, with `ρ` its inverse depth and `c` the
//! camera's move. One frame pair alone can't tell rotation and translation apart well, but the depths persist along
//! the tracks and the velocity changes little from one frame to the next (the shake is all rotation), so each pair
//! starts from what the ones before it learned: a filter, one Gauss-Newton solve per pair with the depths eliminated,
//! six unknowns left. Translation and depth share an arbitrary scale, kept at a median inverse depth of 1

use std::collections::HashMap;
use nalgebra::{ Matrix3, Matrix3x6, Matrix6, Rotation3, Vector3, Vector6 };
use super::MIN_BAND_POINTS;

/// Tracking noise, pixels of the tracked frame
const NOISE_PX: f64 = 0.3;
/// Residuals beyond this count less (Cauchy), pixels of the tracked frame
const ROBUST_PX: f64 = 1.5;
/// Gauss-Newton iterations per frame pair
const ITERATIONS: usize = 4;
/// How unsure the velocity is with nothing known yet, per axis (variance, in units of the median inverse depth)
const VELOCITY_VAR_START: f64 = 1e-2;
/// How much the velocity may change from one frame pair to the next: a fixed part and a part relative to it
const VELOCITY_NOISE: f64 = 2e-4;
const VELOCITY_NOISE_REL: f64 = 0.03;
/// How much an inverse depth may change from one frame pair to the next, relative to it
const DEPTH_NOISE_REL: f64 = 0.05;

pub struct VisualOdometry {
    /// Per track: inverse depth at the last frame and its variance
    depth: HashMap<u32, (f64, f64)>,
    /// The camera's move over the last frame pair, in the orientation of its second frame, and its variance per axis
    velocity: Vector3<f64>,
    velocity_var: Vector3<f64>,
}

impl Default for VisualOdometry {
    fn default() -> Self {
        Self { depth: HashMap::new(), velocity: Vector3::zeros(), velocity_var: Vector3::repeat(VELOCITY_VAR_START) }
    }
}

impl VisualOdometry {
    /// Starts over: the next frame pair knows nothing from the ones before
    pub fn reset(&mut self) { *self = Self::default(); }

    /// The rotation `m` of a frame pair (`b ≈ m·a` for points at infinity), from the bearings `a`, `b` of its tracks
    /// `ids` in the two frames, starting from `m0`. `px` is the size of a pixel of the tracked frame, in radians
    pub fn step(&mut self, a: &[Vector3<f64>], b: &[Vector3<f64>], ids: &[u32], m0: Matrix3<f64>, px: f64) -> Matrix3<f64> {
        let n = a.len();
        let weight = 1.0 / (NOISE_PX * px).powi(2);
        let robust = ROBUST_PX * px;
        let known = ids.iter().filter(|id| self.depth.contains_key(id)).count();
        let (rho0, var, v0, v_var) = if known < MIN_BAND_POINTS {
            // A new run of tracks (the start, after a gap) has nothing to go on from
            self.reset();
            Self::start(a, b, m0, robust)
        } else {
            // New tracks start at the median depth, loosely
            let (rho0, var) = ids.iter().map(|id| self.depth.get(id).copied().unwrap_or((1.0, 4.0))).unzip();
            // The velocity turns with the camera
            let q = VELOCITY_NOISE + VELOCITY_NOISE_REL * self.velocity.norm();
            (rho0, var, m0 * self.velocity, self.velocity_var.add_scalar(q * q))
        };
        let mut rho = rho0.clone();

        let (mut m, mut v) = (m0, v0);
        let mut h = Matrix6::zeros();
        let mut hxr = vec![Vector6::zeros(); n];
        let mut hrr = vec![0.0; n];
        let mut gr = vec![0.0; n];
        for _ in 0..ITERATIONS {
            // r = b × (m·a − ρ·v), linearized in the rotation (m → exp(ω)·m), the velocity and each ρ
            let mut hxx = Matrix6::zeros();
            let mut gx = Vector6::zeros();
            for i in 0..n {
                let ma = m * a[i];
                let bx = b[i].cross_matrix();
                let r = b[i].cross(&(ma - v * rho[i]));
                let w = weight / (1.0 + (r.norm() / robust).powi(2));
                let mut jx = Matrix3x6::zeros();
                jx.fixed_view_mut::<3, 3>(0, 0).copy_from(&(-bx * ma.cross_matrix()));
                jx.fixed_view_mut::<3, 3>(0, 3).copy_from(&(-bx * rho[i]));
                let jr = -b[i].cross(&v);
                let jxt = jx.transpose();
                hxx += jxt * jx * w;
                gx += jxt * r * w;
                hxr[i] = jxt * jr * w;
                hrr[i] = w * jr.norm_squared() + 1.0 / var[i];
                gr[i] = w * jr.dot(&r) + (rho[i] - rho0[i]) / var[i];
            }
            for k in 0..3 {
                hxx[(3 + k, 3 + k)] += 1.0 / v_var[k];
                gx[3 + k] += (v[k] - v0[k]) / v_var[k];
            }
            // The depths eliminated (Schur complement): six unknowns left
            h = hxx;
            let mut g = gx;
            for i in 0..n {
                h -= hxr[i] * hxr[i].transpose() / hrr[i];
                g -= hxr[i] * (gr[i] / hrr[i]);
            }
            let Some(chol) = h.cholesky() else { self.reset(); return m0; };
            let dx = -chol.solve(&g);
            m = Rotation3::from_scaled_axis(dx.fixed_rows::<3>(0).into_owned()).into_inner() * m;
            v += dx.fixed_rows::<3>(3);
            for i in 0..n { rho[i] = (rho[i] - (gr[i] + hxr[i].dot(&dx)) / hrr[i]).max(0.0); }
        }
        let Some(cov) = h.try_inverse() else { self.reset(); return m; };

        // On to the second frame: the depths from there, and the scale back to a median inverse depth of 1
        let c = m.transpose() * v;
        let mut depth: Vec<(f64, f64)> = (0..n).map(|i| {
            let r = rho[i] / (a[i] - c * rho[i]).norm().max(1e-6);
            (r, 1.0 / hrr[i] + (DEPTH_NOISE_REL * r).powi(2))
        }).collect();
        let mut sorted: Vec<f64> = depth.iter().map(|d| d.0).collect();
        sorted.sort_by(|x, y| x.total_cmp(y));
        let s = sorted.get(n / 2).copied().unwrap_or(1.0);
        let s = if s > 1e-9 { s } else { 1.0 };
        for d in depth.iter_mut() { d.0 /= s; d.1 /= s * s; }
        self.depth = ids.iter().copied().zip(depth).collect();
        self.velocity = v * s;
        self.velocity_var = Vector3::new(cov[(3, 3)], cov[(4, 4)], cov[(5, 5)]) * (s * s);
        m
    }

    /// Where a first frame pair starts from: the direction of the translation is the one in all the planes that each
    /// point's two bearings span, less the rotation `m0`; the depths follow from it. Started from equal depths and no
    /// velocity instead, the first pair over flat ground can settle on the other way a plane's motion can be explained,
    /// with a rotation pixels off, which the pairs after it then keep
    fn start(a: &[Vector3<f64>], b: &[Vector3<f64>], m0: Matrix3<f64>, robust: f64) -> (Vec<f64>, Vec<f64>, Vector3<f64>, Vector3<f64>) {
        let n = a.len();
        let ma: Vec<Vector3<f64>> = a.iter().map(|a| m0 * a).collect();
        let mut planes = Matrix3::zeros();
        for (ma, b) in ma.iter().zip(b) {
            let normal = ma.cross(b);
            planes += normal * normal.transpose() / (1.0 + (normal.norm() / (5.0 * robust)).powi(2));
        }
        let eigen = planes.symmetric_eigen();
        let mut dir: Vector3<f64> = eigen.eigenvectors.column(eigen.eigenvalues.imin()).into_owned();
        // b × (m0·a) = ρ·(b × dir) for each point
        let mut rho: Vec<f64> = (0..n).map(|i| {
            let bd = b[i].cross(&dir);
            b[i].cross(&ma[i]).dot(&bd) / bd.norm_squared().max(1e-12)
        }).collect();
        let mut sorted = rho.clone();
        sorted.sort_by(|x, y| x.total_cmp(y));
        let mut median = sorted.get(n / 2).copied().unwrap_or(1.0);
        if median < 0.0 {
            dir = -dir;
            median = -median;
            rho.iter_mut().for_each(|r| *r = -*r);
        }
        let s = median.max(1e-9);
        let rho: Vec<f64> = rho.into_iter().map(|r| (r / s).max(0.0)).collect();
        (rho, vec![4.0; n], dir * s, Vector3::repeat(VELOCITY_VAR_START + (0.3 * s).powi(2)))
    }
}
