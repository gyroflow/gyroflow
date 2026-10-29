// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! KLT on the GPU, through wgpu (`gpu_klt.wgsl`): `Tracker::klt` (OpenCV's calcOpticalFlowPyrLK) and the candidate
//! corners `Tracker::replenish` picks from (the local maxima of cornerMinEigenVal). Each frame goes up once: its
//! pyramid, derivatives and corner strengths are made once and kept for the pair after it (OpenCV builds both pyramids
//! again on every call), and LK tracks the points of the frame before into it and back. Only the points and the
//! candidates come back. On the same points it agrees with OpenCV to about a thousandth of a pixel, see the tests in
//! `tracker.rs`
//!
//! Fast on a phone too (Adreno 650, 960x540 and 1500 points: 75 ms a frame before, 20 now, the same tracks to the bit
//! but for the order of LK's sums). Adreno caches textures and not storage buffers, and is slow at anything heavy on
//! workgroup memory: so each frame's levels and their derivatives are the mip levels of textures, r8uint and rg16sint
//! where the GPU can write those (exact either way), and level 0's corner strengths are 4 small passes through
//! textures. The levels go up in a submission of their own while the CPU picks the points to track (`submit_levels`),
//! which also keeps the GPU from clocking down between frames.
//! What it takes of the GPU's memory, a pixel of the frame tracked: ~41 bytes (r8uint/rg16sint; ~60 with f32s), the
//! two frames' pyramids 13 (32), level 0's steps between 20, the rest 8. PairKlt tracks at the sync's processing
//! resolution, up to the whole frame: 340 MB at 3840x2160. Array layers of level 0's size a level (for gathers, which
//! read only the first mip level and made LK 11% faster on Adreno) and a texture of the Sobel products took twice that

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
/// The levels above level 0 (`down`) in tiles of T x T pixels a workgroup; level 0's kernels take 8 x 8
const TILE: u32 = 16;

const UNIFORM: wgpu::BindingType = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
const fn storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None }
}
/// A texture read, by loads only: r32float and rg32float can't be filtered (without FLOAT32_FILTERABLE), nor integers
const fn read(format: wgpu::TextureFormat) -> wgpu::BindingType {
    let sample_type = match format {
        wgpu::TextureFormat::R8Uint => wgpu::TextureSampleType::Uint,
        wgpu::TextureFormat::Rg16Sint => wgpu::TextureSampleType::Sint,
        _ => wgpu::TextureSampleType::Float { filterable: false },
    };
    wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }
}
const fn written(format: wgpu::TextureFormat) -> wgpu::BindingType {
    wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 }
}
/// Level 0's steps between: the Sobel products' sums along the rows (xx, xy, yy), the corner strengths
const SUMS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const STRENGTHS: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Level 0's kernels, in the order they run
const LEVEL0: [&str; 4] = ["to_img", "box_rows", "box_cols", "nms"];

/// The levels' images and derivatives: f32s, or where the GPU can write them (Adreno can), r8uint and rg16sint: the
/// images are whole grey levels and the derivatives whole numbers within +-4080, as exact in a quarter and half the
/// bytes (see `gpu_klt.wgsl`'s <formats>)
#[derive(Clone, Copy, PartialEq)]
struct Formats { image: wgpu::TextureFormat, derivatives: wgpu::TextureFormat }

