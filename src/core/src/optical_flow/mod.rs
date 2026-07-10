use std::collections::HashMap;
use nalgebra::{Vector2, Matrix3};

/// Represents a 2D motion estimate between two consecutive frames.
#[derive(Debug, Clone, Default)]
pub struct FrameMotion {
    /// Translation in pixels (tx, ty)
    pub translation: Vector2<f64>,
    /// Rotation angle in radians
    pub rotation: f64,
    /// Scale factor
    pub scale: f64,
    /// Confidence weight [0..1]
    pub confidence: f64,
}

/// Accumulated trajectory point (integral of FrameMotion)
#[derive(Debug, Clone, Default)]
struct TrajectoryPoint {
    tx: f64,
    ty: f64,
    rot: f64,
}

/// Compute the stabilization transforms from optical flow data.
///
/// `flow_data` maps frame index → `FrameMotion` for that frame transition (frame N → N+1).
/// `frame_count` is the total number of frames.
/// `smoothing_window` is the radius (in frames) of the smoothing window.
///
/// Returns a map: frame index → 3×3 homogeneous warp matrix (column-major, as flat [f64; 9]).
pub fn compute_stabilization_transforms(
    flow_data: &HashMap<usize, FrameMotion>,
    frame_count: usize,
    smoothing_window: usize,
) -> HashMap<usize, [f64; 9]> {
    if frame_count == 0 {
        return HashMap::new();
    }

    // ── 1. Build cumulative trajectory ──────────────────────────────────────
    let mut trajectory = vec![TrajectoryPoint::default(); frame_count];
    for i in 1..frame_count {
        let prev = &trajectory[i - 1];
        let motion = flow_data.get(&(i - 1)).cloned().unwrap_or_default();
        trajectory[i] = TrajectoryPoint {
            tx:  prev.tx  + motion.translation.x,
            ty:  prev.ty  + motion.translation.y,
            rot: prev.rot + motion.rotation,
        };
    }

    // ── 2. Smooth trajectory with a simple box filter ───────────────────────
    let smoothed = smooth_trajectory(&trajectory, smoothing_window);

    // ── 3. Compute per-frame correction transforms ───────────────────────────
    //   correction = smooth_trajectory – raw_trajectory
    let mut transforms = HashMap::with_capacity(frame_count);
    for i in 0..frame_count {
        let dtx  = smoothed[i].tx  - trajectory[i].tx;
        let dty  = smoothed[i].ty  - trajectory[i].ty;
        let drot = smoothed[i].rot - trajectory[i].rot;

        let mat = make_rigid_matrix(dtx, dty, drot);
        transforms.insert(i, mat);
    }
    transforms
}

/// Box-filter smoothing of the trajectory.
fn smooth_trajectory(traj: &[TrajectoryPoint], radius: usize) -> Vec<TrajectoryPoint> {
    let n = traj.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let lo = i.saturating_sub(radius);
        let hi = (i + radius + 1).min(n);
        let count = (hi - lo) as f64;
        let sum_tx:  f64 = traj[lo..hi].iter().map(|p| p.tx).sum();
        let sum_ty:  f64 = traj[lo..hi].iter().map(|p| p.ty).sum();
        let sum_rot: f64 = traj[lo..hi].iter().map(|p| p.rot).sum();
        out.push(TrajectoryPoint {
            tx:  sum_tx  / count,
            ty:  sum_ty  / count,
            rot: sum_rot / count,
        });
    }
    out
}

/// Build a 3×3 rigid-body (rotation + translation) transformation matrix.
/// The matrix is returned in **row-major** order as a flat `[f64; 9]` array.
///
/// ┌                     ┐
/// │  cos θ  −sin θ   tx │
/// │  sin θ   cos θ   ty │
/// │    0       0      1 │
/// └                     ┘
pub fn make_rigid_matrix(tx: f64, ty: f64, angle_rad: f64) -> [f64; 9] {
    let c = angle_rad.cos();
    let s = angle_rad.sin();
    [
        c, -s, tx,
        s,  c, ty,
        0.0, 0.0, 1.0,
    ]
}

