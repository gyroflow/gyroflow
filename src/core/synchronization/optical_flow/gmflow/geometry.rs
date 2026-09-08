// SPDX-License-Identifier: GPL-3.0-or-later

use super::OpticalFlowPair;
#[cfg(feature = "use-gmflow")]
use gyroflow_gmflow::{HEIGHT, WIDTH};
#[cfg(not(feature = "use-gmflow"))]
const WIDTH: usize = 576;
#[cfg(not(feature = "use-gmflow"))]
const HEIGHT: usize = 320;
pub(super) const MIN_MATCHES: usize = 10;
const GRID_STEP: usize = 12;
const PATCH_RADIUS: u32 = 3;
const MIN_TEXTURE_VARIANCE: f32 = 3.0;

#[derive(Clone, Copy)]
pub(super) struct Layout {
    size: (u32, u32),
    resized: (u32, u32),
    padding: (u32, u32),
    scale: (f32, f32),
}

impl Layout {
    pub fn new(size: (u32, u32)) -> Option<Self> {
        if size.0 == 0 || size.1 == 0 {
            return None;
        }
        let scale = (WIDTH as f64 / size.0 as f64).min(HEIGHT as f64 / size.1 as f64);
        let w = (size.0 as f64 * scale).round().clamp(1.0, WIDTH as f64) as u32;
        let h = (size.1 as f64 * scale).round().clamp(1.0, HEIGHT as f64) as u32;
        Some(Self {
            size,
            resized: (w, h),
            padding: ((WIDTH as u32 - w) / 2, (HEIGHT as u32 - h) / 2),
            scale: (w as f32 / size.0 as f32, h as f32 / size.1 as f32),
        })
    }

    #[cfg(feature = "use-gmflow")]
    pub fn prepare(&self, img: &image::GrayImage) -> Result<Vec<f32>, String> {
        if img.width() < self.size.0 || img.height() < self.size.1 {
            return Err("image storage is smaller than its visible dimensions".into());
        }
        // GrayImage's width may be a decoder stride. Crop before resizing.
        let visible = image::imageops::crop_imm(img, 0, 0, self.size.0, self.size.1);
        let resized = image::imageops::resize(
            &*visible,
            self.resized.0,
            self.resized.1,
            image::imageops::FilterType::Triangle,
        );
        let mut luminance = Vec::with_capacity(WIDTH * HEIGHT);
        for y in 0..HEIGHT {
            let iy = (y as i32 - self.padding.1 as i32).clamp(0, self.resized.1 as i32 - 1) as u32;
            for x in 0..WIDTH {
                let ix =
                    (x as i32 - self.padding.0 as i32).clamp(0, self.resized.0 as i32 - 1) as u32;
                luminance.push(resized.get_pixel(ix, iy).0[0] as f32);
            }
        }
        // UniMatch includes RGB normalization; inputs remain in the 0..255 range.
        Ok(luminance.repeat(3))
    }

    fn model_point(&self, (x, y): (f32, f32)) -> (f32, f32) {
        (
            (x + 0.5) * self.scale.0 - 0.5 + self.padding.0 as f32,
            (y + 0.5) * self.scale.1 - 0.5 + self.padding.1 as f32,
        )
    }

    pub fn features(&self, img: &image::GrayImage) -> Vec<(f32, f32)> {
        let mut points = Vec::new();
        let margin = GRID_STEP as u32 / 2;
        for y in (margin..self.resized.1.saturating_sub(margin)).step_by(GRID_STEP) {
            for x in (margin..self.resized.0.saturating_sub(margin)).step_by(GRID_STEP) {
                let p = (
                    (x as f32 + 0.5) / self.scale.0 - 0.5,
                    (y as f32 + 0.5) / self.scale.1 - 0.5,
                );
                if p.0 < PATCH_RADIUS as f32
                    || p.1 < PATCH_RADIUS as f32
                    || p.0 > self.size.0 as f32 - (PATCH_RADIUS + 1) as f32
                    || p.1 > self.size.1 as f32 - (PATCH_RADIUS + 1) as f32
                {
                    continue;
                }
                let (cx, cy) = (p.0.round() as u32, p.1.round() as u32);
                let (mut sum, mut squared_sum) = (0.0, 0.0);
                for iy in cy - PATCH_RADIUS..=cy + PATCH_RADIUS {
                    for ix in cx - PATCH_RADIUS..=cx + PATCH_RADIUS {
                        let v = img.get_pixel(ix, iy).0[0] as f32;
                        sum += v;
                        squared_sum += v * v;
                    }
                }
                let count = (2 * PATCH_RADIUS + 1).pow(2) as f32;
                let variance = squared_sum / count - (sum / count).powi(2);
                if variance > MIN_TEXTURE_VARIANCE {
                    points.push(p);
                }
            }
        }
        points
    }

