// SPDX-License-Identifier: GPL-3.0-or-later

//! Local correction after the camera/lens projection. All fields use normalized
//! output coordinates; they are independent of the decoder and render resolution.
use crate::stabilization::{ ComputeParams, undistort_points_with_rolling_shutter };
use rayon::prelude::*;

pub const COLS: usize = 9;
pub const ROWS: usize = 7;
pub const NODES: usize = COLS * ROWS;
pub const BUFFER_FLOATS: usize = NODES * 2;
pub const MAX_DISPLACEMENT: f32 = 0.04;
pub const MAX_KERNEL_BUFFER: usize = crate::gyro_source::splines::MAX_BUFFER_SIZE + 10 + BUFFER_FLOATS;
type Point = [f32; 2];

fn zooming_enabled(params: &ComputeParams) -> bool {
    params.adaptive_zoom_window < -0.9 || params.adaptive_zoom_window > 0.0001
}

pub fn zoom_reserve_factor(params: &ComputeParams) -> f64 {
    if zooming_enabled(params) && params.optical_stabilization &&
        params.optical_motion.as_ref().is_some_and(|d| d.complete && d.version == 1 && !d.pairs.is_empty()) {
        1.0 + 2.0 * MAX_DISPLACEMENT as f64
    } else { 1.0 }
}

/// Reserve only the magnification left after the camera projection, explicit
/// FOV and focal-length compensation. This does not override a user crop or an
/// already-over-limit result from the existing approximate camera smoother.
pub fn crop_margins(params: &ComputeParams) -> Vec<f32> {
    if params.frame_count == 0 || params.width == 0 || params.output_width == 0 ||
        !params.scaled_fps.is_finite() || params.scaled_fps <= 0.0 { return Vec::new(); }
    if !zooming_enabled(params) || !params.optical_stabilization ||
        !params.optical_motion.as_ref().is_some_and(|d| d.complete && d.version == 1 && !d.pairs.is_empty()) {
        return Vec::new();
    }
    let ratio = params.width as f64 / params.output_width.max(1) as f64;
    let mut margins: Vec<f32> = (0..params.frame_count).map(|frame| {
        let timestamp = crate::timestamp_at_frame(frame as i32, params.scaled_fps);
        let fov_scale = params.keyframes.value_at_video_timestamp(&crate::KeyframeType::Fov, timestamp).unwrap_or(params.fov_scale);
        let adaptive = params.fovs.get(frame).or_else(|| if params.fovs.len() > 1 { params.fovs.last() } else { None }).copied().unwrap_or(1.0);
        let base_fov = (adaptive * fov_scale).max(0.001) * ratio * crate::smoothing::focal_length::compensation_at(params, frame);
        let limit = params.keyframes.value_at_video_timestamp(&crate::KeyframeType::MaxZoom, timestamp).or(params.max_zoom);
        if let Some(limit) = limit.filter(|v| v.is_finite() && *v > 50.0) {
            let mut limit = limit / 100.0;
            if params.video_speed_affects_zooming_limit && (params.video_speed != 1.0 || params.keyframes.is_keyframed(&crate::KeyframeType::VideoSpeed)) {
                let speed = params.keyframes.value_at_video_timestamp(&crate::KeyframeType::VideoSpeed, timestamp).unwrap_or(params.video_speed).abs();
                limit *= (1.0 + ((speed - 1.0) / 4.0)).min(1.8);
            }
            if !base_fov.is_finite() || !limit.is_finite() { return 0.0; }
            // Adding displacement m requires a 1 + 2m magnification reserve.
            ((base_fov * limit - 1.0) * 0.5).clamp(0.0, MAX_DISPLACEMENT as f64) as f32
        } else { MAX_DISPLACEMENT }
    }).collect();
    if margins.is_empty() { return margins; }
    if params.adaptive_zoom_window < -0.9 {
        // A static crop must not breathe as the per-frame budget changes.
        let minimum = margins.iter().copied().fold(MAX_DISPLACEMENT, f32::min);
        margins.fill(minimum);
        return margins;
    }
    // Erode then average within the same radius. Every averaged minimum contains
    // this frame, so smoothing cannot spend more than its original headroom.
    let radius = (params.scaled_fps * 0.2).ceil().clamp(1.0, 120.0) as usize;
    let lower: Vec<f32> = (0..margins.len()).map(|i| margins[i.saturating_sub(radius)..(i + radius + 1).min(margins.len())].iter().copied().fold(MAX_DISPLACEMENT, f32::min)).collect();
    (0..margins.len()).map(|i| {
        let samples = &lower[i.saturating_sub(radius)..(i + radius + 1).min(lower.len())];
        (samples.iter().map(|v| *v as f64).sum::<f64>() / samples.len() as f64) as f32
    }).collect()
}

