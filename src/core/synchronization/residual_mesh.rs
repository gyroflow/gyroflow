// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 Gyroflow contributors

use crate::gyro_source::splines;

const GRID: usize = splines::MAX_GRID_SIZE;
const MIN_TRACKS: usize = 36;
const MIN_CELL_TRACKS: usize = 3;
const MIN_VALID_FIELDS: usize = 3;
const MAX_DISPLACEMENT_RATIO: f32 = 0.035;
const EMA_ALPHA: f32 = 0.35;

#[derive(Clone, Copy, Debug, Default)]
pub struct Correspondence {
    pub start: (f32, f32),
    pub end: (f32, f32),
    pub confidence: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct CellEstimate {
    displacement: (f32, f32),
    confidence: f32,
    observations: usize,
    direct_support: bool,
    dispersion: f32,
    neighbor_disagreement: f32,
}

pub fn build_residual_meshes(
    frame_count: usize,
    frame_size: (u32, u32),
    pairs: &[(usize, Vec<Correspondence>)],
) -> Vec<Option<(Vec<f64>, Vec<f32>)>> {
    if frame_count == 0 || frame_size.0 < 2 || frame_size.1 < 2 {
        return Vec::new();
    }

    let mut raw = vec![None; frame_count];
    let mut valid_fields = 0usize;
    for (next_frame, correspondences) in pairs {
        if *next_frame >= frame_count {
            continue;
        }
        if let Some(field) = residual_field(frame_size, correspondences) {
            raw[*next_frame] = Some(field);
            valid_fields += 1;
        }
    }
    if valid_fields < MIN_VALID_FIELDS {
        return Vec::new();
    }

    let smoothed = smooth_temporally(raw);
    smoothed.into_iter()
        .map(|field| field.map(|field| build_mesh_pair(frame_size, &field)))
        .collect()
}

pub fn compose_mesh_corrections(
    existing: &[(Vec<f64>, Vec<f32>)],
    residual: Vec<Option<(Vec<f64>, Vec<f32>)>>,
) -> Vec<(Vec<f64>, Vec<f32>)> {
    if existing.is_empty() {
        let frame_size = residual.iter()
            .find_map(|mesh| mesh.as_ref().map(|mesh| (mesh.1[3] as u32, mesh.1[4] as u32)))
            .unwrap_or((2, 2));
        let identity = build_mesh_pair(frame_size, &identity_field());
        return residual.into_iter().map(|mesh| mesh.unwrap_or_else(|| identity.clone())).collect();
    }
    let mut out = existing.to_vec();
    for (idx, residual_pair) in residual.into_iter().enumerate() {
        let Some(residual_pair) = residual_pair else {
            continue;
        };
        if idx >= out.len() {
            let identity = build_mesh_pair((residual_pair.1[3] as u32, residual_pair.1[4] as u32), &identity_field());
            out.resize_with(idx + 1, || identity.clone());
        }
        if let Some(composed) = compose_pair(&out[idx], &residual_pair) {
            out[idx] = composed;
        } else {
            out[idx] = residual_pair;
        }
    }
    out
}

fn compose_pair(existing: &(Vec<f64>, Vec<f32>), residual: &(Vec<f64>, Vec<f32>)) -> Option<(Vec<f64>, Vec<f32>)> {
    if !same_mesh_space_f64(&existing.0, &residual.0) || !same_mesh_space_f32(&existing.1, &residual.1) {
        return None;
    }
    Some((
        compose_mesh_f64(&existing.0, &residual.0)?,
        compose_mesh_f32(&existing.1, &residual.1)?,
    ))
}

fn same_mesh_space_f64(a: &[f64], b: &[f64]) -> bool {
    a.len() >= 9 && b.len() >= 9 &&
    a[1] as usize == GRID && a[2] as usize == GRID &&
    b[1] as usize == GRID && b[2] as usize == GRID &&
    (a[3] - b[3]).abs() < 0.01 && (a[4] - b[4]).abs() < 0.01 &&
    (a[5] - b[5]).abs() < 0.01 && (a[6] - b[6]).abs() < 0.01 &&
    (a[7] - b[7]).abs() < 0.01 && (a[8] - b[8]).abs() < 0.01
}

fn same_mesh_space_f32(a: &[f32], b: &[f32]) -> bool {
    a.len() >= 9 && b.len() >= 9 &&
    a[1] as usize == GRID && a[2] as usize == GRID &&
    b[1] as usize == GRID && b[2] as usize == GRID &&
    (a[3] - b[3]).abs() < 0.01 && (a[4] - b[4]).abs() < 0.01 &&
    (a[5] - b[5]).abs() < 0.01 && (a[6] - b[6]).abs() < 0.01 &&
    (a[7] - b[7]).abs() < 0.01 && (a[8] - b[8]).abs() < 0.01
}

fn compose_mesh_f64(existing: &[f64], residual: &[f64]) -> Option<Vec<f64>> {
    let mut nodes = [(0.0f32, 0.0f32); GRID * GRID];
    for i in 0..GRID * GRID {
        let node = 9 + i * 2;
        let x = i % GRID;
        let y = i / GRID;
        let base_x = residual[3] as f32 * x as f32 / (GRID - 1) as f32;
        let base_y = residual[4] as f32 * y as f32 / (GRID - 1) as f32;
        nodes[i] = (
            (existing.get(node)? + residual.get(node)? - base_x as f64) as f32,
            (existing.get(node + 1)? + residual.get(node + 1)? - base_y as f64) as f32,
        );
    }
    let mut mesh = build_absolute_mesh((existing[3] as u32, existing[4] as u32), &nodes);
    preserve_fpd_f64(&mut mesh, existing);
    Some(mesh)
}

fn compose_mesh_f32(existing: &[f32], residual: &[f32]) -> Option<Vec<f32>> {
    let mut nodes = [(0.0f32, 0.0f32); GRID * GRID];
    for i in 0..GRID * GRID {
        let node = 9 + i * 2;
        let x = i % GRID;
        let y = i / GRID;
        let base_x = residual[3] * x as f32 / (GRID - 1) as f32;
        let base_y = residual[4] * y as f32 / (GRID - 1) as f32;
        nodes[i] = (
            *existing.get(node)? + *residual.get(node)? - base_x,
            *existing.get(node + 1)? + *residual.get(node + 1)? - base_y,
        );
    }
    let mut mesh = build_absolute_mesh((existing[3] as u32, existing[4] as u32), &nodes).into_iter().map(|x| x as f32).collect::<Vec<_>>();
    preserve_fpd_f32(&mut mesh, existing);
    Some(mesh)
}

fn preserve_fpd_f64(mesh: &mut Vec<f64>, existing: &[f64]) {
    let offset = existing.first().copied().unwrap_or_default() as usize;
    if offset > 0 && offset < existing.len() && existing.len() - offset <= 20 {
        mesh.truncate(mesh[0] as usize);
        mesh.extend_from_slice(&existing[offset..]);
    }
}

fn preserve_fpd_f32(mesh: &mut Vec<f32>, existing: &[f32]) {
    let offset = existing.first().copied().unwrap_or_default() as usize;
    if offset > 0 && offset < existing.len() && existing.len() - offset <= 20 {
        mesh.truncate(mesh[0] as usize);
        mesh.extend_from_slice(&existing[offset..]);
    }
}

fn residual_field(frame_size: (u32, u32), correspondences: &[Correspondence]) -> Option<[(f32, f32); GRID * GRID]> {
    let tracks = valid_tracks(frame_size, correspondences);
    if tracks.len() < MIN_TRACKS {
        return None;
    }

    let global = robust_global_affine(&tracks).unwrap_or_else(|| {
        let translation = robust_global_translation(&tracks);
        AffineFlow::translation(translation)
    });
    let residuals = tracks.into_iter()
        .filter_map(|track| {
            let observed = (track.end.0 - track.start.0, track.end.1 - track.start.1);
            let predicted = global.predict(track.start);
            let residual = (observed.0 - predicted.0, observed.1 - predicted.1);
            if residual.0.is_finite() && residual.1.is_finite() {
                Some((track.start, residual, track.confidence.max(0.001)))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut cells = aggregate_cells(frame_size, &residuals);
    reject_foreground(&mut cells);
    fill_empty_cells(&mut cells);
    regularize_spatially(&mut cells);
    apply_confidence(&mut cells);
    Some(nodes_from_cells(frame_size, &cells))
}

fn valid_tracks(frame_size: (u32, u32), correspondences: &[Correspondence]) -> Vec<Correspondence> {
    let max_disp = (frame_size.0 as f32).hypot(frame_size.1 as f32) * 0.35;
    correspondences.iter().copied().filter(|c| {
        let dx = c.end.0 - c.start.0;
        let dy = c.end.1 - c.start.1;
        c.start.0.is_finite() && c.start.1.is_finite() &&
        c.end.0.is_finite() && c.end.1.is_finite() &&
        c.start.0 >= 0.0 && c.start.1 >= 0.0 &&
        c.end.0 >= 0.0 && c.end.1 >= 0.0 &&
        c.start.0 < frame_size.0 as f32 && c.end.0 < frame_size.0 as f32 &&
        c.start.1 < frame_size.1 as f32 && c.end.1 < frame_size.1 as f32 &&
        dx.hypot(dy) <= max_disp
    }).collect()
}

fn robust_global_translation(tracks: &[Correspondence]) -> (f32, f32) {
    let mut dx = tracks.iter().map(|c| c.end.0 - c.start.0).collect::<Vec<_>>();
    let mut dy = tracks.iter().map(|c| c.end.1 - c.start.1).collect::<Vec<_>>();
    (median(&mut dx), median(&mut dy))
}

#[derive(Clone, Copy, Debug)]
struct AffineFlow {
    ax: f32,
    bx: f32,
    cx: f32,
    ay: f32,
    by: f32,
    cy: f32,
}

impl AffineFlow {
    fn translation(v: (f32, f32)) -> Self {
        Self { ax: 0.0, bx: 0.0, cx: v.0, ay: 0.0, by: 0.0, cy: v.1 }
    }
    fn predict(&self, p: (f32, f32)) -> (f32, f32) {
        (
            self.ax * p.0 + self.bx * p.1 + self.cx,
            self.ay * p.0 + self.by * p.1 + self.cy,
        )
    }
}

fn robust_global_affine(tracks: &[Correspondence]) -> Option<AffineFlow> {
    let initial_translation = robust_global_translation(tracks);
    let prefiltered = tracks.iter().copied().filter(|track| {
        let observed = (track.end.0 - track.start.0, track.end.1 - track.start.1);
        (observed.0 - initial_translation.0).hypot(observed.1 - initial_translation.1) < 12.0
    }).collect::<Vec<_>>();
    let initial = fit_affine(prefiltered.iter().copied()).or_else(|| fit_affine(tracks.iter().copied()))?;
    let mut errors = tracks.iter().map(|track| {
        let observed = (track.end.0 - track.start.0, track.end.1 - track.start.1);
        let predicted = initial.predict(track.start);
        (observed.0 - predicted.0).hypot(observed.1 - predicted.1)
    }).collect::<Vec<_>>();
    let med = median(&mut errors);
    let threshold = (med * 2.5).max(2.0);
    fit_affine(tracks.iter().copied().filter(|track| {
        let observed = (track.end.0 - track.start.0, track.end.1 - track.start.1);
        let predicted = initial.predict(track.start);
        (observed.0 - predicted.0).hypot(observed.1 - predicted.1) <= threshold
    }))
}

fn fit_affine<I: Iterator<Item = Correspondence>>(tracks: I) -> Option<AffineFlow> {
    let mut ata = [[0.0f64; 3]; 3];
    let mut atx = [0.0f64; 3];
    let mut aty = [0.0f64; 3];
    let mut count = 0usize;
    for track in tracks {
        let row = [track.start.0 as f64, track.start.1 as f64, 1.0];
        let dx = (track.end.0 - track.start.0) as f64;
        let dy = (track.end.1 - track.start.1) as f64;
        let w = track.confidence.max(0.001) as f64;
        for r in 0..3 {
            atx[r] += row[r] * dx * w;
            aty[r] += row[r] * dy * w;
            for c in 0..3 {
                ata[r][c] += row[r] * row[c] * w;
            }
        }
        count += 1;
    }
    if count < 6 {
        return None;
    }
    let sx = solve_3x3(ata, atx)?;
    let sy = solve_3x3(ata, aty)?;
    Some(AffineFlow {
        ax: sx[0] as f32,
        bx: sx[1] as f32,
        cx: sx[2] as f32,
        ay: sy[0] as f32,
        by: sy[1] as f32,
        cy: sy[2] as f32,
    })
}

fn solve_3x3(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let pivot = (col..3).max_by(|&ra, &rb| a[ra][col].abs().total_cmp(&a[rb][col].abs()))?;
        if a[pivot][col].abs() < 1e-9 {
            return None;
        }
        if pivot != col {
            a.swap(pivot, col);
            b.swap(pivot, col);
        }
        let div = a[col][col];
        for c in col..3 {
            a[col][c] /= div;
        }
        b[col] /= div;
        for r in 0..3 {
            if r == col {
                continue;
            }
            let factor = a[r][col];
            for c in col..3 {
                a[r][c] -= factor * a[col][c];
            }
            b[r] -= factor * b[col];
        }
    }
    if b.iter().all(|x| x.is_finite()) { Some(b) } else { None }
}

fn aggregate_cells(frame_size: (u32, u32), residuals: &[((f32, f32), (f32, f32), f32)]) -> [CellEstimate; GRID * GRID] {
    let mut buckets: Vec<Vec<((f32, f32), f32)>> = vec![Vec::new(); GRID * GRID];
    for &(point, residual, confidence) in residuals {
        let x = ((point.0 / frame_size.0.max(1) as f32) * GRID as f32).floor().clamp(0.0, (GRID - 1) as f32) as usize;
        let y = ((point.1 / frame_size.1.max(1) as f32) * GRID as f32).floor().clamp(0.0, (GRID - 1) as f32) as usize;
        buckets[y * GRID + x].push((residual, confidence));
    }

    let mut cells = [CellEstimate::default(); GRID * GRID];
    for (idx, bucket) in buckets.iter().enumerate() {
        if bucket.len() < MIN_CELL_TRACKS {
            continue;
        }
        let mut xs = bucket.iter().map(|x| x.0.0).collect::<Vec<_>>();
        let mut ys = bucket.iter().map(|x| x.0.1).collect::<Vec<_>>();
        let displacement = (median(&mut xs), median(&mut ys));
        let mut deviations = bucket.iter()
            .map(|x| (x.0.0 - displacement.0).hypot(x.0.1 - displacement.1))
            .collect::<Vec<_>>();
        let dispersion = median(&mut deviations);
        let support_gain = bucket.len() as f32 / (bucket.len() as f32 + 16.0);
        let dispersion_gain = 1.0 / (1.0 + (dispersion / 1.5).powi(2));
        let confidence = (support_gain * dispersion_gain).clamp(0.0, 1.0);
        cells[idx] = CellEstimate {
            displacement,
            confidence,
            observations: bucket.len(),
            direct_support: true,
            dispersion,
            neighbor_disagreement: 0.0,
        };
    }
    cells
}

fn reject_foreground(cells: &mut [CellEstimate; GRID * GRID]) {
    let original = *cells;
    for y in 0..GRID {
        for x in 0..GRID {
            let idx = y * GRID + x;
            if original[idx].confidence <= 0.0 {
                continue;
            }
            let mut neighbors = Vec::new();
            for yy in y.saturating_sub(1)..=(y + 1).min(GRID - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(GRID - 1) {
                    let nidx = yy * GRID + xx;
                    if nidx != idx && original[nidx].confidence > 0.0 {
                        neighbors.push(original[nidx].displacement);
                    }
                }
            }
            if neighbors.len() < 3 {
                continue;
            }
            let mut nx = neighbors.iter().map(|v| v.0).collect::<Vec<_>>();
            let mut ny = neighbors.iter().map(|v| v.1).collect::<Vec<_>>();
            let local = (median(&mut nx), median(&mut ny));
            let unsupported = (original[idx].displacement.0 - local.0).hypot(original[idx].displacement.1 - local.1);
            cells[idx].neighbor_disagreement = unsupported;
            let neighbor_gain = 1.0 / (1.0 + (unsupported / 2.0).powi(2));
            cells[idx].confidence *= neighbor_gain.clamp(0.15, 1.0);
            if unsupported > 3.0 {
                cells[idx].confidence *= 0.15;
            }
        }
    }
}

fn fill_empty_cells(cells: &mut [CellEstimate; GRID * GRID]) {
    for _ in 0..2 {
        let previous = *cells;
        for y in 0..GRID {
            for x in 0..GRID {
                let idx = y * GRID + x;
                if previous[idx].confidence > 0.05 {
                    continue;
                }
                let mut sum = (0.0, 0.0);
                let mut weight = 0.0;
                for yy in y.saturating_sub(1)..=(y + 1).min(GRID - 1) {
                    for xx in x.saturating_sub(1)..=(x + 1).min(GRID - 1) {
                        let nidx = yy * GRID + xx;
                        let w = previous[nidx].confidence;
                        if w > 0.05 {
                            sum.0 += previous[nidx].displacement.0 * w;
                            sum.1 += previous[nidx].displacement.1 * w;
                            weight += w;
                        }
                    }
                }
                if weight > 0.25 {
                    cells[idx].displacement = (sum.0 / weight, sum.1 / weight);
                    cells[idx].confidence = (weight / 4.0).min(0.35);
                    cells[idx].direct_support = false;
                    cells[idx].dispersion = 0.0;
                    cells[idx].neighbor_disagreement = 0.0;
                }
            }
        }
    }
}

fn regularize_spatially(cells: &mut [CellEstimate; GRID * GRID]) {
    let previous = *cells;
    for y in 0..GRID {
        for x in 0..GRID {
            let idx = y * GRID + x;
            let mut sum = (
                previous[idx].displacement.0 * previous[idx].confidence,
                previous[idx].displacement.1 * previous[idx].confidence,
            );
            let mut weight = 1.0 + previous[idx].confidence;
            for yy in y.saturating_sub(1)..=(y + 1).min(GRID - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(GRID - 1) {
                    let nidx = yy * GRID + xx;
                    if nidx == idx {
                        continue;
                    }
                    let w = previous[nidx].confidence * 0.35;
                    sum.0 += previous[nidx].displacement.0 * w;
                    sum.1 += previous[nidx].displacement.1 * w;
                    weight += w;
                }
            }
            cells[idx].displacement = (sum.0 / weight, sum.1 / weight);
        }
    }
}

fn apply_confidence(cells: &mut [CellEstimate; GRID * GRID]) {
    for cell in cells {
        let mut gain = cell.confidence.clamp(0.0, 1.0);
        if !cell.direct_support {
            gain = gain.min(0.25);
        }
        cell.displacement.0 *= gain;
        cell.displacement.1 *= gain;
        cell.confidence = gain;
    }
}

fn nodes_from_cells(frame_size: (u32, u32), cells: &[CellEstimate; GRID * GRID]) -> [(f32, f32); GRID * GRID] {
    let mut nodes = [(0.0, 0.0); GRID * GRID];
    for y in 0..GRID {
        for x in 0..GRID {
            let mut sum = (0.0, 0.0);
            let mut weight = 0.0;
            for cy in y.saturating_sub(1)..=y.min(GRID - 2) {
                for cx in x.saturating_sub(1)..=x.min(GRID - 2) {
                    let cell = cells[cy * GRID + cx];
                    let w = cell.confidence.max(0.02);
                    sum.0 += cell.displacement.0 * w;
                    sum.1 += cell.displacement.1 * w;
                    weight += w;
                }
            }
            nodes[y * GRID + x] = (sum.0 / weight.max(0.001), sum.1 / weight.max(0.001));
        }
    }
    protect_foldover(frame_size, &mut nodes);
    nodes
}

fn smooth_temporally(raw: Vec<Option<[(f32, f32); GRID * GRID]>>) -> Vec<Option<[(f32, f32); GRID * GRID]>> {
    let mut out = Vec::with_capacity(raw.len());
    let mut prev = identity_field();
    for field in raw {
        let Some(field) = field else {
            out.push(None);
            continue;
        };
        let mut smoothed = identity_field();
        for i in 0..GRID * GRID {
            smoothed[i] = (
                prev[i].0 * (1.0 - EMA_ALPHA) + field[i].0 * EMA_ALPHA,
                prev[i].1 * (1.0 - EMA_ALPHA) + field[i].1 * EMA_ALPHA,
            );
        }
        out.push(Some(smoothed));
        prev = smoothed;
    }
    out
}

fn build_mesh_pair(frame_size: (u32, u32), field: &[(f32, f32); GRID * GRID]) -> (Vec<f64>, Vec<f32>) {
    let forward = build_mesh(frame_size, field, -1.0);
    let inverse = build_mesh(frame_size, field, 1.0).into_iter().map(|x| x as f32).collect();
    (forward, inverse)
}

fn build_mesh(frame_size: (u32, u32), field: &[(f32, f32); GRID * GRID], sign: f32) -> Vec<f64> {
    let mut nodes = [(0.0f32, 0.0f32); GRID * GRID];
    for y in 0..GRID {
        for x in 0..GRID {
            let base_x = frame_size.0 as f32 * x as f32 / (GRID - 1) as f32;
            let base_y = frame_size.1 as f32 * y as f32 / (GRID - 1) as f32;
            let d = field[y * GRID + x];
            nodes[y * GRID + x] = (base_x + sign * d.0, base_y + sign * d.1);
        }
    }
    build_absolute_mesh(frame_size, &nodes)
}

fn build_absolute_mesh(frame_size: (u32, u32), nodes: &[(f32, f32); GRID * GRID]) -> Vec<f64> {
    let mut mesh = Vec::with_capacity(9 + GRID * GRID * 2 + GRID * GRID * 4 * 2 + 1);
    mesh.extend([
        0.0,
        GRID as f64,
        GRID as f64,
        frame_size.0 as f64,
        frame_size.1 as f64,
        0.0,
        0.0,
        frame_size.0 as f64,
        frame_size.1 as f64,
    ]);

    for y in 0..GRID {
        for x in 0..GRID {
            let node = nodes[y * GRID + x];
            mesh.push(node.0 as f64);
            mesh.push(node.1 as f64);
        }
    }

    append_spline_coefficients(&mut mesh, frame_size);
    mesh[0] = mesh.len() as f64;
    mesh.push(0.0);
    mesh
}

fn append_spline_coefficients(mesh: &mut Vec<f64>, frame_size: (u32, u32)) {
    let mut a = [0.0; splines::MAX_GRID_SIZE];
    let mut b = [0.0; splines::MAX_GRID_SIZE];
    let mut c = [0.0; splines::MAX_GRID_SIZE];
    let mut d = [0.0; splines::MAX_GRID_SIZE];
    let mut alpha = [0.0; splines::MAX_GRID_SIZE - 1];
    let mut mu = [0.0; splines::MAX_GRID_SIZE];
    let mut z = [0.0; splines::MAX_GRID_SIZE];

    for mesh_offset in 0..=1 {
        for row in 0..GRID {
            splines::BivariateSpline::cubic_spline_coefficients(
                &mesh[9 + mesh_offset..],
                2,
                row * GRID,
                frame_size.0 as f64,
                GRID,
                &mut a,
                &mut b,
                &mut c,
                &mut d,
                &mut alpha,
                &mut mu,
                &mut z,
            );
            mesh.extend(a);
            mesh.extend(b);
            mesh.extend(c);
            mesh.extend(d);
        }
    }
}

fn protect_foldover(frame_size: (u32, u32), nodes: &mut [(f32, f32); GRID * GRID]) {
    let max_dx = frame_size.0 as f32 / (GRID - 1) as f32 * 0.45;
    let max_dy = frame_size.1 as f32 / (GRID - 1) as f32 * 0.45;
    for _ in 0..8 {
        let mut changed = false;
        for y in 0..GRID {
            for x in 0..GRID {
                let idx = y * GRID + x;
                if x > 0 {
                    let left = nodes[idx - 1];
                    if (nodes[idx].0 - left.0).abs() > max_dx {
                        nodes[idx].0 = left.0 + (nodes[idx].0 - left.0).signum() * max_dx;
                        changed = true;
                    }
                }
                if y > 0 {
                    let up = nodes[idx - GRID];
                    if (nodes[idx].1 - up.1).abs() > max_dy {
                        nodes[idx].1 = up.1 + (nodes[idx].1 - up.1).signum() * max_dy;
                        changed = true;
                    }
                }
                nodes[idx] = clamp_displacement(frame_size, nodes[idx], 1.0);
            }
        }
        if !changed {
            break;
        }
    }
}

fn clamp_displacement(frame_size: (u32, u32), displacement: (f32, f32), confidence: f32) -> (f32, f32) {
    let max = (frame_size.0 as f32).hypot(frame_size.1 as f32) * MAX_DISPLACEMENT_RATIO * confidence.max(0.1);
    let norm = displacement.0.hypot(displacement.1);
    if !norm.is_finite() {
        (0.0, 0.0)
    } else if norm > max {
        let scale = max / norm;
        (displacement.0 * scale, displacement.1 * scale)
    } else {
        displacement
    }
}

fn identity_field() -> [(f32, f32); GRID * GRID] {
    [(0.0, 0.0); GRID * GRID]
}

fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

pub fn is_valid_mesh(mesh: &[f32]) -> bool {
    if mesh.len() > splines::MAX_BUFFER_SIZE || mesh.len() < 9 {
        return false;
    }
    let offset = mesh[0] as usize;
    offset > 10 &&
    offset < mesh.len() &&
    mesh[1] as usize == GRID &&
    mesh[2] as usize == GRID &&
    mesh[3] > 1.0 &&
    mesh[4] > 1.0 &&
    mesh.iter().all(|x| x.is_finite())
}

pub fn has_non_identity_mesh(mesh: &[f32]) -> bool {
    if !is_valid_mesh(mesh) {
        return false;
    }
    (0..GRID * GRID).any(|i| {
        let node = 9 + i * 2;
        let x = i % GRID;
        let y = i / GRID;
        let base_x = mesh[3] * x as f32 / (GRID - 1) as f32;
        let base_y = mesh[4] * y as f32 / (GRID - 1) as f32;
        (mesh[node] - base_x).abs() > 0.01 || (mesh[node + 1] - base_y).abs() > 0.01
    })
}

pub fn is_valid_forward_mesh(mesh: &[f64]) -> bool {
    if mesh.len() > splines::MAX_BUFFER_SIZE || mesh.len() < 9 {
        return false;
    }
    let offset = mesh[0] as usize;
    offset > 10 &&
    offset < mesh.len() &&
    mesh[1] as usize == GRID &&
    mesh[2] as usize == GRID &&
    mesh[3] > 1.0 &&
    mesh[4] > 1.0 &&
    mesh.iter().all(|x| x.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(size: (u32, u32), global: (f32, f32), local: impl Fn(f32, f32) -> (f32, f32)) -> Vec<Correspondence> {
        let mut out = Vec::new();
        for y in 0..18 {
            for x in 0..24 {
                let px = 10.0 + x as f32 * (size.0 as f32 - 20.0) / 23.0;
                let py = 10.0 + y as f32 * (size.1 as f32 - 20.0) / 17.0;
                let l = local(px, py);
                out.push(Correspondence {
                    start: (px, py),
                    end: (px + global.0 + l.0, py + global.1 + l.1),
                    confidence: 1.0,
                });
            }
        }
        out
    }

    fn max_inverse_delta(mesh: &[f32], size: (u32, u32)) -> f32 {
        mesh[9..9 + GRID * GRID * 2].chunks(2).enumerate().map(|(i, p)| {
            let x = i % GRID;
            let y = i / GRID;
            let bx = size.0 as f32 * x as f32 / (GRID - 1) as f32;
            let by = size.1 as f32 * y as f32 / (GRID - 1) as f32;
            (p[0] - bx).abs().max((p[1] - by).abs())
        }).fold(0.0, f32::max)
    }

    fn residual_at(cell: (usize, usize), residuals: &[(f32, f32)], size: (u32, u32)) -> Vec<((f32, f32), (f32, f32), f32)> {
        let cell_w = size.0 as f32 / GRID as f32;
        let cell_h = size.1 as f32 / GRID as f32;
        residuals.iter().enumerate().map(|(i, &r)| {
            let px = cell.0 as f32 * cell_w + 5.0 + (i % 3) as f32;
            let py = cell.1 as f32 * cell_h + 5.0 + (i / 3) as f32;
            ((px, py), r, 1.0)
        }).collect()
    }

    fn finalized_cells(size: (u32, u32), residuals: &[((f32, f32), (f32, f32), f32)]) -> [CellEstimate; GRID * GRID] {
        let mut cells = aggregate_cells(size, residuals);
        reject_foreground(&mut cells);
        fill_empty_cells(&mut cells);
        regularize_spatially(&mut cells);
        apply_confidence(&mut cells);
        cells
    }

    #[test]
    fn perfect_global_motion_produces_identity_residual_mesh() {
        let size = (640, 360);
        let pairs = (1..4).map(|i| (i, tracks(size, (12.0, -7.0), |_, _| (0.0, 0.0)))).collect::<Vec<_>>();
        let meshes = build_residual_meshes(5, size, &pairs);
        assert_eq!(meshes.len(), 5);
        let mesh = &meshes[1].as_ref().unwrap().1;
        assert!(is_valid_mesh(mesh));
        let max_delta = max_inverse_delta(mesh, size);
        assert!(max_delta < 0.01, "{max_delta}");
    }

    #[test]
    fn perfect_affine_camera_motion_produces_identity_residual_mesh() {
        let size = (640, 360);
        let pairs = (1..4).map(|i| (i, tracks(size, (3.0, -2.0), |x, y| {
            (0.012 * x - 0.006 * y, 0.004 * x + 0.009 * y)
        }))).collect::<Vec<_>>();
        let meshes = build_residual_meshes(5, size, &pairs);
        let mesh = &meshes[1].as_ref().unwrap().1;
        let max_delta = max_inverse_delta(mesh, size);
        assert!(max_delta < 0.1, "{max_delta}");
    }

    #[test]
    fn perfect_global_rotation_produces_identity_residual_mesh() {
        let size = (640, 360);
        let cx = size.0 as f32 * 0.5;
        let cy = size.1 as f32 * 0.5;
        let angle = 0.035f32;
        let pairs = (1..4).map(|i| (i, tracks(size, (0.0, 0.0), |x, y| {
            let dx = x - cx;
            let dy = y - cy;
            let rx = dx * angle.cos() - dy * angle.sin() + cx;
            let ry = dx * angle.sin() + dy * angle.cos() + cy;
            (rx - x, ry - y)
        }))).collect::<Vec<_>>();
        let meshes = build_residual_meshes(5, size, &pairs);
        let max_delta = max_inverse_delta(&meshes[1].as_ref().unwrap().1, size);
        assert!(max_delta < 0.1, "{max_delta}");
    }

    #[test]
    fn coherent_residual_becomes_counteracting_inverse_mesh() {
        let size = (640, 360);
        let pairs = (1..4).map(|i| (i, tracks(size, (0.0, 0.0), |x, _| {
            let n = x / size.0 as f32 - 0.5;
            (18.0 * n * n, 0.0)
        }))).collect::<Vec<_>>();
        let meshes = build_residual_meshes(5, size, &pairs);
        let right = 9 + ((GRID / 2) * GRID + GRID - 1) * 2;
        assert!(meshes[1].as_ref().unwrap().1[right] > size.0 as f32, "inverse mesh should sample to the right");
        assert!(meshes[1].as_ref().unwrap().0[right] < size.0 as f64, "forward mesh should move left");
    }

    #[test]
    fn isolated_foreground_motion_does_not_contaminate_background() {
        let size = (640, 360);
        let foreground = |x: f32, y: f32| {
            if x > 250.0 && x < 390.0 && y > 120.0 && y < 240.0 { (35.0, 0.0) } else { (2.0, 0.5) }
        };
        let pairs = (1..4).map(|i| (i, tracks(size, (0.0, 0.0), foreground))).collect::<Vec<_>>();
        let meshes = build_residual_meshes(5, size, &pairs);
        let left_center = 9 + ((GRID / 2) * GRID + 1) * 2;
        let right_center = 9 + ((GRID / 2) * GRID + GRID - 2) * 2;
        let left_delta = meshes[1].as_ref().unwrap().1[left_center] - size.0 as f32 / (GRID - 1) as f32;
        let right_delta = meshes[1].as_ref().unwrap().1[right_center] - size.0 as f32 * (GRID - 2) as f32 / (GRID - 1) as f32;
        assert!(left_delta < 8.0, "{left_delta}");
        assert!(right_delta < 8.0, "{right_delta}");
    }

    #[test]
    fn occluded_and_sparse_cells_fill_conservatively() {
        let size = (640, 360);
        let mut sparse = tracks(size, (4.0, -3.0), |x, y| {
            if x > 420.0 && y > 180.0 { (8.0, 2.0) } else { (0.0, 0.0) }
        });
        sparse.retain(|c| !(c.start.0 > 170.0 && c.start.0 < 470.0 && c.start.1 > 90.0 && c.start.1 < 270.0));
        let pairs = (1..4).map(|i| (i, sparse.clone())).collect::<Vec<_>>();
        let meshes = build_residual_meshes(5, size, &pairs);
        assert_eq!(meshes.len(), 5);
        let mesh = &meshes[1].as_ref().unwrap().1;
        assert!(is_valid_mesh(mesh));
        assert!(mesh.iter().all(|x| x.is_finite()));
        assert!(max_inverse_delta(mesh, size) < 18.0);
    }

    #[test]
    fn extreme_displacement_is_clamped_without_foldover() {
        let size = (640, 360);
        let pairs = (1..4).map(|i| (i, tracks(size, (0.0, 0.0), |x, y| {
            let sx = if (x as i32 / 30) % 2 == 0 { 180.0 } else { -180.0 };
            let sy = if (y as i32 / 30) % 2 == 0 { 120.0 } else { -120.0 };
            (sx, sy)
        }))).collect::<Vec<_>>();
        let meshes = build_residual_meshes(5, size, &pairs);
        let mesh = &meshes[1].as_ref().unwrap().1;
        assert!(is_valid_mesh(mesh));
        let max_step_x = size.0 as f32 / (GRID - 1) as f32 * 1.45;
        let max_step_y = size.1 as f32 / (GRID - 1) as f32 * 1.45;
        for y in 0..GRID {
            for x in 0..GRID {
                let idx = 9 + (y * GRID + x) * 2;
                if x > 0 {
                    let prev = 9 + (y * GRID + x - 1) * 2;
                    assert!(mesh[idx] - mesh[prev] > 0.0);
                    assert!(mesh[idx] - mesh[prev] <= max_step_x);
                }
                if y > 0 {
                    let prev = 9 + ((y - 1) * GRID + x) * 2;
                    assert!(mesh[idx + 1] - mesh[prev + 1] > 0.0);
                    assert!(mesh[idx + 1] - mesh[prev + 1] <= max_step_y);
                }
            }
        }
    }

    #[test]
    fn temporal_smoothing_reduces_jitter_and_preserves_trend() {
        let size = (640, 360);
        let jitter = vec![
            (1, tracks(size, (0.0, 0.0), |x, _| {
                let n = x / size.0 as f32 - 0.5;
                (18.0 * n * n, 0.0)
            })),
            (2, tracks(size, (0.0, 0.0), |x, _| {
                let n = x / size.0 as f32 - 0.5;
                (-18.0 * n * n, 0.0)
            })),
            (3, tracks(size, (0.0, 0.0), |x, _| {
                let n = x / size.0 as f32 - 0.5;
                (18.0 * n * n, 0.0)
            })),
        ];
        let meshes = build_residual_meshes(4, size, &jitter);
        let right = 9 + ((GRID / 2) * GRID + GRID - 1) * 2;
        let d1 = meshes[1].as_ref().unwrap().1[right] - size.0 as f32;
        let d2 = meshes[2].as_ref().unwrap().1[right] - size.0 as f32;
        assert!((d2 - d1).abs() < 20.0 * EMA_ALPHA + 0.1);
    }

    #[test]
    fn invalid_tracks_fail_closed_to_identity_mesh() {
        let size = (640, 360);
        let meshes = build_residual_meshes(1, size, &[(0, vec![
            Correspondence {
            start: (f32::NAN, 0.0),
            end: (1.0, 1.0),
            confidence: 1.0,
            },
            Correspondence {
                start: (10.0, 10.0),
                end: (f32::INFINITY, 20.0),
                confidence: 1.0,
            },
            Correspondence {
                start: (10.0, 10.0),
                end: (size.0 as f32 + 1.0, 20.0),
                confidence: 1.0,
            },
        ])]);
        assert!(meshes.is_empty());
    }

    #[test]
    fn coherent_support_retains_significant_gain() {
        let size = (640, 360);
        let residuals = residual_at((4, 4), &[(4.0, 0.0); 24], size);
        let cells = finalized_cells(size, &residuals);
        let cell = cells[4 * GRID + 4];
        assert!(cell.confidence > 0.55, "{:?}", cell);
        assert!(cell.displacement.0 > 1.0, "{:?}", cell);
    }

    #[test]
    fn sparse_support_attenuates_gain() {
        let size = (640, 360);
        let residuals = residual_at((4, 4), &[(4.0, 0.0); 3], size);
        let cells = finalized_cells(size, &residuals);
        let cell = cells[4 * GRID + 4];
        assert!(cell.confidence < 0.35, "{:?}", cell);
        assert!(cell.displacement.0 < 1.5, "{:?}", cell);
    }

    #[test]
    fn residual_dispersion_attenuates_gain() {
        let size = (640, 360);
        let mut mixed = Vec::new();
        for i in 0..12 {
            mixed.push(if i % 2 == 0 { (8.0, 0.0) } else { (-8.0, 0.0) });
        }
        let residuals = residual_at((4, 4), &mixed, size);
        let cells = finalized_cells(size, &residuals);
        let cell = cells[4 * GRID + 4];
        assert!(cell.dispersion > 7.0, "{:?}", cell);
        assert!(cell.confidence < 0.08, "{:?}", cell);
    }

    #[test]
    fn neighbor_disagreement_attenuates_gain() {
        let size = (640, 360);
        let mut residuals = Vec::new();
        for c in [(3, 4), (4, 3), (5, 4), (4, 5)] {
            residuals.extend(residual_at(c, &[(2.0, 0.0); 16], size));
        }
        residuals.extend(residual_at((4, 4), &[(12.0, 0.0); 16], size));
        let cells = finalized_cells(size, &residuals);
        let center = cells[4 * GRID + 4];
        let neighbor = cells[4 * GRID + 3];
        assert!(!center.direct_support || center.neighbor_disagreement > 6.0, "{:?}", center);
        assert!(center.confidence < neighbor.confidence, "{:?} {:?}", center, neighbor);
    }

    #[test]
    fn unsupported_interpolated_area_is_conservative() {
        let size = (640, 360);
        let residuals = residual_at((2, 2), &[(9.0, 0.0); 24], size);
        let cells = finalized_cells(size, &residuals);
        let inferred = cells[3 * GRID + 3];
        assert!(!inferred.direct_support, "{:?}", inferred);
        assert!(inferred.confidence <= 0.25, "{:?}", inferred);
    }

    #[test]
    fn well_supported_local_deformation_is_not_suppressed_to_identity() {
        let size = (640, 360);
        let residuals = residual_at((4, 4), &[(6.0, -2.0); 32], size);
        let cells = finalized_cells(size, &residuals);
        let cell = cells[4 * GRID + 4];
        assert!(cell.confidence > 0.5, "{:?}", cell);
        assert!(cell.displacement.0.abs() > 1.0, "{:?}", cell);
    }
}
