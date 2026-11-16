// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Anton Belkin

//! Bundle adjustment with rolling shutter: the pose of every frame, the 3D point of every track, the lens and the
//! readout time, fitted together to where the tracks were seen.
//!
//! - A point is drawn where the camera was when its row was read: the pose of the frame is the pose at the middle of
//!   the readout, and a row `v` is read `(v / height - 0.5) × readout` later, the camera moved on by its rotation and
//!   translation rates (central differences of the neighbouring frames' poses). Without this the lens comes out 8-17 px
//!   off on a walking clip, and the edges of the stabilized picture jump.
//! - Levenberg-Marquardt with the points eliminated (Schur complement over their 3x3 blocks): a step is one dense solve
//!   of the size of the cameras and the lens. Cauchy loss by reweighting.
//! - Every n-th track is held out of the fit: its point is fitted with the cameras and the lens fixed, and its
//!   reprojection error says how well they do on tracks they haven't seen.
//! - It has come to rest when the lens has stopped moving over the frame and the cost has stopped falling: the lens
//!   alone also stands still under heavy damping, with the cost still on its way down.

use nalgebra::{ DMatrix, DVector, Matrix2x3, Matrix2x6, Matrix3, Matrix6, Matrix6x3, Rotation3, SMatrix, UnitQuaternion, Vector2, Vector3, Vector6 };
use rayon::prelude::*;
use super::lens::Lens;

/// The lens unknowns: one scale of both focal lengths, fx, fy, cx, cy, k1..k4, the readout time
const GLOBALS: usize = 10;
/// Steps of their finite differences
const GLOBAL_EPS: [f64; GLOBALS] = [1e-6, 1e-3, 1e-3, 1e-3, 1e-3, 1e-6, 1e-6, 1e-6, 1e-6, 1e-5];

type Matrix2xG = SMatrix<f64, 2, GLOBALS>;
type MatrixGx3 = SMatrix<f64, GLOBALS, 3>;

/// Which of the lens unknowns a fit moves
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Free {
    /// One scale for both focal lengths
    pub scale: bool,
    pub focal: bool,
    pub centre: bool,
    pub distortion: bool,
    pub readout: bool,
}
impl Free {
    pub const NONE: Free = Free { scale: false, focal: false, centre: false, distortion: false, readout: false };
    pub const READOUT: Free = Free { readout: true, ..Free::NONE };
    /// The first stage of an estimate: the two unknowns that matter most
    pub const SCALE_READOUT: Free = Free { scale: true, readout: true, ..Free::NONE };
    pub const CENTRE_READOUT: Free = Free { centre: true, readout: true, ..Free::NONE };
    pub const ALL: Free = Free { scale: false, focal: true, centre: true, distortion: true, readout: true };

    fn indices(&self) -> Vec<usize> {
        let mask = [self.scale, self.focal, self.focal, self.centre, self.centre, self.distortion, self.distortion, self.distortion, self.distortion, self.readout];
        (0..GLOBALS).filter(|i| mask[*i]).collect()
    }
}

fn apply_globals(lens: &Lens, tau: f64, d: &[f64; GLOBALS]) -> (Lens, f64) {
    let mut l = *lens;
    let s = d[0].exp();
    l.fx = l.fx * s + d[1];
    l.fy = l.fy * s + d[2];
    l.cx += d[3];
    l.cy += d[4];
    for i in 0..4 { l.k[i] += d[5 + i]; }
    (l, tau + d[9])
}

/// The rotation vector of a rotation. By way of the quaternion: `Rotation3::scaled_axis` takes the angle from an
/// `acos` of the trace, which rounding can put a hair past 1 for a rotation next to none - and the fit then carries
/// a NaN
pub(super) fn rotation_vector(r: &Rotation3<f64>) -> Vector3<f64> {
    UnitQuaternion::from_rotation_matrix(r).scaled_axis()
}

/// One sighting of a point
#[derive(Clone, Copy, Debug)]
pub struct Sighting {
    pub cam: u32,
    pub point: u32,
    pub uv: [f64; 2],
}