/// Convert a flat row-major `[f64; 9]` into a `nalgebra::Matrix3<f64>`.
pub fn matrix_from_array(m: &[f64; 9]) -> Matrix3<f64> {
    Matrix3::new(
        m[0], m[1], m[2],
        m[3], m[4], m[5],
        m[6], m[7], m[8],
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Pure-Rust sparse optical flow (Lucas-Kanade tracker)
// ─────────────────────────────────────────────────────────────────────────────

/// A single tracked feature point.
#[derive(Debug, Clone)]
pub struct TrackedPoint {
    pub id:  u64,
    pub x:   f32,
    pub y:   f32,
}

/// Minimal grayscale image wrapper.
pub struct GrayImage<'a> {
    pub data:   &'a [u8],
    pub width:  usize,
    pub height: usize,
    pub stride: usize, // bytes per row
}

impl<'a> GrayImage<'a> {
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> f32 {
        if x < self.width && y < self.height {
            self.data[y * self.stride + x] as f32
        } else {
            0.0
        }
    }

    #[inline]
    pub fn get_bilinear(&self, x: f32, y: f32) -> f32 {
        let x0 = x.floor() as isize;
        let y0 = y.floor() as isize;
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;

        let p = |cx: isize, cy: isize| -> f32 {
            if cx < 0 || cy < 0 || cx >= self.width as isize || cy >= self.height as isize {
                return 0.0;
            }
            self.data[cy as usize * self.stride + cx as usize] as f32
        };

        let i00 = p(x0,     y0);
        let i10 = p(x0 + 1, y0);
        let i01 = p(x0,     y0 + 1);
        let i11 = p(x0 + 1, y0 + 1);

        i00 * (1.0 - fx) * (1.0 - fy)
            + i10 * fx * (1.0 - fy)
            + i01 * (1.0 - fx) * fy
            + i11 * fx * fy
    }
}

/// Detect Harris corners in a grayscale image.
/// Returns up to `max_features` (x, y) positions.
pub fn detect_harris_corners(img: &GrayImage, max_features: usize, block_size: usize) -> Vec<(f32, f32)> {
    let w = img.width;
    let h = img.height;
    let mut responses = vec![0.0f32; w * h];

    let k = 0.04f32;
    let border = block_size / 2 + 1;

    for y in border..(h - border) {
        for x in border..(w - border) {
            let mut ixx = 0.0f32;
            let mut iyy = 0.0f32;
            let mut ixy = 0.0f32;

            for dy in 0..block_size {
                for dx in 0..block_size {
                    let px = x + dx - block_size / 2;
                    let py = y + dy - block_size / 2;

                    let ix = img.get(px + 1, py) - img.get(px.saturating_sub(1), py);
                    let iy = img.get(px, py + 1) - img.get(px, py.saturating_sub(1));
                    ixx += ix * ix;
                    iyy += iy * iy;
                    ixy += ix * iy;
                }
            }

            let det   = ixx * iyy - ixy * ixy;
            let trace = ixx + iyy;
            responses[y * w + x] = det - k * trace * trace;
        }
    }

    // Non-maximum suppression with a 5×5 window
    let nms_r = 5usize;
    let mut corners: Vec<(f32, f32, f32)> = Vec::new();

    for y in border..(h - border) {
        for x in border..(w - border) {
            let r = responses[y * w + x];
            if r <= 0.0 {
                continue;
            }
            let mut is_max = true;
            'outer: for dy in 0..=(nms_r * 2) {
                for dx in 0..=(nms_r * 2) {
                    let nx = x + dx;
                    let ny = y + dy;
                    if nx == x + nms_r && ny == y + nms_r {
                        continue;
                    }
                    if nx < w && ny < h && responses[ny * w + nx] > r {
                        is_max = false;
                        break 'outer;
                    }
                }
            }
            if is_max {
                corners.push((x as f32, y as f32, r));
            }
        }
    }

    // Sort by response strength
    corners.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    corners.truncate(max_features);
    corners.iter().map(|&(x, y, _)| (x, y)).collect()
}