pub fn prepare(params: &mut ComputeParams) {
    params.optical_crop_margins = std::sync::Arc::new(crop_margins(params));
    params.optical_grids = std::sync::Arc::new(derive(params));
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct OpticalMotionData {
    pub version: u32,
    pub complete: bool,
    /// Frame times are in the file's original timebase, before an FPS override.
    pub pairs: Vec<OpticalMotionPair>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct OpticalMotionPair {
    pub from_us: i64,
    pub to_us: i64,
    pub size: (u32, u32),
    pub from: Vec<(f32, f32)>,
    pub to: Vec<(f32, f32)>,
}

#[derive(Clone, Debug)]
pub struct MotionGrid(pub [Point; NODES]);
impl Default for MotionGrid {
    fn default() -> Self { Self([[0.0; 2]; NODES]) }
}
impl MotionGrid {
    pub fn sample(&self, point: Point) -> Point {
        let x = point[0].clamp(0.0, 1.0) * (COLS - 1) as f32;
        let y = point[1].clamp(0.0, 1.0) * (ROWS - 1) as f32;
        let (i, j) = ((x as usize).min(COLS - 2), (y as usize).min(ROWS - 2));
        let (dx, dy) = (x - i as f32, y - j as f32);
        std::array::from_fn(|k| {
            (self.0[j * COLS + i][k] * (1.0 - dx) + self.0[j * COLS + i + 1][k] * dx) * (1.0 - dy) +
            (self.0[(j + 1) * COLS + i][k] * (1.0 - dx) + self.0[(j + 1) * COLS + i + 1][k] * dx) * dy
        })
    }

    /// A bound on the infinity norm of the bilinear field's Jacobian. A value
    /// below one makes p -> p + field(p) injective, including clamped borders.
    pub fn derivative_bound(&self) -> f32 {
        let mut bound = 0.0f32;
        for y in 0..ROWS - 1 {
            for x in 0..COLS - 1 {
                for k in 0..2 {
                    let dx = (0..2).map(|j| (self.0[(y + j) * COLS + x + 1][k] - self.0[(y + j) * COLS + x][k]).abs() * (COLS - 1) as f32).fold(0.0f32, f32::max);
                    let dy = (0..2).map(|i| (self.0[(y + 1) * COLS + x + i][k] - self.0[y * COLS + x + i][k]).abs() * (ROWS - 1) as f32).fold(0.0f32, f32::max);
                    bound = bound.max(dx + dy);
                }
            }
        }
        bound
    }

    pub fn limit(&mut self, displacement: f32, derivative: f32) {
        if self.0.iter().flatten().any(|v| !v.is_finite()) { *self = Self::default(); return; }
        let maximum = self.0.iter().flatten().fold(0.0f32, |a, b| a.max(b.abs()));
        let scale = (displacement / maximum.max(1e-9)).min(derivative / self.derivative_bound().max(1e-9)).min(1.0);
        for v in self.0.iter_mut().flatten() { *v *= scale; }
    }

    pub fn inverse(&self, target: Point) -> Point {
        let mut p = target;
        for _ in 0..16 {
            let d = self.sample(p);
            p = [target[0] - d[0], target[1] - d[1]];
        }
        p
    }

    pub fn append_buffer(&self, buffer: &mut Vec<f32>, inverted: bool) -> i32 {
        // Preserve the camera-mesh header and sentinel when no camera mesh exists.
        if buffer.is_empty() { buffer.resize(10, 0.0); }
        let offset = buffer.len() as i32;
        for y in 0..ROWS {
            for x in 0..COLS {
                let p = self.0[(if inverted { ROWS - 1 - y } else { y }) * COLS + x];
                buffer.extend_from_slice(&[p[0], p[1] * if inverted { -1.0 } else { 1.0 }]);
            }
        }
        offset
    }
}

fn median(mut values: Vec<f32>) -> f32 {
    values.sort_unstable_by(f32::total_cmp);
    let n = values.len();
    if n % 2 == 0 { (values[n / 2 - 1] + values[n / 2]) * 0.5 } else { values[n / 2] }
}

/// Robust local motion, with support distributed over the image. A small moving
/// foreground cannot create a usable field from a handful of clustered tracks.
pub fn estimate_field(from: &[Point], to: &[Point]) -> Option<MotionGrid> {
    let samples: Vec<(Point, Point)> = from.iter().zip(to).filter_map(|(a, b)| {
        if a.iter().chain(b).any(|v| !v.is_finite() || *v < -0.25 || *v > 1.25) { return None; }
        let d = [b[0] - a[0], b[1] - a[1]];
        (d[0].abs() < 0.15 && d[1].abs() < 0.15).then_some((*a, d))
    }).collect();
    if samples.len() < 20 { return None; }
    let center: Point = std::array::from_fn(|k| median(samples.iter().map(|p| p.1[k]).collect()));
    let errors: Vec<f32> = samples.iter().map(|p| (p.1[0] - center[0]).hypot(p.1[1] - center[1])).collect();
    let threshold = (median(errors.clone()) * 3.0).max(0.008);
    let samples: Vec<_> = samples.into_iter().zip(errors).filter_map(|(p, e)| (e <= threshold).then_some(p)).collect();
    let mut occupied = [false; 12];
    for (p, _) in &samples {
        occupied[(p[1].clamp(0.0, 0.999) * 3.0) as usize * 4 + (p[0].clamp(0.0, 0.999) * 4.0) as usize] = true;
    }
    if samples.len() < 20 || occupied.iter().filter(|v| **v).count() < 8 { return None; }
    let mut field = MotionGrid::default();
    for y in 0..ROWS {
        for x in 0..COLS {
            let q = [x as f32 / (COLS - 1) as f32, y as f32 / (ROWS - 1) as f32];
            let local: Vec<_> = samples.iter().filter(|p| {
                ((p.0[0] - q[0]) * (COLS - 1) as f32).hypot((p.0[1] - q[1]) * (ROWS - 1) as f32) <= 2.0
            }).collect();
            field.0[y * COLS + x] = if local.len() >= 4 {
                std::array::from_fn(|k| median(local.iter().map(|p| p.1[k]).collect()))
            } else { center };
        }
    }
    // Spatial regularization suppresses abrupt changes between support regions.
    let observed = field.clone();
    for _ in 0..8 {
        let previous = field.clone();
        for y in 0..ROWS {
            for x in 0..COLS {
                let neighbours = [x.checked_sub(1).map(|i| y * COLS + i), (x + 1 < COLS).then_some(y * COLS + x + 1),
                    y.checked_sub(1).map(|j| j * COLS + x), (y + 1 < ROWS).then_some((y + 1) * COLS + x)];
                let indices: Vec<_> = neighbours.into_iter().flatten().collect();
                field.0[y * COLS + x] = std::array::from_fn(|k| {
                    (observed.0[y * COLS + x][k] + indices.iter().map(|i| previous.0[*i][k] * 0.5).sum::<f32>()) / (1.0 + indices.len() as f32 * 0.5)
                });
            }
        }
    }
    // Reject a discontinuous inter-frame map, rather than silently changing its
    // motion to make it invertible. The final correction has its own tighter cap.
    (field.derivative_bound() < 0.5).then_some(field)
}

/// Follow each vertex through neighboring frame maps, then fit a Gaussian-
/// weighted local line to its trajectory. Unlike accumulated fixed-grid vectors,
/// these positions account for transport through a spatially varying field.
pub fn smooth_fields(flows: &[Option<MotionGrid>], fps: f64, sigma_seconds: f64) -> Vec<MotionGrid> {
    if !fps.is_finite() || fps <= 0.0 || !sigma_seconds.is_finite() || sigma_seconds <= 0.0 { return Vec::new(); }
    let sigma = (sigma_seconds * fps).clamp(1.0, 120.0);
    let radius = (sigma * 3.0).ceil() as usize;
    (0..flows.len() + 1).into_par_iter().map(|frame| {
        let mut grid = MotionGrid::default();
        for y in 0..ROWS {
            for x in 0..COLS {
                let origin = [x as f32 / (COLS - 1) as f32, y as f32 / (ROWS - 1) as f32];
                let mut samples = vec![(0.0f64, origin)];
                for direction in [-1isize, 1] {
                    let mut p = origin;
                    for distance in 1..=radius {
                        let index = frame as isize + if direction < 0 { -(distance as isize) } else { distance as isize - 1 };
                        if index < 0 { break; }
                        let Some(Some(flow)) = flows.get(index as usize) else { break; };
                        p = if direction < 0 { flow.inverse(p) } else { let d = flow.sample(p); [p[0] + d[0], p[1] + d[1]] };
                        if p.iter().any(|v| *v < -0.25 || *v > 1.25 || !v.is_finite()) { break; }
                        samples.push((distance as f64 * direction as f64, p));
                    }
                }
                if samples.len() < 5 { continue; }
                let (mut s0, mut sx, mut sxx, mut sy, mut sxy) = (0.0, 0.0, 0.0, [0.0; 2], [0.0; 2]);
                for (t, p) in samples {
                    let weight = (-0.5 * (t / sigma).powi(2)).exp();
                    s0 += weight; sx += weight * t; sxx += weight * t * t;
                    for k in 0..2 { sy[k] += weight * p[k] as f64; sxy[k] += weight * t * p[k] as f64; }
                }
                let determinant = s0 * sxx - sx * sx;
                if determinant > 1e-9 {
                    // Backward sampling: observed position minus smoothed target.
                    grid.0[y * COLS + x] = std::array::from_fn(|k| origin[k] - ((sxx * sy[k] - sx * sxy[k]) / determinant) as f32);
                }
            }
        }
        grid.limit(MAX_DISPLACEMENT, 0.25);
        grid
    }).collect()
}

pub fn derive(params: &ComputeParams) -> Vec<MotionGrid> {
    let Some(data) = params.optical_motion.as_ref().filter(|d| d.complete && d.version == 1) else { return Vec::new(); };
    if !params.optical_stabilization || params.frame_count < 2 { return Vec::new(); }
    if !params.optical_time_scale.is_finite() || params.optical_time_scale <= 0.0 || params.output_width == 0 || params.output_height == 0 { return Vec::new(); }
    let mut projection = params.clone();
    projection.optical_grids = Default::default();
    projection.framebuffer_inverted = false;
    let mut flows = vec![None; params.frame_count - 1];
    for pair in &data.pairs {
        if pair.size.0 == 0 || pair.size.1 == 0 || pair.from.len() != pair.to.len() || pair.from.len() > 4096 { continue; }
        let from_ms = pair.from_us as f64 / 1000.0 / params.optical_time_scale;
        let to_ms = pair.to_us as f64 / 1000.0 / params.optical_time_scale;
        let from_frame = crate::frame_at_timestamp(from_ms, params.scaled_fps) as usize;
        let to_frame = crate::frame_at_timestamp(to_ms, params.scaled_fps) as usize;
        if from_frame >= flows.len() || to_frame != from_frame + 1 { continue; }
        let project = |points: &[(f32, f32)], ms, frame| {
            let points: Vec<_> = points.iter().map(|p| (p.0 * params.width as f32 / pair.size.0 as f32, p.1 * params.height as f32 / pair.size.1 as f32)).collect();
            undistort_points_with_rolling_shutter(&points, ms, Some(frame), &projection, params.lens_correction_amount, true, false)
                .iter().map(|p| [p.0 / params.output_width as f32, p.1 / params.output_height as f32]).collect::<Vec<_>>()
        };
        flows[from_frame] = estimate_field(&project(&pair.from, from_ms, from_frame), &project(&pair.to, to_ms, to_frame));
    }
    let supported = flows.iter().filter(|v| v.is_some()).count();
    let mut grids = smooth_fields(&flows, params.scaled_fps, 0.2);
    if zooming_enabled(params) {
        for (frame, grid) in grids.iter_mut().enumerate() {
            grid.limit(params.optical_crop_margins.get(frame).copied().unwrap_or(0.0), 0.25);
        }
    }
    let maximum = grids.iter().flat_map(|g| g.0.iter().flatten()).fold(0.0f32, |a, b| a.max(b.abs()));
    log::info!("Residual optical correction: {supported}/{} frame pairs, {} frames, maximum normalized displacement {maximum:.5}", flows.len(), grids.len());
    grids
}

/// Same layout/interpolation as the render kernels. Buffer offsets are set only
/// by FrameTransform, but slice access remains checked for CPU callers and tests.
pub fn sample_buffer(buffer: &[f64], offset: i32, point: Point) -> Point {
    if offset <= 0 || point.iter().any(|v| !v.is_finite()) { return [0.0; 2]; }
    let Some(values) = buffer.get(offset as usize..offset as usize + BUFFER_FLOATS) else { return [0.0; 2]; };
    let x = point[0].clamp(0.0, 1.0) * (COLS - 1) as f32;
    let y = point[1].clamp(0.0, 1.0) * (ROWS - 1) as f32;
    let (i, j) = ((x as usize).min(COLS - 2), (y as usize).min(ROWS - 2));
    let (dx, dy) = (x - i as f32, y - j as f32);
    std::array::from_fn(|k| {
        let at = |x, y| values[(y * COLS + x) * 2 + k] as f32;
        (at(i, j) * (1.0 - dx) + at(i + 1, j) * dx) * (1.0 - dy) +
        (at(i, j + 1) * (1.0 - dx) + at(i + 1, j + 1) * dx) * dy
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constant(d: Point) -> MotionGrid { MotionGrid([d; NODES]) }
    fn lattice() -> Vec<Point> {
        (0..12).flat_map(|y| (0..16).map(move |x| [(x as f32 + 0.5) / 16.0, (y as f32 + 0.5) / 12.0])).collect()
    }

    fn crop_params() -> ComputeParams {
        let mut p = ComputeParams::default();
        p.width = 1920; p.height = 1080; p.output_width = 1920; p.output_height = 1080;
        p.scaled_fps = 30.0; p.frame_count = 60; p.fov_scale = 1.0;
        p.adaptive_zoom_window = 4.0; p.max_zoom = Some(130.0);
        p.optical_stabilization = true;
        p.optical_motion = Some(std::sync::Arc::new(OpticalMotionData { version: 1, complete: true,
            pairs: vec![OpticalMotionPair { from_us: 0, to_us: 33_333, size: (320, 240), from: vec![(10.0, 10.0)], to: vec![(11.0, 10.0)] }] }));
        p
    }

    #[test]
    fn crop_reserve_uses_available_budget_and_smooths_below_its_ceiling() {
        let mut p = crop_params();
        p.fovs = (0..60).map(|i| if (25..30).contains(&i) { 1.04 / 1.3 } else { 1.0 }).collect();
        let margins = crop_margins(&p);
        assert_eq!(margins[0], MAX_DISPLACEMENT);
        assert!(margins[24] < MAX_DISPLACEMENT, "Budget reduction should start before the tight section");
        assert!(margins[31] < MAX_DISPLACEMENT, "Recovery should also be gradual");
        for (fov, margin) in p.fovs.iter().zip(&margins) {
            assert!(margin >= &0.0 && margin <= &MAX_DISPLACEMENT);
            assert!((1.0 + 2.0 * *margin as f64) / fov <= 1.3 + 1e-8);
        }
        p.adaptive_zoom_window = -1.0;
        let margins = crop_margins(&p);
        assert!(margins.iter().all(|m| (*m - 0.02).abs() < 1e-6), "Static reserve must not pump");
    }

    #[test]
    fn crop_reserve_accounts_for_manual_fov_output_size_and_focal_compensation() {
        let mut p = crop_params();
        p.fovs = vec![1.0; 60];
        p.fov_scale = 0.8;
        assert!(crop_margins(&p).iter().all(|m| (*m - 0.02).abs() < 1e-6));
        p.output_width = 3840;
        assert!(crop_margins(&p).iter().all(|m| *m == 0.0));
        p.output_width = 1920; p.fov_scale = 1.0;
        p.focal_length_smoothing_enabled = true;
        p.focal_lengths = vec![Some(800.0); 60];
        p.smoothed_focal_lengths = vec![Some(1000.0); 60];
        assert!(crop_margins(&p).iter().all(|m| (*m - 0.02).abs() < 1e-6));
    }

    #[test]
    fn crop_reserve_does_not_add_zoom_to_an_existing_over_limit_view() {
        let mut p = crop_params();
        p.fovs = vec![0.7; 60];
        assert!(crop_margins(&p).iter().all(|m| *m == 0.0));
        p.max_zoom = None;
        assert!(crop_margins(&p).iter().all(|m| *m == MAX_DISPLACEMENT));
        p.adaptive_zoom_window = 0.0;
        assert!(crop_margins(&p).is_empty());
        p.adaptive_zoom_window = 4.0; p.optical_stabilization = false;
        assert!(crop_margins(&p).is_empty());
    }

    #[test]
    fn optical_reserve_requests_room_before_the_camera_zoom_limit() {
        let mut p = crop_params();
        assert!((zoom_reserve_factor(&p) - 1.08).abs() < 1e-6);
        p.optical_stabilization = false;
        assert_eq!(zoom_reserve_factor(&p), 1.0);
        p.optical_stabilization = true; p.adaptive_zoom_window = 0.0;
        assert_eq!(zoom_reserve_factor(&p), 1.0);
        p.adaptive_zoom_window = 4.0; p.optical_motion = None;
        assert_eq!(zoom_reserve_factor(&p), 1.0);
    }

    #[test]
    fn optical_reserve_invalidates_camera_and_zoom_state_when_toggled() {
        let mut p = crop_params();
        let smoothing = crate::smoothing::Smoothing::default();
        let on = (smoothing.get_state_checksum(123, &p), crate::zooming::get_checksum(&p, 123));
        p.optical_stabilization = false;
        let off = (smoothing.get_state_checksum(123, &p), crate::zooming::get_checksum(&p, 123));
        assert_ne!(on.0, off.0);
        assert_ne!(on.1, off.1);
    }

    #[test]
    fn optical_crop_tracks_keyframed_fov_and_zoom_limits() {
        let mut p = crop_params();
        p.fovs = vec![0.85; p.frame_count];
        p.keyframes.set(&crate::KeyframeType::MaxZoom, 0, 130.0);
        p.keyframes.set(&crate::KeyframeType::MaxZoom, 1_000_000, 120.0);
        p.keyframes.set(&crate::KeyframeType::MaxZoom, 2_000_000, 140.0);
        p.keyframes.set(&crate::KeyframeType::Fov, 0, 1.0);
        p.keyframes.set(&crate::KeyframeType::Fov, 2_000_000, 0.95);
        for (i, margin) in crop_margins(&p).iter().enumerate() {
            let ts = crate::timestamp_at_frame(i as i32, p.scaled_fps);
            let manual = p.keyframes.value_at_video_timestamp(&crate::KeyframeType::Fov, ts).unwrap();
            let limit = p.keyframes.value_at_video_timestamp(&crate::KeyframeType::MaxZoom, ts).unwrap() / 100.0;
            let base_zoom = 1.0 / (0.85 * manual);
            let total_zoom = (1.0 + 2.0 * *margin as f64) * base_zoom;
            assert!(total_zoom <= base_zoom.max(limit) + 1e-8, "Frame {i} overspent its keyframed limit");
        }
    }

    #[test]
    fn project_keeps_calibration_metadata_without_gyro_samples() {
        let manager = crate::StabilizationManager::default();
        let mut md = crate::gyro_source::FileMetadata::default();
        md.lens_params.insert(0, crate::gyro_source::LensParams { pixel_focal_length: Some((100.0, 110.0)), ..Default::default() });
        let project = serde_json::json!({
            "version": 4, "videofile": "file:///optical-fixture-missing.mp4",
            "video_info": { "width": 32, "height": 32, "num_frames": 2, "fps": 30.0, "duration_ms": 66.667 },
            "gyro_source": { "filepath": "file:///optical-fixture-missing.mp4", "file_metadata": crate::util::compress_to_base91_cbor(&md).unwrap() }
        });
        manager.import_gyroflow_data(&serde_json::to_vec(&project).unwrap(), false, None, |_| {},
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), &mut false, false).unwrap();
        let gyro = manager.gyro.read();
        let metadata = gyro.file_metadata.read();
        assert!(!metadata.has_motion());
        assert_eq!(metadata.lens_params[&0].pixel_focal_length, Some((100.0, 110.0)));
    }

    #[test]
    fn residual_smoothing_removes_jitter_and_keeps_a_constant_pan() {
        let path: Vec<f32> = (0..120).map(|i| i as f32 * 0.0005 + (i as f32 * std::f32::consts::TAU / 9.0).sin() * 0.015).collect();
        let flows: Vec<_> = path.windows(2).map(|w| Some(constant([w[1] - w[0], 0.0]))).collect();
        let correction = smooth_fields(&flows, 30.0, 0.2);
        let result: Vec<_> = path.iter().zip(correction).map(|(p, c)| p - c.sample([0.5, 0.5])[0]).collect();
        let acceleration = |p: &[f32]| p[20..100].windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]).powi(2)).sum::<f32>();
        assert!(acceleration(&result) < acceleration(&path) * 0.01);
        assert!((result[90] - result[30] - 60.0 * 0.0005).abs() < 0.0002);
    }

    #[test]
    fn residual_local_line_keeps_pan_at_both_ends_of_a_clip() {
        let correction = smooth_fields(&vec![Some(constant([0.001, -0.0005])); 89], 30.0, 0.2);
        assert!(correction.iter().flat_map(|g| g.0.iter().flatten()).all(|v| v.abs() < 1e-5));
    }

    #[test]
    fn residual_preserves_spatially_varying_motion() {
        let mut flows = Vec::new();
        for i in 0..89 {
            let delta = ((i + 1) as f32 * 0.7).sin() - (i as f32 * 0.7).sin();
            let mut grid = MotionGrid::default();
            for y in 0..ROWS { for x in 0..COLS {
                grid.0[y * COLS + x] = [delta * (0.002 + 0.01 * y as f32 / (ROWS - 1) as f32), 0.0];
            }}
            flows.push(Some(grid));
        }
        let grids = smooth_fields(&flows, 30.0, 0.2);
        let grid = &grids[40];
        assert!(grid.sample([0.5, 0.9])[0].abs() > grid.sample([0.5, 0.1])[0].abs() * 2.0,
            "A local field must not collapse to one global translation");
    }

    #[test]
    fn residual_cut_does_not_mix_unrelated_trajectories() {
        let left: Vec<_> = (0..39).map(|i| Some(constant([(i as f32).sin() * 0.004, 0.0]))).collect();
        let mut all = left.clone();
        all.push(None);
        all.extend(vec![Some(constant([0.01, 0.02])); 39]);
        let alone = smooth_fields(&left, 30.0, 0.2);
        let joined = smooth_fields(&all, 30.0, 0.2);
        for (a, b) in alone.iter().zip(&joined) { assert_eq!(a.0, b.0); }
        assert!(smooth_fields(&[None, None], 30.0, 0.2).iter().flat_map(|g| g.0.iter().flatten()).all(|v| *v == 0.0));
    }

    #[test]
    fn residual_estimator_rejects_clustered_support_and_foreground_outliers() {
        let from = lattice();
        let to: Vec<_> = from.iter().enumerate().map(|(i, p)| [p[0] + if i % 7 == 0 { 0.09 } else { 0.005 }, p[1] - 0.003]).collect();
        let field = estimate_field(&from, &to).unwrap();
        for p in field.0 { assert!((p[0] - 0.005).abs() < 1e-6 && (p[1] + 0.003).abs() < 1e-6); }
        assert!(estimate_field(&vec![[0.2, 0.2]; 40], &vec![[0.21, 0.2]; 40]).is_none());
        assert!(estimate_field(&from, &vec![[f32::NAN, 0.0]; from.len()]).is_none());
    }

    #[test]
    fn residual_bounded_warp_is_invertible_and_has_a_separate_payload() {
        let mut grid = MotionGrid::default();
        for (i, p) in grid.0.iter_mut().enumerate() { *p = [(i as f32).sin() * 0.5, (i as f32).cos() * 0.5]; }
        grid.limit(MAX_DISPLACEMENT, 0.25);
        assert!(grid.derivative_bound() <= 0.250001);
        assert!(grid.0.iter().flatten().all(|v| v.abs() <= MAX_DISPLACEMENT));
        let camera_payload = vec![0.0, 14.0, 28.0, 42.0, 56.0, 70.0, 84.0, 98.0, 112.0, 126.0];
        let mut payload = camera_payload.clone();
        let offset = grid.append_buffer(&mut payload, false);
        assert_eq!(&payload[..camera_payload.len()], camera_payload.as_slice());
        let payload: Vec<f64> = payload.iter().map(|v| *v as f64).collect();
        for p in lattice() {
            let d = grid.sample(p);
            let q = grid.inverse([p[0] + d[0], p[1] + d[1]]);
            assert!((p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6);
            assert_eq!(sample_buffer(&payload, offset, p), d);
        }
        assert!(MAX_KERNEL_BUFFER <= 2048, "Qt preview texture must contain both payloads");
    }

    #[test]
    fn residual_metadata_round_trip_and_incomplete_data_are_safe() {
        let data = OpticalMotionData { version: 1, complete: true, pairs: vec![OpticalMotionPair {
            from_us: 0, to_us: 33_333, size: (320, 240), from: vec![(1.0, 2.0)], to: vec![(3.0, 4.0)] }] };
        let mut metadata = crate::gyro_source::FileMetadata::default();
        metadata.optical_motion = Some(data);
        let restored: crate::gyro_source::FileMetadata = serde_json::from_str(&serde_json::to_string(&metadata).unwrap()).unwrap();
        assert_eq!(restored.optical_motion.unwrap().pairs[0].to, vec![(3.0, 4.0)]);
        assert!(metadata.thin().optical_motion.is_none());
        let mut params = ComputeParams::default();
        params.optical_stabilization = true;
        params.optical_motion = Some(std::sync::Arc::new(OpticalMotionData { version: 1, complete: false, pairs: vec![] }));
        assert!(derive(&params).is_empty());
    }

    #[test]
    fn residual_cpu_render_samples_the_shifted_input_at_multiple_sizes() {
        use crate::gpu::{ Buffers, BufferDescription, BufferSource };
        use crate::stabilization::{ KernelParams, Stabilization, Luma8, distortion_models::DistortionModel };
        for scale in [1, 2] {
            let (w, h) = (64 * scale, 48 * scale);
            let mut input: Vec<u8> = (0..h).flat_map(|y| (0..w).map(move |x| ((x * 13 + y * 7) % 251) as u8)).collect();
            let original = input.clone();
            let mut output = vec![0u8; input.len()];
            let mut payload = Vec::new();
            let offset = constant([2.0 / 64.0, -1.0 / 48.0]).append_buffer(&mut payload, false);
            let kp = KernelParams { width: w as i32, height: h as i32, stride: w as i32,
                output_width: w as i32, output_height: h as i32, output_stride: w as i32,
                output_rect: [0, 0, w as i32, h as i32], source_rect: [0, 0, w as i32, h as i32],
                flags: 4096, optical_mesh_offset: offset, matrix_count: 1, f: [1.0, 1.0], fov: 1.0,
                lens_correction_amount: 1.0, bytes_per_pixel: 1, pix_element_count: 1,
                pixel_value_limit: 255.0, max_pixel_value: 255.0, ..Default::default() };
            let mut buffers = Buffers {
                input: BufferDescription { size: (w, h, w), data: BufferSource::Cpu { buffer: &mut input }, ..Default::default() },
                output: BufferDescription { size: (w, h, w), data: BufferSource::Cpu { buffer: &mut output }, ..Default::default() },
            };
            let matrices = [[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]];
            assert!(Stabilization::undistort_image_cpu::<2, Luma8>(&mut buffers, &kp, &DistortionModel::from_name("opencv_standard"), None, &matrices, &[], &payload));
            for y in 8..h - 8 { for x in 8..w - 8 {
                assert_eq!(output[y * w + x], original[(y - scale) * w + x + 2 * scale], "Wrong sample at ({x},{y}), scale {scale}");
            }}
        }
    }

    #[test]
    fn residual_wgsl_validates_for_buffer_and_texture_inputs() {
        use crate::stabilization::distortion_models::DistortionModel;
        for name in ["opencv_standard", "opencv_fisheye", "sony", "generic_polynomial", "gopro", "poly3", "poly5", "ptlens", "insta360"] {
            for (remove, closing) in [("{buffer_input}", "{/buffer_input}"), ("{texture_input}", "{/texture_input}")] {
                let lens = DistortionModel::from_name(name);
                let functions = format!("{}\nfn digital_undistort_point(uv: vec2<f32>) -> vec2<f32> {{ return uv; }}\nfn digital_distort_point(uv: vec2<f32>) -> vec2<f32> {{ return uv; }}", lens.wgsl_functions());
                let mut source = include_str!("../gpu/wgpu_undistort.wgsl").replace("LENS_MODEL_FUNCTIONS;", &functions).replace("SCALAR", "f32");
                while let Some(start) = source.find(remove) {
                    let end = source[start..].find(closing).unwrap() + start + closing.len();
                    source.replace_range(start..end, "");
                }
                let module = wgpu::naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&source)));
                wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all()).validate(&module).unwrap();
            }
        }
    }
}
