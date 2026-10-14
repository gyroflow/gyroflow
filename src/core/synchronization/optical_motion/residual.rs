// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Mxtl7
//
//! The local residual pass of the optical stabilization: what a global rotation leaves unexplained, corrected in small
//! groups of pixels.
//!
//! The correction the optical pipeline fits ([`crate::gyro_source::optical_correction::OpticalCorrection`]) is one
//! rotation per band of rows. What moves differently from that - parallax the high-pass left behind, lens breathing
//! that varies inside a frame, OIS, a rolling shutter the metadata understates - is left on the image. The mesh of the
//! stabilization already corrects such warps per frame (Sony's, from metadata): this pass measures the same kind of
//! warp from the tracks, per frame, and serves it to the render on the same plate: a mesh block in the exact format
//! [`crate::gyro_source::file_metadata::MESH_HEADER`] documents and the kernels interpolate (`interpolate_mesh`),
//! composited behind the camera's own mesh, which always wins (never on top of IBIS, see `compose_residual_mesh`).
//!
//! Direction of the correction. The undistort kernels sample the inverse mesh at the **destination** pixel and the
//! sampled value *is* the source position (`opencl_undistort.cl`: `uv = interpolate_mesh(0, mesh_data, ...)`). A
//! region of the image that moved by `+r` px relative to where the global pass put it needs its source position pulled
//! by `+r` to be brought back, so the block holds **`+residual`** on the grid positions (not the negative of it).
//!
//! What is measured is the residual of the **global rotation only**: the bands' `ρ = M·δ(ta) - δ(tb)` is what the
//! global pass corrects by construction, so the prediction uses the per-band rotation the robust fit itself produced
//! (its `ρ` is a rotation vector, ±ρ the same rotation to the fit; `sgn` disambiguates against the pair's quats), and
//! what the bands' fit downweighted or rejected (a moving car, water, a reflection) never contributes at full weight -
//! or at all, when the user excluded the track (the track editor of `1968949a`).
//!
//! Everything is derived from what the analysis measured anyway: the per-pair fields are computed in `Measuring`,
//! where the per-point weights and band fits already exist, and kept on `OpticalMeasurements` (about 1.3 kB a pair).
//! Turning the pass on, off, or to a new strength re-builds the mesh from them in milliseconds; no re-analysis.

use nalgebra::{ Matrix3, Rotation3, UnitQuaternion, Vector3 };
use serde::{ Deserialize, Serialize };

use super::Observation;
use crate::gyro_source::file_metadata::{ MeshTable, MESH_HEADER };
use crate::gyro_source::splines::{ BivariateSpline, MESH_BLOCK_SIZE, MAX_GRID_SIZE };

/// Grid cells along the width; the kernels hard-code 9×9 (`GRID_SIZE`)
pub const RESIDUAL_GRID_SIZE: usize = MAX_GRID_SIZE;

/// Per-point weights at or below this are the band fit's rejects: they don't contribute
pub const ROBUST_REJECT_EPS: f64 = 1e-6;

/// One displacement, in image px: what this cell moved by beyond the global correction, the weight it was measured
/// with, and how many points said it
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResidualCell {
    pub dx: f64,
    pub dy: f64,
    pub weight: f64,
    pub points: u32,
}

/// The residual field of one frame pair, binned into `RESIDUAL_GRID_SIZE²` cells over the image
#[derive(Clone, Debug, Default)]
pub struct ResidualField {
    /// Frame pair, as `Pair::seq` numbers them
    pub seq: usize,
    /// Frame the correction of this field applies to (`Pair::a.index`)
    pub frame: usize,
    /// The frame's size in px, the tables' mesh space
    pub size: (f64, f64),
    /// `RESIDUAL_GRID_SIZE²`, row-major
    pub cells: Vec<ResidualCell>,
}