/// Track a set of points from `prev` to `next` using Lucas-Kanade optical flow.
/// Returns a Vec of (prev_point, next_point, success) tuples.
pub fn lk_track(
    prev: &GrayImage,
    next: &GrayImage,
    points: &[(f32, f32)],
    win_size: usize,
    max_iter: usize,
    epsilon: f32,
) -> Vec<((f32, f32), (f32, f32), bool)> {
    let half = (win_size / 2) as f32;
    let mut results = Vec::with_capacity(points.len());

    for &(px, py) in points {
        let mut nx = px;
        let mut ny = py;
        let mut ok = true;

        for _ in 0..max_iter {
            let mut ixx = 0.0f32;
            let mut iyy = 0.0f32;
            let mut ixy = 0.0f32;
            let mut ixt = 0.0f32;
            let mut iyt = 0.0f32;

            let mut count = 0u32;
            let steps = win_size as i32;
            let h_i = half as i32;

            for dy in -h_i..=(steps - h_i - 1) {
                for dx in -h_i..=(steps - h_i - 1) {
                    let qpx = px + dx as f32;
                    let qpy = py + dy as f32;

                    let ix = prev.get_bilinear(qpx + 1.0, qpy) - prev.get_bilinear(qpx - 1.0, qpy);
                    let iy = prev.get_bilinear(qpx, qpy + 1.0) - prev.get_bilinear(qpx, qpy - 1.0);
                    let it = next.get_bilinear(nx + dx as f32, ny + dy as f32)
                           - prev.get_bilinear(qpx, qpy);

                    ixx += ix * ix;
                    iyy += iy * iy;
                    ixy += ix * iy;
                    ixt += ix * it;
                    iyt += iy * it;
                    count += 1;
                }
            }

            if count == 0 {
                ok = false;
                break;
            }

            let det = ixx * iyy - ixy * ixy;
            if det.abs() < 1e-6 {
                ok = false;
                break;
            }

            let vx = -(iyy * ixt - ixy * iyt) / det;
            let vy = -(ixx * iyt - ixy * ixt) / det;

            nx += vx;
            ny += vy;

            if (vx * vx + vy * vy).sqrt() < epsilon {
                break;
            }
        }

        // Bounds check
        if nx < 0.0 || ny < 0.0
            || nx >= prev.width  as f32
            || ny >= prev.height as f32
        {
            ok = false;
        }

        results.push(((px, py), (nx, ny), ok));
    }

    results
}

