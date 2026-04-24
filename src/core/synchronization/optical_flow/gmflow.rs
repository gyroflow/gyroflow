// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 yasumorishima <fwyasu11@gmail.com>

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

    // Aspect-preserving letterbox mapping parameters. `scale` is model-pixels per
    // original-pixel; `pad_x` / `pad_y` are the left/top zero-padding offsets in
    // model space. Kept separate from the tensor so `compute_flow` can move the
    // tensor into `forward` without cloning.
    pub struct LetterboxParams {
        pub scale: f32,
        pub pad_x: usize,
        pub pad_y: usize,
    }

    pub fn preprocess(img: &GrayImage) -> (Tensor<Backend, 4>, LetterboxParams) {
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let scale = (MODEL_W as f32 / iw).min(MODEL_H as f32 / ih);
        let new_w = ((iw * scale).round() as usize).min(MODEL_W).max(1);
        let new_h = ((ih * scale).round() as usize).min(MODEL_H).max(1);
        let pad_x = (MODEL_W - new_w) / 2;
        let pad_y = (MODEL_H - new_h) / 2;

        let resized = image::imageops::resize(
            img,
            new_w as u32,
            new_h as u32,
            image::imageops::FilterType::Triangle,
        );

        // Zero-padded letterbox into [1, 3, MODEL_H, MODEL_W]. gmflow expects raw
        // 0-255 f32 (normalises internally). Write the resized luminance into the
        // first channel slot in place, then memcpy it into channels 2 and 3 so we
        // do not allocate an auxiliary per-channel buffer.
        let mut data = vec![0.0f32; 3 * MODEL_H * MODEL_W];
        {
            let src_pixels = resized.as_raw();
            let (first, rest) = data.split_at_mut(MODEL_H * MODEL_W);
            for y in 0..new_h {
                let sr = y * new_w;
                let dr = (y + pad_y) * MODEL_W + pad_x;
                for x in 0..new_w {
                    first[dr + x] = src_pixels[sr + x] as f32;
                }
            }
            let (second, third) = rest.split_at_mut(MODEL_H * MODEL_W);
            second.copy_from_slice(first);
            third.copy_from_slice(first);
        }

        let tensor = Tensor::<Backend, 4>::from_data(
            TensorData::new(data, [1usize, 3, MODEL_H, MODEL_W]),
            &device(),
        );
        (tensor, LetterboxParams { scale, pad_x, pad_y })
    }

    // Returns `None` if the backend produces a non-f32 tensor instead of panicking.
    // Emits the first frame letterbox parameters so callers can invert the mapping
    // back into original-frame coordinates. Tensors are moved (not cloned) into
    // `forward` to avoid ~4.4 MB of redundant allocations per pair at 320x576.
    pub fn compute_flow(img0: &GrayImage, img1: &GrayImage) -> Option<(Vec<f32>, LetterboxParams)> {
        let (t0, lb0) = preprocess(img0);
        let (t1, _) = preprocess(img1);
        let m = model();
        let flow = m.forward(t0, t1);
        let data = flow.into_data().to_vec::<f32>().ok()?;
        Some((data, lb0))
    }
}

#[derive(Clone)]
#[cfg_attr(not(feature = "use-burn"), allow(dead_code))]
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
        // Populate `features` with the same texture-filtered grid that drives the flow
        // sampling so the "Show detected features" overlay renders something useful.
        let features = texture_filtered_grid(width, height, &img);
        Self {
            features,
            img,
            matched_points: Default::default(),
            timestamp_us,
            size: (width, height),
            used: Default::default(),
        }
    }
}

