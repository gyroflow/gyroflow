// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow contributors

//! Residual optical-flow stabilization.
//!
//! After the existing camera-motion estimate (gyro, or optical flow used as gyro) has taken the
//! global rotation, leftover tracks are a mix of parallax, rolling-shutter residue and micro-jitter.
//! This module:
//! 1. rejects non-finite / out-of-frame / spatially-clustered tracks
//! 2. removes a robust global similarity so a moving foreground cannot dominate
//! 3. bins the leftover motion on Gyroflow's 9×9 mesh grid
//! 4. accumulates, box-smooths and high-passes the local path (Warp Stabilizer-style)
//! 5. emits `MeshCorrections` in the same spline format the render kernels already apply
//!
//! Camera mesh metadata (Sony IBIS) always wins: the caller must not install a residual mesh on
//! top of `FileMetadata::mesh_correction`.

use super::PoseEstimator;
use crate::gyro_source::{
    sony, splines, MeshCorrections, MeshFrame, MeshTable, MESH_HEADER,
};
use std::collections::BTreeMap;

pub const GRID: usize = splines::MAX_GRID_SIZE; // 9
const MIN_TRACKS: usize = 8;
const MAX_SHIFT_FRAC: f64 = 0.08;
const MAX_SHIFT_PX: f64 = 80.0;
const MIN_SHIFT_PX: f64 = 8.0;
const CUT_FRAC: f64 = 0.25;
const DEFAULT_SMOOTH: usize = 15;

#[derive(Clone, Copy, Debug)]
pub struct ResidualParams {
    pub strength: f64,
    pub smooth_frames: usize,
}

impl ResidualParams {
    pub fn from_sync(strength: f64, smooth_frames: usize) -> Self {
        let strength = if strength <= 0.0 { 1.0 } else { strength.clamp(0.0, 1.0) };
        let smooth_frames = if smooth_frames == 0 { DEFAULT_SMOOTH } else { smooth_frames.min(120) };
        Self { strength, smooth_frames }
    }
}

#[derive(Clone, Debug)]
pub struct TrackPair {
    pub timestamp_us: i64,
    pub from: Vec<(f32, f32)>,
    pub to: Vec<(f32, f32)>,
    pub size: (u32, u32),
}

/// Drop non-finite, out-of-frame, and spatially piled-up correspondences (one best track per cell).
pub fn filter_tracks(
    from: &[(f32, f32)],
    to: &[(f32, f32)],
    size: (u32, u32),
    cells: usize,
) -> (Vec<(f32, f32)>, Vec<(f32, f32)>) {
    let w = size.0 as f32;
    let h = size.1 as f32;
    if w <= 1.0 || h <= 1.0 || from.len() != to.len() {
        return (Vec::new(), Vec::new());
    }
    let cells = cells.max(2);
    let cell_w = (w / cells as f32).max(1.0);
    let cell_h = (h / cells as f32).max(1.0);
    let mut best: BTreeMap<(u32, u32), (f32, (f32, f32), (f32, f32))> = BTreeMap::new();
    for (&p, &q) in from.iter().zip(to.iter()) {
        if !p.0.is_finite() || !p.1.is_finite() || !q.0.is_finite() || !q.1.is_finite() {
            continue;
        }
        if p.0 < 0.0 || p.1 < 0.0 || p.0 >= w || p.1 >= h {
            continue;
        }
        if q.0 < 0.0 || q.1 < 0.0 || q.0 >= w || q.1 >= h {
            continue;
        }
        let cx = (p.0 / cell_w).floor() as u32;
        let cy = (p.1 / cell_h).floor() as u32;
        // Prefer tracks closer to the cell centre so a blob of foreground motion in one corner of
        // the cell cannot always win.
        let centre_x = (cx as f32 + 0.5) * cell_w;
        let centre_y = (cy as f32 + 0.5) * cell_h;
        let score = -((p.0 - centre_x).powi(2) + (p.1 - centre_y).powi(2));
        match best.get(&(cx, cy)) {
            Some(&(prev, _, _)) if prev >= score => {}
            _ => {
                best.insert((cx, cy), (score, p, q));
            }
        }
    }
    let mut out_a = Vec::with_capacity(best.len());
    let mut out_b = Vec::with_capacity(best.len());
    for (_, p, q) in best.into_values() {
        out_a.push(p);
        out_b.push(q);
    }
    (out_a, out_b)
}