impl ResidualField {
    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(|c| c.points == 0)
    }
    /// The fraction of the grid that got any points: pairs below `ResidualSettings::min_coverage` are dropped whole
    /// (a pair with a handful of stray cells would leave holes the interpolation smears into wrong source positions)
    pub fn coverage(&self) -> f64 {
        if self.cells.is_empty() { return 0.0; }
        self.cells.iter().filter(|c| c.points > 0).count() as f64 / self.cells.len() as f64
    }
    /// Cells whose displacement sits farther than `prune_px` from most of their neighbours (a discontinuity the
    /// interpolation between cells would smear into ringing) are zeroed: they fall back on the interpolated field
    pub fn prune_discontinuities(&mut self, prune_px: f64) {
        let n = RESIDUAL_GRID_SIZE as isize;
        let at = |x: isize, y: isize| -> Option<usize> {
            (x >= 0 && y >= 0 && x < n && y < n).then(|| (y * n + x) as usize)
        };
        let mut kill = vec![false; self.cells.len()];
        for y in 0..n {
            for x in 0..n {
                let Some(ici) = at(x, y) else { continue };
                let c = &self.cells[ici];
                if c.points == 0 { continue; }
                let mut far = 0usize;
                for (dx, dy) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
                    if let Some(ni) = at(x + dx, y + dy) {
                        let nb = &self.cells[ni];
                        if nb.points > 0 && (nb.dx - c.dx).hypot(nb.dy - c.dy) > prune_px { far += 1; }
                    }
                }
                if far >= 3 { kill[ici] = true; }
            }
        }
        for (k, kill) in kill.into_iter().enumerate() {
            if kill { self.cells[k] = ResidualCell::default(); }
        }
    }
}

/// Settings of the residual pass. Off by default: the global pass stays in charge until the user opts in, and the
/// project file format is untouched (these live in the synchronization settings, not serialized into the project)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Copy)]
#[serde(default)]
pub struct ResidualSettings {
    pub enabled: bool,
    /// Gain of the correction, 0..=1
    pub strength: f64,
    /// The correction is capped at this many px, so no cell can move a pixel farther than that
    pub max_residual_px: f64,
    /// Residuals beyond this many px (between neighbouring cells) are treated as discontinuities and pruned
    pub prune_px: f64,
    /// Frame pairs whose cell coverage fell below this fraction of the grid are dropped whole
    pub min_coverage: f64,
    /// Half width of the temporal high-pass over each cell's trajectory, in frame pairs (0 = off). As the bands'
    /// high-pass, it takes the slow part out - that's the scene moving behind a parallax, not shake - and keeps the
    /// fast residual the global pass left
    pub hp_pairs: u32,
}
impl Default for ResidualSettings {
    fn default() -> Self {
        Self { enabled: false, strength: 1.0, max_residual_px: 12.0, prune_px: 24.0, min_coverage: 0.2, hp_pairs: 15 }
    }
}

/// One band's fit, as the prediction of the field needs it
#[derive(Clone, Copy, Debug)]
pub struct ResidualBand {
    /// The quaternions' own rotation between the band's times, `R(tb)ᵀ·R(ta)`
    pub m: Matrix3<f64>,
    /// The rotation the band moved by beyond the quaternions, radians, quat frame
    pub rho: Vector3<f64>,
}

/// One tracked point of a pair, as the field computation needs it
#[derive(Clone, Copy, Debug)]
pub struct ResidualPoint {
    /// Index into the pair's `obs`
    pub k: usize,
    /// Index into the pair's bands
    pub band: usize,
    /// Weight the band's robust fit gave it
    pub weight: f64,
}

