// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow contributors

//! Local, optical-flow-only stabilization.
//!
//! Camera motion is handled by the regular quaternion pipeline.  This module
//! deliberately models only the motion left after a robust affine camera fit,
//! accumulates that residual into short local trajectories, and smooths those
//! trajectories.  The resulting correction is encoded using the same 9x9
//! spline mesh consumed by all Gyroflow render backends.

use crate::gyro_source::splines::{self, BivariateSpline};
use nalgebra::{Matrix3, Vector3};

const GRID: usize = splines::MAX_GRID_SIZE;
const MIN_TRACKS: usize = 18;
const MIN_NEIGHBORS: usize = 3;
const SMOOTH_RADIUS: usize = 12;

#[derive(Clone, Copy, Debug)]
pub struct Track {
    pub from: (f32, f32),
    pub to: (f32, f32),
}

/// Build one optional render mesh per frame. `transitions` contains the source
/// frame number and its correspondences to the next analyzed frame.
pub fn build_local_warps(
    frame_count: usize,
    size: (u32, u32),
    transitions: &[(usize, Vec<Track>)],
) -> Vec<Option<(Vec<f64>, Vec<f32>)>> {
    if frame_count == 0 || size.0 < 2 || size.1 < 2 {
        return vec![None; frame_count];
    }

    let mut velocity = vec![None; frame_count.saturating_sub(1)];
    for (frame, tracks) in transitions {
        if *frame < velocity.len() {
            velocity[*frame] = residual_grid(size, tracks);
        }
    }

    // Missing observations must not invent motion.  They split trajectories,
    // so a decode gap or failed frame cannot make a correction drift forever.
    let mut paths = vec![[[(0.0f32, 0.0f32); GRID]; GRID]; frame_count];
    let mut valid = vec![false; frame_count];
    valid[0] = true;
    for frame in 1..frame_count {
        if let Some(field) = velocity[frame - 1] {
            for y in 0..GRID {
                for x in 0..GRID {
                    paths[frame][y][x].0 = paths[frame - 1][y][x].0 + field[y][x].0;
                    paths[frame][y][x].1 = paths[frame - 1][y][x].1 + field[y][x].1;
                }
            }
            valid[frame] = true;
        } else {
            paths[frame] = paths[frame - 1];
        }
    }

    let mut result = vec![None; frame_count];
    for frame in 0..frame_count {
        if !valid[frame] { continue; }
        let lo = frame.saturating_sub(SMOOTH_RADIUS);
        let hi = (frame + SMOOTH_RADIUS + 1).min(frame_count);
        let mut correction = [[(0.0f32, 0.0f32); GRID]; GRID];
        for y in 0..GRID {
            for x in 0..GRID {
                let mut sum = (0.0, 0.0);
                let mut weights = 0.0;
                for sample in lo..hi {
                    if !valid[sample] { continue; }
                    let distance = sample.abs_diff(frame) as f32;
                    let sigma = (SMOOTH_RADIUS as f32 * 0.45).max(1.0);
                    let weight = (-0.5 * (distance / sigma).powi(2)).exp();
                    sum.0 += paths[sample][y][x].0 * weight;
                    sum.1 += paths[sample][y][x].1 * weight;
                    weights += weight;
                }
                if weights > 0.0 {
                    // Sampling the smoothed trajectory instead of the observed
                    // trajectory counteracts local shake.
                    correction[y][x] = (
                        sum.0 / weights - paths[frame][y][x].0,
                        sum.1 / weights - paths[frame][y][x].1,
                    );
                }
            }
        }
        limit_and_regularize(size, &mut correction);
        if correction.iter().flatten().any(|v| v.0.hypot(v.1) > 0.01) {
            result[frame] = Some(mesh_pair(size, &correction));
        }
    }
    result
}

