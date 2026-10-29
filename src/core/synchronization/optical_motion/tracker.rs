// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Persistent feature tracks from one frame to the next. Every pair of consecutive frames yields the points that
//! survived a forward-backward check, with the id of the track they belong to: the analysis needs whole tracks, to
//! take the slow parallax of each point out of its motion. KLT starts its points at corners, DIS and gmflow on a
//! regular grid (see `TrackingMethod`); all away from glare. KLT runs on the GPU when there is one (`gpu_klt`), and with OpenCV
//! without, or from the frame after the GPU failed. A build without OpenCV has the GPU's KLT only

use std::{ sync::mpsc, time::{ Duration, Instant } };
#[cfg(feature = "use-opencv")]
use opencv::{ core::{ self, Mat, Point2f, Ptr, Rect, Size, TermCriteria, Vec2f, Vector, CV_8UC1 }, prelude::*, imgproc, video };
use super::{ Observation, TrackingMethod, gpu_klt::GpuKlt };

/// Grid the new corners are spread over, see `Tracker::replenish`
const GRID_ROWS: i32 = 6;
const GRID_COLS: i32 = 4;
/// What counts as a corner: its smaller eigenvalue against the strongest one in the frame
const QUALITY: f64 = 0.002;
/// ... or, in a cell with nothing that strong (cloudy sky), against the noise of flat areas, see `replenish`
const NOISE_MULTIPLE: f64 = 8.0;
/// DIS's grid: where there's at least this many times the noise of flat areas around a cell's middle, see `replenish_grid`
#[cfg(feature = "use-opencv")]
const DIS_NOISE_MULTIPLE: f64 = 2.0;
/// No new corners within this many radii of a compact saturated blob (the sun): its glare is a smooth ring the tracker
/// slides along and its flare is fixed to the lens, neither moves like the scene. Blobs from this many pixels up, at
/// 960 px wide - glints on water are smaller, and the robust fit takes care of them among everything else
const GLARE_RADII: f64 = 2.5;
const GLARE_MIN_AREA: f64 = 30.0;
/// Saturated: above this
const GLARE_LEVEL: u8 = 249;
/// KLT's window, pixels: a point sees this much around it
pub(super) const WINDOW: i32 = 21;
/// ... the levels of its pyramid above the frame, when it stops: after this many iterations or a step this short (px),
/// and the smallest eigenvalue of a window it tracks (OpenCV's minEigThreshold, of the window's mean)
pub(super) const KLT_LEVELS: i32 = 3;
pub(super) const KLT_ITERATIONS: i32 = 50;
pub(super) const KLT_EPS: f64 = 0.001;
pub(super) const KLT_MIN_EIG: f64 = 1e-4;
/// How far a point may end up from where it started, tracked forward and back again, pixels
const FB_MAX: f32 = 0.1;
/// ... with DIS. Its two flows are computed apart and smoothed over patches, and disagree by more than KLT's limit
/// at half of the points of water and sky (median 0.12 px on a lake, 0.06 over a forest): at 0.1 px most of its tracks
/// broke before the 15 frames that count, and the water didn't take part at all. How far a point is off grows with
/// the disagreement (0.06 px + 0.8 times it, against KLT where KLT is sure), so it stays the limit, just looser: at
/// 0.3 px five times the measurements count on the lake (twice over the forest), each a little worse, the
/// precision of a band better for it - what's left of the shake of the lake's water in the stabilized picture went from
/// 3.4 to 1.5 px, and down in every third of the frame on both clips. Not looser still, and not looser with the motion
/// (Sundaram et al. 2010): beyond that the points it adds measure less of the motion than there is - blur, ripples -
/// which no fit averages out
#[cfg(feature = "use-opencv")]
const DIS_FB_MAX: f32 = 0.3;
/// DIS: the preset, and the finest level of its pyramid the flow is computed at (0: full resolution; the presets'
/// own are 1 or 2, a half or a quarter, upsampled from there)
#[cfg(feature = "use-opencv")]
const DIS_PRESET: i32 = video::DISOpticalFlow_PRESET_MEDIUM;
#[cfg(feature = "use-opencv")]
const DIS_FINEST_SCALE: i32 = 1;
/// How far a point may end up from where it started, tracked forward and back again through gmflow's flow, pixels of
/// the tracked frame. Tighter than DIS's: measured with the motion from the video alone against the motion data, two
/// stretches of 60 frames of a GoPro HERO6 test clip (2 degrees of motion per frame), the frame-to-frame rotation was
/// off by 0.062 and 0.061 degrees rms at 0.2 px - KLT 0.063 and 0.052, DIS 0.059 and 0.055 - and by 0.067 and 0.20
/// at DIS's 0.3 px. Below 0.15 px too few tracks last (0.10 px: 0.17 degrees, under 0.07 px nothing to measure)
#[cfg(all(feature = "use-opencv", feature = "use-burn"))]
const GMFLOW_FB_MAX: f32 = 0.2;
/// Without OpenCV there's no KLT but the GPU's
const NO_GPU: &str = "Optical analysis needs a GPU in this build";
/// Finding the GPU and setting it up taking longer than this means its driver hung: as if there were none
const SET_UP_TIMEOUT: Duration = Duration::from_secs(30);

/// Where KLT runs: on the CPU, or on a GPU - the one of the device selected for processing (its index in the device
/// list, `GPU_LIST`) when that has a wgpu adapter, see `WgpuWrapper::compute_adapter`
#[derive(Clone, Copy, Debug)]
pub enum KltDevice {
    Cpu,
    Gpu(Option<usize>),
}

impl KltDevice {
    /// The GPU selected for processing (`current_device`, or another one), the CPU when that's what's selected
    pub fn for_processing(current_device: i32) -> Self {
        match current_device {
            i if i < 0 || !std::env::var("NO_WGPU").unwrap_or_default().is_empty() => Self::Cpu,
            i => Self::Gpu(Some(i as usize)),
        }
    }
}

/// Looks for the adapter of the GPU `selected` (see `KltDevice`) and makes its device and pipelines, on a thread: the
/// first time in a process looking for an adapter can take seconds (the wgpu instance is made then, with every backend),
/// and making the device and the pipelines a tenth of one. `found_gpu` waits for it
pub(super) fn find_gpu(selected: Option<usize>) -> mpsc::Receiver<Result<wgpu::Adapter, String>> {
    let (found, finding) = mpsc::channel();
    std::thread::spawn(move || {
        let adapter = crate::gpu::wgpu::WgpuWrapper::compute_adapter(selected).ok_or_else(|| "no GPU to use".to_string());
        let _ = found.send(adapter.and_then(|a| GpuKlt::prepare(&a).map(|_| a)));
    });
    finding
}

/// What `find_gpu` found, or why there's nothing
pub(super) fn found_gpu(finding: mpsc::Receiver<Result<wgpu::Adapter, String>>) -> Result<wgpu::Adapter, String> {
    finding.recv_timeout(SET_UP_TIMEOUT).unwrap_or_else(|e| Err(match e {
        mpsc::RecvTimeoutError::Timeout => format!("it took over {} s to set up", SET_UP_TIMEOUT.as_secs()),
        mpsc::RecvTimeoutError::Disconnected => "looking for it failed".into(),
    }))
}

