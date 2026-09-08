// SPDX-License-Identifier: GPL-3.0-or-later

//! Residual optical stabilization in the camera-stabilized image plane.
//!
//! Tracks stay in normalized source coordinates so changing a lens profile or
//! camera smoothing does not require decoding again. A spatially regularized
//! mesh describes only the motion left after the camera transform. Its vertex
//! paths are smoothed independently within continuous, well-supported segments.
//! The forward map is shared by crop estimation and the inverse render kernels.

use super::{ComputeParams, undistort_points_with_rolling_shutter};
use nalgebra::{Matrix3, Vector3};
use std::{collections::BTreeMap, sync::Arc};

pub const COLS: usize = 9;
pub const ROWS: usize = 7;
pub const NODES: usize = COLS * ROWS;
pub const BUFFER_LEN: usize = NODES * 2;
const MAX_TRACKS: usize = 256;
const MAX_DISPLACEMENT: f64 = 0.04;
const MAX_GRADIENT: f64 = 0.2;
const WINDOW_SECONDS: f64 = 0.5;

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MotionData {
    pub frames: Vec<MotionFrame>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MotionFrame {
    /// Original decoded video time, before frame-rate override or sensor offsets.
    pub timestamp_us: i64,
    pub next_timestamp_us: i64,
    /// Source x/y, destination x/y, each divided by the corresponding image size.
    pub points: Vec<[f32; 4]>,
}

impl MotionFrame {
    pub fn new(
        timestamp_us: i64,
        next_timestamp_us: i64,
        size: (u32, u32),
        pairs: &crate::synchronization::OpticalFlowPair,
    ) -> Self {
        let mut frame = Self {
            timestamp_us,
            next_timestamp_us,
            points: Vec::new(),
        };
        if size.0 == 0 || size.1 == 0 {
            return frame;
        }
        // Keep at most one observation per cell. A textured foreground object
        // must not outweigh a broad, less textured background by point count.
        let mut cells = [false; MAX_TRACKS];
        if let Some((a, b)) = pairs {
            for (&a, &b) in a.iter().zip(b) {
                let p = [
                    a.0 / size.0 as f32,
                    a.1 / size.1 as f32,
                    b.0 / size.0 as f32,
                    b.1 / size.1 as f32,
                ];
                if !p.iter().all(|x| x.is_finite() && (0.0..1.0).contains(x)) {
                    continue;
                }
                let cell = (p[1] * 16.0) as usize * 16 + (p[0] * 16.0) as usize;
                if !cells[cell] {
                    frame.points.push(p);
                    cells[cell] = true;
                }
            }
        }
        frame
    }
}

impl MotionData {
    pub fn is_valid(&self) -> bool {
        self.frames
            .windows(2)
            .all(|w| w[0].timestamp_us < w[1].timestamp_us)
            && self.frames.iter().all(|f| {
                f.timestamp_us >= 0
                    && f.next_timestamp_us > f.timestamp_us
                    && f.points.len() <= MAX_TRACKS
                    && f.points
                        .iter()
                        .flatten()
                        .all(|v| v.is_finite() && (0.0..1.0).contains(v))
            })
    }
}

#[derive(Clone, Debug)]
pub struct Grid(pub [[f32; 2]; NODES]);

impl Default for Grid {
    fn default() -> Self {
        Self([[0.0; 2]; NODES])
    }
}

impl Grid {
    pub fn sample(&self, point: [f32; 2]) -> [f32; 2] {
        let x = point[0].clamp(0.0, 1.0) * (COLS - 1) as f32;
        let y = point[1].clamp(0.0, 1.0) * (ROWS - 1) as f32;
        let ix = (x as usize).min(COLS - 2);
        let iy = (y as usize).min(ROWS - 2);
        let (ax, ay) = (x - ix as f32, y - iy as f32);
        std::array::from_fn(|c| {
            let top = self.0[iy * COLS + ix][c] * (1.0 - ax) + self.0[iy * COLS + ix + 1][c] * ax;
            let bottom = self.0[(iy + 1) * COLS + ix][c] * (1.0 - ax)
                + self.0[(iy + 1) * COLS + ix + 1][c] * ax;
            top * (1.0 - ay) + bottom * ay
        })
    }

    pub fn forward(&self, point: [f32; 2]) -> [f32; 2] {
        let d = self.sample(point);
        [point[0] + d[0], point[1] + d[1]]
    }

    pub fn inverse(&self, point: [f32; 2]) -> [f32; 2] {
        let mut p = point;
        // bound() makes this a contraction, including at the clamped border.
        for _ in 0..8 {
            let d = self.sample(p);
            p = [point[0] - d[0], point[1] - d[1]];
        }
        p
    }

    fn bound(&mut self) {
        let mut scale = 1.0_f64;
        for (i, p) in self.0.iter().enumerate() {
            for &v in p {
                scale = scale.min(MAX_DISPLACEMENT / (v as f64).abs().max(1e-12));
            }
            // Bound the infinity norm of the Jacobian of the displacement.
            // Its two directional contributions together stay below 0.2.
            for (j, divisions) in [(i + 1, COLS - 1), (i + COLS, ROWS - 1)] {
                if j >= NODES || (j == i + 1 && i % COLS == COLS - 1) {
                    continue;
                }
                for c in 0..2 {
                    let derivative = (self.0[j][c] - p[c]).abs() as f64 * divisions as f64;
                    scale = scale.min(MAX_GRADIENT / 2.0 / derivative.max(1e-12));
                }
            }
        }
        for p in &mut self.0 {
            for v in p {
                *v *= scale as f32;
            }
        }
    }
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut v: Vec<_> = values.filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return 0.0;
    }
    let i = v.len() / 2;
    *v.select_nth_unstable_by(i, f64::total_cmp).1
}