    pub fn correspondences(&self, flow: &[f32], features: &[(f32, f32)]) -> OpticalFlowPair {
        if flow.len() != 2 * WIDTH * HEIGHT {
            return None;
        }
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for &p in features {
            let (x, y) = self.model_point(p);
            if x < 0.0 || y < 0.0 || x > (WIDTH - 1) as f32 || y > (HEIGHT - 1) as f32 {
                continue;
            }
            let (ix, iy) = (x.floor() as usize, y.floor() as usize);
            let (nx, ny) = ((ix + 1).min(WIDTH - 1), (iy + 1).min(HEIGHT - 1));
            let (ax, ay) = (x - ix as f32, y - iy as f32);
            let sample = |c| {
                let at = |x, y| flow[c * WIDTH * HEIGHT + y * WIDTH + x];
                let upper = at(ix, iy) * (1.0 - ax) + at(nx, iy) * ax;
                let lower = at(ix, ny) * (1.0 - ax) + at(nx, ny) * ax;
                upper * (1.0 - ay) + lower * ay
            };
            // Rounded resize dimensions need independent x/y scales. Padding
            // cancels from displacement, but remains in the sampling location.
            let q = (
                p.0 + sample(0) / self.scale.0,
                p.1 + sample(1) / self.scale.1,
            );
            if q.0.is_finite()
                && q.1.is_finite()
                && q.0 >= 0.0
                && q.1 >= 0.0
                && q.0 <= (self.size.0 - 1) as f32
                && q.1 <= (self.size.1 - 1) as f32
            {
                a.push(p);
                b.push(q);
            }
        }
        if a.len() >= MIN_MATCHES {
            Some((a, b))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_landscape_and_portrait_sizes_preserve_affine_flow_coordinates() {
        for size in [(1919, 1079), (719, 1281), (1001, 1000), (576, 320)] {
            let layout = Layout::new(size).unwrap();
            let mut flow = vec![0.0; 2 * WIDTH * HEIGHT];
            // Affine displacement exercises fractional sampling, both rounded
            // scales, and padding, instead of only testing constant translations.
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let ox = (x as f32 - layout.padding.0 as f32 + 0.5) / layout.scale.0 - 0.5;
                    let oy = (y as f32 - layout.padding.1 as f32 + 0.5) / layout.scale.1 - 0.5;
                    flow[y * WIDTH + x] = (2.0 + 0.01 * oy) * layout.scale.0;
                    flow[WIDTH * HEIGHT + y * WIDTH + x] = (-1.0 + 0.02 * ox) * layout.scale.1;
                }
            }
            let points: Vec<_> = (1..=12)
                .map(|i| (size.0 as f32 * i as f32 / 15.0, size.1 as f32 * 0.4))
                .collect();
            let (a, b) = layout.correspondences(&flow, &points).unwrap();
            assert_eq!(a.len(), points.len());
            for (p, q) in a.iter().zip(b) {
                assert!((q.0 - p.0 - (2.0 + 0.01 * p.1)).abs() < 0.001);
                assert!((q.1 - p.1 - (-1.0 + 0.02 * p.0)).abs() < 0.001);
            }
        }
    }

    #[test]
    fn invalid_or_outside_flow_is_not_returned_as_a_camera_match() {
        let layout = Layout::new((576, 320)).unwrap();
        let points = vec![(200.0, 100.0); 12];
        assert!(layout.correspondences(&[0.0; 20], &points).is_none());
        for bad in [f32::NAN, f32::INFINITY, 1000.0] {
            assert!(
                layout
                    .correspondences(&vec![bad; 2 * WIDTH * HEIGHT], &points)
                    .is_none()
            );
        }
        assert!(Layout::new((0, 10)).is_none());
    }

    #[test]
    fn uniform_images_and_tiny_images_have_no_useful_features() {
        let flat = image::GrayImage::from_pixel(600, 400, image::Luma([45]));
        assert!(Layout::new((600, 400)).unwrap().features(&flat).is_empty());
        let tiny = image::GrayImage::new(1, 1);
        assert!(Layout::new((1, 1)).unwrap().features(&tiny).is_empty());
    }

    #[cfg(feature = "use-gmflow")]
    #[test]
    fn decoder_stride_bytes_are_excluded_from_network_input() {
        let mut padded = image::GrayImage::from_pixel(608, 320, image::Luma([255]));
        for y in 0..320 {
            for x in 0..576 {
                padded.put_pixel(x, y, image::Luma([37]));
            }
        }
        let data = Layout::new((576, 320)).unwrap().prepare(&padded).unwrap();
        assert!(data.iter().all(|&x| x == 37.0));
        assert_eq!(data.len(), 3 * WIDTH * HEIGHT);
    }
}