pub struct Tracker {
    /// DIS, when it follows the points through the dense flow; KLT without
    #[cfg(feature = "use-opencv")]
    dis: Option<Ptr<video::DISOpticalFlow>>,
    /// gmflow, when it follows the points through its dense flow (on its own GPU, see `optical_flow::GmflowDenseFlow`)
    #[cfg(feature = "use-opencv")]
    gmflow: bool,
    /// Whether to give up on the frame being tracked (cancelled), checked before gmflow's pass
    #[cfg_attr(not(feature = "use-burn"), allow(dead_code))]
    stop: Box<dyn Fn() -> bool + Send + Sync>,
    /// The frame before, when there is one: its pixels (without the padding of rows) when the CPU tracks, none when the
    /// GPU does - it keeps its own
    prev: Option<Vec<u8>>,
    size: (i32, i32),
    points: Vec<[f32; 2]>,
    ids: Vec<u32>,
    next_id: u32,
    max_points: usize,
    /// KLT: the candidate corners of the frame before (see `local_maxima`, the strongest first), the corner strengths of
    /// every 8th pixel of it in both directions (its noise is a percentile of those) and its glare: what the next
    /// `replenish` picks from
    candidates: Vec<u64>,
    noise: Vec<f32>,
    glare: Vec<(i32, i32, i32)>,
    /// Whether `candidates` are sorted yet: from the GPU they're sorted only once needed, while it makes the next frame's
    /// levels
    sorted: bool,
    /// KLT on a GPU: whether it's wanted (not after it failed), its adapter or why there's none (looked for and its device
    /// made on a thread from the start, see `new`) and what's set up on it for the frame size
    gpu_wanted: bool,
    finding: Option<mpsc::Receiver<Result<wgpu::Adapter, String>>>,
    adapter: Result<wgpu::Adapter, String>,
    gpu: Option<GpuKlt>,
    /// Where the tracking ran, how long setting it up took and the tracking itself over how many frames, for the log
    device: String,
    setup: f64,
    busy: f64,
    frames: usize,
}

impl Tracker {
    pub fn new(max_points: usize, method: TrackingMethod, device: KltDevice) -> Result<Self, String> {
        #[cfg(feature = "use-opencv")]
        let dis = match method {
            TrackingMethod::Klt | TrackingMethod::Gmflow => None,
            TrackingMethod::Dis => {
                let mut dis = video::DISOpticalFlow::create(DIS_PRESET).map_err(cv)?;
                if DIS_FINEST_SCALE >= 0 { dis.set_finest_scale(DIS_FINEST_SCALE).map_err(cv)?; }
                Some(dis)
            },
        };
        // gmflow's points are DIS's grid, which OpenCV finds
        if matches!(method, TrackingMethod::Gmflow) && !cfg!(all(feature = "use-opencv", feature = "use-burn")) {
            return Err("gmflow tracking needs a build with the ai-optical-flow feature".into());
        }
        #[cfg(not(feature = "use-opencv"))]
        {
            if matches!(method, TrackingMethod::Dis) { return Err("DIS tracking is not available in this build".into()); }
            if matches!(device, KltDevice::Cpu) { return Err(NO_GPU.into()); }
        }
        // Done while the decoder starts
        let finding = match device {
            KltDevice::Gpu(selected) if matches!(method, TrackingMethod::Klt) => Some(find_gpu(selected)),
            _ => None,
        };
        Ok(Self {
            #[cfg(feature = "use-opencv")]
            dis,
            #[cfg(feature = "use-opencv")]
            gmflow: matches!(method, TrackingMethod::Gmflow),
            stop: Box::new(|| false),
            prev: None, size: (0, 0), points: Vec::new(), ids: Vec::new(), next_id: 0, max_points,
            candidates: Vec::new(), noise: Vec::new(), glare: Vec::new(), sorted: true,
            gpu_wanted: finding.is_some(), finding, adapter: Err("no GPU to use".into()), gpu: None, device: "the CPU".into(), setup: 0.0, busy: 0.0, frames: 0,
        })
    }

    /// Where the tracking ran and how long it took a frame, for the log
    pub fn describe(&self) -> String {
        let setup = if self.setup > 0.0 { format!(" (set up in {:.0} ms)", self.setup * 1000.0) } else { String::new() };
        format!("tracked on {} in {:.1} ms a frame{setup}", self.device, self.busy * 1000.0 / self.frames.max(1) as f64)
    }

    /// What says to give up on a frame halfway (before gmflow's pass): it then tracks nothing, and whoever stopped it
    /// ignores the rest
    pub fn set_stop(&mut self, stop: Box<dyn Fn() -> bool + Send + Sync>) {
        self.stop = stop;
    }

    /// Forget everything: the next frame starts new tracks (a gap in the decoded frames, a cut)
    pub fn reset(&mut self) {
        self.prev = None;
        self.points.clear();
        self.ids.clear();
    }

    /// Tracks the points of the previous frame into this one. Empty for the first frame
    pub fn track(&mut self, width: u32, height: u32, stride: usize, pixels: &[u8]) -> Result<Vec<Observation>, String> {
        let (w, h) = (width as i32, height as i32);
        if pixels.len() < stride * height as usize { return Ok(Vec::new()); }
        if self.size != (w, h) { self.reset(); }
        self.size = (w, h);
        if self.gpu_wanted && self.gpu.as_ref().is_none_or(|g| g.size != (width, height) || g.stride != stride as u32) {
            let started = Instant::now();
            let set_up = self.set_up_gpu(width, height, stride);
            self.setup += started.elapsed().as_secs_f64();
            set_up?;
        }

        let started = Instant::now();
        let mut out = Vec::new();
        let prev = self.prev.take();
        #[cfg(feature = "use-opencv")]
        let dense = match (self.dis.is_some(), self.gmflow) {
            (true, _) => Some(self.dis_frame(prev.as_deref(), pixels, stride).map_err(cv)?),
            #[cfg(feature = "use-burn")]
            (false, true) => Some(self.gmflow_frame(prev.as_deref(), pixels, stride)?),
            _ => None,
        };
        #[cfg(not(feature = "use-opencv"))]
        let dense = None;
        let moved = match dense {
            Some(moved) => moved,
            None => {
                // On the GPU this frame goes up and its levels are made while the CPU picks the points to track into it:
                // the candidates of the frame before sorted and topped up from. All at once after that, the GPU sat idle
                // meanwhile, clocked down and took 1.5 times as long (a phone, Adreno 650)
                let levels = self.gpu.as_mut().map(|gpu| gpu.submit_levels(pixels));
                if prev.is_some() {
                    self.sort_candidates();
                    self.replenish();
                }
                let tracking = prev.is_some() && !self.points.is_empty();
                self.klt_frame(prev.as_deref(), pixels, stride, tracking, levels)?
            }
        };
        if let Some(moved) = moved {
            let mut points = Vec::with_capacity(self.points.len());
            let mut ids = Vec::with_capacity(self.points.len());
            for (i, b) in moved.into_iter().enumerate() {
                let Some(b) = b else { continue };
                let a = self.points[i];
                let inside = b[0] > 2.0 && b[1] > 2.0 && b[0] < (w - 3) as f32 && b[1] < (h - 3) as f32;
                if inside {
                    out.push(Observation { id: self.ids[i], a, b });
                    points.push(b);
                    ids.push(self.ids[i]);
                }
            }
            self.points = points;
            self.ids = ids;
        }
        self.prev = Some(if self.gpu.is_some() { Vec::new() } else { packed(pixels, stride, width as usize, height as usize) });
        self.busy += started.elapsed().as_secs_f64();
        self.frames += 1;
        Ok(out)
    }