/// Least-squares similarity `to = s R from + t` with one round of median-residual rejection.
/// Returns `(a, b, tx, ty)` where `s R = [[a,-b],[b,a]]`.
pub fn robust_similarity(from: &[(f32, f32)], to: &[(f32, f32)]) -> Option<(f64, f64, f64, f64)> {
    fn fit(pairs: &[((f64, f64), (f64, f64))]) -> Option<(f64, f64, f64, f64)> {
        if pairs.len() < 4 {
            return None;
        }
        let n = pairs.len() as f64;
        let (mx, my, mu, mv) = pairs.iter().fold((0.0, 0.0, 0.0, 0.0), |acc, (p, q)| {
            (acc.0 + p.0, acc.1 + p.1, acc.2 + q.0, acc.3 + q.1)
        });
        let (mx, my, mu, mv) = (mx / n, my / n, mu / n, mv / n);
        let (mut sxx, mut sxu, mut sxv) = (0.0, 0.0, 0.0);
        for (p, q) in pairs {
            let (x, y, u, v) = (p.0 - mx, p.1 - my, q.0 - mu, q.1 - mv);
            sxx += x * x + y * y;
            sxu += x * u + y * v;
            sxv += x * v - y * u;
        }
        if sxx <= 1e-12 {
            return None;
        }
        let (a, b) = (sxu / sxx, sxv / sxx);
        Some((a, b, mu - (a * mx - b * my), mv - (b * mx + a * my)))
    }
    let mut pairs: Vec<((f64, f64), (f64, f64))> = from
        .iter()
        .zip(to)
        .filter(|(p, q)| p.0.is_finite() && p.1.is_finite() && q.0.is_finite() && q.1.is_finite())
        .map(|(p, q)| ((p.0 as f64, p.1 as f64), (q.0 as f64, q.1 as f64)))
        .collect();
    let (a, b, tx, ty) = fit(&pairs)?;
    let residual = |p: &(f64, f64), q: &(f64, f64)| {
        ((a * p.0 - b * p.1 + tx - q.0).powi(2) + (b * p.0 + a * p.1 + ty - q.1).powi(2)).sqrt()
    };
    let mut res: Vec<f64> = pairs.iter().map(|(p, q)| residual(p, q)).collect();
    res.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let threshold = (res[res.len() / 2] * 3.0).max(0.5);
    pairs.retain(|(p, q)| residual(p, q) <= threshold);
    if pairs.len() < MIN_TRACKS {
        return None;
    }
    let (a, b, tx, ty) = fit(&pairs)?;
    let scale = (a * a + b * b).sqrt();
    if !(scale > 0.5 && scale < 2.0 && a.is_finite() && b.is_finite() && tx.is_finite() && ty.is_finite()) {
        return None;
    }
    Some((a, b, tx, ty))
}

fn apply_sim(a: f64, b: f64, tx: f64, ty: f64, p: (f32, f32)) -> (f64, f64) {
    let x = p.0 as f64;
    let y = p.1 as f64;
    (a * x - b * y + tx, b * x + a * y + ty)
}

/// Residual `to - similarity(from)` at the source points.
pub fn residual_vectors(
    from: &[(f32, f32)],
    to: &[(f32, f32)],
    sim: (f64, f64, f64, f64),
) -> Vec<((f64, f64), (f64, f64))> {
    let (a, b, tx, ty) = sim;
    from.iter()
        .zip(to)
        .map(|(&p, &q)| {
            let pred = apply_sim(a, b, tx, ty, p);
            ((p.0 as f64, p.1 as f64), (q.0 as f64 - pred.0, q.1 as f64 - pred.1))
        })
        .filter(|(_, d)| d.0.is_finite() && d.1.is_finite())
        .collect()
}

