use super::*;
use crate::undistortion::undistort_points_with_rolling_shutter;

/*
Iterative FOV calculation:
    - gets polygon points around the outline of the undistorted image
    - draws a symetric rectangle around center
    - if a polygon point happens to be inside the rectangle, it becomes the nearest point and the rectangle shrinks, repeat for all points
    - interpolate between the points around the nearest polygon point
    - repeat shrinking the rectangle
*/

#[derive(Clone)]
pub struct FovIterative {
    input_dim: (f64, f64), 
    output_dim: (f64, f64),
    output_inv_aspect: f64,
    compute_params: ComputeParams
}
impl FieldOfViewAlgorithm for FovIterative { 
    fn compute(&self, timestamps: &[f64], range: (f64, f64)) -> (Vec<f64>, Vec<Point2D>) {
        if timestamps.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let rect = points_around_rect(self.input_dim.0, self.input_dim.1, 31, 31);

        let cp = Point2D(self.input_dim.0 / 2.0, self.input_dim.1 / 2.0);
        let center_positions: Vec<Point2D> = timestamps.iter().map(|_| cp).collect();

        let mut fov_values: Vec<f64> = timestamps.iter()
            .zip(&center_positions)
            .map(|(&ts, center)| self.find_fov(&rect, ts, center))
            .collect();

        if range.0 > 0.0 || range.1 < 1.0 {
            // Only within render range.
            if let Some(max_fov) = fov_values.iter().copied().reduce(f64::max) {
                let l = (timestamps.len() - 1) as f64;
                let first_ind = (l * range.0).floor() as usize;
                let last_ind  = (l * range.1).ceil() as usize;
                if fov_values.len() > first_ind {
                    fov_values[0..first_ind].iter_mut().for_each(|v| *v = max_fov);
                }
                if fov_values.len() > last_ind {
                    fov_values[last_ind..].iter_mut().for_each(|v| *v = max_fov);
                }
            }
        }

        (fov_values, center_positions)
    }
}

impl FovIterative { 
    pub fn new(compute_params: ComputeParams) -> Self {
        let ratio = compute_params.video_width as f64 / compute_params.video_output_width.max(1) as f64;
        let input_dim = (compute_params.video_width as f64, compute_params.video_height as f64);
        let output_dim = (compute_params.video_output_width as f64 * ratio, compute_params.video_output_height as f64 * ratio);
        let output_inv_aspect = output_dim.1 / output_dim.0;

        Self {
            input_dim,
            output_dim,
            output_inv_aspect,
            compute_params
        }
    }

    fn find_fov(&self, rect: &[(f64, f64)], ts: f64, center: &Point2D) -> f64 {
        let mut polygon = undistort_points_with_rolling_shutter(&rect, ts, &self.compute_params);
        
        let initial: (f64,f64) = (1000000.0, 1000000.0*self.output_inv_aspect);
        let mut nearest = (None, initial);
        
        for _ in 1..5 {
            nearest = self.nearest_edge(&polygon, center, nearest.1);
            if let Some(idx) = nearest.0 {
                let len = rect.len();
                let relevant = [
                    rect[(idx - 1) % len], 
                    rect[idx],
                    rect[(idx + 1) % len]
                ];

                let distorted = interpolate_points(&relevant, 30);
                polygon = undistort_points_with_rolling_shutter(&distorted, ts, &self.compute_params);
                nearest = self.nearest_edge(&polygon, center, nearest.1);
            } else {
                break;
            }
        }
        
        nearest.1.0 * 2.0 / self.output_dim.0
    }

    fn nearest_edge(&self, polygon: &[(f64, f64)], center: &Point2D, initial: (f64, f64)) -> (Option<usize>,(f64, f64)) {
        polygon
            .iter()
            .enumerate()
            .fold((None, initial), |mp, (i, (x,y))| {
                let ap = ((x - center.0).abs(), (y - center.1).abs());
                if ap.0 < mp.1.0 && ap.1 < mp.1.1 {
                    if ap.1 > ap.0 * self.output_inv_aspect {
                        return (Some(i), (ap.1 / self.output_inv_aspect, ap.1));
                    } else {
                        return (Some(i), (ap.0, ap.0 * self.output_inv_aspect));
                    }
                }
                mp
            })
    }
}

// Returns points placed around a rectangle in a continous order
fn points_around_rect(w: f64, h: f64, w_div: usize, h_div: usize) -> Vec<(f64, f64)> {
    let (wcnt, hcnt) = (w_div.max(2) - 1, h_div.max(2) - 1);
    let (wstep, hstep) = (w / wcnt as f64, h / hcnt as f64);
    
    // ordered!
    let mut distorted_points: Vec<(f64, f64)> = Vec::with_capacity((wcnt + hcnt) * 2);
    for i in 0..wcnt { distorted_points.push((i as f64 * wstep,          0.0)); }
    for i in 0..hcnt { distorted_points.push((w,                         i as f64 * hstep)); }
    for i in 0..wcnt { distorted_points.push(((wcnt - i) as f64 * wstep, h)); }
    for i in 0..hcnt { distorted_points.push((0.0,                       (hcnt - i) as f64 * hstep)); }

    distorted_points
}

// linear interpolates steps between points in array
fn interpolate_points(pts: &[(f64, f64)], steps: usize) -> Vec<(f64,f64)> {
    let d = steps+1;
    let new_len = d * pts.len() - steps;
    (0..new_len).map(|i| {
        let idx1 = i / d;
        let idx2 = (idx1+1).min(pts.len()-1);
        let f = ((i % d) as f64) / (d as f64);
        (pts[idx1].0 + f * (pts[idx2].0 - pts[idx1].0), pts[idx1].1 + f * (pts[idx2].1 - pts[idx1].1))
    }).collect()
}