/// Fit a background affine field, then retain a smooth local residual. The
/// robust fit rejects independently moving subjects; coverage gates prevent a
/// small surviving patch from defining motion for the whole image.
fn fit_motion(points: &[[f64; 4]]) -> Option<[[f64; 2]; NODES]> {
    let points: Vec<_> = points
        .iter()
        .copied()
        .filter(|p| {
            p.iter().all(|x| x.is_finite())
                && (0.0..=1.0).contains(&p[0])
                && (0.0..=1.0).contains(&p[1])
        })
        .collect();
    if points.len() < 12 {
        return None;
    }
    let mut model = [
        Vector3::new(0.0, 0.0, median(points.iter().map(|p| p[2] - p[0]))),
        Vector3::new(0.0, 0.0, median(points.iter().map(|p| p[3] - p[1]))),
    ];
    let predict = |m: &[Vector3<f64>; 2], p: &[f64; 4]| {
        let x = Vector3::new(p[0] - 0.5, p[1] - 0.5, 1.0);
        [m[0].dot(&x), m[1].dot(&x)]
    };
    let error = |m: &[Vector3<f64>; 2], p: &[f64; 4]| {
        let d = predict(m, p);
        (p[2] - p[0] - d[0]).hypot(p[3] - p[1] - d[1])
    };
    for _ in 0..6 {
        let sigma = median(points.iter().map(|p| error(&model, p))).max(0.0015);
        let mut normal = Matrix3::zeros();
        let mut rhs = [Vector3::zeros(); 2];
        for p in &points {
            let e = error(&model, p) / (3.0 * sigma);
            let weight = (1.0 - e * e).max(0.0).powi(2);
            let x = Vector3::new(p[0] - 0.5, p[1] - 0.5, 1.0);
            normal += weight * x * x.transpose();
            for c in 0..2 {
                rhs[c] += weight * x * (p[c + 2] - p[c]);
            }
        }
        let factor = normal.cholesky()?;
        model = [factor.solve(&rhs[0]), factor.solve(&rhs[1])];
    }
    let limit = (3.0 * median(points.iter().map(|p| error(&model, p)))).clamp(0.002, 0.025);
    let inliers: Vec<_> = points
        .iter()
        .filter(|p| error(&model, p) <= limit)
        .collect();
    let mut coverage = [false; 12];
    for p in &inliers {
        coverage[((p[1] * 3.0) as usize).min(2) * 4 + ((p[0] * 4.0) as usize).min(3)] = true;
    }
    if inliers.len() < 12
        || inliers.len() * 2 < points.len()
        || coverage.iter().filter(|&&v| v).count() < 6
    {
        return None;
    }
    let mut field = [[0.0; 2]; NODES];
    for (i, out) in field.iter_mut().enumerate() {
        let p = [
            (i % COLS) as f64 / (COLS - 1) as f64,
            (i / COLS) as f64 / (ROWS - 1) as f64,
            0.0,
            0.0,
        ];
        let base = predict(&model, &p);
        let mut residual = [0.0; 2];
        let mut weight_sum = 2.0; // Shrink weakly observed local motion toward the background field.
        for q in &inliers {
            let distance = ((p[0] - q[0]) * (COLS - 1) as f64).powi(2)
                + ((p[1] - q[1]) * (ROWS - 1) as f64).powi(2);
            let w = (-distance / 2.0).exp();
            let predicted = predict(&model, q);
            for c in 0..2 {
                residual[c] += w * (q[c + 2] - q[c] - predicted[c]);
            }
            weight_sum += w;
        }
        for c in 0..2 {
            out[c] = base[c] + residual[c] / weight_sum;
        }
    }
    field
        .iter()
        .flatten()
        .all(|v| v.is_finite() && v.abs() < 0.2)
        .then_some(field)
}

