// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(unused_variables, dead_code)]
use super::super::OpticalFlowPair;
use super::{ OpticalFlowTrait, OpticalFlowMethod };

use std::collections::BTreeMap;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use parking_lot::RwLock;
#[cfg(feature = "use-opencv")]
use opencv::{ core::{ Mat, Size, CV_8UC1, Vec2f }, prelude::{ MatTraitConst, DenseOpticalFlowTrait } };

#[derive(Clone)]
pub struct OFOpenCVDis {
    features: Vec<(f32, f32)>,
    img: Arc<image::GrayImage>,
    matched_points: Arc<RwLock<BTreeMap<i64, (Vec<(f32, f32)>, Vec<(f32, f32)>)>>>,
    timestamp_us: i64,
    size: (i32, i32),
    used: Arc<AtomicU32>,
}

impl OFOpenCVDis {
    fn valid_layout(img: &image::GrayImage, size: (i32, i32)) -> bool {
        size.0 > 0 && size.1 > 0 && img.width() >= size.0 as u32 && img.height() >= size.1 as u32
    }

    pub fn detect_features(timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        Self {
            features: Vec::new(),
            timestamp_us,
            size: (width as i32, height as i32),
            matched_points: Default::default(),
            img,
            used: Default::default()
        }
    }
}

impl OpticalFlowTrait for OFOpenCVDis {
    fn size(&self) -> (u32, u32) {
        (self.size.0 as u32, self.size.1 as u32)
    }
    fn features(&self) -> &Vec<(f32, f32)> { &self.features }