/// What a fit starts from (`scene::rough_scene` makes it): the pose of every frame (world → camera, rotation vector
/// and translation), the points, and their sightings sorted by point
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub size: (usize, usize),
    /// When each frame was taken, in frame intervals from the first of them: 0, 1, 2... unless frames are missing
    /// (`Fragment`), and the rates the rolling shutter is corrected with are per frame interval
    pub times: Vec<f64>,
    pub rotations: Vec<Vector3<f64>>,
    pub translations: Vec<Vector3<f64>>,
    pub points: Vec<Vector3<f64>>,
    pub sightings: Vec<Sighting>,
    /// Points held out of the fit, see the module
    pub held_out: Vec<bool>,
}

#[derive(Clone, Copy, Debug)]
pub struct FitOptions {
    pub free: Free,
    pub iterations: usize,
    /// Scale of the Cauchy loss, pixels
    pub loss: f64,
    /// Whether the poses move. Without them (and with nothing free) only the points are fitted
    pub cameras: bool,
    /// Fit the held-out points too (see `Bundle::check`)
    pub all_points: bool,
    /// The fit has come to rest when the lens moves less than this over the frame (median, pixels), the readout less
    /// than 0.001 of the frame interval and the cost falls by less than 0.0001 of itself, twice in a row
    pub tolerance: f64,
}
impl FitOptions {
    pub fn new(free: Free, iterations: usize, loss: f64) -> Self { Self { free, iterations, loss, cameras: true, all_points: false, tolerance: 0.05 } }
    pub fn tolerance(mut self, tol: f64) -> Self { self.tolerance = tol; self }
    pub fn points_only(mut self) -> Self { self.cameras = false; self }
}

/// How a fit ended
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    pub cost: f64,
    /// Steps that lowered the cost. With none the fit never left where it started, and that it stayed there says
    /// nothing: no step was found, whether there was one to take or not
    pub steps: usize,
    /// It came to rest (`FitOptions::tolerance`), rather than ran out of iterations or was cancelled
    pub settled: bool,
}
impl Outcome {
    /// It moved, and what it has is a number
    pub fn sound(&self) -> bool { self.steps > 0 && self.cost.is_finite() }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct HeldOut {
    /// Reprojection error of the held-out tracks, rms of the errors capped at 6 px of a 2.7K frame (`Bundle::check`)
    pub rms: f64,
    /// Share of their sightings within a pixel of a 2.7K frame
    pub within_1px: f64,
}

pub struct Bundle {
    size: (f64, f64),
    times: Vec<f64>,
    rotations: Vec<Vector3<f64>>,
    translations: Vec<Vector3<f64>>,
    all_points: Vec<Vector3<f64>>,
    all_sightings: Vec<Sighting>,
    held_out: Vec<bool>,

    // The points being worked on (`select`), and their sightings
    selected: Vec<usize>,
    points: Vec<Vector3<f64>>,
    sightings: Vec<Sighting>,
    /// Where the sightings of each point start, and the end
    starts: Vec<usize>,
    test: Vec<bool>,
}

/// One sighting, linearized
struct Row {
    r: Vector2<f64>,
    w: f64,
    jp: Matrix2x3<f64>,
    jc: Matrix2x6<f64>,
    jg: Matrix2xG,
}

impl Bundle {
    pub fn new(scene: Scene) -> Self {
        let n = scene.points.len();
        let mut b = Self {
            size: (scene.size.0 as f64, scene.size.1 as f64),
            times: if scene.times.len() == scene.rotations.len() { scene.times } else { (0..scene.rotations.len()).map(|i| i as f64).collect() },
            rotations: scene.rotations, translations: scene.translations,
            all_points: scene.points, all_sightings: scene.sightings, held_out: scene.held_out,
            selected: Vec::new(), points: Vec::new(), sightings: Vec::new(), starts: Vec::new(), test: Vec::new(),
        };
        b.select_every(1);
        debug_assert_eq!(b.points.len(), n);
        b
    }

    pub fn cameras(&self) -> usize { self.rotations.len() }
    pub fn points(&self) -> usize { self.points.len() }
    pub fn sightings(&self) -> usize { self.sightings.len() }
    pub fn poses(&self) -> (&[Vector3<f64>], &[Vector3<f64>]) { (&self.rotations, &self.translations) }