/// Bins the residual of one pair into cells of the image. For a point tracked from `a` to `b`: the global pass puts it
/// at `v' = R(ρ)·m·v` in the quat frame (`fit_band`'s own linearization: `p = m·v`, `r ≈ ρ × p`), which in the tracked
/// frame's pixels is `d = f·(v'ₓ, -v'ᵧ)` (`to_quat_frame` flips y), `v` the point's unit bearing. What it *did*
/// beyond that, scaled to image px, is the residual; binned by where the point sits, weighted by the band fit's
/// weight for it, outliers and rejects dropped. `quat_at` evaluates the (already corrected) orientation, for `sgn`
#[allow(clippy::too_many_arguments)]
pub fn field_from_pair(
    seq: usize,
    frame: usize,
    obs: &[Observation],
    bands: &[ResidualBand],
    pts: &[ResidualPoint],
    quat_at: &dyn Fn(f64) -> UnitQuaternion<f64>,
    ta_ms: f64,
    tb_ms: f64,
    focal_px: f64,
    tracked_size: (f64, f64),
    max_residual_px: f64,
) -> ResidualField {
    let mut field = ResidualField { seq, frame, size: tracked_size, cells: vec![ResidualCell::default(); RESIDUAL_GRID_SIZE * RESIDUAL_GRID_SIZE] };
    if obs.is_empty() || pts.is_empty() { return field; }
    let (tr_w, tr_h) = (tracked_size.0.max(1.0), tracked_size.1.max(1.0));
    // sgn per band: the fit's ρ is ±the same rotation; the pair's own rotation tells them apart
    let s = (quat_at(ta_ms).inverse() * quat_at(tb_ms)).to_rotation_matrix().into_inner();
    // The fit's own linearization: p = m·v, r ≈ ρ × p, so the model rotation is R(ρ)·m - ρ applied after m
    let r90: Vec<Matrix3<f64>> = bands.iter().map(|b| {
        let sgn = if b.rho.norm_squared() < 1e-20 { 1.0 }
                  else if b.rho.dot(&(s * b.rho)) >= 0.0 { 1.0 } else { -1.0 };
        Rotation3::new(b.rho * sgn).into_inner() * b.m
    }).collect();

    let mut acc: Vec<(f64, f64, f64, u32)> = vec![(0.0, 0.0, 0.0, 0); RESIDUAL_GRID_SIZE * RESIDUAL_GRID_SIZE];
    for pt in pts {
        let Some(o) = obs.get(pt.k) else { continue };
        let Some(bp) = r90.get(pt.band) else { continue };
        let weight = pt.weight;
        if weight <= ROBUST_REJECT_EPS || !weight.is_finite() { continue; }
        let (ax, ay) = (o.a[0] as f64, o.a[1] as f64);
        let (bx, by) = (o.b[0] as f64, o.b[1] as f64);
        let (cx, cy) = (tr_w * 0.5, tr_h * 0.5);
        let v = Vector3::new((ax - cx) / focal_px, -(ay - cy) / focal_px, 1.0);
        let vp = bp * v;
        // Where the model puts it in the image: the bearing back through the same pinhole (principal point at the
        // centre), so the residual is what the point did beyond the global rotation, in px
        let (px, py) = (cx + focal_px * vp.x / vp.z, cy - focal_px * vp.y / vp.z);
        let rdx = bx - px;
        let rdy = by - py;
        if !rdx.is_finite() || !rdy.is_finite() { continue; }
        if rdx.hypot(rdy) > max_residual_px { continue; }
        let cell = ((ay / tr_h * RESIDUAL_GRID_SIZE as f64).floor().clamp(0.0, (RESIDUAL_GRID_SIZE - 1) as f64) as usize)
            * RESIDUAL_GRID_SIZE
            + (ax / tr_w * RESIDUAL_GRID_SIZE as f64).floor().clamp(0.0, (RESIDUAL_GRID_SIZE - 1) as f64) as usize;
        let a = &mut acc[cell];
        a.0 += rdx * weight;
        a.1 += rdy * weight;
        a.2 += weight;
        a.3 += 1;
    }
    for (cell, a) in acc.iter().enumerate() {
        if a.2 > 0.0 {
            field.cells[cell] = ResidualCell { dx: a.0 / a.2, dy: a.1 / a.2, weight: a.2, points: a.3 };
        }
    }
    field
}