    /// KLT on the GPU, for this frame size, once its adapter is found. On the CPU from here on when there's none or it
    /// can't be set up, and without OpenCV an error then
    fn set_up_gpu(&mut self, width: u32, height: u32, stride: usize) -> Result<(), String> {
        self.gpu = None;
        if let Some(finding) = self.finding.take() {
            self.adapter = found_gpu(finding);
        }
        // Room for every point: `replenish` tops each cell up to its share, so up to twice `max_points`, with fewer than
        // that before it
        let result = match &self.adapter {
            Ok(adapter) => GpuKlt::new(adapter, width, height, stride as u32, 2 * self.max_points),
            Err(e) => Err(e.clone()),
        };
        // The frame before, if any, was on the GPU: neither one set up again nor the CPU has it
        self.reset();
        match result {
            Ok(gpu) => {
                log::info!("Optical analysis: tracking on {}", gpu.name());
                self.device = gpu.name().to_string();
                self.gpu = Some(gpu);
                Ok(())
            },
            Err(e) => {
                self.gpu_wanted = false;
                self.device = "the CPU (the GPU couldn't be set up)".into();
                if !cfg!(feature = "use-opencv") { return Err(format!("{NO_GPU} ({e})")); }
                log::warn!("Optical analysis: tracking on the CPU, the GPU couldn't be set up for it: {e}");
                Ok(())
            }
        }
    }

    /// The candidates in the order `replenish` takes them: the strongest first
    fn sort_candidates(&mut self) {
        if self.sorted { return; }
        self.candidates.sort_unstable_by(|a, b| b.cmp(a));
        self.sorted = true;
    }

    /// KLT: the points of `prev` tracked into this frame (with `tracking`), and this frame's candidate corners, noise and
    /// glare found for the next `replenish`. On the GPU (`levels`: this frame's gone up, see `track`), the glare found
    /// while it works; with OpenCV when there's no GPU, and from the frame after the one it failed on
    fn klt_frame(&mut self, prev: Option<&[u8]>, pixels: &[u8], stride: usize, tracking: bool, levels: Option<Result<(), String>>) -> Result<Option<Vec<Option<[f32; 2]>>>, String> {
        let (w, h) = self.size;
        if let Some(gpu) = &mut self.gpu {
            let points = if tracking { &self.points[..] } else { &[] };
            let submitted = levels.unwrap_or_else(|| gpu.submit_levels(pixels)).and_then(|_| gpu.submit_lk(points.iter().copied()));
            self.glare = glare(pixels, stride, w, h);
            let finished = submitted.and_then(|_| gpu.finish());
            match finished {
                Ok(()) => {
                    // Taking them over gives `gpu` the old ones to fill again
                    std::mem::swap(&mut self.candidates, &mut gpu.candidates);
                    std::mem::swap(&mut self.noise, &mut gpu.noise);
                    self.sorted = false;
                    return Ok(tracking.then(|| checked(&self.points, &gpu.fwd, &gpu.bwd, FB_MAX)));
                },
                Err(e) if cfg!(feature = "use-opencv") => {
                    log::warn!("Optical analysis: tracking on the CPU from the next frame, the GPU failed: {e}");
                    self.gpu = None;
                    self.gpu_wanted = false;
                    self.device = "the CPU (the GPU failed)".into();
                    // The frame before is on the GPU only: its points are lost, new ones start on this frame
                    self.points.clear();
                    self.ids.clear();
                },
                Err(e) => return Err(format!("{NO_GPU}, and it failed: {e}")),
            }
        }
        let tracking = tracking && !self.points.is_empty();
        #[cfg(feature = "use-opencv")]
        return self.klt_frame_cpu(prev, pixels, stride, tracking).map_err(cv);
        #[cfg(not(feature = "use-opencv"))]
        { let _ = (prev, tracking); Err(NO_GPU.into()) }
    }

    /// `klt_frame` with OpenCV
    #[cfg(feature = "use-opencv")]
    fn klt_frame_cpu(&mut self, prev: Option<&[u8]>, pixels: &[u8], stride: usize, tracking: bool) -> opencv::Result<Option<Vec<Option<[f32; 2]>>>> {
        let (w, h) = self.size;
        let cur = frame_mat(pixels, stride, w, h)?;
        let moved = match prev {
            Some(prev) if tracking => Some(Self::klt(&frame_mat(prev, w as usize, w, h)?, &cur, &self.points, FB_MAX)?),
            _ => None,
        };
        let eig = corner_strengths(&cur)?;
        let eig = eig.data_typed::<f32>()?;
        self.candidates = local_maxima(eig, w, h);
        self.sorted = true;
        self.noise = noise_samples(eig, w, h);
        self.glare = glare(pixels, stride, w, h);
        Ok(moved)
    }

    /// Where each point went, by pyramidal Lucas-Kanade: None where it lost it, or didn't come back within `fb_max` px of
    /// where it started
    #[cfg(feature = "use-opencv")]
    pub(super) fn klt(prev: &Mat, cur: &Mat, points: &[[f32; 2]], fb_max: f32) -> opencv::Result<Vec<Option<[f32; 2]>>> {
        let pa: Vector<Point2f> = points.iter().map(|p| Point2f::new(p[0], p[1])).collect();
        let mut pb = Vector::<Point2f>::new();
        let mut pa2 = Vector::<Point2f>::new();
        let mut st = Vector::<u8>::new();
        let mut st2 = Vector::<u8>::new();
        let mut err = Vector::<f32>::new();
        let criteria = TermCriteria::new(3 /* COUNT | EPS */, KLT_ITERATIONS, KLT_EPS)?;
        let window = Size::new(WINDOW, WINDOW);
        video::calc_optical_flow_pyr_lk(prev, cur, &pa, &mut pb, &mut st, &mut err, window, KLT_LEVELS, criteria, 0, KLT_MIN_EIG)?;
        video::calc_optical_flow_pyr_lk(cur, prev, &pb, &mut pa2, &mut st2, &mut err, window, KLT_LEVELS, criteria, 0, KLT_MIN_EIG)?;
        (0..pa.len()).map(|i| {
            let (a, b, a2) = (pa.get(i)?, pb.get(i)?, pa2.get(i)?);
            let fb = ((a2.x - a.x).powi(2) + (a2.y - a.y).powi(2)).sqrt();
            Ok((st.get(i)? == 1 && st2.get(i)? == 1 && fb < fb_max).then_some([b.x, b.y]))
        }).collect()
    }

    /// DIS: the grid topped up on the frame before, and its points followed into this one
    #[cfg(feature = "use-opencv")]
    fn dis_frame(&mut self, prev: Option<&[u8]>, pixels: &[u8], stride: usize) -> opencv::Result<Option<Vec<Option<[f32; 2]>>>> {
        let Some(prev) = prev else { return Ok(None) };
        let (w, h) = self.size;
        let prev_mat = frame_mat(prev, w as usize, w, h)?;
        self.replenish_grid(prev, &prev_mat)?;
        if self.points.is_empty() { return Ok(None); }
        let cur = frame_mat(pixels, stride, w, h)?;
        Ok(Some(Self::dense(self.dis.as_mut().unwrap(), &prev_mat, &cur, &self.points)?))
    }