    /// Works on every n-th point from here on: a third of them is enough to find the focal length and the readout,
    /// and three times quicker. What was fitted so far is kept
    pub fn select_every(&mut self, nth: usize) {
        self.store();
        self.selected = (0..self.all_points.len()).step_by(nth.max(1)).collect();
        let mut index = vec![u32::MAX; self.all_points.len()];
        for (i, p) in self.selected.iter().enumerate() { index[*p] = i as u32; }
        self.points = self.selected.iter().map(|p| self.all_points[*p]).collect();
        self.sightings = self.all_sightings.iter().filter(|s| index[s.point as usize] != u32::MAX).map(|s| Sighting { point: index[s.point as usize], ..*s }).collect();
        self.starts = vec![0; self.points.len() + 1];
        for s in &self.sightings { self.starts[s.point as usize + 1] += 1; }
        for i in 0..self.points.len() { self.starts[i + 1] += self.starts[i]; }
        self.test = self.sightings.iter().map(|s| self.held_out[self.selected[s.point as usize]]).collect();
    }

    fn store(&mut self) {
        for (i, p) in self.selected.iter().enumerate() { self.all_points[*p] = self.points[i]; }
    }

    /// Rotation and translation rates of every frame, per frame interval: what the camera does while a frame is read.
    /// From the frames next to each one, over the time between them - which is more than a frame interval where a
    /// frame is missing
    fn motion(times: &[f64], rotations: &[Vector3<f64>], translations: &[Vector3<f64>]) -> (Vec<Vector3<f64>>, Vec<Vector3<f64>>) {
        let n = rotations.len();
        let r: Vec<Rotation3<f64>> = rotations.iter().map(|w| Rotation3::new(*w)).collect();
        (0..n).map(|i| {
            let (lo, hi) = (i.saturating_sub(1), (i + 1).min(n - 1));
            let d = times[hi] - times[lo];
            if hi == lo || !(d > 0.0) { return (Vector3::zeros(), Vector3::zeros()); }
            let rel = r[hi] * r[lo].inverse();
            (rotation_vector(&rel) / d, (translations[hi] - rel * translations[lo]) / d)
        }).unzip()
    }

    /// Where a point of the camera's space (at the middle of the readout) is drawn, given the row it was seen in
    #[inline]
    fn draw(&self, lens: &Lens, tau: f64, om: &Vector3<f64>, vel: &Vector3<f64>, v: f64, mut xc: Vector3<f64>) -> [f64; 2] {
        if tau != 0.0 {
            let s = (v / self.size.1 - 0.5) * tau;
            xc = Rotation3::new(om * s) * xc + vel * s;
        }
        lens.project(&xc)
    }

    fn residuals(&self, lens: &Lens, tau: f64, rotations: &[Vector3<f64>], translations: &[Vector3<f64>], motion: &(Vec<Vector3<f64>>, Vec<Vector3<f64>>), points: &[Vector3<f64>]) -> Vec<Vector2<f64>> {
        let r: Vec<Matrix3<f64>> = rotations.iter().map(|w| *Rotation3::new(*w).matrix()).collect();
        self.sightings.par_iter().map(|s| {
            let c = s.cam as usize;
            let p = self.draw(lens, tau, &motion.0[c], &motion.1[c], s.uv[1], r[c] * points[s.point as usize] + translations[c]);
            Vector2::new(p[0] - s.uv[0], p[1] - s.uv[1])
        }).collect()
    }

    fn cost(r: &[Vector2<f64>], used: &[bool], loss: f64) -> f64 {
        r.iter().zip(used).filter(|(_, u)| **u).map(|(r, _)| loss * loss * (r.norm_squared() / (loss * loss)).ln_1p()).sum()
    }