/// Takes the slow part out of each cell's trajectory over ±`half` pairs: a weighted mean of the window around it,
/// subtracted (what `Measuring::high_pass` does per track, here per cell). The slow part is the scene moving behind a
/// parallax - not shake - and must not be "stabilized"
pub fn high_pass_fields(fields: &mut [ResidualField], half: usize) {
    if fields.is_empty() || half == 0 { return; }
    let n_cells = RESIDUAL_GRID_SIZE * RESIDUAL_GRID_SIZE;
    let n = fields.len();
    let mut out = vec![(0.0f64, 0.0f64); n * n_cells];
    for c in 0..n_cells {
        for i in 0..n {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(n);
            let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
            for j in lo..hi {
                let cell = &fields[j].cells[c];
                if cell.points > 0 {
                    sx += cell.dx * cell.weight;
                    sy += cell.dy * cell.weight;
                    sw += cell.weight;
                }
            }
            if sw > 0.0 {
                let cell = &fields[i].cells[c];
                if cell.points > 0 {
                    out[i * n_cells + c] = (cell.dx - sx / sw, cell.dy - sy / sw);
                }
            }
        }
    }
    for (i, f) in fields.iter_mut().enumerate() {
        for c in 0..n_cells {
            let cell = &mut f.cells[c];
            if cell.points > 0 {
                let (hx, hy) = out[i * n_cells + c];
                cell.dx = hx;
                cell.dy = hy;
            }
        }
    }
}

/// The kernel-consumable block of one field: `MESH_BLOCK_SIZE` floats in the layout of [`MESH_HEADER`] - header,
/// 9×9 grid positions (x, y interleaved), then the natural cubic spline coefficients of every row, for x and then
/// for y (the exact layout `sony::append_row_coefficients` writes and `interpolate_mesh` reads). The nodes sit where
/// the kernels look them up (`size / (n-1) · i`); the block holds **positions**: identity + the residual, since the
/// kernels read the sampled value as the source uv. A cell without points takes the average of the filled ones
/// around it; a field with nothing at all gives `None`
pub fn mesh_table_from_field(cells: &[Option<(f64, f64)>], size: (f64, f64), settings: &ResidualSettings) -> Option<MeshTable> {
    let n = RESIDUAL_GRID_SIZE;
    if cells.len() != n * n || cells.iter().all(|c| c.is_none()) { return None; }
    let s = settings.strength.clamp(0.0, 1.0);
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    let gain = settings.max_residual_px.max(1e-6);
    let value = |comp: usize, x: usize, y: usize| -> f64 {
        if let Some((dx, dy)) = cells[y * n + x] {
            let d = if comp == 0 { dx } else { dy };
            return (s * d).clamp(-gain, gain);
        }
        // Empty cell: the average of the filled ones around it
        let (mut sum, mut sw) = (0.0f64, 0.0f64);
        for yy in y.saturating_sub(2)..=(y + 2).min(n - 1) {
            for xx in x.saturating_sub(2)..=(x + 2).min(n - 1) {
                if let Some((dx, dy)) = cells[yy * n + xx] {
                    let d = if comp == 0 { dx } else { dy };
                    let dist = (((xx as f64 - x as f64) * w / (n - 1) as f64).powi(2)
                        + ((yy as f64 - y as f64) * h / (n - 1) as f64).powi(2)).sqrt();
                    let weight = 1.0 / (1.0 + dist / gain);
                    sum += d * weight;
                    sw += weight;
                }
            }
        }
        if sw > 0.0 { (s * sum / sw).clamp(-gain, gain) } else { 0.0 }
    };
    let (step_x, step_y) = (w / (n - 1) as f64, h / (n - 1) as f64);
    let mut block: Vec<f64> = Vec::with_capacity(MESH_BLOCK_SIZE);
    // Header: [block length, divisions x, y, mesh size x, y, capture area origin x, y, size x, y]. The mesh space of
    // this block is the image itself: the whole frame, no crop (the kernels map the image onto it and back)
    block.push(MESH_BLOCK_SIZE as f64);
    block.push((n - 1) as f64);
    block.push((n - 1) as f64);
    block.push(w);
    block.push(h);
    block.push(0.0);
    block.push(0.0);
    block.push(w);
    block.push(h);
    // The grid: the source position each destination node maps to - its own position plus the residual
    for y in 0..n {
        for x in 0..n {
            block.push(x as f64 * step_x + value(0, x, y));
            block.push(y as f64 * step_y + value(1, x, y));
        }
    }
    // The row spline coefficients of every row, for x and then for y: exactly what `sony::append_row_coefficients`
    // appends, computed from the grid section that is in place now
    let mut a = [0.0; MAX_GRID_SIZE];
    let mut b = [0.0; MAX_GRID_SIZE];
    let mut c = [0.0; MAX_GRID_SIZE];
    let mut d = [0.0; MAX_GRID_SIZE];
    let mut alpha = [0.0; MAX_GRID_SIZE - 1];
    let mut mu = [0.0; MAX_GRID_SIZE];
    let mut z = [0.0; MAX_GRID_SIZE];
    for comp in 0..2 {
        for j in 0..n {
            BivariateSpline::cubic_spline_coefficients(&block[MESH_HEADER + comp..], 2, j * n, w, n, &mut a, &mut b, &mut c, &mut d, &mut alpha, &mut mu, &mut z);
            block.extend_from_slice(&a);
            block.extend_from_slice(&b);
            block.extend_from_slice(&c);
            block.extend_from_slice(&d);
        }
    }
    debug_assert_eq!(block.len(), MESH_BLOCK_SIZE);
    let inverse = block.iter().map(|v| *v as f32).collect();
    Some(MeshTable { forward: Vec::new(), inverse, refinement: Vec::new() })
}

