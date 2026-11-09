// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Gyro bias measured from the accelerometer over the whole clip, for the integrators that fuse both.
//!
//! The integrators correct the inclination with the accelerometer only slowly (VQF with a 40 s time constant, so the horizon
//! doesn't follow the centripetal acceleration in turns), and the bias estimators they have assume either rest (VQF: below
//! 2 °/s per axis) or recordings much longer than a clip. A bias they miss turns into a horizon that rotates steadily: the
//! Sony a7 IV has about 2 °/s around the optical axis, which is 30° over a 15 s clip (#1233).
//!
//! With the bias right, the accelerometer rotated into the inertial frame by the gyro integration keeps pointing at the same
//! "up" over the clip. A remaining bias `db` (rad/s, sensor frame) rotates that frame by `phi(t) = Int R(s) db ds`, so the
//! measured up drifts by `phi(t) x u`: linear in `db` and in `u`, solved by least squares and re-integrated (Gauss-Newton).
//! Only what the data measures significantly is kept: the walking or flight accelerations limit how well a window measures the
//! bias (to about 0.05 °/s), the part along gravity doesn't change the inclination at all (on a camera that doesn't rotate it
//! can't be measured), and what isn't measured is left to the integrator's own estimator - as are biases too small to matter,
//! which it handles. Blocks with a strong linear acceleration are ignored and the rest are reweighted by their residual, so
//! sustained turns don't pull the estimate.
//!
//! On a vehicle the accelerometer doesn't show gravity for long stretches (a banked turn, a roll, a loop keep it pointing
//! at the floor), which a window can take for a bias of several °/s, and an aerobatic flight is mostly such windows (#1287).
//! So each window measures the bias on its own, against none, and a measurement counts only when the windows next to it
//! agree: a bias drifts slowly and they see it too, while an acceleration the model doesn't fit fools one window at a time.

use nalgebra::{ Matrix3, Vector3, UnitQuaternion };
use crate::gyro_source::TimeIMU;

const DEG2RAD: f64 = std::f64::consts::PI / 180.0;

/// Length of the windows the bias is measured in: long enough to see it drift, short enough to follow its temperature drift
/// and for the first iteration (which starts from zero bias) to stay close to linear
const WINDOW_MS: f64 = 20_000.0;
/// The accelerometer is averaged over blocks of this length in the inertial frame
const BLOCK_MS: f64 = 50.0;
/// Bias directions observed less than this fraction of the best observed one aren't updated by the iterations: nearly
/// unobservable ones would take huge steps from noise and keep the iterations from converging
const MIN_RELATIVE_OBSERVABILITY: f64 = 1e-4;
/// A bias component is kept when it's this many standard errors away from what the window started from
const SIGNIFICANCE: f64 = 3.0;
/// Smaller biases are left to the integrator's own estimator, which handles them: at that level VQF's measures them a bit
/// differently than this, and removing them here too made the horizon of long clips slightly worse. The bias is scaled by
/// `1 - (MIN_BIAS / |bias|)²`, so it fades in instead of switching on
const MIN_BIAS_DEG_S: f64 = 0.15;
/// A bias larger than this is not a bias but motion data the model doesn't fit (or a wrong IMU orientation)
const MAX_BIAS_DEG_S: f64 = 10.0;

#[derive(Debug, Clone, Default)]
pub struct GyroBias {
    /// (window centre in ms, bias in deg/s in the frame of the samples)
    windows: Vec<(f64, Vector3<f64>)>,
}

impl GyroBias {
    /// Bias at a timestamp, in deg/s: linearly interpolated between the windows, held before the first and after the last
    pub fn at(&self, timestamp_ms: f64) -> [f64; 3] {
        let i = self.windows.partition_point(|w| w.0 <= timestamp_ms);
        let b = if i == 0 {
            self.windows[0].1
        } else if i == self.windows.len() {
            self.windows[i - 1].1
        } else {
            let (t0, b0) = self.windows[i - 1];
            let (t1, b1) = self.windows[i];
            b0 + (b1 - b0) * ((timestamp_ms - t0) / (t1 - t0))
        };
        [b.x, b.y, b.z]
    }

