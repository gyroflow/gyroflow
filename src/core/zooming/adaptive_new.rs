use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use super::*;
use super::field_of_view::FieldOfView;

#[derive(Clone)]
pub struct AdaptiveNew {
    compute_params: ComputeParams,
    mode: Mode, 
}
impl ZoomingAlgorithm for AdaptiveNew {   
    fn get_state_checksum(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        if self.compute_params.distortion_coeffs.len() >= 4 {
            hasher.write_u64(self.compute_params.distortion_coeffs[0].to_bits());
            hasher.write_u64(self.compute_params.distortion_coeffs[1].to_bits());
            hasher.write_u64(self.compute_params.distortion_coeffs[2].to_bits());
            hasher.write_u64(self.compute_params.distortion_coeffs[3].to_bits());
        }
        
        hasher.write_usize(self.compute_params.video_width);
        hasher.write_usize(self.compute_params.video_height);
        hasher.write_usize(self.compute_params.video_output_width);
        hasher.write_usize(self.compute_params.video_output_height);
        hasher.write_u64(self.compute_params.scaled_fps.to_bits());
        hasher.write_u64(self.compute_params.trim_start.to_bits());
        hasher.write_u64(self.compute_params.trim_end.to_bits());
        hasher.write_u64(self.compute_params.video_rotation.to_bits());
        match self.mode {
            Mode::Disabled => hasher.write_u64(0),
            Mode::StaticZoom => hasher.write_u64(1),
            Mode::DynamicZoom(w) => hasher.write_u64(w.to_bits())
        }
        hasher.finish()
    }   

    fn compute(&self, timestamps: &[f64]) -> Vec<(f64, Point2D)> {
         if self.mode == Mode::Disabled || timestamps.is_empty() {
            return Vec::new();
        }

        let fov_est = FieldOfView::new(self.compute_params.clone());
        let (mut fov_values, center_position) = fov_est.compute(timestamps, (self.compute_params.trim_start, self.compute_params.trim_end));

        match self.mode {
            Mode::DynamicZoom(window_s) => {
                let mut frames = (window_s * self.compute_params.scaled_fps).floor() as usize;
                if frames % 2 == 0 {
                    frames += 1;
                }
    
                let fov_values_pad = pad_edge(&fov_values, (frames / 2, frames / 2));
                let fov_min = min_rolling(&fov_values_pad, frames);
                let fov_min_pad = pad_edge(&fov_min, (frames / 2, frames / 2));
    
                let gaussian = gaussian_window_normalized(frames, frames as f64 / 6.0);
                fov_values = convolve(&fov_min_pad, &gaussian);
            },
            Mode::StaticZoom => {
                if let Some(max_f) = fov_values.iter().copied().reduce(f64::min) {
                    fov_values.iter_mut().for_each(|v| *v = max_f);
                } else {
                    log::warn!("Unable to find min of fov_values, len: {}", fov_values.len());
                }
            }
            _ => { }
        }

        fov_values.iter().copied().zip(center_position.iter().copied()).collect()
    }
}

impl AdaptiveNew {
    pub fn new(compute_params: ComputeParams, mode: Mode) -> Self {
        Self {
            compute_params,
            mode
        }
    }
}

fn min_rolling(a: &[f64], window: usize) -> Vec<f64> {
    a.windows(window).filter_map(|window| {
        window.iter().copied().reduce(f64::min)
    }).collect()
}

fn convolve(v: &[f64], filter: &[f64]) -> Vec<f64> {
    v.windows(filter.len()).map(|window| {
        window.iter().zip(filter).map(|(x, y)| x * y).sum()
    }).collect()
}

fn gaussian_window(m: usize, std: f64) -> Vec<f64> {
    let step = 1.0 / m as f64;
    let n: Vec<f64> = (0..m).map(|i| (i as f64 * step) - (m as f64 - 1.0) / 2.0).collect();
    let sig2 = 2.0 * std * std;
    n.iter().map(|&v| (-v).powf(2.0) / sig2).collect()
}
fn gaussian_window_normalized(m: usize, std: f64) -> Vec<f64> {
    let mut w = gaussian_window(m, std);
    let sum: f64 = w.iter().sum();
    w.iter_mut().for_each(|v| *v /= sum);
    w
}

fn pad_edge(arr: &[f64], pad_to: (usize, usize)) -> Vec<f64> {
    let first = *arr.first().unwrap_or(&0.0);
    let last = *arr.last().unwrap_or(&0.0);

    let mut new_arr = vec![0.0; arr.len() + pad_to.0 + pad_to.1];
    new_arr[pad_to.0..pad_to.0 + arr.len()].copy_from_slice(arr);

    for i in 0..pad_to.0 { new_arr[i] = first; }
    for i in pad_to.0 + arr.len()..new_arr.len() { new_arr[i] = last; }

    new_arr
}