/// The residual the block `table` holds at one image pixel, in px (the CPU side of what `interpolate_mesh` evaluates:
/// the block holds positions, so the residual is what they sit off the identity by; what the tests assert against)
pub fn cell_residual(table: &MeshTable, size: (f64, f64), x: f64, y: f64) -> (f64, f64) {
    let block: Vec<f64> = table.inverse.iter().map(|v| *v as f64).collect();
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    let s = BivariateSpline::new(RESIDUAL_GRID_SIZE, RESIDUAL_GRID_SIZE);
    (s.interpolate(w, h, &block, 0, x, y) - x, s.interpolate(w, h, &block, 1, x, y) - y)
}

/// The per-frame mesh the render path consumes, for one clip: one table per field that made the cut, and which frame
/// each applies to
#[derive(Clone, Debug, Default)]
pub struct ResidualMesh {
    pub tables: Vec<MeshTable>,
    /// `(frame index, table index)`, ascending in frame
    pub frames: Vec<(usize, u32)>,
    /// The image size the tables were built for (px)
    pub size: (f64, f64),
}
impl ResidualMesh {
    pub fn is_empty(&self) -> bool { self.frames.is_empty() }
    pub fn has_frame(&self, frame: usize) -> bool {
        self.frames.iter().any(|(f, _)| *f == frame)
    }
}

/// Builds the whole mesh from the fields: the high-pass over the cells' trajectories, the pairs below `min_coverage`
/// dropped, the discontinuities pruned, one table per surviving pair (with the previous pair's value averaged in, so
/// the video moves with the measurement, not a frame behind it)
pub fn build_mesh(fields: &mut Vec<ResidualField>, size: (f64, f64), settings: &ResidualSettings) -> ResidualMesh {
    let half = (settings.hp_pairs as usize) / 2;
    high_pass_fields(fields, half);
    for f in fields.iter_mut() {
        f.prune_discontinuities(settings.prune_px);
    }
    fields.retain(|f| f.coverage() >= settings.min_coverage);
    let mut prev: Vec<Option<(f64, f64)>> = vec![None; RESIDUAL_GRID_SIZE * RESIDUAL_GRID_SIZE];
    let mut tables = Vec::with_capacity(fields.len());
    let mut frames = Vec::with_capacity(fields.len());
    for field in fields.iter() {
        let mut cells: Vec<Option<(f64, f64)>> = vec![None; RESIDUAL_GRID_SIZE * RESIDUAL_GRID_SIZE];
        for (i, c) in field.cells.iter().enumerate() {
            if c.points == 0 { continue; }
            let this = (c.dx, c.dy);
            let smoothed = match prev[i] {
                Some(p) => ((p.0 + this.0) * 0.5, (p.1 + this.1) * 0.5),
                None => this,
            };
            prev[i] = Some(this);
            cells[i] = Some(smoothed);
        }
        if let Some(table) = mesh_table_from_field(&cells, field.size, settings) {
            frames.push((field.frame, tables.len() as u32));
            tables.push(table);
        }
    }
    ResidualMesh { tables, frames, size }
}