    /// `gyro` (deg/s) without the bias at `timestamp_ms`; as it is without a bias
    pub fn remove(bias: Option<&Self>, timestamp_ms: f64, gyro: &[f64; 3]) -> [f64; 3] {
        match bias {
            Some(b) => { let b = b.at(timestamp_ms); [gyro[0] - b[0], gyro[1] - b[1], gyro[2] - b[2]] },
            None => *gyro
        }
    }

    /// Measures the bias of `imu` (gyro in deg/s, accelerometer in any unit). None when there's no accelerometer, or no window
    /// gave a bias that explains the accelerometer better than none
    pub fn estimate(imu: &[TimeIMU]) -> Option<Self> {
        let first = imu.first()?.timestamp_ms;
        let last = imu.last()?.timestamp_ms;
        let duration = last - first;
        if !(duration > 2.0 * BLOCK_MS) || !imu.iter().any(|x| x.accl.is_some()) { return None; }

        // Short clips are one window, longer ones get windows overlapping by half
        let count = if duration <= 1.5 * WINDOW_MS { 1 } else { (duration / (WINDOW_MS / 2.0)).round() as usize - 1 };
        let length = if count == 1 { duration } else { duration / (count + 1) as f64 * 2.0 };

        let mut measured = Vec::with_capacity(count);
        let mut prev = Vector3::zeros();
        for k in 0..count {
            let from = first + k as f64 * length / 2.0;
            let to = from + length;
            let a = imu.partition_point(|x| x.timestamp_ms < from);
            let b = imu.partition_point(|x| x.timestamp_ms <= to);
            // The iterations start from the previous window's bias, it changes slowly
            if let Some(bias) = estimate_window(&imu[a..b], prev) {
                measured.push(((from + to) / 2.0, bias));
                prev = bias;
            }
        }
        // Each measurement takes the median of itself and its neighbours (`median_of_neighbours`); the windows that measured
        // nothing (the model didn't fit) are interpolated across
        let windows: Vec<(f64, Vector3<f64>)> = (0..measured.len()).map(|i| {
            let bias = if count == 1 { measured[i].1 } else { median_of_neighbours(&measured, i) };
            let n = bias.norm();
            (measured[i].0, if n > MIN_BIAS_DEG_S { bias * (1.0 - (MIN_BIAS_DEG_S / n).powi(2)) } else { Vector3::zeros() })
        }).collect();
        if windows.iter().all(|w| w.1 == Vector3::zeros()) { return None; }
        log::debug!("Gyro bias from the accelerometer (deg/s): {}", windows.iter().map(|(t, b)| format!("{:.1} s: [{:.3}, {:.3}, {:.3}]", t / 1000.0, b.x, b.y, b.z)).collect::<Vec<_>>().join(", "));
        Some(Self { windows })
    }
}

/// Component-wise median of measurement `i` (window centre, bias) and the nearest ones before and after it; the first and the
/// last take the three nearest. With fewer than three measurements the missing ones are zero, so a measurement always needs
/// another one to agree
fn median_of_neighbours(measured: &[(f64, Vector3<f64>)], i: usize) -> Vector3<f64> {
    let n = measured.len() as isize;
    let mid = if n >= 3 { (i as isize).clamp(1, n - 2) } else { i as isize };
    let at = |k: isize| if k < 0 || k >= n { Vector3::zeros() } else { measured[k as usize].1 };
    let (a, b, c) = (at(mid - 1), at(mid), at(mid + 1));
    Vector3::from_fn(|r, _| a[r].min(b[r]).max(a[r].max(b[r]).min(c[r])))
}

struct Block {
    /// Unit "up" from the accelerometer in the inertial frame of the integration
    up: Vector3<f64>,
    /// `Int R(s) ds` from the window start to the end of the block
    phi: Matrix3<f64>,
    /// Length of the accelerometer average, relative to the window's median
    magnitude: f64,
}