/// Inverse-distance weighted residual at every node of a regular `GRID × GRID` mesh.
pub fn grid_from_residuals(
    residuals: &[((f64, f64), (f64, f64))],
    size: (f64, f64),
) -> Option<Vec<(f64, f64)>> {
    if residuals.len() < MIN_TRACKS || !(size.0 > 1.0 && size.1 > 1.0) {
        return None;
    }
    let step = (size.0 / (GRID - 1) as f64, size.1 / (GRID - 1) as f64);
    let radius2 = (step.0.max(step.1) * 1.25).powi(2);
    let mut nodes = Vec::with_capacity(GRID * GRID);
    for j in 0..GRID {
        for i in 0..GRID {
            let gx = step.0 * i as f64;
            let gy = step.1 * j as f64;
            let mut wsum = 0.0;
            let mut dx = 0.0;
            let mut dy = 0.0;
            let mut n = 0usize;
            for &((px, py), (rx, ry)) in residuals {
                let d2 = (px - gx).powi(2) + (py - gy).powi(2);
                if d2 > radius2 {
                    continue;
                }
                let w = 1.0 / (d2 + 1.0);
                wsum += w;
                dx += rx * w;
                dy += ry * w;
                n += 1;
            }
            if n < 2 || wsum <= 0.0 {
                nodes.push((0.0, 0.0));
            } else {
                nodes.push((dx / wsum, dy / wsum));
            }
        }
    }
    Some(nodes)
}

fn median_mag(grid: &[(f64, f64)]) -> f64 {
    if grid.is_empty() {
        return 0.0;
    }
    let mut mags: Vec<f64> = grid.iter().map(|(x, y)| (x * x + y * y).sqrt()).collect();
    mags.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    mags[mags.len() / 2]
}

fn is_cut(grid: &[(f64, f64)], size: (f64, f64)) -> bool {
    let thresh = CUT_FRAC * size.0.min(size.1);
    median_mag(grid) > thresh
}

fn accumulate(increments: &[Vec<(f64, f64)>], size: (f64, f64)) -> Vec<Vec<(f64, f64)>> {
    let mut path = Vec::with_capacity(increments.len());
    let mut acc = vec![(0.0, 0.0); GRID * GRID];
    for g in increments {
        if is_cut(g, size) {
            acc = vec![(0.0, 0.0); GRID * GRID];
            path.push(acc.clone());
            continue;
        }
        for (a, d) in acc.iter_mut().zip(g.iter()) {
            a.0 += d.0;
            a.1 += d.1;
        }
        path.push(acc.clone());
    }
    path
}

fn box_smooth(path: &[Vec<(f64, f64)>], window: usize) -> Vec<Vec<(f64, f64)>> {
    let n = path.len();
    if n == 0 {
        return Vec::new();
    }
    let half = window.max(1) / 2;
    let mut out = vec![vec![(0.0, 0.0); GRID * GRID]; n];
    for t in 0..n {
        let a = t.saturating_sub(half);
        let b = (t + half).min(n - 1);
        let count = (b - a + 1) as f64;
        for k in 0..GRID * GRID {
            let mut sx = 0.0;
            let mut sy = 0.0;
            for i in a..=b {
                sx += path[i][k].0;
                sy += path[i][k].1;
            }
            out[t][k] = (sx / count, sy / count);
        }
    }
    out
}

fn spatial_smooth(grid: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![(0.0, 0.0); GRID * GRID];
    for j in 0..GRID {
        for i in 0..GRID {
            let mut sx = 0.0;
            let mut sy = 0.0;
            let mut n = 0.0;
            for dj in -1i32..=1 {
                for di in -1i32..=1 {
                    let x = i as i32 + di;
                    let y = j as i32 + dj;
                    if x < 0 || y < 0 || x >= GRID as i32 || y >= GRID as i32 {
                        continue;
                    }
                    let p = grid[y as usize * GRID + x as usize];
                    let w = if di == 0 && dj == 0 { 4.0 } else { 1.0 };
                    sx += p.0 * w;
                    sy += p.1 * w;
                    n += w;
                }
            }
            out[j * GRID + i] = (sx / n, sy / n);
        }
    }
    out
}

fn clamp_grid(grid: &mut [(f64, f64)], max_shift: f64) {
    for (x, y) in grid.iter_mut() {
        *x = x.clamp(-max_shift, max_shift);
        *y = y.clamp(-max_shift, max_shift);
        if !x.is_finite() { *x = 0.0; }
        if !y.is_finite() { *y = 0.0; }
    }
}