impl Formats {
    const WIDE: Self = Self { image: wgpu::TextureFormat::R32Float, derivatives: wgpu::TextureFormat::Rg32Float };
    const NARROW: Self = Self { image: wgpu::TextureFormat::R8Uint, derivatives: wgpu::TextureFormat::Rg16Sint };
    fn of(adapter: &wgpu::Adapter) -> Self {
        let usable = |f| adapter.get_texture_format_features(f).allowed_usages.contains(wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING);
        let narrow = adapter.features().contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) && usable(Self::NARROW.image) && usable(Self::NARROW.derivatives);
        if narrow { Self::NARROW } else { Self::WIDE }
    }
    fn features(self) -> wgpu::Features {
        if self == Self::NARROW { wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES } else { wgpu::Features::empty() }
    }
    /// The kernels' bindings (group 0), as `gpu_klt.wgsl` declares them: level 0's (`LEVEL0`), the other levels', LK's
    fn level0(self) -> [Vec<(u32, wgpu::BindingType)>; 4] {
        [
            vec![(0, UNIFORM), (1, storage(true)), (3, written(self.image))],
            vec![(0, UNIFORM), (2, read(self.image)), (4, written(self.derivatives)), (15, written(SUMS))],
            vec![(0, UNIFORM), (5, storage(false)), (16, read(SUMS)), (17, written(STRENGTHS))],
            vec![(0, UNIFORM), (5, storage(false)), (6, storage(false)), (18, read(STRENGTHS))],
        ]
    }
    fn down(self) -> Vec<(u32, wgpu::BindingType)> {
        vec![(0, UNIFORM), (2, read(self.image)), (3, written(self.image)), (4, written(self.derivatives))]
    }
    fn lk(self) -> Vec<(u32, wgpu::BindingType)> {
        vec![(7, UNIFORM), (8, storage(true)), (9, storage(false)), (10, read(self.image)), (11, read(self.derivatives)), (12, read(self.image))]
    }
}

/// What a process makes once for a GPU: the device and the pipelines. A device that failed (lost, an error, a wait for
/// it that timed out) is made again for the next analysis. The adapter is one of `WgpuWrapper`'s, the same one for the
/// same GPU every time (see `compute_adapter`): two GPUs of a kind have the same name and ids
struct Shared {
    adapter: wgpu::Adapter,
    name: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    failed: Arc<AtomicBool>,
    level0: Vec<wgpu::ComputePipeline>,
    down: wgpu::ComputePipeline,
    lk: wgpu::ComputePipeline,
    formats: Formats,
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
        let formats = Formats::of(adapter);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("tracking"),
            required_features: formats.features(),
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
        // Without wgpu's checks (clamped indices, bounded loops, guarded divisions: a few times the instructions of LK's
        // inner loop). Every index is in its buffer and every texel in its texture by construction: coordinates are
        // reflected into the frame or checked against it, the candidates appended only within their room, and every
        // loop is bounded by its own count
        let source = wgpu::ShaderSource::Wgsl(kernels(formats).into());
        let module = unsafe { device.create_shader_module_trusted(wgpu::ShaderModuleDescriptor { label: Some("klt"), source }, wgpu::ShaderRuntimeChecks::unchecked()) };
        let pipeline = |entry: &str, bindings: &[(u32, wgpu::BindingType)]| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry), layout: Some(&layout(&device, entry, bindings)), module: &module, entry_point: Some(entry), compilation_options: Default::default(), cache: None
        });
        let level0 = LEVEL0.iter().zip(formats.level0()).map(|(entry, bindings)| pipeline(entry, &bindings)).collect();
        let (down, lk) = (pipeline("down", &formats.down()), pipeline("lk", &formats.lk()));
        let errors: Vec<wgpu::Error> = scopes.into_iter().rev().filter_map(|s| pollster::block_on(s.pop())).collect();
        if let Some(e) = errors.first() { return Err(e.to_string()); }
        // Not kept when it failed some other way while it was made
        if failed.load(Relaxed) { return Err("the device failed while setting up".into()); }

        let name = format!("{} ({:?})", info.name, info.backend);
        let shared = Arc::new(Self { adapter: adapter.clone(), name, device, queue, failed, level0, down, lk, formats });
        *cached = Some(shared.clone());
        Ok(shared)
    }
}

