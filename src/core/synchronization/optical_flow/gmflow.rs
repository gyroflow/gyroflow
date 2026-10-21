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
    use std::time::{ Duration, Instant };
    use image::GrayImage;
    use parking_lot::Mutex;
    use burn::tensor::{ Bytes, Device, DeviceKind, Tensor, TensorData };
    use burn::tensor::wgpu::WgpuBackend;
    use burn_store::{ BurnpackStore, ModuleSnapshot };
    use sha2::{ Digest, Sha256 };

    pub const MODEL_W: usize = 576;
    pub const MODEL_H: usize = 320;

    // Model weights are hosted separately rather than embedded to avoid a ~145 MB library bloat
    // for users who do not enable the ai-optical-flow feature. The .bpk is the weights file that burn-onnx 0.22 writes
    // next to the generated code, from the ONNX that build.rs pins (gmflow-scale2-regrefine6, MIT, derived from
    // autonomousvision/unimatch): its tensors are named after that graph, so the two go together. Each build writes
    // fresh random parameter ids into it and nothing else differs; loading matches tensors by name, so the hosted file
    // is one such build. The SHA-256 pin defends against tampering on the download path. GYROFLOW_GMFLOW_WEIGHTS=<path>
    // takes the file from there instead of downloading it (offline installs, packaging, tests); it must still match
    // the pin.
    const MODEL_URL: &str = "https://github.com/yasumorishima/gyroflow-models/releases/download/v1.1.0/gmflow-scale2-regrefine6-320x576-opset16-sim.bpk";
    const MODEL_SHA256: &str = "4f89e462a8457023fdadb9383c9b04d8fd56420329e56aa7c36d5e7387f269d5";
    const MODEL_FILENAME: &str = "gmflow-scale2-regrefine6-320x576-opset16-sim.bpk";

    // gmflow runs on the GPU only: Vulkan on Windows, Linux and Android, Metal on macOS and iOS (the graphics API is
    // pinned, there is no fallback to another one). Without a usable GPU it doesn't run, and says why.
    #[cfg(target_vendor = "apple")]
    const GRAPHICS_API: WgpuBackend = WgpuBackend::Metal;
    #[cfg(not(target_vendor = "apple"))]
    const GRAPHICS_API: WgpuBackend = WgpuBackend::Vulkan;

    // The largest single tensor of the model, which the GPU must be able to hold as one storage buffer: the transformer's
    // feed-forward layer, [2, 11520, 1024] f32 (both frames at 1/4 of 576x320, 4 x 256 channels). Measured on the exported
    // graph (every output, static shapes). The local correlations, 456 MiB as unimatch writes them, are exported 9 window
    // offsets at a time so that they stay below this; 128 MiB is all that Vulkan guarantees, and what many Android GPUs have.
    const LARGEST_BUFFER: u64 = 2 * 11520 * 1024 * 4;

    // The device, picked once per process: a hardware GPU (discrete, then integrated, then virtual) that can hold the
    // model's largest buffer. GYROFLOW_GMFLOW_DEVICE=any takes the graphics API's default adapter even when it's a software
    // one such as Mesa lavapipe; for tests only (CI has no GPU), far too slow for real use.
    static GPU: OnceLock<Result<(Device, String), String>> = OnceLock::new();
    // Set when the GPU failed while running the model (a lost device, a shader the driver rejects): gmflow stays off for
    // the rest of the process, with the reason logged once
    static GPU_FAILED: Mutex<Option<String>> = Mutex::new(None);
    static MODEL: Mutex<Option<Arc<Model>>> = Mutex::new(None);

    fn gpu() -> Result<&'static (Device, String), String> {
        if let Some(why) = &*GPU_FAILED.lock() { return Err(why.clone()); }
        GPU.get_or_init(|| {
            let picked = select();
            match &picked {
                Ok((_, name)) => log::info!("gmflow: running on {}", name),
                Err(why) => log::error!("gmflow: can't run: {}", why),
            }
            picked
        }).as_ref().map_err(|e| e.clone())
    }

    // What gmflow runs on, for tests: "wgpu: <adapter> (<type>, <api>)", or why it can't run
    #[cfg(test)]
    pub fn device_name() -> String {
        match gpu() { Ok((_, name)) => name.clone(), Err(why) => format!("no GPU: {}", why) }
    }

    fn select() -> Result<(Device, String), String> {
        set_kernel_cache();
        // burn selects adapters by the type the graphics API reports, so software renderers (lavapipe, SwiftShader:
        // type Cpu) are never among these
        let any = std::env::var("GYROFLOW_GMFLOW_DEVICE").unwrap_or_default().eq_ignore_ascii_case("any");
        let kinds = if any { vec![DeviceKind::DefaultDevice] } else { vec![DeviceKind::DiscreteGpu(0), DeviceKind::IntegratedGpu(0), DeviceKind::VirtualGpu(0)] };
        let mut errors = Vec::new();
        for kind in kinds {
            match init(kind) {
                Ok(found) => return Ok(found),
                Err(e) => errors.push(e),
            }
        }
        Err(format!("no usable {:?} GPU ({})", GRAPHICS_API, errors.join("; ")))
    }

    // A missing adapter comes back as an error; a panic inside the wgpu stack (a broken driver) is caught too
    fn init(kind: DeviceKind) -> Result<(Device, String), String> {
        let label = format!("{:?}", kind);
        let started = std::panic::catch_unwind(|| Device::wgpu_options().graphics_api(GRAPHICS_API).device_kind(kind).init_with_setup());
        let (device, setup) = match started {
            Ok(Ok(found)) => found,
            Ok(Err(e)) => return Err(format!("{}: {}", label, e)),
            Err(_) => return Err(format!("{}: the wgpu runtime panicked while starting", label)),
        };
        let info = setup.adapter.get_info();
        let name = format!("wgpu: {} ({:?}, {:?})", info.name, info.device_type, info.backend);
        // What the adapter can do, not the device: cubecl raises the device's binding size to u64::MAX where Vulkan has
        // 64-bit indexing, so only the adapter's limit says what a buffer can be
        let limits = setup.adapter.limits();
        let room = (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size);
        if room < LARGEST_BUFFER {
            return Err(format!("{}: {} holds at most {} MiB in one buffer, gmflow needs {} MiB", label, info.name, room >> 20, LARGEST_BUFFER >> 20));
        }
        Ok((device, name))
    }

    // cubecl keeps the kernels it compiled and the results of tuning them (seconds to tens of seconds on first use) in a
    // cache: in Gyroflow's data directory, not in the working directory or a Cargo `target/` above it (its default)
    #[cfg(not(target_os = "ios"))]
    fn set_kernel_cache() {
        use burn::cubecl::config::{ CubeClRuntimeConfig, RuntimeConfig, cache::CacheConfig };
        // Its CUBECL_* environment variables still apply
        let mut config = CubeClRuntimeConfig::default().override_from_env();
        config.environment.path = CacheConfig::Directory(crate::settings::data_dir().join("ai_models").join("gpu_cache"));
        if !CubeClRuntimeConfig::try_set(config) {
            log::debug!("gmflow: cubecl was configured already, keeping its kernel cache location");
        }
    }
    // cubecl has no file cache on iOS
    #[cfg(target_os = "ios")]
    fn set_kernel_cache() {}

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
        actual[..] == expected[..]
    }

    fn ensure_model_bytes() -> Result<Vec<u8>, String> {
        if let Some(local) = std::env::var_os("GYROFLOW_GMFLOW_WEIGHTS") {
            let bytes = std::fs::read(&local).map_err(|e| format!("GYROFLOW_GMFLOW_WEIGHTS {:?}: {}", local, e))?;
            if !verify_sha256(&bytes, MODEL_SHA256) {
                return Err(format!("GYROFLOW_GMFLOW_WEIGHTS {:?} is not the weights this build expects (SHA-256 {})", local, MODEL_SHA256));
            }
            log::info!("gmflow: using the model at {:?}", local);
            return Ok(bytes);
        }
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

    // How long a failure to get the weights (no network, a bad download) is taken as the answer before trying again:
    // every pair of a sync or of an analysis asks for the model, and each try may wait out the download's timeouts.
    const RETRY_AFTER: Duration = Duration::from_secs(60);

    // Loads the weights onto `device`, downloading them first if they're not cached. Errors are logged when they
    // happen, not when a recent one is handed out again. A file that doesn't load is the answer for good: it passed the
    // SHA-256 pin, so it would fail the same way every time.
    fn load_model(device: &Device) -> Result<Arc<Model>, String> {
        static WEIGHTS_FAILED: Mutex<Option<(Instant, String)>> = Mutex::new(None);
        static LOAD_FAILED: OnceLock<String> = OnceLock::new();
        if let Some(why) = LOAD_FAILED.get() { return Err(why.clone()); }
        if let Some((at, why)) = &*WEIGHTS_FAILED.lock() {
            if at.elapsed() < RETRY_AFTER { return Err(why.clone()); }
        }
        let raw = ensure_model_bytes().map_err(|e| {
            let why = format!("could not get the model: {}", e);
            log::error!("gmflow: {}", why);
            *WEIGHTS_FAILED.lock() = Some((Instant::now(), why.clone()));
            why
        })?;
        *WEIGHTS_FAILED.lock() = None;
        // Load through the store rather than the generated `from_bytes`, which panics on a
        // bad file; this runs on a rayon worker, so report and disable gmflow instead.
        let mut store = BurnpackStore::from_bytes(Some(Bytes::from_bytes_vec(raw)));
        let mut m = Model::new(device);
        let applied = m.load_from(&mut store).map_err(|e| {
            let why = format!("failed to load model weights: {}", e);
            log::error!("gmflow: {}", why);
            LOAD_FAILED.get_or_init(|| why.clone()).clone()
        })?;
        // Every tensor of the model must come from the file, and every tensor of the file must be used
        if !applied.missing.is_empty() || !applied.unused.is_empty() || !applied.errors.is_empty() {
            let why = format!("the weights file doesn't match the model: {} missing, {} unused, {} errors (first: {:?} {:?} {:?})",
                applied.missing.len(), applied.unused.len(), applied.errors.len(), applied.missing.first(), applied.unused.first(), applied.errors.first());
            log::error!("gmflow: {}", why);
            return Err(LOAD_FAILED.get_or_init(|| why).clone());
        }
        Ok(Arc::new(m))
    }

    // The model on the GPU, loaded if it isn't yet. One load at a time: the others wait for it.
    fn model_on(device: &Device) -> Result<Arc<Model>, String> {
        let mut slot = MODEL.lock();
        if let Some(m) = &*slot { return Ok(m.clone()); }
        let m = load_model(device)?;
        *slot = Some(m.clone());
        Ok(m)
    }

    // Turns gmflow off for the rest of the process after the GPU failed while running it
    fn gpu_failed(name: &str, why: String) -> String {
        let why = format!("the GPU failed ({}: {})", name, why);
        let mut failed = GPU_FAILED.lock();
        if failed.is_none() {
            log::error!("gmflow: {}; gmflow is off until Gyroflow is restarted", why);
            *failed = Some(why.clone());
        }
        why
    }

    // Gets gmflow ready before a long run (Motion data -> Optical correction with gmflow tracking): picks the GPU,
    // downloads the weights if needed and loads them, so that waiting and failing happen before the first frame
    // rather than inside it. What it will run on, or why it can't run.
    pub fn prepare() -> Result<String, String> {
        let (device, name) = gpu()?;
        // Loading the weights allocates GPU memory: a panic there is the GPU's
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| model_on(device))) {
            Ok(Ok(_)) => Ok(name.clone()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(gpu_failed(name, "panic in the wgpu runtime while loading the weights".to_owned())),
        }
    }

    // Aspect-preserving letterbox mapping parameters. `scale` is model-pixels per
    // original-pixel; `pad_x` / `pad_y` are the left/top zero-padding offsets in
    // model space. Kept separate from the tensor so `compute_flow` can move the
    // tensor into `forward` without cloning.
    pub struct LetterboxParams {
        pub scale: f32,
        // The exact ratios of each axis: `scale` but for the rounding of the resized size
        pub sx: f32,
        pub sy: f32,
        pub pad_x: usize,
        pub pad_y: usize,
    }

    // The model's input for one frame: [1, 3, MODEL_H, MODEL_W], letterboxed, raw 0-255 grey in all 3 channels
    pub fn preprocess(img: &GrayImage) -> (Vec<f32>, LetterboxParams) {
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
        (data, LetterboxParams { scale, sx: new_w as f32 / iw, sy: new_h as f32 / ih, pad_x, pad_y })
    }

    // One pass of the model on the GPU, inputs as `preprocess` makes them; the flow, [1, 2, MODEL_H, MODEL_W]. An error or
    // a panic inside the wgpu runtime (a shader that doesn't compile, a lost device, out of memory) turns gmflow off.
    pub fn forward(in0: Vec<f32>, in1: Vec<f32>) -> Result<Vec<f32>, String> {
        let (device, name) = gpu()?;
        // One pass at a time: the sync asks for many pairs in parallel, and several passes side by side would only
        // share the GPU while each holding its own activations (out of memory on small GPUs)
        static ONE_PASS: Mutex<()> = Mutex::new(());
        let _one = ONE_PASS.lock();
        // Waited while another pass failed on the GPU: don't run on it again
        if let Some(why) = &*GPU_FAILED.lock() { return Err(why.clone()); }
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Result<Vec<f32>, String>, String> {
            // No weights is not the GPU's fault: Err on the outside
            let model = model_on(device)?;
            let shape = [1usize, 3, MODEL_H, MODEL_W];
            let t0 = Tensor::<4>::from_data(TensorData::new(in0, shape), device);
            let t1 = Tensor::<4>::from_data(TensorData::new(in1, shape), device);
            let flow = model.forward(t0, t1);
            Ok(flow.try_into_data().map_err(|e| short_error(&e.to_string()))
                .and_then(|d| d.try_into_vec::<f32>().map_err(|e| format!("output is not f32: {:?}", e))))
        }));
        match ran {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(gpu_failed(name, e)),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(gpu_failed(name, "panic in the wgpu runtime".to_owned())),
        }
    }

    // One pass on a device of the caller's (tests: a GPU set up with smaller limits), weights loaded for it
    #[cfg(test)]
    pub fn forward_on(device: &Device, in0: Vec<f32>, in1: Vec<f32>) -> Result<Vec<f32>, String> {
        let model = load_model(device)?;
        let shape = [1usize, 3, MODEL_H, MODEL_W];
        let flow = model.forward(Tensor::<4>::from_data(TensorData::new(in0, shape), device), Tensor::<4>::from_data(TensorData::new(in1, shape), device));
        flow.try_into_data().map_err(|e| short_error(&e.to_string()))?.try_into_vec::<f32>().map_err(|e| format!("output is not f32: {:?}", e))
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
    pub fn compute_flow(img0: &GrayImage, img1: &GrayImage) -> Option<(Vec<f32>, LetterboxParams)> {
        let (in0, lb0) = preprocess(img0);
        let (in1, _) = preprocess(img1);
        match forward(in0, in1) {
            Ok(flow) => Some((flow, lb0)),
            Err(e) => {
                log::debug!("gmflow: no flow: {}", e);
                None
            }
        }
    }
}