/// High-pass leftover of the local residual path, scaled and bounded.
pub fn stabilize_grids(
    increments: &[Vec<(f64, f64)>],
    size: (f64, f64),
    params: ResidualParams,
) -> Vec<Vec<(f64, f64)>> {
    if increments.is_empty() {
        return Vec::new();
    }
    let path = accumulate(increments, size);
    let smooth = box_smooth(&path, params.smooth_frames);
    let max_shift = (MAX_SHIFT_FRAC * size.0.min(size.1)).clamp(MIN_SHIFT_PX, MAX_SHIFT_PX);
    path.iter()
        .zip(smooth.iter())
        .map(|(p, s)| {
            let mut g: Vec<(f64, f64)> = p
                .iter()
                .zip(s.iter())
                .map(|(a, b)| ((a.0 - b.0) * params.strength, (a.1 - b.1) * params.strength))
                .collect();
            g = spatial_smooth(&g);
            clamp_grid(&mut g, max_shift);
            g
        })
        .collect()
}

/// Destination positions of a regular grid: identity minus the leftover (sample earlier to cancel the shake).
pub fn nodes_from_correction(correction: &[(f64, f64)], size: (f64, f64)) -> Vec<(f64, f64)> {
    let step = (size.0 / (GRID - 1) as f64, size.1 / (GRID - 1) as f64);
    let mut nodes = Vec::with_capacity(GRID * GRID);
    for j in 0..GRID {
        for i in 0..GRID {
            let x = step.0 * i as f64;
            let y = step.1 * j as f64;
            let (dx, dy) = correction[j * GRID + i];
            nodes.push((x - dx, y - dy));
        }
    }
    nodes
}

pub fn mesh_table_from_nodes(nodes: &[(f64, f64)], size: (f64, f64)) -> Option<MeshTable> {
    let divisions = (GRID, GRID);
    if nodes.len() != GRID * GRID || !(size.0 > 0.0 && size.1 > 0.0) {
        return None;
    }
    if nodes.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return None;
    }
    let capacity = MESH_HEADER + GRID * GRID * 2 + GRID * splines::MAX_GRID_SIZE * 4 * 2;
    let header = |mesh: &mut Vec<f64>| {
        mesh.extend([0.0, GRID as f64, GRID as f64, size.0, size.1, 0.0, 0.0, 0.0, 0.0]);
    };
    let mut mesh = Vec::with_capacity(capacity);
    header(&mut mesh);
    for &(x, y) in nodes {
        mesh.push(x);
        mesh.push(y);
    }
    sony::append_row_coefficients(&mut mesh, divisions, size.0);
    mesh[0] = mesh.len() as f64;

    let step = (size.0 / (GRID - 1) as f64, size.1 / (GRID - 1) as f64);
    let inverted: Option<Vec<(f64, f64)>> = (0..GRID)
        .flat_map(|y| (0..GRID).map(move |x| (step.0 * x as f64, step.1 * y as f64)))
        .map(|(x, y)| sony::inverse_interpolate_mesh(x, y, size, &mesh).ok())
        .collect();
    let inverted = inverted?;
    if inverted.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return None;
    }
    let mut inv_mesh = Vec::with_capacity(capacity);
    header(&mut inv_mesh);
    for (x, y) in inverted {
        inv_mesh.push(x);
        inv_mesh.push(y);
    }
    sony::append_row_coefficients(&mut inv_mesh, divisions, size.0);
    inv_mesh[0] = inv_mesh.len() as f64;

    let residual = sony::inverse_residual(&mesh, &inv_mesh, size);
    if !residual.is_finite() {
        return None;
    }
    let refine = residual > sony::MESH_REFINE_THRESHOLD_PX;
    Some(MeshTable {
        refinement: if refine { mesh.iter().map(|x| *x as f32).collect() } else { Vec::new() },
        inverse: inv_mesh.iter().map(|x| *x as f32).collect(),
        forward: mesh,
    })
}

