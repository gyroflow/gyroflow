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
    use std::io::Read;
    use std::path::PathBuf;
    use std::sync::{ Arc, OnceLock };
    use std::time::Duration;
    use image::GrayImage;
    use burn::tensor::{ Bytes, Device, Tensor, TensorData };
    use burn_store::{ BurnpackStore, ModuleSnapshot };
    use sha2::{ Digest, Sha256 };

    pub const MODEL_W: usize = 576;
    pub const MODEL_H: usize = 320;

    // Model weights are hosted separately rather than embedded to avoid a ~145 MB library bloat
    // for users who do not enable the ai-optical-flow feature. The .bpk is a burn-store dump of
    // the gmflow-scale2-regrefine6 weights (MIT, derived from autonomousvision/unimatch) converted
    // via burn-onnx 0.21.0-pre.3; burn 0.22 loads it unchanged (same parameter names and shapes).
    // The SHA-256 pin defends against tampering on the download path.
    const MODEL_URL: &str = "https://github.com/yasumorishima/gyroflow-models/releases/download/v1.0.0/gmflow-scale2-regrefine6-320x576-opset16-sim.bpk";
    const MODEL_SHA256: &str = "31feee698842928715e9ba693b69a492d85ace31c646df6f3eabd53e240cd5e9";
    const MODEL_FILENAME: &str = "gmflow-scale2-regrefine6-320x576-opset16-sim.bpk";

    // The CPU model always exists as the fallback; the GPU one only when a GPU was picked.
    static CPU_MODEL: OnceLock<Option<Arc<Model>>> = OnceLock::new();
    #[cfg(feature = "use-burn-gpu")]
    static GPU: OnceLock<Option<(Device, String)>> = OnceLock::new();
    #[cfg(feature = "use-burn-gpu")]
    static GPU_MODEL: OnceLock<Option<Arc<Model>>> = OnceLock::new();
    #[cfg(feature = "use-burn-gpu")]
    static GPU_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    // The device is picked once per process and logged once. With the `use-burn-gpu` feature a
    // hardware GPU is tried first (discrete, then integrated) through burn's wgpu runtime;
    // without one, or if it fails to start, gmflow runs on burn's flex CPU backend. Unlike
    // NdArray (6 dimensions at most), flex handles the 7-D reshape in gmflow's convex upsampling
    // (unimatch utils.py, upsample_flow_with_mask). If the GPU fails later, while running the
    // model, gmflow switches to the CPU for the rest of the process.
    //
    // GYROFLOW_GMFLOW_DEVICE overrides the choice: `cpu` forces flex, `gpu` takes wgpu's default
    // adapter even when it is a software one such as Mesa lavapipe (used by CI, far too slow for
    // real use, which is why it is never picked on its own).
    #[cfg(feature = "use-burn-gpu")]
    fn gpu() -> Option<&'static (Device, String)> {
        if GPU_FAILED.load(std::sync::atomic::Ordering::Relaxed) { return None; }
        GPU.get_or_init(|| match gpu::select() {
            Ok(found) => {
                log::info!("gmflow: running on {}", found.1);
                Some(found)
            }
            Err(why) => {
                log::info!("gmflow: running on the CPU (flex); {}", why);
                None
            }
        }).as_ref()
    }

    // What gmflow ran on, for tests: "flex (CPU)", "wgpu: <adapter> (<type>, <api>)", or
    // "flex (CPU) after wgpu: ... failed" when a GPU was picked and then failed at run time.
    #[cfg(test)]
    pub fn device_name() -> String {
        #[cfg(feature = "use-burn-gpu")]
        if let Some(Some((_, name))) = GPU.get() {
            if GPU_FAILED.load(std::sync::atomic::Ordering::Relaxed) {
                return format!("flex (CPU) after {} failed", name);
            }
            return name.clone();
        }
        "flex (CPU)".to_owned()
    }

    #[cfg(feature = "use-burn-gpu")]
    mod gpu {
        use burn::tensor::{ Device, DeviceKind };

        pub fn select() -> Result<(Device, String), String> {
            let choice = std::env::var("GYROFLOW_GMFLOW_DEVICE").unwrap_or_default().to_ascii_lowercase();
            let kinds = match choice.as_str() {
                "cpu" => return Err("GYROFLOW_GMFLOW_DEVICE=cpu".to_owned()),
                "gpu" => vec![DeviceKind::DefaultDevice],
                _ => vec![DeviceKind::DiscreteGpu(0), DeviceKind::IntegratedGpu(0)],
            };
            let mut errors = Vec::new();
            for kind in kinds {
                match init(kind) {
                    Ok(found) => return Ok(found),
                    Err(e) => errors.push(e),
                }
            }
            Err(format!("no usable GPU: {}", errors.join("; ")))
        }

        // A missing adapter comes back as an error; a panic inside the wgpu stack (a broken
        // driver) is caught too, so a bad GPU setup only costs the GPU, not gmflow.
        fn init(kind: DeviceKind) -> Result<(Device, String), String> {
            let label = format!("{:?}", kind);
            let started = std::panic::catch_unwind(|| Device::wgpu_options().device_kind(kind).init_with_setup());
            match started {
                Ok(Ok((device, setup))) => {
                    let info = setup.adapter.get_info();
                    Ok((device, format!("wgpu: {} ({:?}, {:?})", info.name, info.device_type, info.backend)))
                }
                Ok(Err(e)) => Err(format!("{}: {}", label, e)),
                Err(_) => Err(format!("{}: the wgpu runtime panicked while starting", label)),
            }
        }
    }

    fn cache_path() -> PathBuf {
        crate::settings::data_dir().join("ai_models").join(MODEL_FILENAME)
    }

    fn verify_sha256(bytes: &[u8], expected_hex: &str) -> bool {
        if expected_hex.len() != 64 { return false; }
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let actual = hasher.finalize();
        let mut expected = [0u8; 32];
        for i in 0..32 {
            match u8::from_str_radix(&expected_hex[i * 2..i * 2 + 2], 16) {
                Ok(b) => expected[i] = b,
                Err(_) => return false,
            }
        }
        actual.as_slice() == expected
    }

    fn ensure_model_bytes() -> Result<Vec<u8>, String> {
        let path = cache_path();
        if let Ok(bytes) = std::fs::read(&path) {
            if verify_sha256(&bytes, MODEL_SHA256) {
                log::info!("gmflow: using cached model at {:?}", path);
                return Ok(bytes);
            }
            log::warn!("gmflow: cached model failed SHA-256, re-downloading");
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {:?}: {}", parent, e))?;
        }
        log::info!("gmflow: downloading model from {}", MODEL_URL);
        // 600 s global budget (covers the ~145 MB download on a >= 2.4 Mbps link)
        // plus a 30 s connect timeout so a DNS / TLS stall cannot hang a rayon worker
        // indefinitely. Without these, ureq falls back to unbounded waits.
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(600)))
            .timeout_connect(Some(Duration::from_secs(30)))
            .build()
            .into();
        let mut reader = agent
            .get(MODEL_URL)
            .call()
            .map_err(|e| format!("HTTP GET {}: {}", MODEL_URL, e))?
            .into_body()
            .into_reader();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(|e| format!("read body: {}", e))?;
        if !verify_sha256(&bytes, MODEL_SHA256) {
            return Err(format!("downloaded model SHA-256 mismatch (expected {})", MODEL_SHA256));
        }
        // Process-unique tmp name avoids two concurrent gyroflow instances
        // truncating each other's partial file on a cold cache. Rename remains
        // atomic per process, and a stale tmp from a crashed peer is orphaned
        // rather than reused as a partial.
        let tmp = path.with_extension(format!("bpk.tmp.{}", std::process::id()));
        std::fs::write(&tmp, &bytes).map_err(|e| format!("write {:?}: {}", tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("rename cache: {}", e))?;
        log::info!("gmflow: cached model at {:?}", path);
        Ok(bytes)
    }

    fn load_model(device: &Device) -> Option<Arc<Model>> {
        match ensure_model_bytes() {
            Ok(raw) => {
                // Load through the store rather than the generated `from_bytes`, which panics on a
                // bad file; this runs on a rayon worker, so report and disable gmflow instead.
                let mut store = BurnpackStore::from_bytes(Some(Bytes::from_bytes_vec(raw)));
                let mut m = Model::new(device);
                match m.load_from(&mut store) {
                    Ok(_) => Some(Arc::new(m)),
                    Err(e) => {
                        log::error!("gmflow: failed to load model weights: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                log::error!("gmflow: failed to load model: {}", e);
                None
            }
        }
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

    pub fn preprocess(img: &GrayImage, device: &Device) -> (Tensor<4>, LetterboxParams) {
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

        let tensor = Tensor::<4>::from_data(
            TensorData::new(data, [1usize, 3, MODEL_H, MODEL_W]),
            device,
        );
        (tensor, LetterboxParams { scale, pad_x, pad_y })
    }

    fn run(model: &Model, device: &Device, img0: &GrayImage, img1: &GrayImage) -> Result<(Vec<f32>, LetterboxParams), String> {
        let (t0, lb0) = preprocess(img0, device);
        let (t1, _) = preprocess(img1, device);
        let flow = model.forward(t0, t1);
        let data = flow.try_into_data().map_err(|e| short_error(&e.to_string()))?;
        let data = data.try_into_vec::<f32>().map_err(|e| format!("output is not f32: {:?}", e))?;
        Ok((data, lb0))
    }

    // burn's execution errors list every kernel skipped after the first failure (hundreds of
    // lines); keep the causes only, on one line.
    fn short_error(text: &str) -> String {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("skipped `") && !l.starts_with("No backtrace") && !l.starts_with("Asked at") && !l.ends_with("record(s) were dropped; the walk may stop before the root)"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    // Returns the flow and the first frame's letterbox parameters, so callers can invert the
    // mapping back into original-frame coordinates, or `None` (logged) if gmflow cannot run.
    // Tensors are moved (not cloned) into `forward` to avoid ~4.4 MB of redundant allocations per
    // pair at 320x576. On the GPU, an error or a panic inside the wgpu runtime (a shader that
    // does not compile, a lost device, out of memory) turns the GPU off for good and the pair is
    // computed again on the CPU, so a bad driver costs speed, not the sync.
    pub fn compute_flow(img0: &GrayImage, img1: &GrayImage) -> Option<(Vec<f32>, LetterboxParams)> {
        #[cfg(feature = "use-burn-gpu")]
        if let Some((device, name)) = gpu() {
            // Loading the weights onto the GPU is inside the guard too: it allocates GPU memory.
            let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                GPU_MODEL.get_or_init(|| load_model(device)).as_ref().map(|m| run(m, device, img0, img1))
            }));
            let why = match ran {
                Ok(Some(Ok(v))) => return Some(v),
                Ok(Some(Err(e))) => Some(e),
                Ok(None) => None, // no weights (already logged); the CPU will not have them either
                Err(_) => Some("panic in the wgpu runtime".to_owned()),
            };
            if let Some(why) = why {
                if !GPU_FAILED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    log::error!("gmflow: the GPU failed ({}: {}); continuing on the CPU (flex)", name, why);
                }
            }
        }
        let device = Device::flex();
        let m = CPU_MODEL.get_or_init(|| {
            #[cfg(not(feature = "use-burn-gpu"))]
            log::info!("gmflow: running on the CPU (flex)");
            load_model(&device)
        }).as_ref()?;
        match run(m, &device, img0, img1) {
            Ok(v) => Some(v),
            Err(e) => {
                log::error!("gmflow: {}", e);
                None
            }
        }
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
                    log::error!("gmflow: no flow (model not loaded, or the output was not f32)");
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
        println!("gmflow device: {}", inference::device_name());
        // CI sets GYROFLOW_GMFLOW_DEVICE to say which path this run must take.
        let wanted = std::env::var("GYROFLOW_GMFLOW_DEVICE").unwrap_or_default().to_ascii_lowercase();
        match wanted.as_str() {
            // A GPU must have been picked; it may then fail at run time and hand over to the CPU
            // (lavapipe, for one, cannot hold gmflow's largest buffer).
            "gpu" => assert!(inference::device_name().contains("wgpu:"), "asked for the GPU, got {}", inference::device_name()),
            "cpu" => assert_eq!(inference::device_name(), "flex (CPU)"),
            _ => {}
        }
        let (pts_a, pts_b) = pair.expect("gmflow returned no matches");
        assert!(!pts_a.is_empty(), "no tracked points");
        let mut dxs: Vec<f32> = pts_a.iter().zip(pts_b.iter()).map(|((ax,_),(bx,_))| bx - ax).collect();
        dxs.sort_by(|a,b| a.partial_cmp(b).unwrap());
        let median = dxs[dxs.len()/2];
        println!("gmflow median dx = {median:.3} (expected ~{shift})");
        assert!((median - shift as f32).abs() < 2.0, "median dx {median} not near expected {shift}");
    }
}
