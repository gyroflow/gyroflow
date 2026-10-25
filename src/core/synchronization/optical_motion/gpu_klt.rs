// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! KLT on the GPU, through wgpu (`gpu_klt.wgsl`): `Tracker::klt` (OpenCV's calcOpticalFlowPyrLK) and the candidate
//! corners `Tracker::replenish` picks from (the local maxima of cornerMinEigenVal). Each frame goes up once: its
//! pyramid, derivatives and corner strengths are made once and kept for the pair after it (OpenCV builds both pyramids
//! again on every call), and one submission a frame tracks the points of the frame before into it and back. Only the
//! points and the candidates come back. On the same points it agrees with OpenCV to about a thousandth of a pixel, see
//! the tests in `tracker.rs`

use std::sync::{ Arc, Mutex, PoisonError, atomic::{ AtomicBool, Ordering::Relaxed }, mpsc };
use std::time::Duration;
use wgpu::util::DeviceExt;
use super::tracker::{ WINDOW, KLT_LEVELS, KLT_ITERATIONS, KLT_EPS, KLT_MIN_EIG, local_maxima };

/// Room for the candidate corners, in pixels of the frame per candidate: a local maximum has to stand above its 8
/// neighbours, and the strengths are sums over 7x7, so they're a few pixels apart even in noise. Only a frame of
/// equal strengths over an area could have more; its candidates are found on the CPU then
const PIXELS_PER_CANDIDATE: u32 = 4;
/// Waiting for the GPU this long means it hung, and the tracking goes on on the CPU
const TIMEOUT: Duration = Duration::from_secs(10);

/// What a process makes once for a GPU: the device and the pipelines. A device that failed (lost, an error, a wait for
/// it that timed out) is made again for the next analysis. The adapter is one of `WgpuWrapper`'s, the same one for the
/// same GPU every time (see `compute_adapter`): two GPUs of a kind have the same name and ids
struct Shared {
    adapter: wgpu::Adapter,
    name: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    failed: Arc<AtomicBool>,
    to_float: wgpu::ComputePipeline,
    down: wgpu::ComputePipeline,
    scharr: wgpu::ComputePipeline,
    cov: wgpu::ComputePipeline,
    eig: wgpu::ComputePipeline,
    corners: wgpu::ComputePipeline,
    lk: wgpu::ComputePipeline,
}

static SHARED: Mutex<Option<Arc<Shared>>> = Mutex::new(None);

impl Shared {
    fn get(adapter: &wgpu::Adapter) -> Result<Arc<Self>, String> {
        let info = adapter.get_info();
        let mut cached = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(s) = cached.as_ref().filter(|s| s.adapter == *adapter && !s.failed.load(Relaxed)) {
            return Ok(s.clone());
        }
        if !adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS) {
            return Err(format!("{} has no compute shaders", info.name));
        }
        let adapter_limits = adapter.limits();
        let mut limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter_limits.clone());
        limits.max_buffer_size = adapter_limits.max_buffer_size;
        limits.max_storage_buffer_binding_size = adapter_limits.max_storage_buffer_binding_size;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("tracking"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        })).map_err(|e| e.to_string())?;
        let failed = Arc::new(AtomicBool::new(false));
        let f = failed.clone();
        device.on_uncaptured_error(Arc::new(move |e| { log::error!("GPU tracking: {e}"); f.store(true, Relaxed); }));
        let f = failed.clone();
        // Destroyed is also the device dropped, when one made again replaces it
        device.set_device_lost_callback(move |reason, message| {
            if reason == wgpu::DeviceLostReason::Unknown { log::error!("GPU tracking: device lost: {message}"); }
            f.store(true, Relaxed);
        });

        // The backend's compiler (FXC, DXC, Metal's) rejecting a kernel is an internal error, not a validation one. All
        // popped before any is looked at: they go in the reverse order, or wgpu panics
        let scopes = [wgpu::ErrorFilter::Validation, wgpu::ErrorFilter::Internal, wgpu::ErrorFilter::OutOfMemory].map(|f| device.push_error_scope(f));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("klt"), source: wgpu::ShaderSource::Wgsl(include_str!("gpu_klt.wgsl").into()) });
        let pipeline = |entry: &str| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry), layout: None, module: &module, entry_point: Some(entry), compilation_options: Default::default(), cache: None
        });
        let (to_float, down, scharr, cov, eig, corners, lk) = (pipeline("to_float"), pipeline("pyr_down"), pipeline("scharr"), pipeline("cov"), pipeline("min_eig"), pipeline("corners"), pipeline("lk"));
        let errors: Vec<wgpu::Error> = scopes.into_iter().rev().filter_map(|s| pollster::block_on(s.pop())).collect();
        if let Some(e) = errors.first() { return Err(e.to_string()); }
        // Not kept when it failed some other way while it was made
        if failed.load(Relaxed) { return Err("the device failed while setting up".into()); }

        let shared = Arc::new(Self { adapter: adapter.clone(), name: format!("{} ({:?})", info.name, info.backend), device, queue, failed, to_float, down, scharr, cov, eig, corners, lk });
        *cached = Some(shared.clone());
        Ok(shared)
    }
}