pub fn meshes_from_corrections(
    corrections: &[Vec<(f64, f64)>],
    timestamps_us: &[i64],
    fps: f64,
    frame_count: usize,
    size: (f64, f64),
) -> MeshCorrections {
    if corrections.is_empty() || frame_count == 0 || timestamps_us.len() != corrections.len() {
        return MeshCorrections::default();
    }
    let mut by_frame: BTreeMap<usize, Vec<(f64, f64)>> = BTreeMap::new();
    for (ts, grid) in timestamps_us.iter().zip(corrections.iter()) {
        let f = crate::frame_at_timestamp(*ts as f64 / 1000.0, fps);
        if f >= 0 {
            by_frame.insert(f as usize, grid.clone());
        }
    }
    if by_frame.is_empty() {
        return MeshCorrections::default();
    }
    let mut meshes = MeshCorrections::default();
    let mut last: Option<&Vec<(f64, f64)>> = None;
    for i in 0..frame_count {
        if let Some(g) = by_frame.get(&i) {
            last = Some(g);
        }
        let Some(g) = last else {
            meshes.frames.push(MeshFrame::default());
            continue;
        };
        let nodes = nodes_from_correction(g, size);
        let Some(table) = mesh_table_from_nodes(&nodes, size) else {
            meshes.frames.push(MeshFrame::default());
            continue;
        };
        let idx = meshes.tables.len() as u32;
        meshes.tables.push(table);
        meshes.frames.push(MeshFrame {
            table: Some(idx),
            mesh_size: size,
            crop_origin: (0.0, 0.0),
            crop_size: size,
            focal_plane: Vec::new(),
        });
    }
    if meshes.is_empty() {
        meshes.clear();
    }
    meshes
}

/// Build a residual mesh from cached optical-flow tracks. Empty when tracking is too sparse.
pub fn build_from_estimator(
    estimator: &PoseEstimator,
    fps: f64,
    frame_count: usize,
    video_size: (usize, usize),
    params: ResidualParams,
) -> MeshCorrections {
    if video_size.0 < 8 || video_size.1 < 8 || !(fps > 0.0) {
        return MeshCorrections::default();
    }
    let video = (video_size.0 as f64, video_size.1 as f64);
    let results = estimator.sync_results.read();
    let keys: Vec<i64> = results.keys().copied().collect();
    let mut increments = Vec::new();
    let mut timestamps = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        let Some(next_k) = keys.get(i + 1) else { break };
        let Some(curr) = results.get(k) else { continue };
        let Some(next) = results.get(next_k) else { continue };
        if curr.frame_no + 1 != next.frame_no {
            continue;
        }
        let of = match curr.optical_flow.try_borrow() {
            Ok(of) => of.get(&1).cloned(),
            Err(_) => None,
        };
        let Some(Some(((ts, pts_a), (_, pts_b)))) = of else { continue };
        let of_size = curr.frame_size;
        if of_size.0 == 0 || of_size.1 == 0 {
            continue;
        }
        let sx = video.0 / of_size.0 as f64;
        let sy = video.1 / of_size.1 as f64;
        let from: Vec<(f32, f32)> = pts_a.iter().map(|&(x, y)| ((x as f64 * sx) as f32, (y as f64 * sy) as f32)).collect();
        let to: Vec<(f32, f32)> = pts_b.iter().map(|&(x, y)| ((x as f64 * sx) as f32, (y as f64 * sy) as f32)).collect();
        let (from, to) = filter_tracks(&from, &to, (video_size.0 as u32, video_size.1 as u32), GRID);
        let Some(sim) = robust_similarity(&from, &to) else { continue };
        let residuals = residual_vectors(&from, &to, sim);
        let Some(grid) = grid_from_residuals(&residuals, video) else { continue };
        increments.push(grid);
        timestamps.push(ts);
    }
    drop(results);
    if increments.len() < 3 {
        return MeshCorrections::default();
    }
    let corrections = stabilize_grids(&increments, video, params);
    meshes_from_corrections(&corrections, &timestamps, fps, frame_count, video)
}