fn integrate_blocks(imu: &[TimeIMU], bias: &Vector3<f64>) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(((imu.last().unwrap().timestamp_ms - imu[0].timestamp_ms) / BLOCK_MS) as usize + 1);
    let mut r = UnitQuaternion::<f64>::identity();
    let mut phi = Matrix3::<f64>::zeros();
    let mut acc_sum = Vector3::<f64>::zeros();
    let mut acc_count = 0usize;
    let mut block_start = imu[0].timestamp_ms;
    let mut prev_ts = imu[0].timestamp_ms;
    for x in imu {
        if let Some(g) = x.gyro {
            let dt = (x.timestamp_ms - prev_ts) / 1000.0;
            prev_ts = x.timestamp_ms;
            phi += r.to_rotation_matrix().matrix() * dt;
            r *= UnitQuaternion::from_scaled_axis((Vector3::new(g[0], g[1], g[2]) - bias) * (DEG2RAD * dt));
        }
        if let Some(a) = x.accl {
            let a = Vector3::new(a[0], a[1], a[2]);
            if a.norm_squared() > 0.0 {
                acc_sum += r * a;
                acc_count += 1;
            }
        }
        if x.timestamp_ms - block_start >= BLOCK_MS {
            if acc_count > 0 {
                let v = acc_sum / acc_count as f64;
                let n = v.norm();
                if n > 0.0 { blocks.push(Block { up: v / n, phi, magnitude: n }); }
            }
            acc_sum = Vector3::zeros();
            acc_count = 0;
            block_start = x.timestamp_ms;
        }
    }
    if blocks.len() > 2 {
        let mut m: Vec<f64> = blocks.iter().map(|b| b.magnitude).collect();
        m.sort_by(|a, b| a.total_cmp(b));
        let median = m[m.len() / 2];
        for b in blocks.iter_mut() { b.magnitude /= median; }
    }
    blocks
}

/// Difference between the measured up and the one the model explains (about the angle in rad), per block
fn residuals(blocks: &[Block], u: &Vector3<f64>, db: &Vector3<f64>) -> Vec<Vector3<f64>> {
    blocks.iter().map(|b| b.up - (u + (b.phi * db).cross(u))).collect()
}
fn norms(res: &[Vector3<f64>]) -> Vec<f64> { res.iter().map(|r| r.norm()).collect() }

/// How much the residuals' correlation in time (an acceleration lasts longer than a block) inflates the variance of a
/// least-squares estimate over independent ones: 1 + 2 sum of the autocorrelation, up to 2 s
fn variance_inflation(res: &[Vector3<f64>], w: &[f64]) -> f64 {
    let r: Vec<Vector3<f64>> = res.iter().zip(w).filter(|x| *x.1 > 0.0).map(|(r, w)| r * w.sqrt()).collect();
    let r0: f64 = r.iter().map(|r| r.norm_squared()).sum();
    if !(r0 > 0.0) { return 1.0; }
    let max_lag = ((2000.0 / BLOCK_MS) as usize).min(r.len() / 4);
    let mut sum = 0.0;
    for lag in 1..=max_lag {
        let rho = r.iter().zip(&r[lag..]).map(|(a, b)| a.dot(b)).sum::<f64>() / r0;
        if rho <= 0.0 { break; }
        sum += rho;
    }
    (1.0 + 2.0 * sum).max(1.0)
}

fn weighted_rms(res: &[f64], w: &[f64]) -> f64 {
    let (s, n) = res.iter().zip(w).fold((0.0, 0.0), |(s, n), (r, w)| (s + w * r * r, n + w));
    if n > 0.0 { (s / n).sqrt() } else { f64::INFINITY }
}

/// The blocks that show gravity (weight 1, the others 0), their mean up, and how well the bias the blocks were integrated
/// with explains the accelerometer: the spread of the measured up around that mean (rad)
fn gate_and_up(blocks: &[Block]) -> Option<(Vec<f64>, Vector3<f64>, f64)> {
    if blocks.len() < 20 { return None; }
    // Blocks with a strong linear acceleration don't show gravity
    let gate: Vec<f64> = blocks.iter().map(|b| if (b.magnitude - 1.0).abs() < 0.15 { 1.0 } else { 0.0 }).collect();
    let u = blocks.iter().zip(&gate).fold(Vector3::zeros(), |s, (b, w)| s + b.up * *w);
    if u.norm() <= 0.0 { return None; }
    let u = u.normalize();
    let rms = weighted_rms(&norms(&residuals(blocks, &u, &Vector3::zeros())), &gate);
    Some((gate, u, rms))
}