fn texture_filtered_grid(width: u32, height: u32, img: &image::GrayImage) -> Vec<(f32, f32)> {
    let w = width as usize;
    let h = height as usize;
    let step = (w / 15).max(1);
    let window_size = ((width as f32 * 0.02).round() as usize).max(10);
    let half_win = window_size / 2;
    let texture_threshold = 3.0_f32;

    let iw = img.width() as isize;
    let ih = img.height() as isize;
    let calculate_texture = |x: usize, y: usize| -> f32 {
        let (xi, yi) = (x as isize, y as isize);
        let sy_ = (yi - half_win as isize).max(0);
        let ey_ = (yi + half_win as isize).min(ih - 1);
        let sx_ = (xi - half_win as isize).max(0);
        let ex_ = (xi + half_win as isize).min(iw - 1);
        let mut sum = 0.0f32;
        let mut sum_sq = 0.0f32;
        let mut count = 0.0f32;
        for ny in sy_..=ey_ {
            for nx in sx_..=ex_ {
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
    let mut out = Vec::new();
    for j in (0..h).step_by(step) {
        for i in (0..w).step_by(step) {
            if calculate_texture(i, j) > texture_threshold {
                out.push((i as f32, j as f32));
            }
        }
    }
    out
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

            let (flow, lb) = match inference::compute_flow(&self.img, &next.img) {
                Some(v) => v,
                None => {
                    log::error!("gmflow: backend returned non-f32 tensor");
                    return None;
                }
            };
            let mh = inference::MODEL_H;
            let mw = inference::MODEL_W;
            let plane = mh * mw;
            if flow.len() < 2 * plane {
                log::error!("gmflow output length {} < expected {}", flow.len(), 2 * plane);
                return None;
            }
            let dx_plane = &flow[0..plane];
            let dy_plane = &flow[plane..2 * plane];

            // Letterbox mapping: original (i, j) -> model (i*scale + pad_x, j*scale + pad_y).
            // Flow at model (mi, mj) is in model-pixel units; divide by scale to recover original.
            let scale = lb.scale.max(1e-6);
            let pad_x = lb.pad_x;
            let pad_y = lb.pad_y;

            // `self.features` was populated at detect-time with the same
            // texture-filtered grid we want here; iterate it rather than repeating
            // the O(grid * window^2) variance scan on every pair, which also keeps
            // the "Show detected features" overlay exactly in sync with the points
            // we actually sample.
            let mut points_a = Vec::with_capacity(self.features.len());
            let mut points_b = Vec::with_capacity(self.features.len());
            for &(fi, fj) in &self.features {
                let mi = ((fi * scale).round() as usize + pad_x).min(mw - 1);
                let mj = ((fj * scale).round() as usize + pad_y).min(mh - 1);
                let idx = mj * mw + mi;
                let dx = dx_plane[idx] / scale;
                let dy = dy_plane[idx] / scale;
                points_a.push((fi, fj));
                points_b.push((fi + dx, fj + dy));
            }

            if points_a.len() >= 10 {
                // Double-checked locking: if another thread already inserted the result
                // for this (self, next) pair, return its value without bumping `used`
                // a second time on either endpoint.
                let mut mp = self.matched_points.write();
                if let Some(matched) = mp.get(&next.timestamp_us) {
                    return Some(matched.clone());
                }
                self.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                next.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let res = (points_a, points_b);
                mp.insert(next.timestamp_us, res.clone());
                return Some(res);
            }
        }
        None
    }

    fn can_cleanup(&self) -> bool {
        // Tolerate accidental overcounts from concurrent cache inserts by treating
        // the counter as a lower bound rather than an exact match.
        self.used.load(std::sync::atomic::Ordering::SeqCst) >= 2
    }
    fn cleanup(&mut self) { self.img = Arc::new(image::GrayImage::default()); }
}

#[cfg(all(test, feature = "use-burn"))]
mod tests {
    use super::*;

    fn synthetic_pair(w: u32, h: u32, shift: u32) -> (Arc<image::GrayImage>, Arc<image::GrayImage>) {
        let mut a = image::GrayImage::new(w, h);
        let mut b = image::GrayImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                // High-frequency texture pattern so the texture filter accepts the samples.
                let v = (((x as i32 * 13 + y as i32 * 7) & 0xff) ^ ((x as i32 / 4) & 0xff)) as u8;
                a.put_pixel(x, y, image::Luma([v]));
                let sx = x.saturating_sub(shift);
                let v2 = (((sx as i32 * 13 + y as i32 * 7) & 0xff) ^ ((sx as i32 / 4) & 0xff)) as u8;
                b.put_pixel(x, y, image::Luma([v2]));
            }
        }
        (Arc::new(a), Arc::new(b))
    }

    #[test]
    #[ignore]
    fn gmflow_detects_known_shift() {
        let (w, h) = (576u32, 320u32);
        let shift = 4u32;
        let (img_a, img_b) = synthetic_pair(w, h, shift);
        let of_a = OpticalFlowMethod::OFGmflow(OFGmflow::detect_features(0, img_a, w, h));
        let of_b = OpticalFlowMethod::OFGmflow(OFGmflow::detect_features(1_000_000, img_b, w, h));
        let pair = if let OpticalFlowMethod::OFGmflow(ref a) = of_a { a.optical_flow_to(&of_b) } else { None };
        let (pts_a, pts_b) = pair.expect("gmflow returned no matches");
        assert!(!pts_a.is_empty(), "no tracked points");
        let mut dxs: Vec<f32> = pts_a.iter().zip(pts_b.iter()).map(|((ax,_),(bx,_))| bx - ax).collect();
        dxs.sort_by(|a,b| a.partial_cmp(b).unwrap());
        let median = dxs[dxs.len()/2];
        println!("gmflow median dx = {median:.3} (expected ~{shift})");
        assert!((median - shift as f32).abs() < 2.0, "median dx {median} not near expected {shift}");
    }
}