    /// Fits what `options` frees, from this lens and readout time (in frame intervals). Returns them and how it ended.
    /// `cancelled` is asked every iteration; cancelled, the fit returns what it has
    pub fn fit(&mut self, mut lens: Lens, mut tau: f64, options: &FitOptions, cancelled: &(dyn Fn() -> bool + Sync)) -> (Lens, f64, Outcome) {
        let loss = options.loss;
        let used: Vec<bool> = if options.all_points { vec![true; self.sightings.len()] } else { self.test.iter().map(|t| !t).collect() };
        let gi = options.free.indices();
        let ng = gi.len();
        let nc = self.rotations.len();
        let ncam = if options.cameras { 6 * nc } else { 0 };
        let n = ncam + ng;
        let npt = self.points.len();
        let frame = (self.size.0 as usize, self.size.1 as usize);

        // The sightings of each camera, for the columns of the reduced system
        let mut of_camera: Vec<Vec<u32>> = vec![Vec::new(); nc];
        if options.cameras {
            for (i, s) in self.sightings.iter().enumerate() { of_camera[s.cam as usize].push(i as u32); }
        }

        let mut lambda = 1e-3;
        let mut calm = 0;
        let (mut steps, mut settled) = (0, false);
        let mut motion = Self::motion(&self.times, &self.rotations, &self.translations);
        let mut r = self.residuals(&lens, tau, &self.rotations, &self.translations, &motion, &self.points);
        let mut c0 = Self::cost(&r, &used, loss);
        // Nothing to go down from: a pose or a point that isn't a number. No step could be told from another
        if !c0.is_finite() { return (lens, tau, Outcome { cost: c0, steps: 0, settled: false }); }

        for it in 0..options.iterations {
            if cancelled() { break; }
            // ---- Linearize. The Jacobian over the point in the camera's space is a finite difference of the drawing
            // (lens, rolling shutter and all); the poses' and the points' follow from it
            let rot: Vec<Matrix3<f64>> = self.rotations.iter().map(|w| *Rotation3::new(*w).matrix()).collect();
            let rows: Vec<Row> = self.sightings.par_iter().enumerate().map(|(i, s)| {
                let c = s.cam as usize;
                let (om, vel) = (&motion.0[c], &motion.1[c]);
                let xr = rot[c] * self.points[s.point as usize];
                let xc = xr + self.translations[c];
                let at = |lens: &Lens, tau: f64, xc: Vector3<f64>| { let p = self.draw(lens, tau, om, vel, s.uv[1], xc); Vector2::new(p[0] - s.uv[0], p[1] - s.uv[1]) };
                let r0 = r[i];
                let e = 1e-6;
                let mut jt = Matrix2x3::zeros();
                for k in 0..3 {
                    let mut d = Vector3::zeros(); d[k] = e;
                    jt.set_column(k, &((at(&lens, tau, xc + d) - r0) / e));
                }
                let mut jc = Matrix2x6::zeros();
                if options.cameras {
                    // d(δω × Xr) / dδω = -[Xr]×
                    let sk = Matrix3::new(0.0, xr.z, -xr.y,  -xr.z, 0.0, xr.x,  xr.y, -xr.x, 0.0);
                    jc.fixed_view_mut::<2, 3>(0, 0).copy_from(&(jt * sk));
                    jc.fixed_view_mut::<2, 3>(0, 3).copy_from(&jt);
                }
                let mut jg = Matrix2xG::zeros();
                for (n, g) in gi.iter().enumerate() {
                    let mut d = [0.0; GLOBALS]; d[*g] = GLOBAL_EPS[*g];
                    let (l2, t2) = apply_globals(&lens, tau, &d);
                    jg.set_column(n, &((at(&l2, t2, xc) - r0) / GLOBAL_EPS[*g]));
                }
                let w = if used[i] { 1.0 / (1.0 + r0.norm_squared() / (loss * loss)) } else { 0.0 };
                Row { r: r0, w, jp: jt * rot[c], jc, jg }
            }).collect();

            // ---- Normal equations by blocks: cameras and lens (U), points (V, 3x3 each), and what ties them (W)
            let mut v = vec![Matrix3::<f64>::zeros(); npt];
            let mut gp = vec![Vector3::<f64>::zeros(); npt];
            let mut wg = vec![MatrixGx3::zeros(); if ng > 0 { npt } else { 0 }];
            let mut wc: Vec<Matrix6x3<f64>> = Vec::new();
            let mut u = DMatrix::<f64>::zeros(n, n);
            let mut gc = DVector::<f64>::zeros(n);
            if options.cameras { wc.reserve(rows.len()); }
            for (row, s) in rows.iter().zip(&self.sightings) {
                let p = s.point as usize;
                let wr = row.r * row.w;
                v[p] += row.jp.transpose() * row.jp * row.w;
                gp[p] -= row.jp.transpose() * wr;
                if options.cameras {
                    let c = 6 * s.cam as usize;
                    let jcw = row.jc.transpose() * row.w;
                    let a: Matrix6<f64> = jcw * row.jc;
                    let g: Vector6<f64> = row.jc.transpose() * wr;
                    for i in 0..6 {
                        gc[c + i] -= g[i];
                        for j in 0..6 { u[(c + i, c + j)] += a[(i, j)]; }
                    }
                    if ng > 0 {
                        let cg = jcw * row.jg;
                        for i in 0..6 { for j in 0..ng { u[(c + i, ncam + j)] += cg[(i, j)]; u[(ncam + j, c + i)] += cg[(i, j)]; } }
                    }
                    wc.push(jcw * row.jp);
                }
                if ng > 0 {
                    let jgw = row.jg.transpose() * row.w;
                    let gg = jgw * row.jg;
                    let g = row.jg.transpose() * wr;
                    for i in 0..ng {
                        gc[ncam + i] -= g[i];
                        for j in 0..ng { u[(ncam + i, ncam + j)] += gg[(i, j)]; }
                    }
                    wg[p] += jgw * row.jp;
                }
            }

            // ---- Steps, with more damping until one lowers the cost
            let mut accepted = None;
            loop {
                let vi: Vec<Matrix3<f64>> = v.iter().map(|v| {
                    (v + Matrix3::identity() * (lambda * (v.trace() / 3.0).max(1e-9) + 1e-12)).try_inverse().unwrap_or_else(Matrix3::zeros)
                }).collect();
                let mut dc = DVector::<f64>::zeros(n);
                if n > 0 {
                    let mut s = u.clone();
                    let mut rhs = gc.clone();
                    if options.cameras {
                        // The columns of each camera: every sighting of it against every sighting of the same point
                        let sightings = &self.sightings;
                        let starts = &self.starts;
                        let (wc, wg, vi, gp) = (&wc, &wg, &vi, &gp);
                        let sub: Vec<Vector6<f64>> = s.as_mut_slice()[..ncam * n].par_chunks_mut(6 * n).zip(of_camera.par_iter()).map(|(cols, mine)| {
                            let mut rhs = Vector6::zeros();
                            for &o in mine {
                                let o = o as usize;
                                let p = sightings[o].point as usize;
                                let z = vi[p] * wc[o].transpose(); // 3x6
                                rhs += wc[o] * (vi[p] * gp[p]);
                                for o2 in starts[p]..starts[p + 1] {
                                    let b = wc[o2] * z; // 6x6: rows of the other camera, columns of this one
                                    let r0 = 6 * sightings[o2].cam as usize;
                                    for j in 0..6 { for i in 0..6 { cols[j * n + r0 + i] -= b[(i, j)]; } }
                                }
                                if ng > 0 {
                                    let b = wg[p] * z; // Gx6
                                    for j in 0..6 { for i in 0..ng { cols[j * n + ncam + i] -= b[(i, j)]; } }
                                }
                            }
                            rhs
                        }).collect();
                        for (c, d) in sub.iter().enumerate() { for i in 0..6 { rhs[6 * c + i] -= d[i]; } }
                        for j in 0..ng { for i in 0..ncam { s[(i, ncam + j)] = s[(ncam + j, i)]; } }
                    }
                    if ng > 0 {
                        for p in 0..npt {
                            let y = wg[p] * vi[p]; // Gx3
                            let b = y * wg[p].transpose();
                            let d = y * gp[p];
                            for i in 0..ng {
                                rhs[ncam + i] -= d[i];
                                for j in 0..ng { s[(ncam + i, ncam + j)] -= b[(i, j)]; }
                            }
                        }
                    }
                    for i in 0..n { s[(i, i)] += lambda * u[(i, i)].max(1e-9) + 1e-9; }
                    dc = solve(s, rhs);
                }
                let dg = { let mut d = [0.0; GLOBALS]; for (i, g) in gi.iter().enumerate() { d[*g] = dc[ncam + i]; } d };
                let dgv = { let mut d = SMatrix::<f64, GLOBALS, 1>::zeros(); for i in 0..ng { d[i] = dc[ncam + i]; } d };

                let mut points = self.points.clone();
                for p in 0..npt {
                    let mut g = gp[p];
                    if options.cameras {
                        for o in self.starts[p]..self.starts[p + 1] {
                            let c = 6 * self.sightings[o].cam as usize;
                            g -= wc[o].transpose() * Vector6::from_fn(|i, _| dc[c + i]);
                        }
                    }
                    if ng > 0 { g -= wg[p].transpose() * dgv; }
                    points[p] += vi[p] * g;
                }
                let (rotations, translations) = if options.cameras {
                    ((0..nc).map(|c| rotation_vector(&(Rotation3::new(Vector3::new(dc[6 * c], dc[6 * c + 1], dc[6 * c + 2])) * Rotation3::new(self.rotations[c])))).collect::<Vec<_>>(),
                     (0..nc).map(|c| self.translations[c] + Vector3::new(dc[6 * c + 3], dc[6 * c + 4], dc[6 * c + 5])).collect::<Vec<_>>())
                } else {
                    (self.rotations.clone(), self.translations.clone())
                };
                let (lens2, tau2) = apply_globals(&lens, tau, &dg);
                let motion2 = if options.cameras { Self::motion(&self.times, &rotations, &translations) } else { motion.clone() };
                let r2 = if lens2.is_finite() { self.residuals(&lens2, tau2, &rotations, &translations, &motion2, &points) } else { Vec::new() };
                let c1 = if r2.is_empty() { f64::INFINITY } else { Self::cost(&r2, &used, loss) };
                // A cost that isn't a number is no lower cost
                if c1.is_finite() && c1 < c0 {
                    accepted = Some((rotations, translations, points, lens2, tau2, motion2, r2, c1));
                    break;
                }
                if lambda > 1e8 { break; }
                lambda *= 5.0;
            }

            let (mut moved, mut dtau, mut rel) = (0.0, 0.0, 0.0);
            if let Some((rotations, translations, points, lens2, tau2, motion2, r2, c1)) = accepted {
                moved = lens.shift(&lens2, frame).median;
                dtau = (tau2 - tau).abs();
                rel = (c0 - c1) / c0;
                self.rotations = rotations; self.translations = translations; self.points = points;
                lens = lens2; tau = tau2; motion = motion2; r = r2; c0 = c1;
                lambda = (lambda / 4.0).max(1e-7);
                steps += 1;
            }
            log::trace!("lens from video: it {it} cost {c0:.0} f {:.1},{:.1} c {:.1},{:.1} k {:?} tau {tau:.4} moved {moved:.3} px", lens.fx, lens.fy, lens.cx, lens.cy, lens.k);
            // The lens standing still isn't rest by itself: under heavy damping it barely moves while the cost still falls
            calm = if (moved < options.tolerance && dtau < 1e-3 && rel < 1e-4) || rel < 1e-5 { calm + 1 } else { 0 };
            if calm >= 2 && it >= 3 { settled = true; break; }
        }
        (lens, tau, Outcome { cost: c0, steps, settled })
    }