/// Submitted and not read back yet
struct Pending {
    submission: wgpu::SubmissionIndex,
    mapped: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    tracked: usize,
}

pub struct GpuKlt {
    shared: Arc<Shared>,
    pub size: (u32, u32),
    pub stride: u32,
    max_points: usize,
    capacity: usize,
    raw: wgpu::Buffer,
    pts: wgpu::Buffer,
    out_fwd: wgpu::Buffer,
    out_bwd: wgpu::Buffer,
    work: wgpu::Buffer,
    cands: wgpu::Buffer,
    staging: wgpu::Buffer,
    /// The pyramid's levels
    dims: Vec<(u32, u32)>,
    /// In `work`: the corner strengths and the noise samples, bytes, and how many of those
    eig_off: u64,
    noise_off: u64,
    noise_len: usize,
    /// In `staging`, bytes: the points forward and back, the candidates (their count first)
    fwd_off: u64,
    bwd_off: u64,
    cands_off: u64,
    // Per slot of the two frames kept: the frame going into it
    g_to_float: Vec<wgpu::BindGroup>,
    g_down: Vec<Vec<wgpu::BindGroup>>,
    g_scharr: Vec<Vec<wgpu::BindGroup>>,
    g_cov: Vec<wgpu::BindGroup>,
    g_eig: wgpu::BindGroup,
    g_corners: wgpu::BindGroup,
    /// LK with I in that slot and J in the other: forward from the points uploaded, back from where they went
    g_fwd: Vec<wgpu::BindGroup>,
    g_bwd: Vec<wgpu::BindGroup>,
    /// The slot of the last frame, and how many went up
    slot: usize,
    frames: usize,
    pending: Option<Pending>,
    upload: Vec<[f32; 4]>,
    /// After `finish`, per point tracked forward and back: x, y, and 1 where it got there
    pub fwd: Vec<[f32; 4]>,
    pub bwd: Vec<[f32; 4]>,
    /// ... and the frame's candidate corners (see `local_maxima`, unsorted) and the corner strengths of every 8th pixel
    /// in both directions
    pub candidates: Vec<u64>,
    pub noise: Vec<f32>,
}

impl GpuKlt {
    /// Makes the device and the pipelines for `adapter` ahead of the first `new` on it
    pub fn prepare(adapter: &wgpu::Adapter) -> Result<(), String> {
        Shared::get(adapter).map(|_| ())
    }

    pub fn new(adapter: &wgpu::Adapter, width: u32, height: u32, stride: u32, max_points: usize) -> Result<Self, String> {
        let shared = Shared::get(adapter)?;
        let device = &shared.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);

        // The pyramid's sizes, as buildOpticalFlowPyramid stops: before a level as small as the window
        let mut dims = vec![(width, height)];
        while dims.len() <= KLT_LEVELS as usize {
            let (w, h) = *dims.last().unwrap();
            let next = (w.div_ceil(2), h.div_ceil(2));
            if next.0 <= WINDOW as u32 || next.1 <= WINDOW as u32 { break; }
            dims.push(next);
        }
        let mut off = 0u32;
        let (mut img, mut grad) = ([[0u32; 8]; 2], [[0u32; 8]; 2]);
        for s in 0..2 {
            for (k, (w, h)) in dims.iter().enumerate() {
                img[s][k] = off; off += w * h;
                grad[s][k] = off; off += 2 * w * h;
            }
        }