/// gmflow's dense flow from one frame to another, for the optical analysis (Motion data -> Optical correction ->
/// Tracking: gmflow). The model sees the frames letterboxed into 576x320, so the flow is that coarse: `at` maps a point
/// of the frame into it and back
#[cfg(feature = "use-burn")]
pub struct GmflowDenseFlow {
    flow: Vec<f32>,
    sx: f32,
    sy: f32,
    pad_x: f32,
    pad_y: f32,
}

#[cfg(feature = "use-burn")]
impl GmflowDenseFlow {
    /// Picks the GPU and loads the model (downloading it on first use), before the first `compute`: what it will run on
    /// ("wgpu: ..."), or why gmflow can't run
    pub fn prepare() -> Result<String, String> { inference::prepare() }

    /// The flow from `a` to `b` (same size), on the GPU, see `inference`. None, logged, when gmflow can't run
    pub fn compute(a: &image::GrayImage, b: &image::GrayImage) -> Option<Self> {
        let (flow, lb) = inference::compute_flow(a, b)?;
        if flow.len() < 2 * inference::MODEL_H * inference::MODEL_W { return None; }
        Some(Self { flow, sx: lb.sx.max(1e-6), sy: lb.sy.max(1e-6), pad_x: lb.pad_x as f32, pad_y: lb.pad_y as f32 })
    }