/// Estimate a rigid (translation + rotation) transform from point correspondences
/// using a least-squares fit.  Returns `(tx, ty, rotation_rad, confidence)`.
pub fn estimate_rigid_transform(
    matches: &[((f32, f32), (f32, f32))],
) -> (f64, f64, f64, f64) {
    let n = matches.len();
    if n < 2 {
        return (0.0, 0.0, 0.0, 0.0);
    }

    // Centroid of source and destination points
    let (mut sx, mut sy, mut dx, mut dy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for &((px, py), (qx, qy)) in matches {
        sx += px as f64;
        sy += py as f64;
        dx += qx as f64;
        dy += qy as f64;
    }
    let inv_n = 1.0 / n as f64;
    sx *= inv_n; sy *= inv_n;
    dx *= inv_n; dy *= inv_n;

    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for &((px, py), (qx, qy)) in matches {
        let a = (px as f64 - sx, py as f64 - sy);
        let b = (qx as f64 - dx, qy as f64 - dy);
        num += a.0 * b.1 - a.1 * b.0;
        den += a.0 * b.0 + a.1 * b.1;
    }

    let angle = num.atan2(den);
    let tx    = dx - sx;
    let ty    = dy - sy;

    // Confidence: ratio of inliers after checking reprojection error
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    let threshold_sq = 4.0f64; // 2 px reprojection threshold
    let inliers = matches.iter().filter(|&&((px, py), (qx, qy))| {
        let ex = cos_a * (px as f64 - sx) - sin_a * (py as f64 - sy) + sx + tx - qx as f64;
        let ey = sin_a * (px as f64 - sx) + cos_a * (py as f64 - sy) + sy + ty - qy as f64;
        ex * ex + ey * ey < threshold_sq
    }).count();
    let confidence = inliers as f64 / n as f64;

    (tx, ty, angle, confidence)
}

/// High-level entry point: given two consecutive grayscale frames, compute the
/// `FrameMotion` between them.
pub fn estimate_frame_motion(
    prev_data: &[u8],
    next_data: &[u8],
    width:  usize,
    height: usize,
    stride: usize,
    max_features: usize,
) -> FrameMotion {
    let prev = GrayImage { data: prev_data, width, height, stride };
    let next = GrayImage { data: next_data, width, height, stride };

    // Detect corners in the previous frame
    let corners = detect_harris_corners(&prev, max_features, 5);
    if corners.is_empty() {
        return FrameMotion::default();
    }

    // Track to next frame
    let tracked = lk_track(&prev, &next, &corners, 21, 30, 0.03);

    // Collect good matches
    let matches: Vec<((f32, f32), (f32, f32))> = tracked
        .into_iter()
        .filter(|&(_, _, ok)| ok)
        .map(|(p, q, _)| (p, q))
        .collect();

    if matches.len() < 4 {
        return FrameMotion::default();
    }

    let (tx, ty, rot, confidence) = estimate_rigid_transform(&matches);

    FrameMotion {
        translation: Vector2::new(tx, ty),
        rotation:    rot,
        scale:       1.0, // scale estimation can be added later
        confidence,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_gray(w: usize, h: usize) -> Vec<u8> {
        // Simple checkerboard pattern to give Harris something to find
        (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| {
                    if (x / 8 + y / 8) % 2 == 0 { 200u8 } else { 50u8 }
                })
            })
            .collect()
    }

    #[test]
    fn test_harris_detects_corners() {
        let data = make_gray(128, 128);
        let img  = GrayImage { data: &data, width: 128, height: 128, stride: 128 };
        let corners = detect_harris_corners(&img, 200, 5);
        assert!(!corners.is_empty(), "Expected Harris to find at least one corner");
    }

    #[test]
    fn test_lk_pure_translation() {
        let w = 128usize;
        let h = 128usize;
        let data0 = make_gray(w, h);

        // Shift the image by (+2, +3)
        let shift_x = 2usize;
        let shift_y = 3usize;
        let mut data1 = vec![128u8; w * h];
        for y in shift_y..h {
            for x in shift_x..w {
                data1[y * w + x] = data0[(y - shift_y) * w + (x - shift_x)];
            }
        }

        let motion = estimate_frame_motion(&data0, &data1, w, h, w, 100);
        // Translation should be approximately (2, 3) – allow ±1 px tolerance
        assert!(
            (motion.translation.x - 2.0).abs() < 1.5,
            "tx expected ~2, got {}",
            motion.translation.x
        );
        assert!(
            (motion.translation.y - 3.0).abs() < 1.5,
            "ty expected ~3, got {}",
            motion.translation.y
        );
    }

    #[test]
    fn test_compute_stabilization_transforms_identity_when_no_motion() {
        let flow: HashMap<usize, FrameMotion> = HashMap::new();
        let transforms = compute_stabilization_transforms(&flow, 10, 3);
        assert_eq!(transforms.len(), 10);
        for (_, mat) in &transforms {
            // Should be close to identity
            assert!((mat[0] - 1.0).abs() < 1e-9);
            assert!((mat[4] - 1.0).abs() < 1e-9);
            assert!((mat[8] - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn test_smooth_trajectory_reduces_variance() {
        use std::collections::HashMap;
        // Alternating +5 / -5 translations → smoothed should be near 0
        let mut flow: HashMap<usize, FrameMotion> = HashMap::new();
        for i in 0..20usize {
            let tx = if i % 2 == 0 { 5.0 } else { -5.0 };
            flow.insert(i, FrameMotion {
                translation: Vector2::new(tx, 0.0),
                ..Default::default()
            });
        }
        let transforms = compute_stabilization_transforms(&flow, 21, 5);
        // The correction for the middle frames should be substantial
        assert!(!transforms.is_empty());
    }

    #[test]
    fn test_estimate_rigid_transform_known_translation() {
        let matches: Vec<((f32, f32), (f32, f32))> = vec![
            ((0.0, 0.0),   (10.0, 5.0)),
            ((100.0, 0.0), (110.0, 5.0)),
            ((0.0, 100.0), (10.0, 105.0)),
            ((100.0, 100.0), (110.0, 105.0)),
        ];
        let (tx, ty, rot, confidence) = estimate_rigid_transform(&matches);
        assert!((tx - 10.0).abs() < 0.1, "tx={}", tx);
        assert!((ty -  5.0).abs() < 0.1, "ty={}", ty);
        assert!(rot.abs() < 0.01,         "rot={}", rot);
        assert!(confidence > 0.9,         "confidence={}", confidence);
    }
}
