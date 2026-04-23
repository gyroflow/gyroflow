// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

#![allow(unused_variables, dead_code)]
use super::super::OpticalFlowPair;
use super::{ OpticalFlowTrait, OpticalFlowMethod };

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use parking_lot::RwLock;

// AI-based optical flow backend built on gmflow (Unimatch).
// The generated `model` module is produced by `burn-onnx` from a simplified
// gmflow-scale2-regrefine6 ONNX at build time. See `build.rs`.
#[cfg(feature = "use-burn")]
#[allow(clippy::all, dead_code, unused_imports, unused_variables)]
mod model {
    include!(concat!(env!("OUT_DIR"), "/model/gmflow-scale2-regrefine6-320x576-opset16-sim.rs"));
}

#[cfg(feature = "use-burn")]
mod inference {
    use super::model::Model;
    use std::sync::{ Arc, OnceLock };
    use image::GrayImage;
    use burn::backend::NdArray;
    use burn::tensor::backend::Backend as BurnBackend;
    use burn::tensor::{ Bytes, Tensor, TensorData };

    pub type Backend = NdArray<f32>;
    pub const MODEL_W: usize = 576;
    pub const MODEL_H: usize = 320;

    static MODEL: OnceLock<Arc<Model<Backend>>> = OnceLock::new();

    fn device() -> <Backend as BurnBackend>::Device { Default::default() }

    pub fn model() -> Arc<Model<Backend>> {
        MODEL.get_or_init(|| {
            let raw: &'static [u8] = include_bytes!(concat!(
                env!("OUT_DIR"),
                "/model/gmflow-scale2-regrefine6-320x576-opset16-sim.bpk"
            ));
            let bytes = Bytes::from_bytes_vec(raw.to_vec());
            Arc::new(Model::from_bytes(bytes, &device()))
        }).clone()
    }

    pub fn preprocess(img: &GrayImage) -> Tensor<Backend, 4> {
        let resized = image::imageops::resize(
            img,
            MODEL_W as u32,
            MODEL_H as u32,
            image::imageops::FilterType::Triangle,
        );
        // gmflow expects [B, 3, H, W] with raw 0-255 f32 (it normalises internally).
        // For grayscale frames, duplicate the luminance across all RGB channels.
        let mut data = Vec::with_capacity(3 * MODEL_H * MODEL_W);
        for _ in 0..3 {
            for p in resized.as_raw() { data.push(*p as f32); }
        }
        Tensor::<Backend, 4>::from_data(
            TensorData::new(data, [1usize, 3, MODEL_H, MODEL_W]),
            &device(),
        )
    }

    pub fn compute_flow(img0: &GrayImage, img1: &GrayImage) -> Vec<f32> {
        let t0 = preprocess(img0);
        let t1 = preprocess(img1);
        let m = model();
        let flow = m.forward(t0, t1);
        flow.into_data().to_vec::<f32>().expect("gmflow output is f32")
    }
}

#[derive(Clone)]
pub struct OFGmflow {
    features: Vec<(f32, f32)>,
    img: Arc<image::GrayImage>,
    matched_points: Arc<RwLock<BTreeMap<i64, (Vec<(f32, f32)>, Vec<(f32, f32)>)>>>,
    timestamp_us: i64,
    size: (u32, u32),
    used: Arc<AtomicU32>,
}

impl OFGmflow {
    pub fn detect_features(timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        Self {
            features: Vec::new(),
            img,
            matched_points: Default::default(),
            timestamp_us,
            size: (width, height),
            used: Default::default(),
        }
    }
}

impl OpticalFlowTrait for OFGmflow {
    fn size(&self) -> (u32, u32) { self.size }
    fn features(&self) -> &Vec<(f32, f32)> { &self.features }

    fn optical_flow_to(&self, _to: &OpticalFlowMethod) -> OpticalFlowPair {
        #[cfg(feature = "use-burn")]
        if let OpticalFlowMethod::OFGmflow(next) = _to {
            let (w, h) = self.size;
            if let Some(matched) = self.matched_points.read().get(&next.timestamp_us) {
                return Some(matched.clone());
            }
            if self.img.is_empty() || next.img.is_empty() || w == 0 || h == 0 { return None; }

            let flow = inference::compute_flow(&self.img, &next.img);
            let mh = inference::MODEL_H;
            let mw = inference::MODEL_W;
            let plane = mh * mw;
            if flow.len() < 2 * plane {
                log::error!("gmflow output length {} < expected {}", flow.len(), 2 * plane);
                return None;
            }
            let dx_plane = &flow[0..plane];
            let dy_plane = &flow[plane..2 * plane];

            // Scale factors from model resolution back to original frame resolution.
            let sx = w as f32 / mw as f32;
            let sy = h as f32 / mh as f32;

            // Mirror OFOpenCVDis: sample on a 15-wide grid and filter by local texture variance.
            let step = (w as usize / 15).max(1);
            let window_size = ((w as f32 * 0.02).round() as usize).max(10);
            let half_win = window_size / 2;
            let texture_threshold = 3.0_f32;

            let img = &*self.img;
            let iw = img.width() as isize;
            let ih = img.height() as isize;
            let calculate_texture = |x: usize, y: usize| -> f32 {
                let (x_i, y_i) = (x as isize, y as isize);
                let start_y = (y_i - half_win as isize).max(0);
                let end_y = (y_i + half_win as isize).min(ih - 1);
                let start_x = (x_i - half_win as isize).max(0);
                let end_x = (x_i + half_win as isize).min(iw - 1);
                let mut sum = 0.0f32;
                let mut sum_sq = 0.0f32;
                let mut count = 0.0f32;
                for ny in start_y..=end_y {
                    for nx in start_x..=end_x {
                        let p = img.get_pixel(nx as u32, ny as u32).0[0] as f32;
                        sum += p;
                        sum_sq += p * p;
                        count += 1.0;
                    }
                }
                if count == 0.0 { return 0.0; }
                let mean = sum / count;
                (sum_sq / count) - (mean * mean)
            };

            let mut points_a = Vec::new();
            let mut points_b = Vec::new();
            for j in (0..h as usize).step_by(step) {
                for i in (0..w as usize).step_by(step) {
                    if calculate_texture(i, j) <= texture_threshold { continue; }
                    let mi = ((i as f32 / sx).round() as usize).min(mw - 1);
                    let mj = ((j as f32 / sy).round() as usize).min(mh - 1);
                    let idx = mj * mw + mi;
                    let dx = dx_plane[idx] * sx;
                    let dy = dy_plane[idx] * sy;
                    points_a.push((i as f32, j as f32));
                    points_b.push((i as f32 + dx, j as f32 + dy));
                }
            }

            if points_a.len() >= 10 {
                self.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                next.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let res = (points_a, points_b);
                self.matched_points.write().insert(next.timestamp_us, res.clone());
                return Some(res);
            }
        }
        None
    }

    fn can_cleanup(&self) -> bool {
        self.used.load(std::sync::atomic::Ordering::SeqCst) == 2
    }
    fn cleanup(&mut self) { self.img = Arc::new(image::GrayImage::default()); }
}