/// The kernels, with the levels in `formats`
fn kernels(formats: Formats) -> String {
    let src = include_str!("gpu_klt.wgsl");
    if formats == Formats::WIDE { return src.into(); }
    let (head, rest) = src.split_once("// <formats>").expect("formats marker");
    let (_, tail) = rest.split_once("// </formats>").expect("formats marker");
    format!("{head}// r8uint and rg16sint (see gpu_klt.rs)
alias ImageIn = texture_2d<u32>;
alias ImageOut = texture_storage_2d<r8uint, write>;
alias GradOut = texture_storage_2d<rg16sint, write>;
fn img_load(t: ImageIn, p: vec2<i32>, lvl: i32) -> f32 {{ return f32(textureLoad(t, p, lvl).x); }}
fn img_store(p: vec2<i32>, v: f32) {{ textureStore(dst_img, p, vec4<u32>(u32(v), 0u, 0u, 0u)); }}
fn grad_store(p: vec2<i32>, v: vec2<f32>) {{ textureStore(dst_grad, p, vec4<i32>(vec2<i32>(v), 0, 0)); }}
alias GradIn = texture_2d<i32>;
fn grad_load(t: GradIn, p: vec2<i32>, lvl: i32) -> vec2<f32> {{ return vec2<f32>(textureLoad(t, p, lvl).xy); }}
{tail}")
}

fn tv(view: &wgpu::TextureView) -> wgpu::BindingResource<'_> { wgpu::BindingResource::TextureView(view) }

/// A bind group (group 0) of pipeline `p`
fn bind_group(device: &wgpu::Device, p: &wgpu::ComputePipeline, entries: &[(u32, wgpu::BindingResource)]) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &p.get_bind_group_layout(0),
        entries: &entries.iter().map(|(binding, r)| wgpu::BindGroupEntry { binding: *binding, resource: r.clone() }).collect::<Vec<_>>(),
    })
}