    /// Where each point went, by the dense flow of DIS (Kroeger et al. 2016) from this frame to the next and back:
    /// None where the two don't agree at the point (`DIS_FB_MAX`). Slower than KLT, and it smooths the flow over its
    /// patches, but it has a flow everywhere, also where KLT loses its corners
    #[cfg(feature = "use-opencv")]
    fn dense(dis: &mut Ptr<video::DISOpticalFlow>, prev: &Mat, cur: &Mat, points: &[[f32; 2]]) -> opencv::Result<Vec<Option<[f32; 2]>>> {
        // Fresh outputs every time: DIS takes a flow passed in with the right size and type as a first guess
        let (mut forward, mut backward) = (Mat::default(), Mat::default());
        dis.calc(prev, cur, &mut forward)?;
        dis.calc(cur, prev, &mut backward)?;
        let (w, h) = (forward.cols(), forward.rows());
        let (f, b) = (forward.data_typed::<Vec2f>()?, backward.data_typed::<Vec2f>()?);
        // Bilinear, within the frame
        let sample = |flow: &[Vec2f], p: [f32; 2]| -> Option<[f32; 2]> {
            if !(p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= (w - 1) as f32 && p[1] <= (h - 1) as f32) { return None; }
            let (x0, y0) = (p[0].floor() as i32, p[1].floor() as i32);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (fx, fy) = (p[0] - x0 as f32, p[1] - y0 as f32);
            let at = |x: i32, y: i32| flow[(y * w + x) as usize];
            let (q00, q10, q01, q11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
            let lerp = |k: usize| (q00[k] * (1.0 - fx) + q10[k] * fx) * (1.0 - fy) + (q01[k] * (1.0 - fx) + q11[k] * fx) * fy;
            Some([p[0] + lerp(0), p[1] + lerp(1)])
        };
        Ok(points.iter().map(|&a| {
            let moved = sample(f, a)?;
            let back = sample(b, moved)?;
            (((back[0] - a[0]).powi(2) + (back[1] - a[1]).powi(2)).sqrt() < DIS_FB_MAX).then_some(moved)
        }).collect())
    }

    /// gmflow: the grid topped up on the frame before (DIS's, see `replenish_grid`), and its points followed into this one
    #[cfg(all(feature = "use-opencv", feature = "use-burn"))]
    fn gmflow_frame(&mut self, prev: Option<&[u8]>, pixels: &[u8], stride: usize) -> Result<Option<Vec<Option<[f32; 2]>>>, String> {
        let Some(prev) = prev else { return Ok(None) };
        let (w, h) = self.size;
        let prev_mat = frame_mat(prev, w as usize, w, h).map_err(cv)?;
        self.replenish_grid(prev, &prev_mat).map_err(cv)?;
        if self.points.is_empty() { return Ok(None); }
        if (self.stop)() { return Ok(Some(vec![None; self.points.len()])); }
        let cur = packed(pixels, stride, w as usize, h as usize);
        Ok(Some(Self::gmflow(prev, &cur, w as u32, h as u32, &self.points)?))
    }

    /// Where each point went, by gmflow's dense flow (Xu et al. 2022, see `optical_flow::GmflowDenseFlow`) from this
    /// frame to the next and back, like `dense`: None where the two don't agree at the point (`GMFLOW_FB_MAX`). One
    /// bidirectional pass of the model per frame (the CNN features of the previous frame are reused), on the GPU. An
    /// error when it can't run at all (no usable GPU, no model), which stops the analysis: every frame after it would
    /// fail the same way
    #[cfg(all(feature = "use-opencv", feature = "use-burn"))]
    fn gmflow(prev: &[u8], cur: &[u8], w: u32, h: u32, points: &[[f32; 2]]) -> Result<Vec<Option<[f32; 2]>>, String> {
        use crate::synchronization::optical_flow::GmflowDenseFlow;
        let gray = |p: &[u8]| image::GrayImage::from_raw(w, h, p.to_vec()).ok_or_else(|| "gmflow: frame size".to_string());
        let (a, b) = (gray(prev)?, gray(cur)?);
        let (forward, backward) = GmflowDenseFlow::compute_both(&a, &b).ok_or_else(|| "gmflow could not compute the flow, see the log".to_string())?;
        let inside = |p: [f32; 2]| p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= (w - 1) as f32 && p[1] <= (h - 1) as f32;
        Ok(points.iter().map(|&a| {
            let (dx, dy) = forward.at(a[0], a[1])?;
            let moved = [a[0] + dx, a[1] + dy];
            if !inside(moved) { return None; }
            let (bx, by) = backward.at(moved[0], moved[1])?;
            let back = [moved[0] + bx, moved[1] + by];
            (((back[0] - a[0]).powi(2) + (back[1] - a[1]).powi(2)).sqrt() < GMFLOW_FB_MAX).then_some(moved)
        }).collect())
    }

    /// Tops the tracks up with new corners, away from the ones already tracked: first each cell of a grid up to its
    /// share, then with whatever the cells without enough corners (clear sky) left, the strongest anywhere. All at once
    /// the strongest corners (buildings, boats, glints on water) took every track, and rows of cloudy sky - as good a
    /// reference for the rotation as anything, and the only one in their rows - got none: nothing measured them. A
    /// corner is one that stands out from the strongest in the frame, or in a cell with nothing that strong, from the
    /// noise of flat areas: clouds track as well as anything at a hundredth of a pixel back and forth, and flat sky
    /// still gets nothing.
    /// The corners are picked as good_features_to_track picks them (once per cell, then once over the frame), from the
    /// candidates `klt_frame` found when the frame came: local maxima above the threshold, outside the cell's outermost
    /// pixels, the strongest first, each at least the minimum distance from the others
    fn replenish(&mut self) {
        if self.points.len() >= self.max_points { return; }
        let (w, h) = self.size;
        let min_distance = (w.max(h) as f64 / 120.0).max(4.0);
        let mut free = vec![true; (w * h) as usize];
        for p in &self.points { clear_disc(&mut free, w, h, p[0] as f64, p[1] as f64, min_distance); }
        for &(x, y, r) in &self.glare { clear_disc(&mut free, w, h, x as f64, y as f64, r as f64); }
        let candidates = std::mem::take(&mut self.candidates);
        let xy = |k: u64| ((k & 0xffff) as i32, (k >> 16 & 0xffff) as i32);
        let strength = |k: u64| f32::from_bits((k >> 32) as u32) as f64;
        let open = |free: &[bool], k: u64| { let (x, y) = xy(k); free[(y * w + x) as usize] };
        let strongest = |free: &[bool], list: &[u64]| list.iter().find(|&&k| open(free, k)).map(|&k| strength(k));

        // A flat frame: black, a fade
        let Some(strongest_all) = strongest(&free, &candidates) else { return };
        let threshold = QUALITY * strongest_all;
        // The floor of what a corner is where nothing stronger is around: a few times the noise of flat areas
        let Some(noise) = noise_level(&self.noise) else { return };
        let floor = NOISE_MULTIPLE * noise;
        let share = self.max_points / (GRID_ROWS * GRID_COLS) as usize;
        let col_of: Vec<i32> = (0..w).map(|x| (0..GRID_COLS).rev().find(|c| w * c / GRID_COLS <= x).unwrap_or(0)).collect();
        let row_of: Vec<i32> = (0..h).map(|y| (0..GRID_ROWS).rev().find(|r| h * r / GRID_ROWS <= y).unwrap_or(0)).collect();
        let mut cells = vec![Vec::new(); (GRID_ROWS * GRID_COLS) as usize];
        for &k in &candidates {
            let (x, y) = xy(k);
            cells[(row_of[y as usize] * GRID_COLS + col_of[x as usize]) as usize].push(k);
        }
        for r in 0..GRID_ROWS {
            for c in 0..GRID_COLS {
                let (x0, y0, x1, y1) = (w * c / GRID_COLS, h * r / GRID_ROWS, w * (c + 1) / GRID_COLS, h * (r + 1) / GRID_ROWS);
                let has = self.points.iter().filter(|p| { let (x, y) = (p[0] as i32, p[1] as i32); x >= x0 && y >= y0 && x < x1 && y < y1 }).count();
                if has >= share { continue; }
                let list = &cells[(r * GRID_COLS + c) as usize];
                let Some(cell_strongest) = strongest(&free, list) else { continue };
                let cell_threshold = threshold.min(floor.max(QUALITY * cell_strongest));
                if cell_threshold <= 0.0 || cell_strongest < cell_threshold { continue; }
                let mut wanted = share - has;
                for &k in list {
                    if strength(k) <= cell_threshold { break; }
                    let (x, y) = xy(k);
                    if x == x0 || y == y0 || x == x1 - 1 || y == y1 - 1 || !open(&free, k) { continue; }
                    clear_disc(&mut free, w, h, x as f64, y as f64, min_distance);
                    self.add([x as f32, y as f32]);
                    wanted -= 1;
                    if wanted == 0 { break; }
                }
            }
        }
        if self.points.len() >= self.max_points { return; }
        let Some(now_strongest) = strongest(&free, &candidates) else { return };
        let threshold = QUALITY * now_strongest;
        for &k in &candidates {
            if strength(k) <= threshold || self.points.len() >= self.max_points { break; }
            if !open(&free, k) { continue; }
            let (x, y) = xy(k);
            clear_disc(&mut free, w, h, x as f64, y as f64, min_distance);
            self.add([x as f32, y as f32]);
        }
    }

    /// DIS's and gmflow's points: a regular grid, a new point in the middle of every cell (of about the size `max_points` of them fill
    /// the frame with) that has none left. The dense flow doesn't need a corner there, only something to see: where
    /// there's no detail at all (clear sky, clipped highlights) it only has what it smoothed in from around, so those
    /// cells stay empty - a point there would count as a measurement of rows nothing was seen in
    #[cfg(feature = "use-opencv")]
    fn replenish_grid(&mut self, pixels: &[u8], img: &Mat) -> opencv::Result<()> {
        let (w, h) = self.size;
        let spacing = (w as f64 * h as f64 / self.max_points.max(1) as f64).sqrt().max(4.0);
        let (cols, rows) = ((w as f64 / spacing).floor().max(1.0) as i32, (h as f64 / spacing).floor().max(1.0) as i32);
        let (sx, sy) = (w as f64 / cols as f64, h as f64 / rows as f64);
        let mut occupied = vec![false; (cols * rows) as usize];
        for p in &self.points {
            let (c, r) = ((p[0] as f64 / sx).floor() as i32, (p[1] as f64 / sy).floor() as i32);
            if c >= 0 && r >= 0 && c < cols && r < rows { occupied[(r * cols + c) as usize] = true; }
        }
        if occupied.iter().all(|o| *o) { return Ok(()); }
        let mut free = vec![true; (w * h) as usize];
        for (x, y, r) in glare(pixels, w as usize, w, h) { clear_disc(&mut free, w, h, x as f64, y as f64, r as f64); }
        let eig = corner_strengths(img)?;
        let Some(noise) = noise_level(&noise_samples(eig.data_typed::<f32>()?, w, h)) else { return Ok(()) };
        let floor = DIS_NOISE_MULTIPLE * noise;
        // Detail anywhere around the middle of the cell, not just at it
        let around = (spacing / 4.0).max(2.0) as i32;
        for r in 0..rows {
            for c in 0..cols {
                if occupied[(r * cols + c) as usize] { continue; }
                let (x, y) = (((c as f64 + 0.5) * sx) as i32, ((r as f64 + 0.5) * sy) as i32);
                if x < 3 || y < 3 || x >= w - 3 || y >= h - 3 || !free[(y * w + x) as usize] { continue; }
                let near = Rect::new((x - around).max(0), (y - around).max(0), (2 * around + 1).min(w - (x - around).max(0)), (2 * around + 1).min(h - (y - around).max(0)));
                let mut detail = 0.0;
                core::min_max_loc(&Mat::roi(&eig, near)?, None, Some(&mut detail), None, None, &core::no_array())?;
                if detail >= floor { self.add([x as f32, y as f32]); }
            }
        }
        Ok(())
    }

    fn add(&mut self, p: [f32; 2]) {
        self.points.push(p);
        self.ids.push(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
    }
}

#[cfg(feature = "use-opencv")]
pub(super) fn cv(e: opencv::Error) -> String { format!("OpenCV error: {e}") }

/// A frame's pixels as a Mat, borrowed: kept no longer than `pixels`
#[cfg(feature = "use-opencv")]
pub(super) fn frame_mat(pixels: &[u8], stride: usize, w: i32, h: i32) -> opencv::Result<Mat> {
    unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, pixels.as_ptr() as *mut std::ffi::c_void, stride) }
}

