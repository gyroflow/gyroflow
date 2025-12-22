// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

/// Apply Gaussian smoothing to focal length data
/// 
/// # Arguments
/// * `focal_lengths` - Raw focal length values per frame
/// * `strength` - Smoothing strength from 0.0 to 1.0
/// * `window_size` - Size of the Gaussian window (should be odd)
/// 
/// # Returns
/// Smoothed focal length array
pub fn smooth_focal_lengths_gaussian(focal_lengths: &[Option<f64>], strength: f64, window_size: usize) -> Vec<Option<f64>> {
    if focal_lengths.is_empty() || strength <= 0.0 {
        return focal_lengths.to_vec();
    }

    let window_size = if window_size % 2 == 0 { window_size + 1 } else { window_size };
    let half_window = window_size / 2;
    
    // Generate Gaussian kernel
    let sigma = (window_size as f64 / 6.0) * (1.0 + strength * 2.0);
    let mut kernel = vec![0.0; window_size];
    let mut kernel_sum = 0.0;
    
    for i in 0..window_size {
        let x = i as f64 - half_window as f64;
        kernel[i] = (-x * x / (2.0 * sigma * sigma)).exp();
        kernel_sum += kernel[i];
    }
    
    // Normalize kernel
    for k in &mut kernel {
        *k /= kernel_sum;
    }
    
    let mut smoothed = Vec::with_capacity(focal_lengths.len());
    
    for i in 0..focal_lengths.len() {
        if focal_lengths[i].is_none() {
            smoothed.push(None);
            continue;
        }
        
        let mut weighted_sum = 0.0;
        let mut weight_sum = 0.0;
        
        for j in 0..window_size {
            let idx = (i as isize + j as isize - half_window as isize).max(0).min(focal_lengths.len() as isize - 1) as usize;
            if let Some(fl) = focal_lengths[idx] {
                weighted_sum += fl * kernel[j];
                weight_sum += kernel[j];
            }
        }
        
        if weight_sum > 0.0 {
            let smoothed_value = weighted_sum / weight_sum;
            // Blend between original and smoothed based on strength
            if let Some(original) = focal_lengths[i] {
                smoothed.push(Some(original * (1.0 - strength) + smoothed_value * strength));
            } else {
                smoothed.push(Some(smoothed_value));
            }
        } else {
            smoothed.push(focal_lengths[i]);
        }
    }
    
    smoothed
}