/// The pipeline layout of a kernel with these `bindings` in group 0: wgpu's own would take the textures for filterable
fn layout(device: &wgpu::Device, label: &str, bindings: &[(u32, wgpu::BindingType)]) -> wgpu::PipelineLayout {
    let entries: Vec<wgpu::BindGroupLayoutEntry> = bindings.iter().map(|&(binding, ty)| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None }).collect();
    let group = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries: &entries });
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(&group)], immediate_size: 0 })
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
    /// Per slot of the two frames kept: its levels' images and derivatives, the mip levels of these; and level 0's steps
    /// between
    _levels: Vec<wgpu::Texture>,
    _steps: Vec<wgpu::Texture>,
    /// The pyramid's levels
    dims: Vec<(u32, u32)>,
    /// In `work`, bytes: the noise samples (the corner strengths are at 0), and how many of those
    noise_off: u64,
    noise_len: usize,
    /// In `staging`, bytes: the points forward and back, the candidates (their count first)
    fwd_off: u64,
    bwd_off: u64,
    cands_off: u64,
    // Per slot: the frame going into it, level 0's steps then the other levels
    g_level0: Vec<Vec<wgpu::BindGroup>>,
    g_down: Vec<Vec<wgpu::BindGroup>>,
    /// LK with I in that slot and J in the other: forward from the points uploaded, back from where they went
    g_fwd: Vec<wgpu::BindGroup>,
    g_bwd: Vec<wgpu::BindGroup>,
    /// The slot of the last frame, and how many went up; the slot of one whose levels are made and not tracked into yet
    slot: usize,
    frames: usize,
    levels_up: Option<usize>,
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

        let wh = width as u64 * height as u64;
        let capacity = (wh / PIXELS_PER_CANDIDATE as u64).max(1024) as usize;
        let noise_len = (width.div_ceil(8) * height.div_ceil(8)) as usize;
        let storage = |size: u64, extra: wgpu::BufferUsages| device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size, usage: wgpu::BufferUsages::STORAGE | extra, mapped_at_creation: false
        });
        let raw = storage((stride as u64 * height as u64).div_ceil(4) * 4, wgpu::BufferUsages::COPY_DST);
        let pts = storage(max_points as u64 * 16, wgpu::BufferUsages::COPY_DST);
        let out_fwd = storage(max_points as u64 * 16, wgpu::BufferUsages::COPY_SRC);
        let out_bwd = storage(max_points as u64 * 16, wgpu::BufferUsages::COPY_SRC);
        // The corner strengths, the noise samples
        let work = storage((wh + noise_len as u64) * 4, wgpu::BufferUsages::COPY_SRC);
        let cands = storage(8 + capacity as u64 * 8, wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST);
        let noise_off = wh * 4;
        let fwd_off = (noise_len as u64 * 4).next_multiple_of(16);
        let bwd_off = fwd_off + max_points as u64 * 16;
        let cands_off = bwd_off + max_points as u64 * 16;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: cands_off + 8 + capacity as u64 * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false
        });

        // The levels of a slot as the mip levels of one texture: its size a multiple of 2^(levels - 1), so that every
        // mip level is at least as large as its level (each half the one above, rounded up)
        let n = dims.len() as u32;
        let unit = 1u32 << (n - 1);
        let size = wgpu::Extent3d { width: width.next_multiple_of(unit), height: height.next_multiple_of(unit), depth_or_array_layers: 1 };
        let texture = |format| device.create_texture(&wgpu::TextureDescriptor {
            label: None, size, mip_level_count: n, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING, view_formats: &[]
        });
        // The images of slots 0 and 1, then their derivatives
        let f = shared.formats;
        let levels: Vec<wgpu::Texture> = [f.image, f.image, f.derivatives, f.derivatives].into_iter().map(texture).collect();
        let (img, grad) = (|s: usize| &levels[s], |s: usize| &levels[2 + s]);
        let step = |format| device.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING, view_formats: &[]
        });
        // The products' sums along the rows, the strengths
        let steps: Vec<wgpu::Texture> = [SUMS, STRENGTHS].into_iter().map(step).collect();
        let mip = |t: &wgpu::Texture, k: usize| t.create_view(&wgpu::TextureViewDescriptor { base_mip_level: k as u32, mip_level_count: Some(1), ..Default::default() });
        let all = |t: &wgpu::Texture| t.create_view(&Default::default());

        let uniform = |words: &[u32]| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(words), usage: wgpu::BufferUsages::UNIFORM });
        // `Params`: w, h, sw, sh, stride, cap, eig_at, noise_at
        let (w0, h0) = dims[0];
        let p0 = uniform(&[w0, h0, 0, 0, stride, capacity as u32, 0, wh as u32]);
        let g_level0 = (0..2).map(|s| {
            let (vi, vg, rows, eig) = (mip(img(s), 0), mip(grad(s), 0), all(&steps[0]), all(&steps[1]));
            let entries: [&[(u32, wgpu::BindingResource)]; 4] = [
                &[(0, p0.as_entire_binding()), (1, raw.as_entire_binding()), (3, tv(&vi))],
                &[(0, p0.as_entire_binding()), (2, tv(&vi)), (4, tv(&vg)), (15, tv(&rows))],
                &[(0, p0.as_entire_binding()), (5, work.as_entire_binding()), (16, tv(&rows)), (17, tv(&eig))],
                &[(0, p0.as_entire_binding()), (5, work.as_entire_binding()), (6, cands.as_entire_binding()), (18, tv(&eig))],
            ];
            shared.level0.iter().zip(entries).map(|(p, e)| bind_group(device, p, e)).collect::<Vec<_>>()
        }).collect();
        let g_down = (0..2).map(|s| (1..dims.len()).map(|k| {
            let p = uniform(&[dims[k].0, dims[k].1, dims[k - 1].0, dims[k - 1].1, 0, 0, 0, 0]);
            let (src, vi, vg) = (mip(img(s), k - 1), mip(img(s), k), mip(grad(s), k));
            bind_group(device, &shared.down, &[(0, p.as_entire_binding()), (2, tv(&src)), (3, tv(&vi)), (4, tv(&vg))])
        }).collect::<Vec<_>>()).collect();
        // `LkParams`, and the levels' sizes
        let lk = {
            let mut v = vec![max_points as u32, n, KLT_ITERATIONS as u32, 0, ((KLT_EPS * KLT_EPS) as f32).to_bits(), (KLT_MIN_EIG as f32).to_bits(), 0, 0];
            for k in 0..8 { v.extend(if k < dims.len() { [dims[k].0, dims[k].1, 0, 0] } else { [0; 4] }); }
            uniform(&v)
        };
        let lk_group = |s: usize, from: &wgpu::Buffer, to: &wgpu::Buffer| {
            let (vi, vg, vj) = (all(img(s)), all(grad(s)), all(img(1 - s)));
            bind_group(device, &shared.lk, &[(7, lk.as_entire_binding()), (8, from.as_entire_binding()), (9, to.as_entire_binding()), (10, tv(&vi)), (11, tv(&vg)), (12, tv(&vj))])
        };
        let g_fwd = (0..2).map(|s| lk_group(s, &pts, &out_fwd)).collect();
        let g_bwd = (0..2).map(|s| lk_group(s, &out_fwd, &out_bwd)).collect();

        if let Some(e) = pollster::block_on(oom.pop()) { return Err(e.to_string()); }
        if let Some(e) = pollster::block_on(scope.pop()) { return Err(e.to_string()); }
        Ok(Self {
            shared: shared.clone(), size: (width, height), stride, max_points, capacity, raw, pts, out_fwd, out_bwd, work, cands, staging, _levels: levels, _steps: steps, dims,
            noise_off, noise_len, fwd_off, bwd_off, cands_off,
            g_level0, g_down, g_fwd, g_bwd,
            slot: 1, frames: 0, levels_up: None, pending: None, upload: Vec::new(), fwd: Vec::new(), bwd: Vec::new(), candidates: Vec::new(), noise: Vec::new(),
        })
    }

    /// The GPU's name and backend
    pub fn name(&self) -> &str { &self.shared.name }

    /// Uploads the next frame and has its pyramid, derivatives and candidate corners made; with `points` (of the frame
    /// before), tracks them into this one and back. `finish` waits for it
    pub fn submit(&mut self, pixels: &[u8], points: impl ExactSizeIterator<Item = [f32; 2]>) -> Result<(), String> {
        self.submit_levels(pixels)?;
        self.submit_lk(points)
    }

    /// `submit` in two: the next frame up and its levels made, while the points to track are still being picked
    pub fn submit_levels(&mut self, pixels: &[u8]) -> Result<(), String> {
        if self.shared.failed.load(Relaxed) { return Err("the device failed".into()); }
        let (queue, device) = (&self.shared.queue, &self.shared.device);
        let slot = 1 - self.slot;

        let len = self.stride as usize * self.size.1 as usize;
        let padded = len.div_ceil(4) * 4;
        if pixels.len() >= padded {
            queue.write_buffer(&self.raw, 0, &pixels[..padded]);
        } else {
            let mut v = pixels[..len].to_vec();
            v.resize(padded, 0);
            queue.write_buffer(&self.raw, 0, &v);
        }

        let mut enc = device.create_command_encoder(&Default::default());
        enc.clear_buffer(&self.cands, 0, Some(8));
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("levels"), timestamp_writes: None });
            let (w, h) = self.dims[0];
            for (pipeline, group) in self.shared.level0.iter().zip(&self.g_level0[slot]) {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            pass.set_pipeline(&self.shared.down);
            for k in 1..self.dims.len() {
                let (w, h) = self.dims[k];
                pass.set_bind_group(0, &self.g_down[slot][k - 1], &[]);
                pass.dispatch_workgroups(w.div_ceil(TILE), h.div_ceil(TILE), 1);
            }
        }
        queue.submit(Some(enc.finish()));
        self.levels_up = Some(slot);
        Ok(())
    }

    /// ... and `points` (of the frame before) tracked into it and back, after `submit_levels`
    pub fn submit_lk(&mut self, points: impl ExactSizeIterator<Item = [f32; 2]>) -> Result<(), String> {
        if self.shared.failed.load(Relaxed) { return Err("the device failed".into()); }
        if points.len() > self.max_points { return Err(format!("{} points to track, room for {}", points.len(), self.max_points)); }
        let Some(slot) = self.levels_up.take() else { return Err("no frame went up to track into".into()) };
        let (queue, device) = (&self.shared.queue, &self.shared.device);
        let prev = self.slot;
        let n = if self.frames > 0 { points.len() } else { 0 };
        if n > 0 {
            self.upload.clear();
            self.upload.extend(points.take(n).map(|p| [p[0], p[1], 1.0, 0.0]));
            queue.write_buffer(&self.pts, 0, bytemuck::cast_slice(&self.upload));
        }

        let mut enc = device.create_command_encoder(&Default::default());
        if n > 0 {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("lk"), timestamp_writes: None });
            pass.set_pipeline(&self.shared.lk);
            for group in [&self.g_fwd[prev], &self.g_bwd[slot]] {
                pass.set_bind_group(0, group, &[]);
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
        enc.copy_buffer_to_buffer(&self.work, 0, &staging, 0, bytes);
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
