use super::*;
use crate::undistortion::undistort_points_with_rolling_shutter;

/*
Direct FOV calculation:
    - gets polygon points around the outline of the undistorted image
    - (1) draws a symetric rectangle around center with max. distance
    -     if a polygon point happens to be inside the rectangle, it becomes the nearest point and the rectangle shrinks, repeat for all points
    - (2) casts an infinite ray from the center point of the view diagonally (with the output aspect ratio determining the angle)
    -     finds the intersections between the ray an the polygon lines
    - (3) casts a second mirrored ray and finds intersections with the polygon
    -     gets the nearest intersections from both rays
    - from the nearest point of (1),(2) or (3), calculate the FOV
*/

#[derive(Clone)]
pub struct FovDirect {
    input_dim: (f64, f64), 
    output_dim: (f64, f64),
    output_inv_aspect: f64,
    compute_params: ComputeParams
}
impl FieldOfViewAlgorithm for FovDirect { 
    fn compute(&self, timestamps: &[f64], range: (f64, f64)) -> (Vec<f64>, Vec<Point2D>) {
        if timestamps.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let src_rect = points_around_rect(self.input_dim.0, self.input_dim.1, 15, 15);
        let polygons: Vec<Vec<(f64, f64)>> = timestamps
            .iter()
            .map(|&ts| undistort_points_with_rolling_shutter(&src_rect, ts, &self.compute_params))
            .collect();

        let cp = Point2D(self.input_dim.0 / 2.0, self.input_dim.1 / 2.0);
        let center_positions: Vec<Point2D> = polygons.iter().map(|_| cp).collect();

        let mut fov_values: Vec<f64> = polygons.iter()
            .zip(&center_positions)
            .map(|(polygon, center)| self.find_fov(polygon, center))
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

impl FovDirect { 
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

    fn find_fov(&self, polygon: &[(f64, f64)], center: &Point2D) -> f64 {
        let relpoints: Vec<(f64, f64)> = polygon.iter().map(|(x, y)| (x - center.0, y - center.1)).collect();
        let initial_nearest: (f64,f64) = (1000000.0, 1000000.0*self.output_inv_aspect);

        let mut nearest_point = relpoints
            .iter()
            .fold(initial_nearest, |mp, &point| {
                let ap = (point.0.abs(), point.1.abs());
                if ap.0 < mp.0 && ap.1 < mp.1 {
                    if ap.1 > ap.0 * self.output_inv_aspect {
                        return (ap.1 / self.output_inv_aspect, ap.1);
                    } else {
                        return (ap.0, ap.0 * self.output_inv_aspect);
                    }
                }
                mp
            });

        let intersections_up = polygon_line_intersections(&(0.0, 0.0), self.output_inv_aspect, &relpoints);
        let intersections_down = polygon_line_intersections(&(0.0, 0.0), -self.output_inv_aspect, &relpoints);
        let min_intersection: (f64, f64) = intersections_up
            .iter()
            .chain(&intersections_down)
            .fold(nearest_point, |mp, &point| { 
                if point.0.abs() < mp.0.abs() { point } else { mp } 
            });
        nearest_point = (min_intersection.0.abs(), min_intersection.1.abs());

        nearest_point.0 * 2.0 / self.output_dim.0
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

// Return Some((x, y)), where the ray p0->[x,x*rise] intersects the line p2->p3
// Returns None, when they don't intersect
fn line_intersection(p0: &(f64,f64), rise: f64, p2: &(f64,f64), p3: &(f64,f64)) -> Option<(f64,f64)> {
	let s32 = (p3.0 - p2.0, p3.1 - p2.1);
	
	let denom = s32.0 * rise - s32.1;
	if denom == 0.0 { return None; }
	
	let s20 = (p2.0 - p0.0, p2.1 - p0.1);
	let numer = s20.1 - rise * s20.0;
	
	let t = numer / denom;
	if t < 0.0 || t > 1.0 {
        None
    } else {
        Some((p2.0 + t * s32.0, p2.1 + t * s32.1))
    }
}

// Return a Vector with all the points where the line p0->[x,x*rise] intersects with the polygon
fn polygon_line_intersections(p0: &(f64,f64), rise: f64, polygon: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let len = polygon.len();
    polygon
        .iter()
        .enumerate()
        .filter_map(|(i, pp)| {
            let j = (i + len - 1) % len;
            if pp.0.abs() > 500000.0 || pp.1.abs() > 500000.0 || polygon[j].0.abs() > 500000.0 || polygon[j].1.abs() > 500000.0 {
                None
            } else {
                line_intersection(p0, rise, pp, &polygon[j])
            }
        })
        .collect()
}