/// A frame's corner strengths, as `gpu_klt.wgsl` makes them: cornerMinEigenVal, Sobel 3, block 7
#[cfg(feature = "use-opencv")]
pub(super) fn corner_strengths(frame: &Mat) -> opencv::Result<Mat> {
    let mut eig = Mat::default();
    imgproc::corner_min_eigen_val(frame, &mut eig, 7, 3, core::BORDER_DEFAULT)?;
    Ok(eig)
}

/// A frame's pixels without the padding of its rows
fn packed(pixels: &[u8], stride: usize, w: usize, h: usize) -> Vec<u8> {
    if stride == w { return pixels[..w * h].to_vec(); }
    (0..h).flat_map(|y| &pixels[y * stride..y * stride + w]).copied().collect()
}

/// Where the GPU's KLT took each point, with `klt`'s forward-backward check
pub(super) fn checked(points: &[[f32; 2]], fwd: &[[f32; 4]], bwd: &[[f32; 4]], fb_max: f32) -> Vec<Option<[f32; 2]>> {
    points.iter().zip(fwd.iter().zip(bwd)).map(|(a, (f, b))| {
        let fb = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        (f[2] > 0.5 && b[2] > 0.5 && fb < fb_max).then_some([f[0], f[1]])
    }).collect()
}