fn smooth_segment(
    times: &[i64],
    paths: &[[[f64; 2]; NODES]],
    strength: f64,
    output: &mut BTreeMap<i64, Grid>,
) {
    for (i, &time) in times.iter().enumerate() {
        let mut sums = [0.0; 3];
        let mut values = [[[0.0; 2]; NODES]; 2];
        let start =
            times.partition_point(|&t| t < time.saturating_sub((WINDOW_SECONDS * 1e6) as i64));
        for j in start..times.len() {
            let dt = (times[j] - time) as f64 / 1e6;
            if dt > WINDOW_SECONDS {
                break;
            }
            let w = (1.0 - (dt / WINDOW_SECONDS).powi(2)).max(0.0).powi(2);
            sums[0] += w;
            sums[1] += w * dt;
            sums[2] += w * dt * dt;
            for n in 0..NODES {
                for c in 0..2 {
                    values[0][n][c] += w * paths[j][n][c];
                    values[1][n][c] += w * dt * paths[j][n][c];
                }
            }
        }
        let det = sums[0] * sums[2] - sums[1] * sums[1];
        let mut grid = Grid::default();
        if det > 1e-12 {
            for n in 0..NODES {
                for c in 0..2 {
                    // A local linear fit preserves deliberate pans, including at clip ends.
                    let smooth = (values[0][n][c] * sums[2] - values[1][n][c] * sums[1]) / det;
                    grid.0[n][c] = ((smooth - paths[i][n][c]) * strength) as f32;
                }
            }
        }
        let distance_to_boundary =
            (time - times[0]).min(times[times.len() - 1] - time) as f64 / 1e6;
        let blend = (distance_to_boundary / 0.15).clamp(0.0, 1.0);
        let blend = blend * blend * (3.0 - 2.0 * blend);
        for vertex in &mut grid.0 {
            for value in vertex {
                *value *= blend as f32;
            }
        }
        grid.bound();
        output.insert(time, grid);
    }
}