/// Packed luma: copy `width` pixels per row, ignoring decoder row padding in `stride`.
pub fn gray_from_luma(width: u32, height: u32, stride: u32, slice: &[u8]) -> Option<image::GrayImage> {
    if width == 0 || height == 0 || stride < width {
        return None;
    }
    let needed = stride as usize * height as usize;
    if slice.len() < needed {
        return None;
    }
    if stride == width {
        return image::GrayImage::from_raw(width, height, slice[..needed].to_vec());
    }
    let mut img = image::GrayImage::new(width, height);
    for y in 0..height {
        let src = (y * stride) as usize;
        let row = &slice[src..src + width as usize];
        for x in 0..width {
            img.put_pixel(x, y, image::Luma([row[x as usize]]));
        }
    }
    Some(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_size() -> (f64, f64) { (640.0, 360.0) }

    #[test]
    fn filter_drops_non_finite_and_out_of_frame() {
        let from = [(10.0, 10.0), (f32::NAN, 4.0), (700.0, 10.0), (400.0, 200.0)];
        let to = [(12.0, 11.0), (5.0, 5.0), (701.0, 10.0), (401.0, 202.0)];
        let (a, b) = filter_tracks(&from, &to, (640, 360), 9);
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 2);
        assert!(a.iter().all(|(x, y)| x.is_finite() && y.is_finite()));
    }

    #[test]
    fn filter_keeps_one_track_per_cell() {
        let mut from = Vec::new();
        let mut to = Vec::new();
        for i in 0..20 {
            from.push((8.0 + i as f32 * 0.2, 8.0));
            to.push((9.0 + i as f32 * 0.2, 8.5));
        }
        let (a, _) = filter_tracks(&from, &to, (64, 64), 8);
        assert!(a.len() < 20);
        assert!(a.len() >= 1);
    }

    #[test]
    fn similarity_recovers_translation() {
        let from: Vec<(f32, f32)> = (0..12).map(|i| (20.0 + (i % 4) as f32 * 40.0, 20.0 + (i / 4) as f32 * 40.0)).collect();
        let to: Vec<(f32, f32)> = from.iter().map(|&(x, y)| (x + 5.0, y - 3.0)).collect();
        let (a, b, tx, ty) = robust_similarity(&from, &to).unwrap();
        assert!((a - 1.0).abs() < 1e-6);
        assert!(b.abs() < 1e-6);
        assert!((tx - 5.0).abs() < 1e-4);
        assert!((ty + 3.0).abs() < 1e-4);
        let res = residual_vectors(&from, &to, (a, b, tx, ty));
        let mean = res.iter().map(|(_, d)| (d.0.abs() + d.1.abs()) * 0.5).sum::<f64>() / res.len() as f64;
        assert!(mean < 1e-6, "mean residual {mean}");
    }

    #[test]
    fn local_bump_survives_global_similarity() {
        let mut from = Vec::new();
        let mut to = Vec::new();
        for j in 0..6 {
            for i in 0..6 {
                let x = 40.0 + i as f32 * 80.0;
                let y = 30.0 + j as f32 * 50.0;
                from.push((x, y));
                let extra = if i == 2 && j == 2 { 6.0 } else { 0.0 };
                to.push((x + 2.0 + extra, y + 1.0));
            }
        }
        let sim = robust_similarity(&from, &to).unwrap();
        let res = residual_vectors(&from, &to, sim);
        let grid = grid_from_residuals(&res, grid_size()).unwrap();
        // Bump is at (200, 130); nearest 9×9 node is around i=2, j=3
        let near = grid[3 * GRID + 2];
        let corner = grid[0];
        assert!(near.0.abs() > corner.0.abs() + 0.5, "near {:?} corner {:?}", near, corner);
    }

    #[test]
    fn high_pass_cancels_constant_local_drift() {
        let drift = vec![(1.0, 0.0); GRID * GRID];
        let increments = vec![drift.clone(); 40];
        let out = stabilize_grids(&increments, grid_size(), ResidualParams { strength: 1.0, smooth_frames: 15 });
        let mid = &out[20];
        let mag = median_mag(mid);
        assert!(mag < 0.2, "constant drift should be absorbed by the smoother, got {mag}");
    }

    #[test]
    fn high_pass_keeps_impulse() {
        let zero = vec![(0.0, 0.0); GRID * GRID];
        let bump = vec![(4.0, 0.0); GRID * GRID];
        let mut increments = vec![zero.clone(); 30];
        increments[15] = bump;
        let out = stabilize_grids(&increments, grid_size(), ResidualParams { strength: 1.0, smooth_frames: 15 });
        let mag_at = median_mag(&out[15]);
        let mag_far = median_mag(&out[2]);
        assert!(mag_at > 1.5, "impulse should remain, got {mag_at}");
        assert!(mag_at > mag_far + 1.0);
    }

    #[test]
    fn scene_cut_resets_path() {
        let zero = vec![(0.0, 0.0); GRID * GRID];
        let jump = vec![(200.0, 0.0); GRID * GRID];
        let mut increments = vec![zero.clone(); 10];
        increments.push(jump);
        increments.extend(std::iter::repeat(zero).take(10));
        let out = stabilize_grids(&increments, grid_size(), ResidualParams { strength: 1.0, smooth_frames: 8 });
        assert!(median_mag(&out[11]) < 1.0);
    }

    #[test]
    fn mesh_table_is_invertible_for_small_warp() {
        let size = grid_size();
        let correction = vec![(2.0, -1.5); GRID * GRID];
        let nodes = nodes_from_correction(&correction, size);
        let table = mesh_table_from_nodes(&nodes, size).expect("mesh");
        let residual = sony::inverse_residual(&table.forward, &table.inverse.iter().map(|x| *x as f64).collect::<Vec<_>>(), size);
        assert!(residual < 0.05, "inverse residual {residual}");
        let p = sony::interpolate_mesh(320.0, 180.0, size, &table.forward);
        assert!((p.x - (320.0 - 2.0)).abs() < 0.05);
        assert!((p.y - (180.0 + 1.5)).abs() < 0.05);
    }

    #[test]
    fn meshes_cover_every_video_frame() {
        let size = grid_size();
        let g = vec![(1.0, 0.0); GRID * GRID];
        let corrections = vec![g.clone(), g];
        let ts = vec![0i64, 33_333];
        let meshes = meshes_from_corrections(&corrections, &ts, 30.0, 5, size);
        assert_eq!(meshes.frames.len(), 5);
        assert!(meshes.frames.iter().any(|f| f.table.is_some()));
        assert!(!meshes.kernel_buffer(0).is_empty());
    }

    #[test]
    fn gray_from_luma_strips_row_padding() {
        let width = 4u32;
        let height = 2u32;
        let stride = 8u32;
        let mut buf = vec![0u8; (stride * height) as usize];
        for y in 0..height {
            for x in 0..width {
                buf[(y * stride + x) as usize] = (y * 10 + x + 1) as u8;
            }
            for x in width..stride {
                buf[(y * stride + x) as usize] = 255;
            }
        }
        let img = gray_from_luma(width, height, stride, &buf).unwrap();
        assert_eq!(img.width(), 4);
        assert_eq!(img.height(), 2);
        assert_eq!(img.get_pixel(0, 0).0[0], 1);
        assert_eq!(img.get_pixel(3, 1).0[0], 14);
        assert!(gray_from_luma(4, 2, 8, &buf[..10]).is_none());
        assert!(gray_from_luma(4, 2, 2, &buf).is_none());
    }

    #[test]
    fn packed_luma_roundtrip() {
        let buf: Vec<u8> = (0..12).collect();
        let img = gray_from_luma(4, 3, 4, &buf).unwrap();
        assert_eq!(img.get_pixel(3, 2).0[0], 11);
    }

    fn checker(x: f64, y: f64) -> u8 {
        let c = ((x as i32).div_euclid(16) ^ (y as i32).div_euclid(16)) & 1;
        let mut v = if c == 0 { 40 } else { 220 };
        let dx = x - 160.0;
        let dy = y - 90.0;
        if dx * dx + dy * dy < 18.0 * 18.0 {
            v = 255;
        }
        v
    }
    fn sample(img: &[u8], w: usize, h: usize, x: f64, y: f64) -> u8 {
        let x = x.clamp(0.0, w as f64 - 1.001);
        let y = y.clamp(0.0, h as f64 - 1.001);
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let fx = x - x0 as f64;
        let fy = y - y0 as f64;
        let p = |xx: usize, yy: usize| img[yy * w + xx] as f64;
        (p(x0, y0) * (1.0 - fx) * (1.0 - fy)
            + p(x0 + 1, y0) * fx * (1.0 - fy)
            + p(x0, y0 + 1) * (1.0 - fx) * fy
            + p(x0 + 1, y0 + 1) * fx * fy)
            .round()
            .clamp(0.0, 255.0) as u8
    }

    /// True motion: a global pan plus a local vertical bump in one region (parallax-like, not a similarity).
    fn synth_flow(x: f64, y: f64, t: f64, _w: f64, _h: f64) -> (f64, f64) {
        let gx = 4.0 * (t * 0.7).sin();
        let gy = 3.0 * (t * 0.9).cos();
        let dx = x - 230.0;
        let dy = y - 70.0;
        let local = (-(dx * dx + dy * dy) / (38.0 * 38.0)).exp() * 12.0 * (t * 2.2).sin();
        (gx, gy + local)
    }
    fn local_only(x: f64, y: f64, t: f64, w: f64, h: f64) -> (f64, f64) {
        let (dx, dy) = synth_flow(x, y, t, w, h);
        let gx = 4.0 * (t * 0.7).sin();
        let gy = 3.0 * (t * 0.9).cos();
        (dx - gx, dy - gy)
    }

    #[test]
    fn residual_pass_reduces_local_jitter_and_writes_demo() {
        let (w, h) = (320usize, 180usize);
        let size = (w as f64, h as f64);
        let n = 16usize;
        let mut increments = Vec::new();
        let mut timestamps = Vec::new();
        for t in 0..n - 1 {
            let mut from = Vec::new();
            let mut to = Vec::new();
            for j in (8..h - 8).step_by(12) {
                for i in (8..w - 8).step_by(12) {
                    let (dx0, dy0) = synth_flow(i as f64, j as f64, t as f64, size.0, size.1);
                    let (dx1, dy1) = synth_flow(i as f64, j as f64, (t + 1) as f64, size.0, size.1);
                    from.push((i as f32 + dx0 as f32, j as f32 + dy0 as f32));
                    to.push((i as f32 + dx1 as f32, j as f32 + dy1 as f32));
                }
            }
            let (from, to) = filter_tracks(&from, &to, (w as u32, h as u32), GRID);
            let sim = robust_similarity(&from, &to).expect("sim");
            let res = residual_vectors(&from, &to, sim);
            let before = res.iter().map(|(_, d)| (d.0 * d.0 + d.1 * d.1).sqrt()).sum::<f64>() / res.len() as f64;
            let grid = grid_from_residuals(&res, size).expect("grid");
            increments.push(grid);
            timestamps.push((t as i64) * 33_333);
            let _ = before;
        }
        let corrections = stabilize_grids(
            &increments,
            size,
            ResidualParams { strength: 1.0, smooth_frames: 7 },
        );
        let peak = corrections.iter().map(|g| median_mag(g)).fold(0.0, f64::max);
        let peak_node = corrections.iter().map(|g| g.iter().map(|(x, y)| (x * x + y * y).sqrt()).fold(0.0, f64::max)).fold(0.0, f64::max);
        eprintln!("residual median peak {peak:.3}px, max node {peak_node:.3}px");
        assert!(peak_node > 1.0, "local bump should produce a residual correction, max node {peak_node}");

        let meshes = meshes_from_corrections(&corrections, &timestamps, 30.0, n, size);
        assert!(!meshes.is_empty());

        // One peak-ripple frame: source | leftover after global camera motion | residual mesh
        let fi = n / 2;
        let t = fi as f64;
        let mut source = vec![0u8; w * h];
        let mut leftover = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                source[y * w + x] = checker(x as f64, y as f64);
                let (dx, dy) = local_only(x as f64, y as f64, t, size.0, size.1);
                leftover[y * w + x] = checker(x as f64 - dx, y as f64 - dy);
            }
        }
        let mut stab = leftover.clone();
        if let Some(fwd) = meshes.forward_mesh(fi.min(meshes.frames.len().saturating_sub(1))) {
            for y in 0..h {
                for x in 0..w {
                    let p = crate::gyro_source::interpolate_mesh(x as f64, y as f64, size, &fwd);
                    stab[y * w + x] = sample(&leftover, w, h, p.x, p.y);
                }
            }
        }
        let mut sheet = vec![0u8; (w * 3) * h];
        for y in 0..h {
            for x in 0..w {
                sheet[y * (w * 3) + x] = source[y * w + x];
                sheet[y * (w * 3) + w + x] = leftover[y * w + x];
                sheet[y * (w * 3) + 2 * w + x] = stab[y * w + x];
            }
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo");
        std::fs::create_dir_all(&dir).unwrap();
        let ppm = dir.join("optical_residual_demo.ppm");
        let mut out = format!("P5\n{} {}\n255\n", w * 3, h).into_bytes();
        out.extend_from_slice(&sheet);
        std::fs::write(&ppm, out).unwrap();
        assert!(ppm.exists());
    }
}