/// The bias the window `imu` measures, by itself: what's significant of it, the rest zero. The iterations start from
/// `initial` (the previous window's), which the result doesn't otherwise depend on
fn estimate_window(imu: &[TimeIMU], initial: Vector3<f64>) -> Option<Vector3<f64>> {
    if imu.len() < 2 { return None; }
    let mut bias = initial;
    // How well no bias at all explains the accelerometer, which the measurement has to beat
    let mut zero_rms = if initial == Vector3::zeros() { None } else { Some(gate_and_up(&integrate_blocks(imu, &Vector3::zeros()))?.2) };
    for _ in 0..10 {
        let blocks = integrate_blocks(imu, &bias);
        let (gate, mut u, rms) = gate_and_up(&blocks)?;
        if zero_rms.is_none() { zero_rms = Some(rms); }

        // Iteratively reweighted: blocks far from the model (sustained turns, bumps) count less
        let mut w = gate.clone();
        let mut db = Vector3::zeros();
        let mut eig = None;
        for pass in 0..3 {
            if pass > 0 {
                let res = norms(&residuals(&blocks, &u, &db));
                for (w, (r, g)) in w.iter_mut().zip(res.iter().zip(&gate)) { *w = g * (2.0 * DEG2RAD / r.max(1e-9)).min(1.0); }
            }
            // up ~ u + M db, M = -[u]x phi. Unknowns u (3) and db (3); u's block of the normal equations is (sum w) I, so db
            // comes from its Schur complement
            let mut sw = 0.0;
            let mut a_ub = Matrix3::<f64>::zeros();
            let mut a_bb = Matrix3::<f64>::zeros();
            let mut g_u = Vector3::<f64>::zeros();
            let mut g_b = Vector3::<f64>::zeros();
            let ux = u.cross_matrix();
            for (b, w) in blocks.iter().zip(&w) {
                if *w <= 0.0 { continue; }
                let m = -ux * b.phi;
                sw += w;
                a_ub += m * *w;
                a_bb += m.transpose() * m * *w;
                g_u += b.up * *w;
                g_b += m.transpose() * b.up * *w;
            }
            if sw <= 0.0 { return None; }
            let s = a_bb - a_ub.transpose() * a_ub / sw;
            let rhs = g_b - a_ub.transpose() * g_u / sw;
            let e = s.symmetric_eigen();
            let max_ev = e.eigenvalues.max();
            if !(max_ev > 0.0) { return None; }
            db = Vector3::zeros();
            for i in 0..3 {
                if e.eigenvalues[i] > MIN_RELATIVE_OBSERVABILITY * max_ev {
                    let v = e.eigenvectors.column(i);
                    db += v * (v.dot(&rhs) / e.eigenvalues[i]);
                }
            }
            u = (g_u - a_ub * db) / sw;
            u.normalize_mut();
            eig = Some(e);
        }

        let step = db / DEG2RAD;
        bias += step;
        if !bias.iter().all(|x| x.is_finite()) || bias.norm() > MAX_BIAS_DEG_S { return None; }
        if step.norm() < 1e-3 {
            // Keep only what the window measured: the bias along the directions where it's significant. The standard error
            // of each comes from the residuals (inflated by their correlation in time) and how strongly the direction is
            // observed. On a static camera the direction around gravity isn't observed at all, and the first iterations, far
            // from the solution, could have moved the bias along it. Where the window can't tell, the bias isn't carried
            // over from the previous window: a wrong one would stay for as long as the direction remains unobserved
            let eig = eig?;
            let res = residuals(&blocks, &u, &db);
            // (no accelerometer resolves the inclination better than about 0.01°)
            let variance = weighted_rms(&norms(&res), &w).max(0.01 * DEG2RAD).powi(2) * variance_inflation(&res, &w);
            let mut kept = Vector3::zeros();
            for i in 0..3 {
                let v = eig.eigenvectors.column(i);
                let sigma = (variance / eig.eigenvalues[i].max(1e-30)).sqrt() / DEG2RAD;
                let d = v.dot(&bias);
                if d.abs() > SIGNIFICANCE * sigma { kept += v * d; }
            }
            // And only when it explains the accelerometer at least as well as no bias
            return if rms <= zero_rms? { Some(kept) } else { None };
        }
    }
    None
}