pub fn prepare(params: &mut ComputeParams) {
    params.optical_corrections = Arc::default();
    let strength = params.optical_stabilization_strength;
    if !strength.is_finite()
        || strength <= 0.0
        || params.optical_motion.frames.is_empty()
        || !params.optical_motion.is_valid()
    {
        return;
    }
    let mut canonical = params.clone();
    canonical.output_width = canonical.width;
    canonical.output_height = canonical.height;
    canonical.framebuffer_inverted = false;
    let mut corrections = BTreeMap::new();
    let mut times = Vec::new();
    let mut paths = Vec::new();
    let mut path = [[0.0; 2]; NODES];
    let mut expected = None;
    let time_scale = if params.fps_scale.is_finite() && params.fps_scale > 0.0 {
        params.fps_scale
    } else {
        1.0
    };
    for frame in &params.optical_motion.frames {
        let timestamp_us = (frame.timestamp_us as f64 / time_scale).round() as i64;
        let next_timestamp_us = (frame.next_timestamp_us as f64 / time_scale).round() as i64;
        let project = |offset: usize, time: i64| {
            let points: Vec<_> = frame
                .points
                .iter()
                .map(|p| {
                    (
                        p[offset] * canonical.width as f32,
                        p[offset + 1] * canonical.height as f32,
                    )
                })
                .collect();
            undistort_points_with_rolling_shutter(
                &points,
                time as f64 / 1000.0,
                None,
                &canonical,
                canonical
                    .keyframes
                    .value_at_video_timestamp(
                        &crate::keyframes::KeyframeType::LensCorrectionStrength,
                        time as f64 / 1000.0,
                    )
                    .unwrap_or(canonical.lens_correction_amount),
                false,
                false,
            )
        };
        let a = project(0, timestamp_us);
        let b = project(2, next_timestamp_us);
        let points: Vec<_> = a
            .iter()
            .zip(&b)
            .filter(|(a, b)| super::is_valid_point(**a) && super::is_valid_point(**b))
            .map(|(a, b)| {
                [
                    a.0 as f64 / canonical.width as f64,
                    a.1 as f64 / canonical.height as f64,
                    b.0 as f64 / canonical.width as f64,
                    b.1 as f64 / canonical.height as f64,
                ]
            })
            .collect();
        let field = fit_motion(&points).filter(|_| {
            (next_timestamp_us - timestamp_us) as f64 <= 1_500_000.0 / params.scaled_fps.max(1.0)
        });
        if expected != Some(timestamp_us) || field.is_none() {
            smooth_segment(&times, &paths, strength.clamp(0.0, 1.0), &mut corrections);
            times.clear();
            paths.clear();
            path = [[0.0; 2]; NODES];
        }
        if let Some(field) = field {
            if times.is_empty() {
                times.push(timestamp_us);
                paths.push(path);
            }
            for n in 0..NODES {
                for c in 0..2 {
                    path[n][c] += field[n][c];
                }
            }
            times.push(next_timestamp_us);
            paths.push(path);
            expected = Some(next_timestamp_us);
        } else {
            expected = None;
            corrections.insert(timestamp_us, Grid::default());
            corrections.insert(next_timestamp_us, Grid::default());
        }
    }
    smooth_segment(&times, &paths, strength.clamp(0.0, 1.0), &mut corrections);
    params.optical_corrections = Arc::new(corrections);
}