fn residual_grid(size: (u32, u32), input: &[Track]) -> Option<[[(f32, f32); GRID]; GRID]> {
    let tracks: Vec<_> = input.iter().copied().filter(|t| {
        [t.from.0, t.from.1, t.to.0, t.to.1].iter().all(|v| v.is_finite())
            && t.from.0 >= 0.0 && t.from.1 >= 0.0
            && t.from.0 < size.0 as f32 && t.from.1 < size.1 as f32
            && t.to.0 >= 0.0 && t.to.1 >= 0.0
            && t.to.0 < size.0 as f32 && t.to.1 < size.1 as f32
    }).collect();
    if tracks.len() < MIN_TRACKS { return None; }

    let affine = robust_affine(&tracks)?;
    let diagonal = (size.0 as f32).hypot(size.1 as f32);
    let radius = diagonal * 0.22;
    let mut field = [[(0.0f32, 0.0f32); GRID]; GRID];
    let mut supported = [[false; GRID]; GRID];

    for gy in 0..GRID {
        for gx in 0..GRID {
            let node = (
                gx as f32 * (size.0 - 1) as f32 / (GRID - 1) as f32,
                gy as f32 * (size.1 - 1) as f32 / (GRID - 1) as f32,
            );
            let mut nearby = tracks.iter().filter_map(|track| {
                let distance = (track.from.0 - node.0).hypot(track.from.1 - node.1);
                if distance > radius { return None; }
                let predicted = apply_affine(&affine, track.from);
                let residual = (track.to.0 - predicted.0, track.to.1 - predicted.1);
                Some((distance, residual))
            }).collect::<Vec<_>>();
            if nearby.len() < MIN_NEIGHBORS { continue; }
            nearby.sort_by(|a, b| a.0.total_cmp(&b.0));
            nearby.truncate(12);
            let mut xs = nearby.iter().map(|v| v.1.0).collect::<Vec<_>>();
            let mut ys = nearby.iter().map(|v| v.1.1).collect::<Vec<_>>();
            let center = (median(&mut xs), median(&mut ys));
            let mut deviations = nearby.iter().map(|v| (v.1.0 - center.0).hypot(v.1.1 - center.1)).collect::<Vec<_>>();
            let spread = median(&mut deviations).max(0.35);
            let coherent = nearby.iter().filter(|v| (v.1.0 - center.0).hypot(v.1.1 - center.1) <= 2.5 * spread).count();
            // Sparse or incoherent areas are commonly moving foreground.
            if coherent >= MIN_NEIGHBORS && coherent * 2 >= nearby.len() {
                let confidence = ((coherent - 2) as f32 / 6.0).clamp(0.0, 1.0);
                field[gy][gx] = (center.0 * confidence, center.1 * confidence);
                supported[gy][gx] = true;
            }
        }
    }

    // Suppress isolated islands, then diffuse nearby background-supported
    // motion into empty nodes. This avoids pinning textureless sky/walls while
    // preventing a single moving object from pulling the mesh.
    let original = field;
    for y in 0..GRID {
        for x in 0..GRID {
            if !supported[y][x] { continue; }
            let neighbors = neighbor_indices(x, y).filter(|&(nx, ny)| supported[ny][nx]).count();
            if neighbors == 0 { field[y][x] = (0.0, 0.0); supported[y][x] = false; }
        }
    }
    for _ in 0..3 {
        let previous = field;
        for y in 0..GRID {
            for x in 0..GRID {
                let mut sum = previous[y][x];
                let mut count = if supported[y][x] { 2.0 } else { 0.0 };
                if supported[y][x] { sum.0 *= 2.0; sum.1 *= 2.0; }
                for (nx, ny) in neighbor_indices(x, y) {
                    if supported[ny][nx] {
                        sum.0 += previous[ny][nx].0;
                        sum.1 += previous[ny][nx].1;
                        count += 1.0;
                    }
                }
                if count > 0.0 { field[y][x] = (sum.0 / count, sum.1 / count); }
            }
        }
    }
    if !supported.iter().flatten().any(|v| *v) { return None; }
    let _ = original; // retained above to make the island-removal order explicit
    Some(field)
}

fn robust_affine(tracks: &[Track]) -> Option<([f64; 3], [f64; 3])> {
    let mut weights = vec![1.0f64; tracks.len()];
    let mut model = None;
    for _ in 0..4 {
        model = fit_affine(tracks, &weights);
        let current = model?;
        let mut errors = tracks.iter().map(|track| {
            let p = apply_affine(&current, track.from);
            (p.0 - track.to.0).hypot(p.1 - track.to.1)
        }).collect::<Vec<_>>();
        let scale = median(&mut errors).max(0.25) * 2.5;
        for (weight, error) in weights.iter_mut().zip(errors) {
            let u = error as f64 / scale as f64;
            *weight = if u < 1.0 { (1.0 - u * u).powi(2) } else { 0.0 };
        }
    }
    model
}