        let wh = width as u64 * height as u64;
        let capacity = (wh / PIXELS_PER_CANDIDATE as u64).max(1024) as usize;
        let noise_len = (width.div_ceil(8) * height.div_ceil(8)) as usize;
        let storage = |size: u64, extra: wgpu::BufferUsages| device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size, usage: wgpu::BufferUsages::STORAGE | extra, mapped_at_creation: false
        });
        let pyr = storage(off as u64 * 4, wgpu::BufferUsages::empty());
        let raw = storage((stride as u64 * height as u64).div_ceil(4) * 4, wgpu::BufferUsages::COPY_DST);
        let pts = storage(max_points as u64 * 16, wgpu::BufferUsages::COPY_DST);
        let out_fwd = storage(max_points as u64 * 16, wgpu::BufferUsages::COPY_SRC);
        let out_bwd = storage(max_points as u64 * 16, wgpu::BufferUsages::COPY_SRC);
        // The three covariances of each pixel, its corner strength, the noise samples
        let work = storage((wh * 4 + noise_len as u64) * 4, wgpu::BufferUsages::COPY_SRC);
        let cands = storage(8 + capacity as u64 * 8, wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        let (eig_off, noise_off) = (wh * 12, wh * 16);
        let fwd_off = (noise_len as u64 * 4).next_multiple_of(16);
        let bwd_off = fwd_off + max_points as u64 * 16;
        let cands_off = bwd_off + max_points as u64 * 16;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: cands_off + 8 + capacity as u64 * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false
        });

        let uniform = |words: &[u32]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(words), usage: wgpu::BufferUsages::UNIFORM });
        // `Params`: w, h, sw, sh, src, dst, stride, aux
        let params = |v: [u32; 8]| uniform(&v);
        let group = |p: &wgpu::ComputePipeline, entries: &[(u32, &wgpu::Buffer)]| device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &p.get_bind_group_layout(0),
            entries: &entries.iter().map(|(binding, b)| wgpu::BindGroupEntry { binding: *binding, resource: b.as_entire_binding() }).collect::<Vec<_>>(),
        });
        // `LkParams`, with I in slot `s`
        let lk_params = |s: usize| {
            let mut v = vec![max_points as u32, dims.len() as u32, KLT_ITERATIONS as u32, 0, ((KLT_EPS * KLT_EPS) as f32).to_bits(), (KLT_MIN_EIG as f32).to_bits(), 0, 0];
            for k in 0..8 {
                if k < dims.len() { v.extend([img[s][k], grad[s][k], img[1 - s][k], dims[k].0 << 16 | dims[k].1]); } else { v.extend([0; 4]); }
            }
            uniform(&v)
        };
        let (w0, h0) = dims[0];
        let (eig_at, noise_at) = ((eig_off / 4) as u32, (noise_off / 4) as u32);
        let g_to_float = (0..2).map(|s| group(&shared.to_float, &[(0, &params([w0, h0, 0, 0, 0, img[s][0], stride, 0])), (1, &pyr), (3, &raw)])).collect();
        let g_down = (0..2).map(|s| (1..dims.len()).map(|k| {
            group(&shared.down, &[(0, &params([dims[k].0, dims[k].1, dims[k - 1].0, dims[k - 1].1, img[s][k - 1], img[s][k], 0, 0])), (1, &pyr)])
        }).collect::<Vec<_>>()).collect();
        let g_scharr = (0..2).map(|s| (0..dims.len()).map(|k| {
            group(&shared.scharr, &[(0, &params([dims[k].0, dims[k].1, 0, 0, img[s][k], grad[s][k], 0, 0])), (1, &pyr)])
        }).collect::<Vec<_>>()).collect();
        let g_cov = (0..2).map(|s| group(&shared.cov, &[(0, &params([w0, h0, 0, 0, img[s][0], 0, 0, 0])), (1, &pyr), (7, &work)])).collect();
        let g_eig = group(&shared.eig, &[(0, &params([w0, h0, 0, 0, 0, eig_at, 0, 0])), (7, &work)]);
        let g_corners = group(&shared.corners, &[(0, &params([w0, h0, 0, 0, 0, eig_at, capacity as u32, noise_at])), (7, &work), (8, &cands)]);
        let g_fwd = (0..2).map(|s| group(&shared.lk, &[(4, &lk_params(s)), (2, &pyr), (5, &pts), (6, &out_fwd)])).collect();
        let g_bwd = (0..2).map(|s| group(&shared.lk, &[(4, &lk_params(s)), (2, &pyr), (5, &out_fwd), (6, &out_bwd)])).collect();

        if let Some(e) = pollster::block_on(oom.pop()) { return Err(e.to_string()); }
        if let Some(e) = pollster::block_on(scope.pop()) { return Err(e.to_string()); }
        Ok(Self {
            shared: shared.clone(), size: (width, height), stride, max_points, capacity, raw, pts, out_fwd, out_bwd, work, cands, staging, dims,
            eig_off, noise_off, noise_len, fwd_off, bwd_off, cands_off,
            g_to_float, g_down, g_scharr, g_cov, g_eig, g_corners, g_fwd, g_bwd,
            slot: 1, frames: 0, pending: None, upload: Vec::new(), fwd: Vec::new(), bwd: Vec::new(), candidates: Vec::new(), noise: Vec::new(),
        })
    }

    /// The GPU's name and backend
    pub fn name(&self) -> &str { &self.shared.name }

    /// Uploads the next frame and has its pyramid, derivatives and candidate corners made; with `points` (of the frame
    /// before), tracks them into this one and back. `finish` waits for it
    pub fn submit(&mut self, pixels: &[u8], points: impl ExactSizeIterator<Item = [f32; 2]>) -> Result<(), String> {
        if self.shared.failed.load(Relaxed) { return Err("the device failed".into()); }
        if points.len() > self.max_points { return Err(format!("{} points to track, room for {}", points.len(), self.max_points)); }
        let (queue, device) = (&self.shared.queue, &self.shared.device);
        let prev = self.slot;
        let slot = 1 - prev;
        let n = if self.frames > 0 { points.len() } else { 0 };

        let len = self.stride as usize * self.size.1 as usize;
        let padded = len.div_ceil(4) * 4;
        if pixels.len() >= padded {
            queue.write_buffer(&self.raw, 0, &pixels[..padded]);
        } else {
            let mut v = pixels[..len].to_vec();
            v.resize(padded, 0);
            queue.write_buffer(&self.raw, 0, &v);
        }
        if n > 0 {
            self.upload.clear();
            self.upload.extend(points.take(n).map(|p| [p[0], p[1], 1.0, 0.0]));
            queue.write_buffer(&self.pts, 0, bytemuck::cast_slice(&self.upload));
        }

        let mut enc = device.create_command_encoder(&Default::default());
        enc.clear_buffer(&self.cands, 0, Some(8));
        {
            let groups = |(w, h): (u32, u32)| (w.div_ceil(8), h.div_ceil(8));
            let (gx, gy) = groups(self.dims[0]);
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("klt"), timestamp_writes: None });
            pass.set_pipeline(&self.shared.to_float);
            pass.set_bind_group(0, &self.g_to_float[slot], &[]);
            pass.dispatch_workgroups(gx, gy, 1);
            pass.set_pipeline(&self.shared.down);
            for k in 1..self.dims.len() {
                let (x, y) = groups(self.dims[k]);
                pass.set_bind_group(0, &self.g_down[slot][k - 1], &[]);
                pass.dispatch_workgroups(x, y, 1);
            }
            pass.set_pipeline(&self.shared.scharr);
            for k in 0..self.dims.len() {
                let (x, y) = groups(self.dims[k]);
                pass.set_bind_group(0, &self.g_scharr[slot][k], &[]);
                pass.dispatch_workgroups(x, y, 1);
            }
            for (pipeline, group) in [(&self.shared.cov, &self.g_cov[slot]), (&self.shared.eig, &self.g_eig), (&self.shared.corners, &self.g_corners)] {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(gx, gy, 1);
            }
            if n > 0 {
                pass.set_pipeline(&self.shared.lk);
                pass.set_bind_group(0, &self.g_fwd[prev], &[]);
                pass.dispatch_workgroups(n as u32, 1, 1);
                pass.set_bind_group(0, &self.g_bwd[slot], &[]);
                pass.dispatch_workgroups(n as u32, 1, 1);
            }
        }
        enc.copy_buffer_to_buffer(&self.work, self.noise_off, &self.staging, 0, self.noise_len as u64 * 4);
        if n > 0 {
            enc.copy_buffer_to_buffer(&self.out_fwd, 0, &self.staging, self.fwd_off, n as u64 * 16);
            enc.copy_buffer_to_buffer(&self.out_bwd, 0, &self.staging, self.bwd_off, n as u64 * 16);
        }
        enc.copy_buffer_to_buffer(&self.cands, 0, &self.staging, self.cands_off, 8 + self.capacity as u64 * 8);
        let submission = queue.submit(Some(enc.finish()));
        let (tx, mapped) = mpsc::channel();
        self.staging.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        self.pending = Some(Pending { submission, mapped, tracked: n });
        self.slot = slot;
        self.frames += 1;
        Ok(())
    }

    /// Waits for what `submit` sent, and reads it back into `fwd`, `bwd`, `candidates` and `noise`
    pub fn finish(&mut self) -> Result<(), String> {
        let Some(p) = self.pending.take() else { return Ok(()) };
        self.wait(p.submission, &p.mapped)?;
        if self.shared.failed.load(Relaxed) { self.staging.unmap(); return Err("the device failed".into()); }
        let count = {
            let data = self.staging.slice(..).get_mapped_range().map_err(|e| e.to_string())?;
            let at = |off: u64, len: usize| &data[off as usize..off as usize + len];
            self.noise.clear();
            self.noise.extend_from_slice(bytemuck::cast_slice(at(0, self.noise_len * 4)));
            self.fwd.clear();
            self.bwd.clear();
            self.fwd.extend_from_slice(bytemuck::cast_slice(at(self.fwd_off, p.tracked * 16)));
            self.bwd.extend_from_slice(bytemuck::cast_slice(at(self.bwd_off, p.tracked * 16)));
            let count = bytemuck::cast_slice::<u8, u32>(at(self.cands_off, 4))[0] as usize;
            self.candidates.clear();
            if count <= self.capacity {
                let items: &[[u32; 2]] = bytemuck::cast_slice(at(self.cands_off + 8, count * 8));
                self.candidates.extend(items.iter().map(|c| (c[1] as u64) << 32 | c[0] as u64));
            }
            count
        };
        self.staging.unmap();
        if count > self.capacity { self.candidates_from_strengths()?; }
        Ok(())
    }

    /// More candidates than there's room for (strengths equal over an area): the frame's corner strengths come back,
    /// and the candidates are found in them here
    fn candidates_from_strengths(&mut self) -> Result<(), String> {
        let (w, h) = self.size;
        let (device, queue) = (&self.shared.device, &self.shared.queue);
        let bytes = w as u64 * h as u64 * 4;
        let staging = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.work, self.eig_off, &staging, 0, bytes);
        let submission = queue.submit(Some(enc.finish()));
        let (tx, rx) = mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        self.wait(submission, &rx)?;
        let data = staging.slice(..).get_mapped_range().map_err(|e| e.to_string())?;
        self.candidates = local_maxima(bytemuck::cast_slice(&data), w as i32, h as i32);
        Ok(())
    }

    /// Waits for `submission` and for the buffer it maps. A device lost meanwhile panics in wgpu's poll (it returns only
    /// timeouts), and on the analysis' thread pool a panic takes the app down: an error here. Any failure, a timeout too,
    /// has the device made again for the next analysis
    fn wait(&self, submission: wgpu::SubmissionIndex, mapped: &mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>) -> Result<(), String> {
        if self.shared.failed.load(Relaxed) { return Err("the device failed".into()); }
        let device = &self.shared.device;
        let waited = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: Some(TIMEOUT) })))
            .map_err(|_| "the device was lost".to_string())
            .and_then(|polled| polled.map_err(|e| e.to_string()))
            .and_then(|_| mapped.recv_timeout(TIMEOUT).map_err(|e| e.to_string())?.map_err(|e| e.to_string()));
        if waited.is_err() { self.shared.failed.store(true, Relaxed); }
        waited
    }
}