pub fn grid_at(params: &ComputeParams, timestamp_ms: f64) -> Option<&Grid> {
    if !timestamp_ms.is_finite() {
        return None;
    }
    let timestamp = (timestamp_ms * 1000.0).round() as i64;
    let before = params.optical_corrections.range(..=timestamp).next_back();
    let after = params.optical_corrections.range(timestamp..).next();
    let closest = match (before, after) {
        (Some(a), Some(b)) => {
            if a.0.abs_diff(timestamp) <= b.0.abs_diff(timestamp) {
                a
            } else {
                b
            }
        }
        (Some(a), None) | (None, Some(a)) => a,
        _ => return None,
    };
    // Only a decoded frame's own correction, never a field held across missing data.
    (closest.0.abs_diff(timestamp) < (500_000.0 / params.scaled_fps.max(1.0)) as u64
        && closest.1.0.iter().flatten().any(|&value| value != 0.0))
    .then_some(closest.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(motion: impl Fn(f64, f64) -> [f64; 2]) -> Vec<[f64; 4]> {
        (0..12)
            .flat_map(|y| (0..16).map(move |x| ((x as f64 + 0.5) / 16.0, (y as f64 + 0.5) / 12.0)))
            .map(|(x, y)| {
                let d = motion(x, y);
                [x, y, x + d[0], y + d[1]]
            })
            .collect()
    }

    #[test]
    fn moving_foreground_does_not_drive_background() {
        let field = fit_motion(&tracks(|x, y| {
            if x > 0.3 && x < 0.7 && y > 0.2 && y < 0.8 {
                [0.1, -0.06]
            } else {
                [0.005, -0.003]
            }
        }))
        .unwrap();
        for p in field {
            assert!((p[0] - 0.005).abs() < 0.0001);
            assert!((p[1] + 0.003).abs() < 0.0001);
        }
    }

    #[test]
    fn local_motion_is_spatially_varying_and_invertible() {
        let field = fit_motion(&tracks(|x, y| [0.01 * (x * 3.0).sin() * y, 0.003 * x])).unwrap();
        assert!((field[NODES - 1][0] - field[0][0]).abs() > 0.001);
        let mut grid = Grid(std::array::from_fn(|i| {
            [field[i][0] as f32 * 50.0, field[i][1] as f32 * 50.0]
        }));
        grid.bound();
        for y in -1..12 {
            for x in -1..12 {
                let p = [x as f32 / 10.0, y as f32 / 10.0];
                let back = grid.inverse(grid.forward(p));
                assert!((p[0] - back[0]).abs() < 1e-6 && (p[1] - back[1]).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn sparse_or_degenerate_support_is_rejected() {
        assert!(fit_motion(&tracks(|_, _| [0.0, 0.0])[..8]).is_none());
        assert!(fit_motion(&vec![[0.5, 0.5, 0.51, 0.5]; 50]).is_none());
    }

    #[test]
    fn smoothing_removes_jitter_and_preserves_pans_at_boundaries() {
        let times: Vec<_> = (0..90).map(|i| i * 33_333).collect();
        let paths: Vec<_> = (0..90).map(|i| [[i as f64 * 0.001, 0.0]; NODES]).collect();
        let mut output = BTreeMap::new();
        smooth_segment(&times, &paths, 1.0, &mut output);
        assert!(
            output
                .values()
                .flat_map(|g| g.0.iter().flatten())
                .all(|x| x.abs() < 1e-6)
        );
        let paths: Vec<_> = (0..90)
            .map(|i| [[(i as f64 * 1.7).sin() * 0.01, 0.0]; NODES])
            .collect();
        smooth_segment(&times, &paths, 1.0, &mut output);
        let residual: f64 = output
            .values()
            .zip(&paths)
            .skip(15)
            .take(60)
            .map(|(g, p)| (g.0[0][0] as f64 + p[0][0]).powi(2))
            .sum();
        assert!(residual < 0.00005);
        assert!(output[&times[0]].0.iter().flatten().all(|&v| v == 0.0));
        assert!(
            output[times.last().unwrap()]
                .0
                .iter()
                .flatten()
                .all(|&v| v == 0.0)
        );
    }

    #[test]
    fn missing_tracking_breaks_paths_and_does_not_hold_a_warp_across_a_cut() {
        let mut p = ComputeParams::default();
        p.width = 640;
        p.height = 360;
        p.output_width = 640;
        p.output_height = 360;
        p.scaled_fps = 30.0;
        p.fov_scale = 1.0;
        p.lens_correction_amount = 1.0;
        p.optical_stabilization_strength = 1.0;
        p.distortion_model =
            super::super::distortion_models::DistortionModel::from_name("opencv_standard");
        p.lens.calib_dimension = crate::lens_profile::Dimensions { w: 640, h: 360 };
        p.lens.fisheye_params.camera_matrix =
            vec![[480.0, 0.0, 320.0], [0.0, 480.0, 180.0], [0.0, 0.0, 1.0]];
        p.optical_motion = Arc::new(MotionData {
            frames: (0..60)
                .map(|i| {
                    let points = if i == 29 {
                        Vec::new()
                    } else {
                        tracks(|_, _| [0.005 * (i as f64 * 1.7).sin(), 0.0])
                            .into_iter()
                            .map(|p| p.map(|v| v as f32))
                            .collect()
                    };
                    MotionFrame {
                        timestamp_us: i * 33_333,
                        next_timestamp_us: (i + 1) * 33_333,
                        points,
                    }
                })
                .collect(),
        });
        prepare(&mut p);
        for frame in [0, 29, 30, 60] {
            assert!(
                p.optical_corrections[&(frame * 33_333)]
                    .0
                    .iter()
                    .flatten()
                    .all(|&v| v == 0.0)
            );
        }
        assert!(
            p.optical_corrections
                .values()
                .any(|g| g.0.iter().flatten().any(|v| v.abs() > 0.001))
        );
        assert!(grid_at(&p, 3000.0).is_none());
        p.fps_scale = 2.0;
        p.scaled_fps = 60.0;
        prepare(&mut p);
        assert_eq!(*p.optical_corrections.last_key_value().unwrap().0, 999_990);
        assert!(
            p.optical_corrections[&499_995]
                .0
                .iter()
                .flatten()
                .all(|&v| v == 0.0)
        );
        assert!(grid_at(&p, f64::NAN).is_none());
    }

    #[test]
    fn project_import_restores_tracks_and_loading_an_older_project_clears_them() {
        let data = MotionData {
            frames: vec![MotionFrame::new(
                0,
                33_333,
                (100, 100),
                &Some((vec![(20.0, 30.0)], vec![(22.0, 29.0)])),
            )],
        };
        let mut project = serde_json::json!({"version":4, "videofile":"file:///optical-test-video.mp4", "optical_motion":crate::util::compress_to_base91_cbor(&data).unwrap(), "stabilization":{"optical_stabilization_strength":0.8}});
        let manager = crate::StabilizationManager::default();
        let mut is_preset = false;
        manager
            .import_gyroflow_data(
                &serde_json::to_vec(&project).unwrap(),
                false,
                None,
                |_| (),
                Arc::default(),
                &mut is_preset,
                false,
            )
            .unwrap();
        assert!(!is_preset);
        assert_eq!(
            manager.params.read().optical_motion.frames[0].points,
            data.frames[0].points
        );
        assert_eq!(manager.params.read().optical_stabilization_strength, 0.8);
        project.as_object_mut().unwrap().remove("optical_motion");
        project["stabilization"]
            .as_object_mut()
            .unwrap()
            .remove("optical_stabilization_strength");
        manager
            .import_gyroflow_data(
                &serde_json::to_vec(&project).unwrap(),
                false,
                None,
                |_| (),
                Arc::default(),
                &mut is_preset,
                false,
            )
            .unwrap();
        assert!(manager.params.read().optical_motion.frames.is_empty());
        assert_eq!(manager.params.read().optical_stabilization_strength, 0.0);
    }

    #[test]
    fn rendered_coordinates_agree_with_the_crop_map_at_different_sizes() {
        use super::super::{
            FrameTransform, RGB8, Stabilization, distortion_models::DistortionModel,
        };
        use crate::gpu::{BufferDescription, BufferSource, Buffers};
        for (w, h, ow, oh, fov) in [
            (128, 80, 128, 80, 1.0),
            (128, 80, 96, 64, 0.8),
            (80, 128, 64, 96, 0.9),
        ] {
            let mut p = ComputeParams::default();
            p.width = w;
            p.height = h;
            p.output_width = ow;
            p.output_height = oh;
            p.scaled_fps = 30.0;
            p.fov_scale = fov;
            p.lens_correction_amount = 1.0;
            p.distortion_model = DistortionModel::from_name("opencv_standard");
            p.lens.calib_dimension = crate::lens_profile::Dimensions { w, h };
            p.lens.fisheye_params.camera_matrix = vec![
                [100.0, 0.0, w as f64 / 2.0],
                [0.0, 100.0, h as f64 / 2.0],
                [0.0, 0.0, 1.0],
            ];
            let grid = Grid(std::array::from_fn(|i| {
                [0.008 + (i / COLS) as f32 * 0.0005, -0.007]
            }));
            p.optical_corrections = Arc::new(BTreeMap::from([(0, grid.clone())]));
            let transform = FrameTransform::at_timestamp(&p, 0.0, 0);
            let mut k = transform.kernel_params;
            k.width = w as i32;
            k.height = h as i32;
            k.output_width = ow as i32;
            k.output_height = oh as i32;
            k.stride = (w * 3) as i32;
            k.output_stride = (ow * 3) as i32;
            k.bytes_per_pixel = 3;
            k.pix_element_count = 3;
            k.max_pixel_value = 255.0;
            k.pixel_value_limit = 255.0;
            k.source_rect = [0, 0, w as i32, h as i32];
            k.output_rect = [0, 0, ow as i32, oh as i32];
            let mut input: Vec<u8> = (0..h)
                .flat_map(|y| (0..w).flat_map(move |x| [x as u8, y as u8, 100]))
                .collect();
            let mut output = vec![0; ow * oh * 3];
            let mut buffers = Buffers {
                input: BufferDescription {
                    size: (w, h, w * 3),
                    data: BufferSource::Cpu { buffer: &mut input },
                    ..Default::default()
                },
                output: BufferDescription {
                    size: (ow, oh, ow * 3),
                    data: BufferSource::Cpu {
                        buffer: &mut output,
                    },
                    ..Default::default()
                },
            };
            assert!(Stabilization::undistort_image_cpu::<2, RGB8>(
                &mut buffers,
                &k,
                &p.distortion_model,
                None,
                &transform.matrices,
                &[],
                &transform.mesh_data
            ));
            for (x, y) in [(ow / 3, oh / 3), (ow / 2, oh / 2), (ow * 2 / 3, oh * 2 / 3)] {
                let i = (y * ow + x) * 3;
                let source = (output[i] as f32, output[i + 1] as f32);
                let back = undistort_points_with_rolling_shutter(
                    &[source],
                    0.0,
                    Some(0),
                    &p,
                    1.0,
                    true,
                    false,
                )[0];
                assert!(
                    (back.0 - x as f32).abs() < 2.0 && (back.1 - y as f32).abs() < 2.0,
                    "{w}x{h} -> {ow}x{oh}: ({x}, {y}) -> {source:?} -> {back:?}"
                );
            }
        }
    }

    #[test]
    fn compressed_motion_round_trip_retains_gaps_and_rejects_invalid_coordinates() {
        let mut data = MotionData {
            frames: vec![
                MotionFrame::new(
                    0,
                    33_333,
                    (100, 100),
                    &Some((vec![(20.0, 30.0)], vec![(22.0, 29.0)])),
                ),
                MotionFrame::new(33_333, 66_666, (100, 100), &None),
            ],
        };
        let packed = crate::util::compress_to_base91_cbor(&data).unwrap();
        let restored: MotionData = crate::util::decompress_from_base91_cbor(&packed).unwrap();
        assert!(restored.is_valid());
        assert_eq!(restored.frames[0].points, data.frames[0].points);
        assert!(restored.frames[1].points.is_empty());
        data.frames[0].points[0][0] = f32::NAN;
        assert!(!data.is_valid());
    }
}

#[cfg(test)]
#[path = "optical_gpu_tests.rs"]
mod gpu_tests;