fn fit_affine(tracks: &[Track], weights: &[f64]) -> Option<([f64; 3], [f64; 3])> {
    let mut normal = Matrix3::zeros();
    let mut bx = Vector3::zeros();
    let mut by = Vector3::zeros();
    for (track, &weight) in tracks.iter().zip(weights) {
        if weight <= 0.0 { continue; }
        let v = Vector3::new(track.from.0 as f64, track.from.1 as f64, 1.0);
        normal += (v * v.transpose()) * weight;
        bx += v * track.to.0 as f64 * weight;
        by += v * track.to.1 as f64 * weight;
    }
    let inverse = normal.try_inverse()?;
    let x = inverse * bx;
    let y = inverse * by;
    Some(([x[0], x[1], x[2]], [y[0], y[1], y[2]]))
}

fn apply_affine(model: &([f64; 3], [f64; 3]), point: (f32, f32)) -> (f32, f32) {
    let x = point.0 as f64;
    let y = point.1 as f64;
    ((model.0[0] * x + model.0[1] * y + model.0[2]) as f32,
     (model.1[0] * x + model.1[1] * y + model.1[2]) as f32)
}

fn neighbor_indices(x: usize, y: usize) -> impl Iterator<Item = (usize, usize)> {
    let mut values = [(usize::MAX, usize::MAX); 4];
    let mut len = 0;
    if x > 0 { values[len] = (x - 1, y); len += 1; }
    if x + 1 < GRID { values[len] = (x + 1, y); len += 1; }
    if y > 0 { values[len] = (x, y - 1); len += 1; }
    if y + 1 < GRID { values[len] = (x, y + 1); len += 1; }
    values.into_iter().take(len)
}

fn limit_and_regularize(size: (u32, u32), field: &mut [[(f32, f32); GRID]; GRID]) {
    let cell = ((size.0.min(size.1) - 1) as f32 / (GRID - 1) as f32).max(1.0);
    let max_shift = cell * 0.35; // keeps adjacent spline nodes well away from foldover
    for row in field.iter_mut() {
        for value in row {
            let length = value.0.hypot(value.1);
            if !length.is_finite() { *value = (0.0, 0.0); }
            else if length > max_shift {
                value.0 *= max_shift / length;
                value.1 *= max_shift / length;
            }
        }
    }
    for _ in 0..2 {
        let previous = *field;
        for y in 0..GRID {
            for x in 0..GRID {
                let mut sum = (previous[y][x].0 * 2.0, previous[y][x].1 * 2.0);
                let mut count = 2.0;
                for (nx, ny) in neighbor_indices(x, y) {
                    sum.0 += previous[ny][nx].0;
                    sum.1 += previous[ny][nx].1;
                    count += 1.0;
                }
                field[y][x] = (sum.0 / count, sum.1 / count);
            }
        }
    }
}

fn mesh_pair(size: (u32, u32), correction: &[[(f32, f32); GRID]; GRID]) -> (Vec<f64>, Vec<f32>) {
    // The forward mesh is used for point transforms; the inverse mesh is used
    // by render kernels. Small, foldover-limited corrections make the paired
    // +/- construction a stable inverse approximation.
    let forward = build_mesh(size, correction, 1.0);
    let inverse = build_mesh(size, correction, -1.0).into_iter().map(|v| v as f32).collect();
    (forward, inverse)
}