/// Where new corners stay away from the sun and the like: centre and radius, see `GLARE_RADII`. Only compact blobs,
/// not too large: the edges of an overexposed white wall are as good as any. The blobs are 8-connected components of
/// the saturated pixels (as OpenCV's connectedComponentsWithStats finds them), joined from the runs of them in each
/// row. Most frames have too few of those for even one blob, and cost a pass that counts them a row at a time
/// (vectorized: about 0.1 ms at 960x540); only the rows with any are looked at again. On one thread: it runs while the
/// GPU tracks, see `klt_frame`, and rayon was what made the tracking slow with every core busy (`local_maxima`)
fn glare(pixels: &[u8], stride: usize, w: i32, h: i32) -> Vec<(i32, i32, i32)> {
    let (wu, hu) = (w as usize, h as usize);
    let (wf, hf) = (w as f64, h as f64);
    let min_area = GLARE_MIN_AREA * (wf / 960.0).powi(2);
    let row = |y: usize| &pixels[y * stride..y * stride + wu];
    let counts: Vec<u32> = (0..hu).map(|y| row(y).iter().map(|&p| (p > GLARE_LEVEL) as u32).sum()).collect();
    if (counts.iter().sum::<u32>() as f64) < min_area { return Vec::new(); }

    // The runs (y, from x, to x past the run), each with the blob it's in: the run it's joined to, first in the frame
    let mut runs: Vec<(u32, u32, u32)> = Vec::new();
    let mut parent: Vec<u32> = Vec::new();
    fn root(parent: &mut [u32], mut i: u32) -> u32 {
        while parent[i as usize] != i {
            parent[i as usize] = parent[parent[i as usize] as usize];
            i = parent[i as usize];
        }
        i
    }
    let mut above = 0..0;
    for y in 0..hu {
        let start = runs.len();
        if counts[y] > 0 {
            let r = row(y);
            let mut x = 0;
            while let Some(skip) = r[x..].iter().position(|&p| p > GLARE_LEVEL) {
                let x0 = x + skip;
                x = x0 + r[x0..].iter().position(|&p| p <= GLARE_LEVEL).unwrap_or(wu - x0);
                let i = runs.len() as u32;
                runs.push((y as u32, x0 as u32, x as u32));
                parent.push(i);
                // 8-connected to the runs above that come within a pixel of it
                for j in above.clone() {
                    let (_, a0, a1) = runs[j];
                    if a0 <= x as u32 && x0 as u32 <= a1 {
                        let (ri, rj) = (root(&mut parent, i), root(&mut parent, j as u32));
                        if ri != rj { parent[ri.max(rj) as usize] = ri.min(rj); }
                    }
                }
                if x >= wu { break; }
            }
        }
        above = start..runs.len();
    }
    // Each blob's area and bounding box (the right and the bottom edge past it)
    let mut blobs = vec![(0u64, u32::MAX, u32::MAX, 0u32, 0u32); runs.len()];
    for i in 0..runs.len() {
        let (y, x0, x1) = runs[i];
        let b = &mut blobs[root(&mut parent, i as u32) as usize];
        *b = (b.0 + (x1 - x0) as u64, b.1.min(x0), b.2.min(y), b.3.max(x1), b.4.max(y + 1));
    }
    blobs.into_iter().filter(|b| b.0 > 0).filter_map(|(area, bx, by, x_end, y_end)| {
        let (area, bx, by, bw, bh) = (area as f64, bx as f64, by as f64, (x_end - bx) as f64, (y_end - by) as f64);
        // The frame's edge may cut it (a half disc is twice as wide as tall): its size is the larger side then
        let cut = bx <= 0.0 || by <= 0.0 || bx + bw >= wf || by + bh >= hf;
        let compact = area >= 0.5 * bw * bh && bw.max(bh) <= if cut { 3.0 } else { 2.0 } * bw.min(bh);
        let radius = bw.max(bh) / 2.0;
        (area >= min_area && compact && radius <= wf / 12.0).then(|| ((bx + bw / 2.0) as i32, (by + bh / 2.0) as i32, (GLARE_RADII * radius) as i32))
    }).collect()
}

/// The candidate corners in a frame's corner strengths (cornerMinEigenVal's, `w` x `h`): the pixels inside the outermost
/// ones whose strength is above zero and none of its 8 neighbours', the strongest first, as `bits of the strength << 32
/// | y << 16 | x` - the strengths are positive, so their bits sort as they do, and a tie goes to the later pixel, as in
/// good_features_to_track. `gpu_klt.wgsl`'s `corners` finds the same.
/// On one thread, as the sort of the GPU's: with every core busy (decoding, the analysis' own fits) rayon's joins wait on
/// threads descheduled elsewhere, which took the GPU's tracking from 13 to 27 ms a frame
pub(super) fn local_maxima(eig: &[f32], w: i32, h: i32) -> Vec<u64> {
    let (w, h) = (w as usize, h as usize);
    let mut out = Vec::new();
    for y in 1..h.saturating_sub(1) {
        let (up, row, down) = (&eig[(y - 1) * w..y * w], &eig[y * w..(y + 1) * w], &eig[(y + 1) * w..(y + 2) * w]);
        for x in 1..w.saturating_sub(1) {
            let v = row[x];
            if v > 0.0 && row[x - 1] <= v && row[x + 1] <= v && up[x - 1..=x + 1].iter().all(|&n| n <= v) && down[x - 1..=x + 1].iter().all(|&n| n <= v) {
                out.push((v.to_bits() as u64) << 32 | (y as u64) << 16 | x as u64);
            }
        }
    }
    out.sort_unstable_by(|a, b| b.cmp(a));
    out
}

/// The corner strengths of every 8th pixel in both directions, the noise of flat areas is found in
#[cfg(feature = "use-opencv")]
fn noise_samples(eig: &[f32], w: i32, h: i32) -> Vec<f32> {
    (0..h as usize).step_by(8).flat_map(|y| (0..w as usize).step_by(8).map(move |x| eig[y * w as usize + x])).collect()
}

/// The noise of flat areas in a frame's corner strengths (`noise_samples`): their 10th percentile - sky, water - and
/// higher still in a frame without any. Areas with none at all (clipped highlights, black bars) aren't noise: a
/// blown-out sky took it to 0, and with it every floor made of it. None for a frame with no detail at all
fn noise_level(samples: &[f32]) -> Option<f64> {
    let mut v: Vec<f32> = samples.iter().copied().filter(|&e| e > 0.0).collect();
    if v.is_empty() { return None; }
    let i = v.len() / 10;
    Some(*v.select_nth_unstable_by(i, |a, b| a.total_cmp(b)).1 as f64)
}

/// Takes a disc out of `free` (`w` x `h`): new corners stay out of it. The pixels closer than `r` to the centre, as
/// good_features_to_track keeps corners apart: one exactly the minimum distance away is fine
pub(super) fn clear_disc(free: &mut [bool], w: i32, h: i32, cx: f64, cy: f64, r: f64) {
    for y in ((cy - r).floor() as i32 + 1).max(0)..=((cy + r).ceil() as i32 - 1).min(h - 1) {
        let half = (r * r - (y as f64 - cy).powi(2)).sqrt();
        let (x0, x1) = (((cx - half).floor() as i32 + 1).max(0), ((cx + half).ceil() as i32 - 1).min(w - 1));
        if x0 <= x1 { free[(y * w + x0) as usize..=(y * w + x1) as usize].fill(false); }
    }
}

#[cfg(all(test, feature = "use-opencv"))]
pub(super) mod tests {
    use super::*;

    pub(crate) const W: i32 = 960;
    pub(crate) const H: i32 = 540;

    /// Detail like footage has: noise blurred into blobs of a few pixels
    pub(crate) fn texture() -> Mat {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let noise: Vec<u8> = (0..W * H).map(|_| { state ^= state << 13; state ^= state >> 7; state ^= state << 17; (state >> 56) as u8 }).collect();
        let noise = Mat::new_rows_cols_with_data(H, W, &noise).unwrap().try_clone().unwrap();
        let (mut blurred, mut out) = (Mat::default(), Mat::default());
        imgproc::gaussian_blur_def(&noise, &mut blurred, Size::new(0, 0), 2.0).unwrap();
        core::normalize(&blurred, &mut out, 0.0, 255.0, core::NORM_MINMAX, -1, &core::no_array()).unwrap();
        out
    }