    /// The held-out tracks: their points fitted with the cameras and the lens as they are, then their reprojection
    /// error. `px`: pixels of this frame per pixel of a 2.7K one (`px_scale`) - the error comes back in pixels of a
    /// 2.7K frame, capped and counted there, so it means the same for a frame of any size
    pub fn check(&mut self, lens: &Lens, tau: f64, loss: f64, px: f64) -> HeldOut {
        let options = FitOptions { all_points: true, ..FitOptions::new(Free::NONE, 5, loss).points_only() };
        self.fit(*lens, tau, &options, &|| false);
        let motion = Self::motion(&self.times, &self.rotations, &self.translations);
        let e: Vec<f64> = self.residuals(lens, tau, &self.rotations, &self.translations, &motion, &self.points).iter().zip(&self.test).filter(|(_, t)| **t).map(|(r, _)| r.norm() / px).collect();
        if e.is_empty() { return HeldOut { rms: f64::INFINITY, within_1px: 0.0 }; }
        HeldOut {
            rms: (e.iter().map(|e| e.min(6.0) * e.min(6.0)).sum::<f64>() / e.len() as f64).sqrt(),
            within_1px: e.iter().filter(|e| **e < 1.0).count() as f64 / e.len() as f64,
        }
    }

    /// How much the camera moved through space, as the points saw it: the angle between the rays of the first and the
    /// last sighting of a point (each turned back to the middle of its frame's readout), degrees, median over the
    /// points. A camera that only turned has none, whatever the rough scene made of its rolling shutter
    pub fn parallax(&self, lens: &Lens, tau: f64) -> f64 {
        let motion = Self::motion(&self.times, &self.rotations, &self.translations);
        let ray = |s: &Sighting| {
            let c = s.cam as usize;
            let b = lens.unproject(s.uv);
            let dt = (s.uv[1] / self.size.1 - 0.5) * tau;
            Rotation3::new(self.rotations[c]).inverse() * (Rotation3::new(motion.0[c] * dt).inverse() * b)
        };
        let mut angles: Vec<f64> = (0..self.points.len()).filter(|p| self.starts[p + 1] - self.starts[*p] >= 2).map(|p| {
            ray(&self.sightings[self.starts[p]]).dot(&ray(&self.sightings[self.starts[p + 1] - 1])).clamp(-1.0, 1.0).acos().to_degrees()
        }).collect();
        if angles.is_empty() { return 0.0; }
        angles.sort_by(|a, b| a.total_cmp(b));
        angles[angles.len() / 2]
    }