    fn optical_flow_to(&self, _to: &OpticalFlowMethod) -> OpticalFlowPair {
        #[cfg(feature = "use-opencv")]
        if let OpticalFlowMethod::OFOpenCVDis(next) = _to {
            let (w, h) = self.size;
            if self.size != next.size { return None; }
            if let Some(matched) = self.matched_points.read().get(&next.timestamp_us) {
                return Some(matched.clone());
            }
            if !Self::valid_layout(&self.img, self.size) || !Self::valid_layout(&next.img, next.size) { return None; }

            let result = || -> Result<(Vec<(f32, f32)>, Vec<(f32, f32)>), opencv::Error> {
                let mut a1_img = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, self.img.as_raw().as_ptr() as *mut std::ffi::c_void, self.img.width() as usize) }?;
                let mut a2_img = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, next.img.as_raw().as_ptr() as *mut std::ffi::c_void, next.img.width() as usize) }?;
                // DIS requires continuous input, unlike PyrLK. Copy only when
                // visible rows are narrower than the backing image's row pitch.
                if !a1_img.is_continuous() { a1_img = a1_img.try_clone()?; }
                if !a2_img.is_continuous() { a2_img = a2_img.try_clone()?; }

                let mut of = Mat::default();
                let mut reverse = Mat::default();
                let mut optflow = opencv::video::DISOpticalFlow::create(opencv::video::DISOpticalFlow_PRESET_FAST)?;
                optflow.calc(&a1_img, &a2_img, &mut of)?;
                optflow.calc(&a2_img, &a1_img, &mut reverse)?;

                // Reverse flow is defined at target-frame pixels. Interpolate
                // it at the forward endpoint to measure round-trip agreement.
                let reverse_at = |x: f32, y: f32| -> Result<Option<(f32, f32)>, opencv::Error> {
                    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x > (w - 1) as f32 || y > (h - 1) as f32 { return Ok(None); }
                    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
                    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
                    let (dx, dy) = (x - x0 as f32, y - y0 as f32);
                    let p00 = reverse.at_2d::<Vec2f>(y0, x0)?;
                    let p10 = reverse.at_2d::<Vec2f>(y0, x1)?;
                    let p01 = reverse.at_2d::<Vec2f>(y1, x0)?;
                    let p11 = reverse.at_2d::<Vec2f>(y1, x1)?;
                    let interpolate = |k: usize| (p00[k] * (1.0 - dx) + p10[k] * dx) * (1.0 - dy) +
                        (p01[k] * (1.0 - dx) + p11[k] * dx) * dy;
                    Ok(Some((interpolate(0), interpolate(1))))
                };

                let mut points_a = Vec::new();
                let mut points_b = Vec::new();
                let step = (w as usize / 15).max(1); // Approximately 15 columns
                
                // Calculate window size as 2% of image width, minimum 10
                let window_size = (w as f32 * 0.02).round() as usize;
                let window_size = window_size.max(10);
                let texture_threshold = 3.0; // Threshold for texture clarity
                
                // Pre-calculate half window size for efficiency
                let half_win = window_size / 2;
                
                // Function to calculate variance of grayscale values in a window (more accurate than gradient)
                let calculate_texture = |img: &image::GrayImage, x: usize, y: usize| -> f32 {
                    let mut sum = 0.0;
                    let mut sum_sq = 0.0;
                    let mut count = 0.0;
                    
                    // Cache image dimensions as isize for faster comparisons
                    let img_width = w as isize;
                    let img_height = h as isize;
                    let x_isize = x as isize;
                    let y_isize = y as isize;
                    
                    // Calculate valid pixel boundaries once
                    let start_y = (y_isize - half_win as isize).max(0);
                    let end_y = (y_isize + half_win as isize).min(img_height - 1);
                    let start_x = (x_isize - half_win as isize).max(0);
                    let end_x = (x_isize + half_win as isize).min(img_width - 1);
                    
                    // Iterate only over valid pixels, avoiding repeated boundary checks
                    for ny in start_y..=end_y {
                        for nx in start_x..=end_x {
                            let pixel = img.get_pixel(nx as u32, ny as u32).0[0] as f32;
                            sum += pixel;
                            sum_sq += pixel * pixel;
                            count += 1.0;
                        }
                    }
                    
                    if count == 0.0 { return 0.0; }
                    
                    let mean = sum / count;
                    let variance = (sum_sq / count) - (mean * mean);
                    variance
                };
                
                for i in (0..a1_img.cols()).step_by(step) {
                    for j in (0..a1_img.rows()).step_by(step) {
                        // Check texture clarity using accurate variance method
                        let texture = calculate_texture(&self.img, i as usize, j as usize);
                        if texture > texture_threshold {
                            let pt = of.at_2d::<Vec2f>(j, i)?;
                            let (x, y) = (i as f32 + pt[0], j as f32 + pt[1]);
                            if let Some(back) = reverse_at(x, y)? {
                                if (pt[0] + back.0).hypot(pt[1] + back.1) <= 1.5 &&
                                    calculate_texture(&next.img, x.round() as usize, y.round() as usize) > texture_threshold {
                                    points_a.push((i as f32, j as f32));
                                    points_b.push((x, y));
                                }
                            }
                        }
                    }
                }
                Ok((points_a, points_b))
            }();

            match result {
                Ok(res) => {
                    // Keep at least ten reliable correspondences for pose estimation.
                    if res.0.len() >= 10 && res.1.len() >= 10 {
                        self.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        next.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        self.matched_points.write().insert(next.timestamp_us, res.clone());
                        return Some(res);
                    }
                },
                Err(e) => {
                    log::error!("OpenCV error: {:?}", e);
                }
            }
        }
        None
    }
    fn can_cleanup(&self) -> bool {
        self.used.load(std::sync::atomic::Ordering::SeqCst) == 2
    }
    fn cleanup(&mut self) {
        self.img = Arc::new(image::GrayImage::default());
    }
}

#[cfg(all(test, feature = "use-opencv"))]
mod tests {
    use super::*;

    fn texture(width: u32, height: u32) -> image::GrayImage {
        image::GrayImage::from_fn(width, height, |x, y| {
            let mut v = (x / 4).wrapping_mul(73_856_093) ^ (y / 4).wrapping_mul(19_349_663);
            v ^= v >> 13;
            v = v.wrapping_mul(1_274_126_177);
            image::Luma([(v >> 24) as u8])
        })
    }