    /// The frame turned by `angle` degrees around its middle and moved by `shift`, and that affine transform
    pub(crate) fn moved(frame: &Mat, angle: f64, shift: (f64, f64)) -> (Mat, [f64; 6]) {
        let mut m = imgproc::get_rotation_matrix_2d(Point2f::new(W as f32 / 2.0, H as f32 / 2.0), angle, 1.0).unwrap();
        *m.at_2d_mut::<f64>(0, 2).unwrap() += shift.0;
        *m.at_2d_mut::<f64>(1, 2).unwrap() += shift.1;
        let mut out = Mat::default();
        imgproc::warp_affine(frame, &mut out, &m, Size::new(W, H), imgproc::INTER_LINEAR, core::BORDER_REFLECT_101, Default::default()).unwrap();
        (out, std::array::from_fn(|i| *m.at_2d::<f64>(i as i32 / 3, i as i32 % 3).unwrap()))
    }

    pub(crate) fn apply(m: &[f64; 6], p: (f64, f64)) -> (f64, f64) { (m[0] * p.0 + m[1] * p.1 + m[2], m[3] * p.0 + m[4] * p.1 + m[5]) }

    pub(crate) fn invert(m: &[f64; 6]) -> [f64; 6] {
        let d = m[0] * m[4] - m[1] * m[3];
        let (a, b, c, e) = (m[4] / d, -m[1] / d, -m[3] / d, m[0] / d);
        [a, b, -(a * m[2] + b * m[5]), c, e, -(c * m[2] + e * m[5])]
    }

    /// Exactly the pixels closer than the radius, also of a radius that isn't whole, a centre between pixels and a disc
    /// the frame's edge cuts
    #[test]
    fn clear_disc_keeps_the_minimum_distance() {
        let (w, h) = (40, 30);
        for (cx, cy, r) in [(20.0, 15.0, 8.0), (20.0, 15.0, 14.216667), (3.4, 27.8, 5.5), (0.0, 0.0, 4.0)] {
            let mut free = vec![true; (w * h) as usize];
            clear_disc(&mut free, w, h, cx, cy, r);
            for y in 0..h {
                for x in 0..w {
                    let closer = (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2) < r * r;
                    assert_eq!(free[(y * w + x) as usize], !closer, "({x}, {y}) by a disc of {r} at ({cx}, {cy})");
                }
            }
        }
    }

    pub(crate) fn gpu() -> Option<wgpu::Adapter> {
        let adapter = crate::gpu::wgpu::WgpuWrapper::compute_adapter(None);
        if adapter.is_none() { eprintln!("no GPU, nothing to test"); }
        adapter
    }

    pub(crate) fn median(mut v: Vec<f32>) -> f32 { v.sort_by(f32::total_cmp); v[v.len() / 2] }

    #[test]
    fn gpu_finds_and_tracks_as_opencv() {
        let Some(adapter) = gpu() else { return };
        let a = texture();
        let (b, m) = moved(&a, 0.3, (2.3, -1.7));
        let truth = |p: [f32; 2]| { let t = apply(&m, (p[0] as f64, p[1] as f64)); [t.0 as f32, t.1 as f32] };
        let mut gpu = GpuKlt::new(&adapter, W as u32, H as u32, W as u32, 2000).unwrap();

        // The candidate corners: the GPU's against those in OpenCV's corner strengths
        gpu.submit(a.data_bytes().unwrap(), std::iter::empty()).unwrap();
        gpu.finish().unwrap();
        let mut found = gpu.candidates.clone();
        found.sort_unstable_by(|a, b| b.cmp(a));
        let mut eig = Mat::default();
        imgproc::corner_min_eigen_val(&a, &mut eig, 7, 3, core::BORDER_DEFAULT).unwrap();
        let eig = eig.data_typed::<f32>().unwrap();
        let expected = local_maxima(eig, W, H);
        assert!(found.len().abs_diff(expected.len()) * 200 < expected.len(), "{} candidates, OpenCV's strengths have {}", found.len(), expected.len());
        let near_top: std::collections::HashSet<u64> = expected[..1200].iter().map(|k| k & 0xffff_ffff).collect();
        let same = found[..1000].iter().filter(|k| near_top.contains(&(*k & 0xffff_ffff))).count();
        assert!(same >= 995, "{same} of the 1000 strongest candidates are OpenCV's");
        for &k in &found[..1000] {
            let (x, y) = ((k & 0xffff) as i32, (k >> 16 & 0xffff) as i32);
            let (v, e) = (f32::from_bits((k >> 32) as u32), eig[(y * W + x) as usize]);
            assert!((v - e).abs() <= 1e-4 * e, "strength {v} at {x},{y}, OpenCV's {e}");
        }
        let noise = noise_samples(eig, W, H);
        assert_eq!(gpu.noise.len(), noise.len());
        assert!((noise_level(&gpu.noise).unwrap() / noise_level(&noise).unwrap() - 1.0).abs() < 1e-3);

        // Tracking: the GPU's and OpenCV's KLT from the same points
        let points: Vec<[f32; 2]> = expected[..1500].iter().map(|k| [(k & 0xffff) as f32, (k >> 16 & 0xffff) as f32]).collect();
        gpu.submit(b.data_bytes().unwrap(), points.iter().copied()).unwrap();
        gpu.finish().unwrap();
        let on_gpu = checked(&points, &gpu.fwd, &gpu.bwd, FB_MAX);
        let on_cpu = Tracker::klt(&a, &b, &points, FB_MAX).unwrap();
        let both: Vec<([f32; 2], [f32; 2], [f32; 2])> = points.iter().zip(on_gpu.iter().zip(&on_cpu)).filter_map(|(p, (g, c))| Some((truth(*p), (*g)?, (*c)?))).collect();
        let kept = (on_gpu.iter().filter(|p| p.is_some()).count(), on_cpu.iter().filter(|p| p.is_some()).count());
        assert!(both.len() * 100 >= kept.0.max(kept.1) * 99, "kept by both {} of {kept:?}", both.len());
        let dist = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        let apart: Vec<f32> = both.iter().map(|(_, g, c)| dist(*g, *c)).collect();
        let off: Vec<f32> = both.iter().map(|(t, g, _)| dist(*t, *g)).collect();
        eprintln!("{} candidates ({} in OpenCV's strengths), {same} of the strongest 1000 the same; kept {kept:?}, by both {}, apart median {} px, max {}; off the true motion median {} px",
            found.len(), expected.len(), both.len(), median(apart.clone()), apart.iter().fold(0.0f32, |a, b| a.max(*b)), median(off.clone()));
        assert!(median(apart.clone()) < 0.001 && apart.iter().all(|d| *d < 0.05), "GPU vs OpenCV: median {} px, max {}", median(apart.clone()), apart.iter().fold(0.0f32, |a, b| a.max(*b)));
        assert!(median(off.clone()) < 0.02, "GPU vs the true motion: median {} px", median(off));
    }

