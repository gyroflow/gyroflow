// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow contributors

//! Compact local optical stabilization data.
//!
//! The optical analysis stores only a 9x9 displacement grid per corrected frame. Values are normalized to the
//! video dimensions and quantized to i16 with one scale for the whole analysis. Full spline tables are transient:
//! they are built when a frame is rendered. The inverse uses a few fixed-point iterations because these corrections
//! are deliberately small, avoiding the expensive general-purpose mesh inversion used for camera metadata.

use nalgebra::Vector2;

use super::{ sony::interpolate_mesh, splines, MESH_HEADER };

pub const OPTICAL_GRID: usize = 9;

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct OpticalResidualFrame {
    pub frame: u32,
    /// x,y pairs, normalized by the rendered video width/height and divided by scale.
    pub q: Vec<i16>,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct OpticalResidualCorrection {
    /// Common normalized displacement represented by one quantization unit.
    pub scale: f32,
    /// Temporal smoothing window used to make the stabilized local path, in seconds.
    pub smoothing_seconds: f32,
    pub frames: Vec<OpticalResidualFrame>,
}

impl OpticalResidualCorrection {
    pub fn from_normalized_frames(smoothing_seconds: f64, frames: Vec<(usize, Vec<[f32; 2]>)>) -> Self {
        let max = frames.iter().flat_map(|(_, g)| g.iter()).flat_map(|v| v.iter())
            .fold(0.0f32, |m, v| m.max(v.abs()));
        let scale = if max > 0.0 { max / i16::MAX as f32 } else { 1.0 };
        let frames = frames.into_iter().filter_map(|(frame, grid)| {
            if grid.len() != OPTICAL_GRID * OPTICAL_GRID { return None; }
            let q: Vec<i16> = grid.into_iter().flat_map(|v| {
                [(v[0] / scale).round() as i16, (v[1] / scale).round() as i16]
            }).collect();
            Some(OpticalResidualFrame { frame: frame as u32, q })
        }).collect();
        Self { scale, smoothing_seconds: smoothing_seconds as f32, frames }
    }

    pub fn is_empty(&self) -> bool { self.frames.is_empty() }
    pub fn has_frame(&self, frame: usize) -> bool { self.find(frame).is_some() }

    fn find(&self, frame: usize) -> Option<&OpticalResidualFrame> {
        self.frames.binary_search_by_key(&(frame as u32), |f| f.frame).ok().map(|i| &self.frames[i])
    }

    #[cfg(test)]
    pub(crate) fn normalized_grid(&self, frame: usize) -> Option<Vec<[f32; 2]>> {
        let f = self.find(frame)?;
        if f.q.len() != OPTICAL_GRID * OPTICAL_GRID * 2 || self.scale <= 0.0 { return None; }
        Some(f.q.chunks_exact(2).map(|v| [
            v[0] as f32 * self.scale,
            v[1] as f32 * self.scale,
        ]).collect())
    }

    fn displacements(&self, frame: usize, size: (usize, usize)) -> Option<Vec<[f64; 2]>> {
        let f = self.find(frame)?;
        if f.q.len() != OPTICAL_GRID * OPTICAL_GRID * 2 || self.scale <= 0.0 || size.0 == 0 || size.1 == 0 {
            return None;
        }
        Some(f.q.chunks_exact(2).map(|v| [
            v[0] as f64 * self.scale as f64 * size.0 as f64,
            v[1] as f64 * self.scale as f64 * size.1 as f64,
        ]).collect())
    }

    fn append_row_coefficients(mesh: &mut Vec<f64>, size_x: f64) {
        let n = OPTICAL_GRID;
        let mut a = [0.0; splines::MAX_GRID_SIZE];
        let mut b = [0.0; splines::MAX_GRID_SIZE];
        let mut c = [0.0; splines::MAX_GRID_SIZE];
        let mut d = [0.0; splines::MAX_GRID_SIZE];
        let mut alpha = [0.0; splines::MAX_GRID_SIZE - 1];
        let mut mu = [0.0; splines::MAX_GRID_SIZE];
        let mut z = [0.0; splines::MAX_GRID_SIZE];
        for mesh_offset in 0..=1 {
            for j in 0..n {
                splines::BivariateSpline::cubic_spline_coefficients(
                    &mesh[MESH_HEADER + mesh_offset..], 2, j * n, size_x, n,
                    &mut a, &mut b, &mut c, &mut d, &mut alpha, &mut mu, &mut z
                );
                mesh.extend_from_slice(&a);
                mesh.extend_from_slice(&b);
                mesh.extend_from_slice(&c);
                mesh.extend_from_slice(&d);
            }
        }
    }

    fn forward_block(&self, frame: usize, size: (usize, usize)) -> Option<Vec<f64>> {
        let disp = self.displacements(frame, size)?;
        let sz = (size.0 as f64, size.1 as f64);
        let step = (sz.0 / (OPTICAL_GRID - 1) as f64, sz.1 / (OPTICAL_GRID - 1) as f64);
        let mut mesh = Vec::with_capacity(MESH_HEADER + OPTICAL_GRID * OPTICAL_GRID * 2 + OPTICAL_GRID * splines::MAX_GRID_SIZE * 8);
        mesh.extend([0.0, OPTICAL_GRID as f64, OPTICAL_GRID as f64, sz.0, sz.1, 0.0, 0.0, sz.0, sz.1]);
        for y in 0..OPTICAL_GRID {
            for x in 0..OPTICAL_GRID {
                let d = disp[y * OPTICAL_GRID + x];
                mesh.push(step.0 * x as f64 + d[0]);
                mesh.push(step.1 * y as f64 + d[1]);
            }
        }
        Self::append_row_coefficients(&mut mesh, sz.0);
        mesh[0] = mesh.len() as f64;
        Some(mesh)
    }

    fn inverse_block(forward: &[f64], size: (usize, usize)) -> Option<Vec<f64>> {
        if size.0 == 0 || size.1 == 0 { return None; }
        let sz = (size.0 as f64, size.1 as f64);
        let step = (sz.0 / (OPTICAL_GRID - 1) as f64, sz.1 / (OPTICAL_GRID - 1) as f64);
        let mut inv = Vec::with_capacity(forward.len());
        inv.extend([0.0, OPTICAL_GRID as f64, OPTICAL_GRID as f64, sz.0, sz.1, 0.0, 0.0, sz.0, sz.1]);
        for y in 0..OPTICAL_GRID {
            for x in 0..OPTICAL_GRID {
                let q = Vector2::new(step.0 * x as f64, step.1 * y as f64);
                let mut p = q;
                for _ in 0..5 {
                    let fp = interpolate_mesh(p.x, p.y, sz, forward);
                    let e = q - fp;
                    p += e;
                    if e.norm_squared() < 1e-10 { break; }
                }
                if !p.x.is_finite() || !p.y.is_finite() { return None; }
                inv.push(p.x);
                inv.push(p.y);
            }
        }
        Self::append_row_coefficients(&mut inv, sz.0);
        inv[0] = inv.len() as f64;
        Some(inv)
    }

    /// GPU mesh buffer: inverse lookup, empty focal-plane table, then forward mesh for refinement.
    pub fn kernel_buffer(&self, frame: usize, size: (usize, usize)) -> Option<Vec<f32>> {
        let forward = self.forward_block(frame, size)?;
        let inverse = Self::inverse_block(&forward, size)?;
        let mut out = Vec::with_capacity(inverse.len() + 4 + forward.len());
        out.extend(inverse.iter().map(|v| *v as f32));
        out.extend([0.0; 4]);
        out.extend(forward.iter().map(|v| *v as f32));
        Some(out)
    }

    /// CPU/point-path mesh: raw -> corrected, plus an empty focal-plane table.
    pub fn forward_mesh(&self, frame: usize, size: (usize, usize)) -> Option<Vec<f64>> {
        let mut out = self.forward_block(frame, size)?;
        out.extend([0.0; 4]);
        Some(out)
    }

    pub fn hash_into(&self, hasher: &mut impl std::hash::Hasher) {
        hasher.write_u32(self.scale.to_bits());
        hasher.write_u32(self.smoothing_seconds.to_bits());
        hasher.write_usize(self.frames.len());
        for f in self.frames.iter().step_by((self.frames.len() / 256).max(1)) {
            hasher.write_u32(f.frame);
            for v in f.q.iter().step_by((f.q.len() / 32).max(1)) { hasher.write_i16(*v); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stabilization::{distortion_models::DistortionModel, KernelParams, Stabilization};

    #[test]
    fn compact_roundtrip_and_inverse_are_consistent() {
        let mut grid = vec![[0.0f32; 2]; OPTICAL_GRID * OPTICAL_GRID];
        for y in 0..OPTICAL_GRID {
            for x in 0..OPTICAL_GRID {
                let i = y * OPTICAL_GRID + x;
                grid[i] = [0.002 * (x as f32 / 8.0 - 0.5), -0.0015 * (y as f32 / 8.0 - 0.5)];
            }
        }
        let r = OpticalResidualCorrection::from_normalized_frames(0.35, vec![(7, grid)]);
        assert!(r.has_frame(7));
        assert!(!r.has_frame(6));
        let size = (1920, 1080);
        let fwd = r.forward_block(7, size).unwrap();
        let inv = OpticalResidualCorrection::inverse_block(&fwd, size).unwrap();
        let sz = (size.0 as f64, size.1 as f64);
        for y in 0..7 {
            for x in 0..7 {
                let q = Vector2::new(sz.0 * x as f64 / 6.0, sz.1 * y as f64 / 6.0);
                let p = interpolate_mesh(q.x, q.y, sz, &inv);
                let q2 = interpolate_mesh(p.x, p.y, sz, &fwd);
                assert!((q2 - q).norm() < 0.02, "{q:?} -> {q2:?}");
            }
        }
    }

    #[test]
    fn render_path_composes_camera_and_optical_mesh_in_inverse_direction() {
        let size = (1920usize, 1080usize);

        // An identity camera mesh exercises the normal metadata-mesh branch. The local optical grid is a
        // constant raw->corrected displacement, so render inversion must sample by the opposite displacement.
        let zero = vec![[0.0f32; 2]; OPTICAL_GRID * OPTICAL_GRID];
        let camera = OpticalResidualCorrection::from_normalized_frames(0.35, vec![(3, zero)])
            .kernel_buffer(3, size).unwrap();

        let dx = 10.0f32 / size.0 as f32;
        let dy = -6.0f32 / size.1 as f32;
        let optical_grid = vec![[dx, dy]; OPTICAL_GRID * OPTICAL_GRID];
        let optical = OpticalResidualCorrection::from_normalized_frames(0.35, vec![(3, optical_grid)])
            .kernel_buffer(3, size).unwrap();

        let mut mesh: Vec<f64> = camera.iter().map(|v| *v as f64).collect();
        let optical_offset = mesh.len();
        mesh.extend(optical.iter().map(|v| *v as f64));

        let mut params = KernelParams {
            width: size.0 as i32,
            height: size.1 as i32,
            f: [1000.0, 1000.0],
            c: [size.0 as f32 / 2.0, size.1 as f32 / 2.0],
            flags: 512 | 4096, // camera metadata mesh + appended optical residual
            reserved2: optical_offset as f32,
            ..Default::default()
        };
        // Zero means "not configured" for these stretch fields in the render path, so leave both at zero.
        params.field_limit = 0.0;

        let identity = [[
            1.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
            0.0, 0.0, 1.0,
            0.0, 0.0, 0.0,
            0.0, 0.0,
            0.0, 0.0,
        ]];
        let p = Stabilization::rotate_and_distort(
            (0.0, 0.0),
            0,
            &params,
            &identity,
            &DistortionModel::default(),
            None,
            &mesh,
        ).unwrap();

        assert!((p.0 - (size.0 as f32 / 2.0 - 10.0)).abs() < 0.15, "x={}", p.0);
        assert!((p.1 - (size.1 as f32 / 2.0 + 6.0)).abs() < 0.15, "y={}", p.1);
    }

    #[test]
    fn render_path_tracks_multiple_temporal_frequencies() {
        let size = (1920usize, 1080usize);
        let fps = 60.0f32;
        let amp_px = 7.0f32;
        let identity = [[
            1.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
            0.0, 0.0, 1.0,
            0.0, 0.0, 0.0,
            0.0, 0.0,
            0.0, 0.0,
        ]];

        for hz in [0.5f32, 2.0, 8.0] {
            let frames = (0..120usize).map(|frame| {
                let t = frame as f32 / fps;
                let dx_px = amp_px * (std::f32::consts::TAU * hz * t).sin();
                let grid = vec![[dx_px / size.0 as f32, 0.0]; OPTICAL_GRID * OPTICAL_GRID];
                (frame, grid)
            }).collect();
            let residual = OpticalResidualCorrection::from_normalized_frames(0.35, frames);

            for frame in [7usize, 19, 41, 73, 101] {
                let t = frame as f32 / fps;
                let expected_dx = amp_px * (std::f32::consts::TAU * hz * t).sin();
                let mesh: Vec<f64> = residual.kernel_buffer(frame, size).unwrap()
                    .into_iter().map(|v| v as f64).collect();
                let params = KernelParams {
                    width: size.0 as i32,
                    height: size.1 as i32,
                    f: [1000.0, 1000.0],
                    c: [size.0 as f32 / 2.0, size.1 as f32 / 2.0],
                    flags: 4096,
                    ..Default::default()
                };
                let p = Stabilization::rotate_and_distort(
                    (0.0, 0.0), 0, &params, &identity, &DistortionModel::default(), None, &mesh,
                ).unwrap();
                let expected_x = size.0 as f32 / 2.0 - expected_dx;
                assert!(
                    (p.0 - expected_x).abs() < 0.2,
                    "hz={hz} frame={frame} expected x={expected_x}, got {}",
                    p.0
                );
            }
        }
    }
}