    /// The poses and the points as they are, to go back to (`restore`) after a fit that was only a question
    pub fn state(&self) -> (Vec<Vector3<f64>>, Vec<Vector3<f64>>, Vec<Vector3<f64>>) {
        (self.rotations.clone(), self.translations.clone(), self.points.clone())
    }
    pub fn restore(&mut self, state: &(Vec<Vector3<f64>>, Vec<Vector3<f64>>, Vec<Vector3<f64>>)) {
        self.rotations = state.0.clone(); self.translations = state.1.clone(); self.points = state.2.clone();
    }
}

/// The reduced system is symmetric and, damped, positive definite: Cholesky, and LU where rounding says otherwise
fn solve(s: DMatrix<f64>, rhs: DVector<f64>) -> DVector<f64> {
    let n = rhs.len();
    match cholesky(s.clone()) {
        Some(l) => {
            // L y = b, Lᵀ x = y
            let mut x = rhs;
            for i in 0..n {
                let mut sum = x[i];
                for k in 0..i { sum -= l[(i, k)] * x[k]; }
                x[i] = sum / l[(i, i)];
            }
            for i in (0..n).rev() {
                let mut sum = x[i];
                for k in i + 1..n { sum -= l[(k, i)] * x[k]; }
                x[i] = sum / l[(i, i)];
            }
            x
        },
        None => s.lu().solve(&rhs).unwrap_or_else(|| DVector::zeros(n)),
    }
}

/// Cholesky of the reduced system, blocked: the panel below each block of columns is solved against it, and the rest
/// of the matrix updated with one matrix multiplication. At 300 frames the system is 1800 wide and this is most of a
/// step; column by column it was spending its time walking the trailing matrix instead of multiplying
fn cholesky(mut a: DMatrix<f64>) -> Option<DMatrix<f64>> {
    const BLOCK: usize = 64;
    let n = a.nrows();
    let mut j = 0;
    while j < n {
        let nb = BLOCK.min(n - j);
        // The block on the diagonal, column by column
        for k in j..j + nb {
            let mut d = a[(k, k)];
            for t in j..k { d -= a[(k, t)] * a[(k, t)]; }
            if !(d > 0.0) || !d.is_finite() { return None; }
            let d = d.sqrt();
            a[(k, k)] = d;
            for i in k + 1..j + nb {
                let mut v = a[(i, k)];
                for t in j..k { v -= a[(i, t)] * a[(k, t)]; }
                a[(i, k)] = v / d;
            }
        }
        let m = n - j - nb;
        if m == 0 { break; }
        // The panel below it: L21 L11ᵀ = A21, a forward substitution a row at a time
        for i in j + nb..n {
            for k in j..j + nb {
                let mut v = a[(i, k)];
                for t in j..k { v -= a[(i, t)] * a[(k, t)]; }
                a[(i, k)] = v / a[(k, k)];
            }
        }
        // And what's left of the matrix: A22 -= L21 L21ᵀ
        let l21 = a.view((j + nb, j), (m, nb)).into_owned();
        let mut a22 = a.view_mut((j + nb, j + nb), (m, m));
        a22.gemm(-1.0, &l21, &l21.transpose(), 1.0);
        j += nb;
    }
    Some(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wider than two blocks: the panel below a block and the update of what's left are both at work
    #[test]
    fn cholesky_solves() {
        for n in [40, 150] {
            let m = DMatrix::from_fn(n, n, |i, j| ((i * 7 + j * 3) % 11) as f64 / 11.0);
            let a = &m * m.transpose() + DMatrix::identity(n, n) * 0.5;
            let x0 = DVector::from_fn(n, |i, _| i as f64 - 3.0);
            let x = solve(a.clone(), &a * &x0);
            let off = (x - x0).amax();
            assert!(off < 1e-6, "{n}: {off}");
            let l = cholesky(a.clone()).unwrap();
            let lower = l.lower_triangle();
            assert!((&lower * lower.transpose() - &a).amax() < 1e-9, "{n}");
        }
    }

    /// A small scene seen through `lens`: a camera going forward and sideways, every 5th point held out
    fn scene(lens: &Lens) -> Scene {
        let size = (2704, 1520);
        let cameras = 14;
        let (rotations, translations): (Vec<Vector3<f64>>, Vec<Vector3<f64>>) = (0..cameras).map(|c| {
            let t = c as f64;
            let r = Rotation3::new(Vector3::new(0.02 * (0.6 * t).sin(), 0.03 * (0.4 * t).cos(), 0.01 * t));
            (rotation_vector(&r), -(r * Vector3::new(0.2 * (0.5 * t).sin(), 0.1 * (0.3 * t).cos(), 0.08 * t)))
        }).unzip();
        let points: Vec<Vector3<f64>> = (0..120).map(|i| {
            let i = i as f64;
            Vector3::new(6.0 * (i * 0.37).sin(), 3.5 * (i * 0.91).cos(), 4.0 + 9.0 * (0.5 + 0.5 * (i * 0.13).sin()))
        }).collect();
        let mut sightings = Vec::new();
        for (p, x) in points.iter().enumerate() {
            for c in 0..cameras {
                let uv = lens.project(&(Rotation3::new(rotations[c]) * x + translations[c]));
                if uv[0] > 0.0 && uv[1] > 0.0 && uv[0] < size.0 as f64 && uv[1] < size.1 as f64 { sightings.push(Sighting { cam: c as u32, point: p as u32, uv }); }
            }
        }
        Scene { size, times: (0..cameras).map(|c| c as f64).collect(), rotations, translations, held_out: (0..points.len()).map(|p| p % 5 == 0).collect(), points, sightings }
    }

    /// The held-out tracks take no part in a fit: moved 40 px away, they leave it exactly where it was
    #[test]
    fn held_out_tracks_are_out_of_the_fit() {
        let truth = Lens::new(1766.95, 1773.07, 1311.74, 774.68, [0.05352, -0.00625, 0.02390, -0.01311]);
        let mut start = truth; start.fx *= 1.02; start.fy *= 1.02;
        let clean = scene(&truth);
        let mut spoiled = clean.clone();
        // One way in one frame and the other in the next: no point is seen there
        for s in spoiled.sightings.iter_mut() { if spoiled.held_out[s.point as usize] { let d = if s.cam % 2 == 0 { 40.0 } else { -40.0 }; s.uv[0] += d; s.uv[1] -= d; } }
        assert!(spoiled.sightings.iter().zip(&clean.sightings).any(|(a, b)| a.uv != b.uv));
        let options = FitOptions::new(Free::ALL, 8, 1.5);
        let (mut a, mut b) = (Bundle::new(clean), Bundle::new(spoiled));
        let (fa, fb) = (a.fit(start, 0.0, &options, &|| false), b.fit(start, 0.0, &options, &|| false));
        assert!(fa.2.sound() && fa.0 != start, "{fa:?}");
        assert_eq!((fa.0, fa.1, fa.2.cost), (fb.0, fb.1, fb.2.cost));
        assert_eq!(a.poses(), b.poses());
        // And they're what the check is made on: the error is theirs
        let (ea, eb) = (a.check(&fa.0, fa.1, 1.5, 1.0).rms, b.check(&fb.0, fb.1, 1.5, 1.0).rms);
        assert!(ea < 1.0 && eb > 5.0, "{ea} {eb}");
    }

    /// A fit that has nothing to go down from, or finds no step, says so: it doesn't pass for one that stayed put
    #[test]
    fn a_fit_that_took_no_step_is_not_sound() {
        let truth = Lens::new(1766.95, 1773.07, 1311.74, 774.68, [0.05352, -0.00625, 0.02390, -0.01311]);
        let mut broken = scene(&truth);
        broken.translations[3].x = f64::NAN;
        let (lens, _, outcome) = Bundle::new(broken).fit(truth, 0.0, &FitOptions::new(Free::ALL, 5, 1.5), &|| false);
        assert!(!outcome.sound() && outcome.steps == 0 && lens == truth, "{outcome:?}");
        // The rotation vector of a rotation next to none is a number
        let tiny = Rotation3::new(Vector3::new(1e-9, -2e-9, 5e-10));
        assert!(rotation_vector(&(tiny * tiny.inverse())).iter().all(|x| x.is_finite()));
        assert!((rotation_vector(&Rotation3::new(Vector3::new(0.3, -0.2, 0.1))) - Vector3::new(0.3, -0.2, 0.1)).norm() < 1e-12);
    }
}