    /// How far the point (x, y) of the first frame, in its pixels, moved by in the second one: bilinear between the
    /// model's pixels, pixel centres matched as the resize does. None outside of what the model saw
    pub fn at(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (w, h) = (inference::MODEL_W, inference::MODEL_H);
        let mx = (x + 0.5) * self.sx - 0.5 + self.pad_x;
        let my = (y + 0.5) * self.sy - 0.5 + self.pad_y;
        if !(mx >= 0.0 && my >= 0.0 && mx <= (w - 1) as f32 && my <= (h - 1) as f32) { return None; }
        let (x0, y0) = (mx.floor() as usize, my.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = (mx - x0 as f32, my - y0 as f32);
        let plane = |k: usize| {
            let at = |x: usize, y: usize| self.flow[k * w * h + y * w + x];
            (at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx) * (1.0 - fy) + (at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx) * fy
        };
        Some((plane(0) / self.sx, plane(1) / self.sy))
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
                    // Why was logged once, where it happened (no usable GPU, no model, the GPU failed)
                    log::debug!("gmflow: no flow");
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
        // Smooth texture at a few scales, so that it survives the model's downscaling without aliasing (a pattern
        // finer than the pixels of a scaled-down frame would move differently from the frame)
        let tex = |x: f32, y: f32| -> u8 {
            let v = 128.0 + 45.0 * (x * 0.071).sin() * (y * 0.053).cos() + 35.0 * ((x * 0.013 + y * 0.029).sin() * 3.0).sin()
                + 25.0 * (x * 0.23 + y * 0.11).sin() * (y * 0.19 - x * 0.07).cos();
            v.round().clamp(0.0, 255.0) as u8
        };
        for y in 0..h {
            for x in 0..w {
                a.put_pixel(x, y, image::Luma([tex(x as f32, y as f32)]));
                // The same frame moved `shift` px right
                b.put_pixel(x, y, image::Luma([tex(x as f32 - shift as f32, y as f32)]));
            }
        }
        (Arc::new(a), Arc::new(b))
    }

    #[test]
    #[ignore]
    fn gmflow_detects_known_shift() {
        // The model's own size, a 16:9 frame scaled down (0.45), and a 4:3 one letterboxed (bars left and right)
        for (w, h) in [(576u32, 320u32), (1280, 720), (640, 480)] {
            println!("frame {w}x{h}");
            known_shift(w, h);
        }
    }

    fn known_shift(w: u32, h: u32) {
        let shift = 4u32;
        let (img_a, img_b) = synthetic_pair(w, h, shift);
        let of_a = OpticalFlowMethod::OFGmflow(OFGmflow::detect_features(0, img_a, w, h));
        let of_b = OpticalFlowMethod::OFGmflow(OFGmflow::detect_features(1_000_000, img_b, w, h));
        let pair = if let OpticalFlowMethod::OFGmflow(ref a) = of_a { a.optical_flow_to(&of_b) } else { None };
        println!("gmflow device: {}", inference::device_name());
        assert!(inference::device_name().starts_with("wgpu:"), "gmflow found no GPU: {}", inference::device_name());
        let (pts_a, pts_b) = pair.expect("gmflow returned no matches");
        assert!(pts_a.len() >= 100, "only {} tracked points", pts_a.len());
        let moved: Vec<(f32, f32)> = pts_a.iter().zip(pts_b.iter()).map(|((ax, ay), (bx, by))| (bx - ax, by - ay)).collect();
        let median = |mut v: Vec<f32>| { v.sort_by(|a, b| a.partial_cmp(b).unwrap()); v[v.len() / 2] };
        let (mx, my) = (median(moved.iter().map(|m| m.0).collect()), median(moved.iter().map(|m| m.1).collect()));
        let near = moved.iter().filter(|m| (m.0 - shift as f32).abs() < 1.0 && m.1.abs() < 1.0).count() as f32 / moved.len() as f32;
        println!("gmflow median ({mx:.3}, {my:.3}), expected ({shift}, 0); {:.1}% of {} points within 1 px", near * 100.0, moved.len());
        assert!((mx - shift as f32).abs() < 0.5 && my.abs() < 0.5, "median ({mx}, {my}) not near ({shift}, 0)");
        assert!(near >= 0.75, "only {:.1}% of the points within 1 px of the shift", near * 100.0);
    }

    // The model on the GPU against PyTorch, on the inputs and outputs that ci-gpu/export_gpu.py writes (GMFLOW_REF_DIR):
    // a synthetic 4 px shift and two pairs of a real video
    #[test]
    #[ignore]
    fn gmflow_matches_pytorch() {
        let dir = std::path::PathBuf::from(std::env::var("GMFLOW_REF_DIR").expect("GMFLOW_REF_DIR: where export_gpu.py wrote pair*.f32"));
        let read = |name: String| -> Vec<f32> {
            let bytes = std::fs::read(dir.join(&name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
        };
        let mut checked = 0;
        for k in 0.. {
            if !dir.join(format!("pair{k}_torch.f32")).exists() { break; }
            let reference = read(format!("pair{k}_torch.f32"));
            let started = std::time::Instant::now();
            let flow = inference::forward(read(format!("pair{k}_in0.f32")), read(format!("pair{k}_in1.f32"))).expect("gmflow did not run");
            println!("pair {k}: one pass {:.3} s{}", started.elapsed().as_secs_f64(), if k == 0 { " (the first: kernels compiled and tuned)" } else { "" });
            assert_eq!(flow.len(), reference.len());
            let diffs: Vec<f32> = flow.iter().zip(&reference).map(|(a, b)| (a - b).abs()).collect();
            assert!(diffs.iter().all(|d| d.is_finite()), "pair {k}: non-finite output");
            let max = diffs.iter().cloned().fold(0.0f32, f32::max);
            let mean = diffs.iter().map(|&d| d as f64).sum::<f64>() / diffs.len() as f64;
            println!("pair {k} on {}: max |gpu - torch| {max:.2e} px, mean {mean:.2e} px", inference::device_name());
            // Pre-registered before the first GPU run
            assert!(max <= 1e-2 && mean <= 1e-4, "pair {k}: max {max:e}, mean {mean:e}");
            checked += 1;
        }
        assert!(checked >= 3, "found {checked} reference pairs in {dir:?}");
        // Warm: the same pair again, a few times
        let (in0, in1) = (read("pair1_in0.f32".to_owned()), read("pair1_in1.f32".to_owned()));
        let runs: usize = std::env::var("GMFLOW_WARM_RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
        let started = std::time::Instant::now();
        for _ in 0..runs { inference::forward(in0.clone(), in1.clone()).expect("gmflow did not run"); }
        println!("warm: {:.3} s per pass on {} (mean of {runs})", started.elapsed().as_secs_f64() / runs as f64, inference::device_name());
    }

    // The whole model with no buffer over 128 MiB, which is all that Vulkan guarantees and what many Android GPUs have,
    // against PyTorch. Vulkan: cubecl makes the device itself, as in Gyroflow (its SPIR-V kernels need the extensions it
    // enables), with the adapter's own limits, so this needs a device with no more than 128 MiB of its own, such as Mesa
    // lavapipe (maxStorageBufferRange 128 MiB, no 64-bit indexing). Metal: the device cubecl makes for its WGSL kernels,
    // with lower limits. Either way the limits the device ended up with are checked before the model runs.
    #[test]
    #[ignore]
    fn gmflow_runs_within_128_mib_buffers() {
        const MIB_128: u64 = 128 << 20;
        #[cfg(not(target_vendor = "apple"))]
        let (device, limits, name) = {
            let (device, setup) = burn::tensor::Device::wgpu_options().graphics_api(burn::tensor::wgpu::WgpuBackend::Vulkan).device_kind(burn::tensor::DeviceKind::DefaultDevice).init_with_setup().expect("Vulkan");
            (device, setup.device.limits(), setup.adapter.get_info().name)
        };
        #[cfg(target_vendor = "apple")]
        let (device, limits, name) = {
            use burn::tensor::wgpu::{ WgpuSetup, wgpu };
            let backend = wgpu::Backend::Metal;
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: backend.into(), ..wgpu::InstanceDescriptor::new_without_display_handle() });
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).expect("no adapter");
            let mut limits = adapter.limits();
            limits.max_buffer_size = limits.max_buffer_size.min(MIB_128);
            limits.max_storage_buffer_binding_size = limits.max_storage_buffer_binding_size.min(MIB_128 as _);
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: adapter.features().difference(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                // As cubecl does: the features it asks for include the experimental ones the adapter has
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            })).expect("device");
            let (limits, name) = (device.limits(), adapter.get_info().name);
            let device = burn::tensor::Device::wgpu_options().setup(WgpuSetup { instance, adapter, device, queue, backend }).init().expect("burn on that device");
            (device, limits, name)
        };
        let room = (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size);
        println!("adapter {:?}, buffers up to {} MiB, bindings up to {} MiB", name, limits.max_buffer_size >> 20, (limits.max_storage_buffer_binding_size as u64) >> 20);
        assert!(room <= MIB_128, "this device allows buffers of {} MiB; the test needs one that allows at most 128 MiB", room >> 20);
        let dir = std::path::PathBuf::from(std::env::var("GMFLOW_REF_DIR").expect("GMFLOW_REF_DIR"));
        let read = |name: &str| -> Vec<f32> { std::fs::read(dir.join(name)).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect() };
        let flow = inference::forward_on(&device, read("pair1_in0.f32"), read("pair1_in1.f32")).expect("gmflow did not run within 128 MiB buffers");
        let reference = read("pair1_torch.f32");
        let max = flow.iter().zip(&reference).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        let mean = flow.iter().zip(&reference).map(|(a, b)| (a - b).abs() as f64).sum::<f64>() / flow.len() as f64;
        println!("within 128 MiB buffers: max |gpu - torch| {max:.2e} px, mean {mean:.2e} px");
        assert!(max <= 1e-2 && mean <= 1e-4, "max {max:e}, mean {mean:e}");
    }

    // Without a usable GPU gmflow must not run, and must say why: GMFLOW_EXPECT_ERROR is a piece of the reason the
    // machine running the test should give (on CI, whose only Vulkan is a software one that the automatic choice doesn't consider: "no usable Vulkan GPU")
    #[test]
    #[ignore]
    fn gmflow_refuses_without_a_gpu() {
        let expected = std::env::var("GMFLOW_EXPECT_ERROR").expect("GMFLOW_EXPECT_ERROR: a piece of the expected reason");
        assert!(std::env::var("GYROFLOW_GMFLOW_DEVICE").is_err(), "GYROFLOW_GMFLOW_DEVICE would change what is picked");
        let why = inference::prepare().expect_err("gmflow picked a device");
        println!("gmflow refused: {why}");
        assert!(why.contains(&expected), "reason {why:?} doesn't contain {expected:?}");
        assert!(inference::compute_flow(&image::GrayImage::new(64, 64), &image::GrayImage::new(64, 64)).is_none(), "gmflow computed a flow anyway");
    }
}