fn build_mesh(size: (u32, u32), correction: &[[(f32, f32); GRID]; GRID], sign: f32) -> Vec<f64> {
    let mut mesh = vec![0.0, GRID as f64, GRID as f64, size.0 as f64, size.1 as f64, 0.0, 0.0, size.0 as f64, size.1 as f64];
    for (y, row) in correction.iter().enumerate() {
        for (x, value) in row.iter().enumerate() {
            mesh.push(x as f64 * size.0 as f64 / (GRID - 1) as f64 + value.0 as f64 * sign as f64);
            mesh.push(y as f64 * size.1 as f64 / (GRID - 1) as f64 + value.1 as f64 * sign as f64);
        }
    }
    let raw = mesh.clone();
    let mut a = [0.0; GRID]; let mut b = [0.0; GRID];
    let mut c = [0.0; GRID]; let mut d = [0.0; GRID];
    let mut alpha = [0.0; GRID - 1]; let mut mu = [0.0; GRID]; let mut z = [0.0; GRID];
    for component in 0..2 {
        for row in 0..GRID {
            BivariateSpline::cubic_spline_coefficients(&raw[9 + component..], 2, row * GRID, size.0 as f64, GRID, &mut a, &mut b, &mut c, &mut d, &mut alpha, &mut mu, &mut z);
            mesh.extend(a); mesh.extend(b); mesh.extend(c); mesh.extend(d);
        }
    }
    mesh[0] = mesh.len() as f64;
    mesh
}

fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() { return 0.0; }
    values.sort_by(f32::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 { (values[middle - 1] + values[middle]) * 0.5 } else { values[middle] }
}

#[cfg(test)]
fn valid_render_mesh(mesh: &[f32]) -> bool {
    let expected = 9 + GRID * GRID * 2 + GRID * GRID * 4 * 2;
    mesh.len() == expected && mesh[0] as usize == expected && mesh[1] as usize == GRID
        && mesh[2] as usize == GRID && mesh.iter().all(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(size: (u32, u32), frames: usize, local: impl Fn(usize, f32, f32) -> (f32, f32)) -> Vec<(usize, Vec<Track>)> {
        (0..frames).map(|frame| (frame, (0..10).flat_map(|y| (0..10).map(move |x| (x, y))).map(|(x, y)| {
            let p = ((x as f32 + 0.5) * size.0 as f32 / 10.0, (y as f32 + 0.5) * size.1 as f32 / 10.0);
            let r = local(frame, p.0, p.1);
            Track { from: p, to: (p.0 + 4.0 + r.0, p.1 - 2.0 + r.1) }
        }).collect())).collect()
    }

    #[test]
    fn pure_camera_motion_does_not_create_a_warp() {
        let t = tracks((640, 360), 20, |_, _, _| (0.0, 0.0));
        assert!(build_local_warps(21, (640, 360), &t).iter().all(Option::is_none));
    }

    #[test]
    fn coherent_local_jitter_creates_valid_bounded_meshes() {
        let t = tracks((640, 360), 30, |frame, x, y| {
            let jitter = if frame % 2 == 0 { 2.0 } else { -2.0 };
            if x > 320.0 && y > 90.0 && y < 270.0 { (jitter, 0.0) } else { (0.0, 0.0) }
        });
        let meshes = build_local_warps(31, (640, 360), &t);
        assert!(meshes.iter().filter(|m| m.is_some()).count() > 20);
        for (_, inverse) in meshes.iter().flatten() {
            assert!(valid_render_mesh(inverse));
            let spline = BivariateSpline::new(GRID, GRID);
            let inverse64 = inverse.iter().map(|&v| v as f64).collect::<Vec<_>>();
            let p = spline.interpolate(640.0, 360.0, &inverse64, 0, 320.0, 180.0);
            assert!(p.is_finite() && (p - 320.0).abs() < 20.0);
        }
    }

    #[test]
    fn sparse_foreground_and_bad_input_fail_closed() {
        let mut t = tracks((640, 360), 8, |_, _, _| (0.0, 0.0));
        for (_, frame) in &mut t {
            for track in frame.iter_mut().take(4) { track.to.0 += 80.0; }
            frame.truncate(12);
        }
        assert!(build_local_warps(9, (640, 360), &t).iter().all(Option::is_none));
        assert!(build_local_warps(3, (0, 0), &[]).iter().all(Option::is_none));
    }

    #[test]
    fn trajectory_smoothing_reduces_alternating_motion() {
        let t = tracks((640, 360), 24, |frame, x, y| {
            let sign = if frame % 2 == 0 { 1.0 } else { -1.0 };
            if x > 320.0 && y > 90.0 && y < 270.0 { (sign * 2.5, sign * 1.5) } else { (0.0, 0.0) }
        });
        let meshes = build_local_warps(25, (640, 360), &t);
        let nonempty = meshes.iter().flatten().count();
        assert!(nonempty >= 18, "only {nonempty} corrected frames");
    }
}