    /// The whole tracker over a few frames, on the GPU and on the CPU: as many tracks, as close to the true motion
    #[test]
    fn tracker_on_gpu_as_on_cpu() {
        if gpu().is_none() { return; }
        let first = texture();
        let frames: Vec<(Mat, [f64; 6])> = (0..6).map(|i| moved(&first, 0.15 * i as f64, (1.1 * i as f64, -0.7 * i as f64))).collect();
        let run = |device: KltDevice| {
            let mut tracker = Tracker::new(1500, TrackingMethod::Klt, device).unwrap();
            let mut off = Vec::new();
            for (i, (frame, m)) in frames.iter().enumerate() {
                for o in tracker.track(W as u32, H as u32, W as usize, frame.data_bytes().unwrap()).unwrap() {
                    let truth = apply(m, apply(&invert(&frames[i - 1].1), (o.a[0] as f64, o.a[1] as f64)));
                    off.push((truth.0 - o.b[0] as f64).hypot(truth.1 - o.b[1] as f64) as f32);
                }
            }
            (off, tracker.describe())
        };
        let (cpu, gpu) = (run(KltDevice::Cpu), run(KltDevice::Gpu(None)));
        assert!(!gpu.1.contains("CPU"), "{}", gpu.1);
        assert!(cpu.0.len().abs_diff(gpu.0.len()) * 50 < cpu.0.len(), "observations: CPU {}, GPU {}", cpu.0.len(), gpu.0.len());
        eprintln!("observations: CPU {}, GPU {}; {}", cpu.0.len(), gpu.0.len(), gpu.1);
        let (c, g) = (median(cpu.0), median(gpu.0));
        eprintln!("off the true motion: median {c} px on the CPU, {g} on the GPU");
        assert!(g < 0.02 && (g - c).abs() < 0.002, "off the true motion: median {c} px on the CPU, {g} on the GPU");
    }

    /// What `glare` replaced: OpenCV's threshold and connected components, with the same choice of blobs
    fn glare_by_opencv(img: &Mat) -> Vec<(i32, i32, i32)> {
        let (w, h) = (W as f64, H as f64);
        let min_area = GLARE_MIN_AREA * (w / 960.0).powi(2);
        let mut bright = Mat::default();
        imgproc::threshold(img, &mut bright, GLARE_LEVEL as f64, 255.0, imgproc::THRESH_BINARY).unwrap();
        let (mut labels, mut stats, mut centroids) = (Mat::default(), Mat::default(), Mat::default());
        let n = imgproc::connected_components_with_stats(&bright, &mut labels, &mut stats, &mut centroids, 8, core::CV_32S).unwrap();
        (1..n).filter_map(|i| {
            let at = |c: i32| *stats.at_2d::<i32>(i, c).unwrap() as f64;
            let (area, bx, by, bw, bh) = (at(imgproc::CC_STAT_AREA), at(imgproc::CC_STAT_LEFT), at(imgproc::CC_STAT_TOP), at(imgproc::CC_STAT_WIDTH), at(imgproc::CC_STAT_HEIGHT));
            let cut = bx <= 0.0 || by <= 0.0 || bx + bw >= w || by + bh >= h;
            let compact = area >= 0.5 * bw * bh && bw.max(bh) <= if cut { 3.0 } else { 2.0 } * bw.min(bh);
            let radius = bw.max(bh) / 2.0;
            (area >= min_area && compact && radius <= w / 12.0).then(|| ((bx + bw / 2.0) as i32, (by + bh / 2.0) as i32, (GLARE_RADII * radius) as i32))
        }).collect()
    }

    #[test]
    fn glare_as_opencv() {
        let frame = texture();
        let mut plain = Mat::default();
        // Detail below saturation: nothing bright but what's drawn
        frame.convert_to(&mut plain, -1, 240.0 / 255.0, 0.0).unwrap();
        let disc = |img: &mut Mat, x: i32, y: i32, r: i32| imgproc::circle(img, core::Point::new(x, y), r, core::Scalar::all(255.0), -1, imgproc::LINE_8, 0).unwrap();
        let none = plain.try_clone().unwrap();
        let mut img = plain;
        for (x, y, r) in [(200, 150, 10), (0, 300, 12), (959, 539, 9), (500, 100, 120), (650, 420, 4)] { disc(&mut img, x, y, r); }
        imgproc::rectangle(&mut img, core::Rect::new(300, 400, 120, 3), core::Scalar::all(255.0), -1, imgproc::LINE_8, 0).unwrap();
        // Two squares meeting at a corner: one blob, 8-connected; and glints, too small to count
        imgproc::rectangle(&mut img, core::Rect::new(800, 100, 6, 6), core::Scalar::all(255.0), -1, imgproc::LINE_8, 0).unwrap();
        imgproc::rectangle(&mut img, core::Rect::new(806, 106, 6, 6), core::Scalar::all(255.0), -1, imgproc::LINE_8, 0).unwrap();
        let mut state = 7u64;
        for _ in 0..3000 {
            state ^= state << 13; state ^= state >> 7; state ^= state << 17;
            let (x, y) = ((state % W as u64) as i32, ((state >> 20) % (H as u64 / 3) + H as u64 * 2 / 3) as i32);
            *img.at_2d_mut::<u8>(y, x).unwrap() = 255;
        }

        for (name, f) in [("no glare", &none), ("glare and glints", &img)] {
            let px = f.data_bytes().unwrap();
            let expected = glare_by_opencv(f);
            let found = glare(px, W as usize, W, H);
            let t = Instant::now();
            for _ in 0..100 { std::hint::black_box(glare(px, W as usize, W, H)); }
            let ours = t.elapsed().as_secs_f64() * 10.0;
            let t = Instant::now();
            for _ in 0..100 { std::hint::black_box(glare_by_opencv(f)); }
            let theirs = t.elapsed().as_secs_f64() * 10.0;
            eprintln!("{name}: {} blobs, {ours:.3} ms (OpenCV's {theirs:.3} ms)", found.len());
            assert_eq!(found, expected, "{name}");
        }
        assert_eq!(glare(none.data_bytes().unwrap(), W as usize, W, H), Vec::new());
    }
}

#[cfg(all(test, feature = "use-opencv", feature = "use-burn"))]
mod gmflow_tests {
    use super::*;

    /// A textured frame and the same one moved by (dx, dy) whole pixels, 960x540 like the analysis decodes
    fn shifted_pair(dx: usize, dy: usize) -> (Vec<u8>, Vec<u8>, u32, u32) {
        let (w, h) = (960usize, 540usize);
        let tex = |x: usize, y: usize| -> u8 {
            let (x, y) = (x as f32, y as f32);
            let v = 128.0 + 50.0 * (x * 0.071).sin() * (y * 0.053).cos() + 40.0 * ((x * 0.013 + y * 0.029).sin() * 3.0).sin()
                + 25.0 * (((x as i32 * 13 + y as i32 * 7) & 0x3f) as f32 / 63.0 - 0.5);
            v.clamp(0.0, 255.0) as u8
        };
        let a: Vec<u8> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| tex(x + dx, y + dy)).collect();
        let b: Vec<u8> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| tex(x, y)).collect();
        (a, b, w as u32, h as u32)
    }

    // Downloads the model and takes a pass of it: run with --ignored
    #[test]
    #[ignore]
    fn gmflow_follows_a_known_shift() {
        let (a, b, w, h) = shifted_pair(3, 1);
        let mut t = Tracker::new(1500, TrackingMethod::Gmflow, KltDevice::Cpu).unwrap();
        assert!(t.track(w, h, w as usize, &a).unwrap().is_empty());
        let obs = t.track(w, h, w as usize, &b).unwrap();
        assert!(obs.len() > 100, "gmflow: only {} points tracked", obs.len());
        let median = |k: usize| { let mut v: Vec<f32> = obs.iter().map(|o| o.b[k] - o.a[k]).collect(); v.sort_by(f32::total_cmp); v[v.len() / 2] };
        let (mx, my) = (median(0), median(1));
        println!("gmflow: {} points, median motion ({mx:.3}, {my:.3}), expected (3, 1)", obs.len());
        assert!((mx - 3.0).abs() < 0.2 && (my - 1.0).abs() < 0.2);
    }
}