    fn shifted(source: &image::GrayImage, occlude: bool) -> image::GrayImage {
        let other = texture(source.width() + 113, source.height() + 79);
        image::GrayImage::from_fn(source.width(), source.height(), |x, y| {
            if occlude && (70..245).contains(&x) && (55..190).contains(&y) {
                *other.get_pixel(x + 113, y + 79)
            } else if x >= 6 && y >= 4 {
                *source.get_pixel(x - 6, y - 4)
            } else {
                image::Luma([0])
            }
        })
    }

    fn frame(timestamp: i64, image: image::GrayImage, width: u32, height: u32) -> OpticalFlowMethod {
        OpticalFlowMethod::OFOpenCVDis(OFOpenCVDis::detect_features(timestamp, Arc::new(image), width, height))
    }

    fn assert_translation(pair: OpticalFlowPair, minimum: usize, percent: usize) {
        let (a, b) = pair.expect("Known translated texture should retain usable support");
        assert_eq!(a.len(), b.len());
        assert!(a.len() >= minimum, "Only {} correspondences survived", a.len());
        let accurate = a.iter().zip(&b).filter(|(a, b)| {
            (b.0 - a.0 - 6.0).hypot(b.1 - a.1 - 4.0) < 0.75
        }).count();
        assert!(accurate * 100 >= a.len() * percent, "{accurate}/{} agree with the known motion", a.len());
        assert!(b.iter().all(|p| p.0 >= 0.0 && p.0 < 320.0 && p.1 >= 0.0 && p.1 < 240.0),
            "Tracks must end on visible image content");
    }

    #[test]
    fn dis_recovers_translation_with_useful_support() {
        let source = texture(320, 240);
        let target = shifted(&source, false);
        assert_translation(frame(0, source, 320, 240).optical_flow_to(&frame(33_333, target, 320, 240)), 100, 95);
    }

    #[test]
    fn dis_occlusion_does_not_dominate_known_background_motion() {
        let source = texture(320, 240);
        let target = shifted(&source, true);
        assert_translation(frame(0, source, 320, 240).optical_flow_to(&frame(33_333, target, 320, 240)), 40, 90);
    }

    #[test]
    fn dis_handles_different_row_pitches() {
        let source = texture(320, 240);
        let target = shifted(&source, false);
        let a = image::GrayImage::from_fn(336, 240, |x, y| if x < 320 { *source.get_pixel(x, y) } else { image::Luma([255]) });
        let b = image::GrayImage::from_fn(352, 240, |x, y| if x < 320 { *target.get_pixel(x, y) } else { image::Luma([0]) });
        assert_translation(frame(0, a, 320, 240).optical_flow_to(&frame(33_333, b, 320, 240)), 100, 95);
    }

    #[test]
    fn dis_rejects_unbacked_or_incompatible_geometry() {
        let a = frame(0, texture(320, 240), 320, 240);
        let smaller = frame(33_333, texture(160, 120), 160, 120);
        assert!(a.optical_flow_to(&smaller).is_none());
        let unbacked = frame(33_333, texture(160, 120), 320, 240);
        assert!(a.optical_flow_to(&unbacked).is_none());
        let wrong_visible_size = frame(33_333, texture(320, 240), 160, 120);
        assert!(a.optical_flow_to(&wrong_visible_size).is_none());
    }

    #[test]
    fn dis_small_images_do_not_panic() {
        let a = frame(0, texture(12, 32), 12, 32);
        let b = frame(33_333, texture(12, 32), 12, 32);
        // The DIS native patch size may reject this input. Either a valid result
        // or no result is acceptable; a zero-step panic is not.
        let _ = a.optical_flow_to(&b);
    }

    #[test]
    fn dis_textureless_or_disappearing_content_has_no_tracks() {
        let blank = image::GrayImage::from_pixel(320, 240, image::Luma([127]));
        assert!(frame(0, blank.clone(), 320, 240).optical_flow_to(&frame(33_333, blank.clone(), 320, 240)).is_none());
        assert!(frame(0, texture(320, 240), 320, 240).optical_flow_to(&frame(33_333, blank, 320, 240)).is_none());
    }
}