/// The buffer the kernels get for a frame: the camera's mesh when it has one (fail closed - never on top of Sony's
/// IBIS, whose mesh is the truth about the lens' own warp), otherwise the residual mesh's block, followed by the
/// 4-value empty focal plane probe and a single 0.0 forward probe - exactly the padding `MeshCorrections::kernel_buffer`
/// puts around a block
pub fn compose_residual_mesh(camera_mesh_data: Vec<f32>, frame: usize, residual: &Option<ResidualMesh>) -> Vec<f32> {
    let Some(residual) = residual else { return camera_mesh_data };
    if !camera_mesh_data.is_empty() { return camera_mesh_data; }
    let Some((_, ti)) = residual.frames.iter().find(|(f, _)| *f == frame) else {
        return Vec::new();
    };
    let table = &residual.tables[*ti as usize];
    let mut buf = Vec::with_capacity(table.inverse.len() + 4 + 1);
    buf.extend_from_slice(&table.inverse);
    buf.extend_from_slice(&[0.0; 4]); // focal plane probe: count 0
    buf.push(0.0); // forward probe: no refinement
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector3;

    const SIZE: (f64, f64) = (960.0, 540.0);

    fn obs(id: u32, a: [f32; 2], b: [f32; 2]) -> Observation {
        Observation { id, a, b }
    }

    fn identity_quat(_t: f64) -> UnitQuaternion<f64> {
        UnitQuaternion::identity()
    }

    fn points_everywhere(obs: &[Observation], band: usize) -> Vec<ResidualPoint> {
        obs.iter().enumerate().map(|(k, _)| ResidualPoint { k, band, weight: 1.0 }).collect()
    }

    #[test]
    fn constant_pixel_shift_yields_no_residual() {
        // A global rotation that exactly explains the motion must leave a ~zero field: the same small-angle rotation
        // the band fit would have found, predicted through the same pinhole the field prediction uses
        let wz = 0.01;
        let bands = vec![ResidualBand { m: Matrix3::identity(), rho: Vector3::new(0.0, 0.0, wz) }];
        let mut obsv = Vec::new();
        for i in 0..40u32 {
            let ax = (i as f32 * 47.0) % 960.0;
            let ay = (i as f32 * 31.0) % 540.0;
            let f = 1000.0;
            let (cx, cy) = (SIZE.0 / 2.0, SIZE.1 / 2.0);
            let v = Vector3::new((ax as f64 - cx) / f, -(ay as f64 - cy) / f, 1.0);
            let r = Rotation3::new(Vector3::new(0.0, 0.0, wz));
            let vp = r.into_inner() * v;
            // The same pinhole the field predicts through, principal point and all
            let bx = (cx + f * vp.x / vp.z) as f32;
            let by = (cy - f * vp.y / vp.z) as f32;
            obsv.push(obs(i, [ax, ay], [bx, by]));
        }
        let pts = points_everywhere(&obsv, 0);
        let field = field_from_pair(0, 0, &obsv, &bands, &pts, &identity_quat, 0.0, 20.0, 1000.0, SIZE, 1e6);
        for c in &field.cells {
            if c.points > 0 {
                // f32 pixel inputs round at ~1e-4 px; 5e-3 is the honest bar for the rotation applied twice
                assert!(c.dx.abs() < 5e-3 && c.dy.abs() < 5e-3, "cell got ({}, {})", c.dx, c.dy);
            }
        }
    }

    #[test]
    fn local_bump_survives_high_pass() {
        // A region that moved 3px extra for one pair keeps that bump after the high-pass (it is not global, so it is
        // what this pass is for), and no phantom residual appears in the neighbouring pairs
        let mut obsv: Vec<Observation> = Vec::new();
        for i in 0..400u32 {
            let ax = ((i % 20) as f32) * 45.0 + 10.0;
            let ay = ((i / 20) as f32) * 45.0 + 10.0;
            obsv.push(obs(i, [ax, ay], [ax, ay]));
        }
        let mut per_pair: Vec<Vec<Observation>> = vec![obsv.clone(); 11];
        for o in per_pair[5].iter_mut() {
            if o.a[0] > 300.0 && o.a[0] < 600.0 && o.a[1] > 200.0 && o.a[1] < 400.0 {
                o.b[0] += 3.0;
            }
        }
        let bands = vec![ResidualBand { m: Matrix3::identity(), rho: Vector3::zeros() }];
        let mut fields: Vec<ResidualField> = per_pair.iter().enumerate().map(|(s, oo)| {
            let pts = points_everywhere(oo, 0);
            field_from_pair(s, s, oo, &bands, &pts, &identity_quat, 0.0, 20.0, 1000.0, SIZE, 1e6)
        }).collect();
        high_pass_fields(&mut fields, 2);
        let found = fields[5].cells.iter().filter(|c| c.points > 0).map(|c| c.dx).fold(0.0, f64::max);
        assert!(found > 1.5, "bump lost after high-pass: {found}");
        for (i, f) in fields.iter().enumerate() {
            if i == 5 { continue; }
            for c in &f.cells {
                if c.points > 0 {
                    assert!(c.dx.abs() < 1.0, "pair {i} has phantom residual {}", c.dx);
                }
            }
        }
    }

    #[test]
    fn mesh_block_matches_kernel_layout() {
        // The block the kernels interpolate: MESH_BLOCK_SIZE floats, mesh_data[0] = block length > 10 (their mesh
        // probe), header geometry in place, and the lookup at a node returns the position the field put there
        // (identity + residual)
        let mut cells: Vec<Option<(f64, f64)>> = vec![None; RESIDUAL_GRID_SIZE * RESIDUAL_GRID_SIZE];
        cells[4 * RESIDUAL_GRID_SIZE + 4] = Some((2.0, -1.0));
        let settings = ResidualSettings { strength: 1.0, max_residual_px: 12.0, ..Default::default() };
        let table = mesh_table_from_field(&cells, SIZE, &settings).unwrap();
        assert_eq!(table.inverse.len(), MESH_BLOCK_SIZE);
        assert!(table.inverse[0] > 10.0);
        assert_eq!(table.inverse[0] as usize, MESH_BLOCK_SIZE);
        assert_eq!((table.inverse[3], table.inverse[4]), (SIZE.0 as f32, SIZE.1 as f32));
        let (rx, ry) = cell_residual(&table, SIZE, 4.0 * SIZE.0 / 8.0, 4.0 * SIZE.1 / 8.0);
        assert!((rx - 2.0).abs() < 0.5, "x residual at the measured node: {rx}");
        assert!((ry + 1.0).abs() < 0.5, "y residual at the measured node: {ry}");
        // The fill reaches two cells out at most: far past it, the corner is exactly identity
        let (cx, cy) = cell_residual(&table, SIZE, 1.0, 1.0);
        assert!(cx.abs() < 1.0 && cy.abs() < 1.0, "corner far from identity: ({cx}, {cy})");
    }

    #[test]
    fn strength_and_cap_scale_the_block() {
        let mut cells: Vec<Option<(f64, f64)>> = vec![None; RESIDUAL_GRID_SIZE * RESIDUAL_GRID_SIZE];
        for y in 3..6 {
            for x in 3..6 {
                cells[y * RESIDUAL_GRID_SIZE + x] = Some((8.0, -4.0));
            }
        }
        let half = ResidualSettings { strength: 0.5, max_residual_px: 100.0, ..Default::default() };
        let table = mesh_table_from_field(&cells, SIZE, &half).unwrap();
        let (rx, ry) = cell_residual(&table, SIZE, 4.0 * SIZE.0 / 8.0, 4.0 * SIZE.1 / 8.0);
        assert!((rx - 4.0).abs() < 0.25 && (ry + 2.0).abs() < 0.25, "half strength: ({rx}, {ry})");
        let capped = ResidualSettings { strength: 1.0, max_residual_px: 3.0, ..Default::default() };
        let table = mesh_table_from_field(&cells, SIZE, &capped).unwrap();
        let (rx, _) = cell_residual(&table, SIZE, 4.0 * SIZE.0 / 8.0, 4.0 * SIZE.1 / 8.0);
        assert!(rx <= 3.0 + 0.25, "cap not applied: {rx}");
    }

    #[test]
    fn excluded_tracks_never_contribute() {
        // The editor contract: a track the user took out (not among the points) leaves no residual behind
        let mut obsv = Vec::new();
        for i in 0..40u32 {
            let ax = (i as f32 * 23.0) % 950.0 + 5.0;
            let ay = (i as f32 * 17.0) % 530.0 + 5.0;
            // Every point moved 3px in x beyond the global model
            obsv.push(obs(i, [ax, ay], [ax + 3.0, ay]));
        }
        // Only the first 20 points are in the measurement; the rest the user excluded
        let pts: Vec<ResidualPoint> = obsv.iter().enumerate().take(20).map(|(k, _)| ResidualPoint { k, band: 0, weight: 1.0 }).collect();
        let bands = vec![ResidualBand { m: Matrix3::identity(), rho: Vector3::zeros() }];
        let field = field_from_pair(0, 0, &obsv, &bands, &pts, &identity_quat, 0.0, 20.0, 1000.0, SIZE, 1e6);
        assert!(field.coverage() < 0.5, "excluded points contributed: coverage {}", field.coverage());
        assert!(field.cells.iter().filter(|c| c.points > 0).all(|c| (c.dx - 3.0).abs() < 1e-9));
        // With no point left at all, no table
        let none: Vec<Option<(f64, f64)>> = vec![None; RESIDUAL_GRID_SIZE * RESIDUAL_GRID_SIZE];
        assert!(mesh_table_from_field(&none, SIZE, &ResidualSettings::default()).is_none());
    }

    #[test]
    fn compose_fails_closed_on_a_camera_mesh() {
        // Where the camera has its own mesh (Sony IBIS), the residual never installs
        let camera = vec![9.0, 0.0, 0.0, 100.0, 100.0, 0.0, 0.0, 100.0, 100.0];
        let mut cells: Vec<Option<(f64, f64)>> = vec![None; RESIDUAL_GRID_SIZE * RESIDUAL_GRID_SIZE];
        cells[40] = Some((2.0, 0.0));
        let table = mesh_table_from_field(&cells, SIZE, &ResidualSettings::default()).unwrap();
        let residual = Some(ResidualMesh { tables: vec![table], frames: vec![(3, 0)], size: SIZE });
        let kept = compose_residual_mesh(camera.clone(), 3, &residual);
        assert_eq!(kept, camera);
        // Without a camera mesh and no residual for the frame: still empty
        assert!(compose_residual_mesh(Vec::new(), 4, &residual).is_empty());
        // With one for the frame: the block, plus the two probes the kernels read past it
        let installed = compose_residual_mesh(Vec::new(), 3, &residual);
        assert_eq!(installed.len(), MESH_BLOCK_SIZE + 4 + 1);
        assert_eq!(installed[0] as usize, MESH_BLOCK_SIZE);
        assert_eq!(installed[MESH_BLOCK_SIZE], 0.0);
        assert_eq!(installed[MESH_BLOCK_SIZE + 4], 0.0);
    }
}
