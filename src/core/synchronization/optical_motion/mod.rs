// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! "Analyze image optically": measures, from the video itself, the rotation the motion data got wrong, and fits the
//! correction the quaternions get (`gyro_source::OpticalCorrection`).
//!
//! Every frame of the clip is tracked (KLT, persistent tracks). For each tracked point the quaternions predict where
//! it should be in the next frame - rolling shutter included, each point at its own row's time - and the residual is
//! what they got wrong, plus the parallax of the camera's translation. Parallax changes slowly along a track while
//! the errors this is about don't, so a temporal high-pass of each track's residuals leaves the rotation error alone,
//! with no depth or translation to estimate. The points of each band of rows of each frame pair then fit one small
//! rotation, and `solver` turns those into a spline δ(t), rolling shutter resolution and all.
//!
//! The quaternions stay in charge of the slow motion (the image's own estimate of it would drift, and mixes rotation
//! up with translation); the image corrects them where it disagrees, which on a camera with good motion data is
//! nowhere

pub mod solver;
mod odometry;
#[cfg(feature = "use-opencv")]
mod tracker;

use std::collections::{ HashMap, VecDeque };
use std::sync::{ Arc, atomic::{ AtomicBool, AtomicU64, Ordering::{ Relaxed, SeqCst } } };
use nalgebra::{ DMatrix, Matrix3, Rotation3, UnitQuaternion, Vector3 };
use rayon::prelude::*;

use crate::StabilizationManager;
use crate::gyro_source::{ GyroSource, OpticalCorrection, OpticalCorrectionSettings, OpticalResidualCorrection, OPTICAL_GRID, TimeQuat, optical_correction::{ self, Fnv } };
use crate::stabilization::{ ComputeParams, FrameTransform, undistort_points_for_optical_motion };
use solver::{ BandMeasurement, SolverParams };

/// Tracks kept alive per frame
#[cfg(feature = "use-opencv")]
const MAX_POINTS: usize = 1500;
/// Bands of rows (along the readout) each frame pair is measured in
const BANDS: usize = 6;
/// Temporal high-pass of the track residuals: window of the local quadratic fit taken out, in frame pairs.
/// Tracks shorter than the minimum don't contribute
const HP_MAX: usize = 61;
const HP_MIN: usize = 15;
/// Frame pairs measured at once
const CHUNK: usize = 240;
const MIN_BAND_POINTS: usize = 25;
/// Floor of the uncertainty of one band's rotation, in pixels of the tracked frame
const SIGMA_FLOOR_PX: f64 = 0.01;
/// Local residual path smoothing. Kept in seconds so its meaning is independent of frame rate.
const LOCAL_SMOOTH_SECONDS: f64 = 0.35;

#[derive(Clone, Copy, Debug)]
pub struct Observation {
    pub id: u32,
    pub a: [f32; 2],
    pub b: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    index: usize,
    timestamp_ms: f64,
    /// Time of the first tracked row (or column), and per tracked pixel along the readout: `FrameTransform::at_timestamp`
    start_ms: f64,
    per_px_ms: f64,
    /// Middle of the readout: the frame's own time
    #[cfg_attr(not(feature = "use-opencv"), allow(dead_code))]
    mid_ms: f64,
}

struct Pair {
    seq: usize,
    a: Frame,
    b: Frame,
    obs: Vec<Observation>,
}

/// One tracked point in one frame pair, against the uncorrected quaternions
struct Derived {
    id: u32,
    seq: usize,
    band: u8,
    frame: usize,
    /// Tracked position in frame b, normalized to the analyzed frame.
    uv: [f32; 2],
    /// Local derivatives of frame-b's undistorted bearing with respect to one tracked pixel in x/y.
    /// These carry the lens, mesh, IBIS/OIS and breathing geometry into the residual mesh instead of
    /// approximating every lens by one focal length.
    jx: Vector3<f64>,
    jy: Vector3<f64>,
    /// Where the quaternions put the point in the second frame, and how far off that was (quaternion frame)
    p: Vector3<f64>,
    r: Vector3<f64>,
    ta_ms: f64,
    tb_ms: f64,
}

#[derive(Clone)]
struct LocalStep {
    /// The a-to-b increment belongs to b. Keeping this explicit prevents one-frame lag.
    frame: usize,
    grid: Vec<[f32; 2]>,
}

/// What the image measured: kept (in memory) so a change of the settings refits the correction in a fraction of a
/// second instead of another pass over the video
pub struct OpticalMeasurements {
    pub bands: Vec<BandMeasurement>,
    pub scaled_fps: f64,
    /// Of the quaternions they were measured against, see `OpticalCorrection::quats_checksum`
    pub quats_checksum: u64,
    /// And of the rest they were measured with, see `context_checksum`
    pub context_checksum: u64,
    /// For a file without motion data, the orientation measured against, see `OpticalCorrection::video_base`
    pub video_base: Vec<(i64, [f32; 4])>,
    /// Local motion left after the robust global camera rotation, stabilized on a compact spatial grid.
    pub residual: OpticalResidualCorrection,
    pub frames: usize,
    pub measured_frames: usize,
    /// `StabilizationManager::optical_generation` when the analysis started: they're only for what was loaded then
    pub generation: u64,
}

/// The solver's parameters for the user's strength: it moves the hand-over to the motion data (the ridge, relative
/// to what the image measured) over three decades, from only the fast errors of a vibrating gyro up to the image
/// overriding the motion data down to about a tenth of a Hz. The correction always works per row: both kinds of damage
/// seen (vibration, a gyro whose gain collapses during a roll) change within a frame, and one correction per frame did
/// worse on every clip tried
pub fn solver_params(settings: &OpticalCorrectionSettings, scaled_fps: f64) -> SolverParams {
    SolverParams {
        spacing_us: 1_000_000.0 / scaled_fps.max(1.0) / 6.0,
        ridge: 10f64.powf(-2.0 - 3.0 * settings.strength.clamp(0.0, 1.0)),
        ..Default::default()
    }
}

/// Fits the correction to the measurements
pub fn solve(m: &OpticalMeasurements, settings: &OpticalCorrectionSettings) -> Result<OpticalCorrection, String> {
    solve_with(m, settings, &solver_params(settings, m.scaled_fps))
}

pub fn solve_with(m: &OpticalMeasurements, settings: &OpticalCorrectionSettings, params: &SolverParams) -> Result<OpticalCorrection, String> {
    let bands = &m.bands;
    let sol = solver::solve(bands, params).ok_or_else(|| "Not enough of the image could be tracked".to_string())?;
    let rms_deg = (bands.iter().map(|b| sol.at(b.ta_us, params.spacing_us).norm_squared()).sum::<f64>() / bands.len().max(1) as f64).sqrt().to_degrees();
    Ok(OpticalCorrection {
        enabled: true,
        settings: *settings,
        start_us: sol.start_us,
        spacing_us: params.spacing_us,
        coeffs: sol.coeffs.iter().map(|c| [c.x as f32, c.y as f32, c.z as f32]).collect(),
        quats_checksum: m.quats_checksum,
        context_checksum: m.context_checksum,
        video_base: m.video_base.clone(),
        residual: m.residual.clone(),
        frames: m.frames,
        measured_frames: m.measured_frames,
        rms_deg,
    })
}

/// The parameters the analysis measures with: the render's, minus what's about the output picture (keyframes, a lens
/// correction below 100%), and with the frames the right way up. The analysis gets them from the decoder, whatever way
/// up the preview's framebuffer is (OpenGL's is upside down, `framebuffer_inverted`), and the point undistortion never
/// reads that flag - left on, it flipped the rolling shutter and the axes against pictures that weren't flipped
pub fn measurement_params(stab: &StabilizationManager) -> ComputeParams {
    let mut params = ComputeParams::from_manager(stab);
    params.keyframes.clear();
    params.lens_correction_amount = 1.0;
    params.framebuffer_inverted = false;
    params
}

/// Fingerprint of what a measurement depends on besides the quaternions (`OpticalCorrection::context_checksum`): when
/// each tracked pixel was read out - the frame timing, the rolling shutter, the sync - and which ray it came from - the
/// lens, with all of its per-frame data (`undistort_points_for_optical_motion`). Taken from what these give at a few
/// frames rather than from the settings behind them, so every setting that moves a pixel's time or ray is in it,
/// including ones this doesn't know of, and none that doesn't is. Rounded far below anything that matters (1 µs,
/// 1 µrad), so the last bit of a computation elsewhere doesn't make a correction stale. The sync points go in as they
/// are: one anywhere in the clip moves the frames around it. Takes the `gyro` lock: not for a caller that holds it
pub fn context_checksum(params: &ComputeParams) -> u64 {
    let mut h = Fnv::default();
    h.eat(params.width as u64);
    h.eat(params.height as u64);
    h.eat_rounded(params.scaled_fps, 1e6);
    for (ts, offset) in params.gyro.read().get_offsets() {
        h.eat(*ts as u64);
        h.eat_rounded(*offset, 1e3);
    }
    let size = (params.width as u32, params.height as u32);
    let rows = if params.frame_readout_direction.is_horizontal() { size.0 } else { size.1 };
    let at = [0.1f32, 0.5, 0.9];
    let grid: Vec<(f32, f32)> = at.iter().flat_map(|y| at.map(|x| (x * size.0 as f32, y * size.1 as f32))).collect();
    let fps = params.scaled_fps.max(1e-9);
    for t in at {
        let index = crate::frame_at_timestamp(t as f64 * params.scaled_duration_ms, fps).max(0) as usize;
        let ts = crate::timestamp_at_frame(index as i32, fps);
        let frame = frame_timing(params, index, ts, rows);
        h.eat(index as u64);
        h.eat_rounded(frame.start_ms, 1e3);
        h.eat_rounded(frame.per_px_ms * rows as f64, 1e3);
        for b in undistort_points_for_optical_motion(&grid, ts, index, params, size) {
            match b {
                Some(b) => for v in [b.0, b.1, b.2] { h.eat_rounded(v as f64, 1e6); },
                None => h.eat(u64::MAX),
            }
        }
    }
    h.0
}

/// When the frame `index` is read out, over `rows` rows (or columns) of the tracked picture: the rolling shutter timing
/// of `FrameTransform::at_timestamp`
fn frame_timing(params: &ComputeParams, index: usize, timestamp_ms: f64, rows: u32) -> Frame {
    let gyro = params.gyro.read();
    let md = gyro.file_metadata.read();
    let readout = FrameTransform::get_frame_readout_time(params, false, timestamp_ms, &md);
    let ts = timestamp_ms + md.per_frame_time_offsets.get(index).unwrap_or(&0.0);
    Frame { index, timestamp_ms, start_ms: ts - readout / 2.0, per_px_ms: readout / rows.max(1) as f64, mid_ms: ts }
}

/// A bearing from the camera's frame into the quaternions' frame: the axis flips `FrameTransform::at_timestamp` applies
/// (to upright frames, see `measurement_params`)
fn to_quat_frame(b: (f32, f32, f32)) -> Vector3<f64> {
    Vector3::new(b.0 as f64, -b.1 as f64, -b.2 as f64).normalize()
}

pub struct OpticalMotionAnalysis {
    /// Its `gyro` is a copy of the motion data without any optical correction: that's what the new one is measured against
    params: ComputeParams,
    fps_scale: Option<f64>,
    scaled_fps: f64,
    quats_checksum: u64,
    context_checksum: u64,
    horizontal_readout: bool,
    track_size: (u32, u32),
    focal_px: f64,
    #[cfg(feature = "use-opencv")]
    tracker: tracker::KltTracker,
    last: Option<Frame>,
    pairs: VecDeque<Pair>,
    next_seq: usize,
    measured_upto: usize,
    measurements: Vec<BandMeasurement>,
    local_steps: Vec<LocalStep>,
    measured_pairs: usize,
    frames: usize,
    total_frames: usize,
    /// The part of the clip analyzed: the trim ranges, in the file's own milliseconds
    ranges_ms: Vec<(f64, f64)>,
    /// A file without motion data: the rotation between each two frames, measured from their tracks and chained, at the
    /// frames' own times. What the rest of the analysis compares the image against instead of the quaternions
    vision: Option<TimeQuat>,
    /// What measures those rotations: the translation and the tracks' depths it keeps from one frame pair to the next
    odometry: odometry::VisualOdometry,
    sg_cache: HashMap<usize, DMatrix<f64>>,
    cancel_flag: Arc<AtomicBool>,
    /// `StabilizationManager::optical_generation`, and its value when this started: another file, a project or Clear
    /// move it on, and cancel what's running for the one before
    generation: (Arc<AtomicU64>, u64),
    /// Latched: the cancel flag is shared, and something else may lower it again before the analysis ends
    cancelled: AtomicBool,
}

impl OpticalMotionAnalysis {
    pub fn from_manager(stab: &StabilizationManager, cancel_flag: Arc<AtomicBool>) -> Result<Self, String> {
        #[cfg(not(feature = "use-opencv"))]
        { let _ = (stab, cancel_flag); return Err("Optical analysis is not available in this build".into()); }

        #[cfg(feature = "use-opencv")]
        {
            // Before anything is read: whatever changes it from here on is a reason to stop
            let generation = stab.optical_generation.load(SeqCst);
            let mut params = measurement_params(stab);

            let mut gyro = stab.gyro.read().clone();
            // Without motion data in the file (also after an analysis of such a file: its orientation is only in the
            // correction) the analysis measures the motion itself
            let vision = (!gyro.file_metadata.read().has_motion()).then(TimeQuat::new);
            if gyro.optical_correction.take().is_some() {
                gyro.integrate();
            }
            if vision.is_some() {
                gyro.quaternions.clear();
            } else if gyro.quaternions.len() < 2 {
                return Err("No motion data to correct".into());
            }
            let quats_checksum = optical_correction::checksum(&gyro.quaternions);
            params.gyro = Arc::new(parking_lot::RwLock::new(gyro));
            let context_checksum = context_checksum(&params);

            let (fps_scale, scaled_fps, horizontal_readout, ranges_ms, total_frames) = {
                let p = stab.params.read();
                // Only what's going to be exported: the trim ranges, or the whole clip without any
                let ranges: Vec<(f64, f64)> = if p.trim_ranges.is_empty() { vec![(0.0, 1.0)] } else { p.trim_ranges.clone() };
                let ranges_ms = ranges.iter().map(|(a, b)| (a * p.duration_ms, b * p.duration_ms)).collect::<Vec<_>>();
                let total_frames = ranges.iter().map(|(a, b)| ((b - a) * p.frame_count as f64).round() as usize).sum();
                (p.fps_scale, p.get_scaled_fps(), p.frame_readout_direction.is_horizontal(), ranges_ms, total_frames)
            };
            Ok(Self {
                params, fps_scale, scaled_fps, quats_checksum, context_checksum, horizontal_readout,
                track_size: (0, 0),
                focal_px: 0.0,
                tracker: tracker::KltTracker::new(MAX_POINTS),
                last: None,
                pairs: VecDeque::new(),
                next_seq: 0,
                measured_upto: 0,
                measurements: Vec::new(),
                local_steps: Vec::new(),
                measured_pairs: 0,
                frames: 0,
                total_frames,
                ranges_ms,
                vision,
                odometry: Default::default(),
                sg_cache: HashMap::new(),
                cancel_flag,
                generation: (stab.optical_generation.clone(), generation),
                cancelled: AtomicBool::new(false),
            })
        }
    }

    /// Whether it's been cancelled, or overtaken by another file, a project or Clear: then the frames fed are ignored
    /// and `finish` says "Cancelled"
    pub fn is_cancelled(&self) -> bool {
        if !self.cancelled.load(Relaxed) && (self.cancel_flag.load(Relaxed) || self.generation.0.load(SeqCst) != self.generation.1) {
            self.cancelled.store(true, Relaxed);
        }
        self.cancelled.load(Relaxed)
    }

    /// Frames fed so far and to be analyzed
    pub fn progress(&self) -> (usize, usize) { (self.frames, self.total_frames.max(self.frames)) }

    /// What to decode: the trim ranges, in the file's own milliseconds
    pub fn ranges_ms(&self) -> Vec<(f64, f64)> { self.ranges_ms.clone() }

    /// Feeds the next decoded frame, in decoding order: 8-bit luma, ideally about 1000 px wide. Frames outside of
    /// `ranges_ms` are ignored
    pub fn feed_frame(&mut self, timestamp_us: i64, width: u32, height: u32, stride: usize, pixels: &[u8]) -> Result<(), String> {
        if self.is_cancelled() { return Ok(()); }
        let file_ms = timestamp_us as f64 / 1000.0;
        if !self.ranges_ms.iter().any(|(a, b)| file_ms >= *a - 0.5 && file_ms <= *b + 0.5) { return Ok(()); }
        let mut ts_ms = file_ms;
        if let Some(scale) = self.fps_scale { ts_ms /= scale; }
        let index = crate::frame_at_timestamp(ts_ms, self.scaled_fps).max(0) as usize;
        if self.last.map(|l| index <= l.index).unwrap_or(false) { return Ok(()); } // a repeated frame

        if self.track_size != (width, height) {
            self.track_size = (width, height);
            let (k, ..) = FrameTransform::get_lens_data_at_timestamp(&self.params, ts_ms, false);
            self.focal_px = k[(0, 0)] * width as f64 / self.params.width.max(1) as f64;
        }
        let frame = self.frame(index, ts_ms);
        let continuous = self.last.map(|l| index == l.index + 1).unwrap_or(false);

        #[cfg(feature = "use-opencv")]
        {
            if !continuous { self.tracker.reset(); }
            let obs = self.tracker.track(width, height, stride, pixels).map_err(|e| format!("OpenCV error: {e:?}"))?;
            if continuous && !obs.is_empty() {
                if let Some(a) = self.last {
                    self.pairs.push_back(Pair { seq: self.next_seq, a, b: frame, obs });
                    self.next_seq += 1;
                    if self.vision.is_some() { self.chain_vision(); }
                }
            }
        }
        #[cfg(not(feature = "use-opencv"))]
        { let _ = (continuous, stride, pixels); }

        self.last = Some(frame);
        self.frames += 1;
        self.process(false);
        Ok(())
    }

    /// Measures everything tracked so far, however recent
    pub fn flush(&mut self) { self.process(true); }

    /// The band measurements taken so far: all of them after `flush`
    pub fn measurements(&self) -> &[BandMeasurement] { &self.measurements }

    /// Measures what's left. The correction is then `solve`d from the measurements, as many times as the settings change
    pub fn finish(mut self) -> Result<OpticalMeasurements, String> {
        self.process(true);
        if self.is_cancelled() { return Err("Cancelled".into()); }
        if self.measurements.is_empty() { return Err("Not enough of the image could be tracked".into()); }
        ::log::info!("Optical analysis: {} frames, {} measured pairs, {} band measurements{}", self.frames, self.measured_pairs, self.measurements.len(), if self.vision.is_some() { ", motion from the video" } else { "" });
        let (quats_checksum, video_base) = match &self.vision {
            // Measured against what `integrate` makes of it
            Some(keys) => {
                let base: Vec<(i64, [f32; 4])> = keys.iter().map(|(t, q)| (*t, [q.w as f32, q.i as f32, q.j as f32, q.k as f32])).collect();
                (optical_correction::checksum(&optical_correction::base_quats(&base)), base)
            },
            None => (self.quats_checksum, Vec::new()),
        };
        let residual = build_local_residual(&self.local_steps, self.scaled_fps);
        Ok(OpticalMeasurements {
            bands: self.measurements,
            scaled_fps: self.scaled_fps,
            quats_checksum,
            context_checksum: self.context_checksum,
            video_base,
            residual,
            frames: self.frames,
            measured_frames: self.measured_pairs,
            generation: self.generation.1,
        })
    }

    fn frame(&self, index: usize, timestamp_ms: f64) -> Frame {
        frame_timing(&self.params, index, timestamp_ms, if self.horizontal_readout { self.track_size.0 } else { self.track_size.1 })
    }

    /// Unit bearings of tracked points of a frame, in the quaternions' frame
    #[cfg_attr(not(feature = "use-opencv"), allow(dead_code))]
    fn bearings(&self, pts: &[(f32, f32)], frame: &Frame) -> Vec<Option<Vector3<f64>>> {
        undistort_points_for_optical_motion(pts, frame.timestamp_ms, frame.index, &self.params, self.track_size)
            .into_iter()
            .map(|b| b.map(to_quat_frame))
            .collect()
    }

    /// Extends the orientation of a file without motion data by the newest frame pair, by the rotation between its
    /// frames `odometry` finds - the translation's parallax told apart from it. Only the slow part of this has to be
    /// right, and only roughly - what's left of a drift ends up in what the stabilization smooths away - since the
    /// correction measured against it takes care of everything faster, per row
    #[cfg_attr(not(feature = "use-opencv"), allow(dead_code))]
    fn chain_vision(&mut self) {
        let Some(pair) = self.pairs.back() else { return };
        let (a, b) = (pair.a.mid_ms, pair.b.mid_ms);
        let pts_a: Vec<(f32, f32)> = pair.obs.iter().map(|o| (o.a[0], o.a[1])).collect();
        let pts_b: Vec<(f32, f32)> = pair.obs.iter().map(|o| (o.b[0], o.b[1])).collect();
        let (ba, bb) = (self.bearings(&pts_a, &pair.a), self.bearings(&pts_b, &pair.b));
        let mut va = Vec::with_capacity(pair.obs.len());
        let mut vb = Vec::with_capacity(pair.obs.len());
        let mut ids = Vec::with_capacity(pair.obs.len());
        for ((o, x), y) in pair.obs.iter().zip(ba).zip(bb) {
            if let (Some(x), Some(y)) = (x, y) { va.push(x); vb.push(y); ids.push(o.id); }
        }
        let m = if va.len() >= MIN_BAND_POINTS {
            let m0 = robust_rotation(&va, &vb);
            self.odometry.step(&va, &vb, &ids, m0, 1.0 / self.focal_px.max(1.0))
        } else {
            self.odometry.reset();
            Matrix3::identity()
        };

        let Some(keys) = self.vision.as_mut() else { return };
        let (ka, kb) = ((a * 1000.0).round() as i64, (b * 1000.0).round() as i64);
        // A new run of tracks (the start, after a gap) goes on from where the orientation was
        let qa = match keys.get(&ka) {
            Some(q) => *q,
            None => {
                let q = keys.values().next_back().copied().unwrap_or_else(UnitQuaternion::identity);
                keys.insert(ka, q);
                q
            }
        };
        // m = R(b)ᵀ·R(a), so R(b) = R(a)·mᵀ
        let qm = UnitQuaternion::from_rotation_matrix(&Rotation3::from_matrix_unchecked(m));
        keys.insert(kb, qa * qm.inverse());
    }

    /// Measures the frame pairs whose tracks are complete enough for the high-pass
    fn process(&mut self, last_call: bool) {
        let end = if last_call { self.next_seq } else { self.next_seq.saturating_sub(HP_MAX) };
        if end <= self.measured_upto || (!last_call && end < self.measured_upto + CHUNK) { return; }
        if self.is_cancelled() { return; }

        let derived = self.derive();
        let rhp = self.high_pass(&derived);

        // (seq, band) -> the points
        let mut groups: HashMap<(usize, u8), Vec<usize>> = HashMap::new();
        for (i, d) in derived.iter().enumerate() {
            if d.seq >= self.measured_upto && d.seq < end && rhp[i].is_some() {
                groups.entry((d.seq, d.band)).or_default().push(i);
            }
        }
        let floor = SIGMA_FLOOR_PX / self.focal_px.max(1.0);
        let gyro = self.params.gyro.clone();
        let vision = &self.vision;
        let mut ms: Vec<(usize, u8, BandMeasurement)> = groups.par_iter().filter_map(|(&(seq, band), idx)| {
            let gyro = gyro.read();
            fit_band(&derived, &rhp, idx, floor, &gyro, vision).map(|mut m| { m.pair = seq; (seq, band, m) })
        }).collect();
        ms.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.ta_us.total_cmp(&b.2.ta_us)));
        // The local mesh is the part left after exactly the same per-band global rotation that feeds the
        // optical gyro solver. Using the high-passed track residuals here also keeps slow parallax out.
        self.measure_local(&derived, &rhp, &groups, &ms);
        let mut seqs: Vec<usize> = ms.iter().map(|(s, _, _)| *s).collect();
        seqs.sort_unstable();
        seqs.dedup();
        self.measured_pairs += seqs.len();
        self.measurements.extend(ms.into_iter().map(|(_, _, m)| m));

        self.measured_upto = end;
        let keep_from = end.saturating_sub(HP_MAX);
        while self.pairs.front().map(|p| p.seq < keep_from).unwrap_or(false) {
            self.pairs.pop_front();
        }
    }

    /// Residuals of every point of the pairs held, against the quaternions
    fn derive(&self) -> Vec<Derived> {
        let params = &self.params;
        let size = self.track_size;
        let horizontal = self.horizontal_readout;
        let track_rows = if horizontal { size.0 } else { size.1 }.max(1) as f32;
        let per_pair: Vec<Vec<Derived>> = self.pairs.par_iter().map(|pair| {
            let pts_a: Vec<(f32, f32)> = pair.obs.iter().map(|o| (o.a[0], o.a[1])).collect();
            let pts_b: Vec<(f32, f32)> = pair.obs.iter().map(|o| (o.b[0], o.b[1])).collect();
            let pts_bx: Vec<(f32, f32)> = pair.obs.iter().map(|o| (o.b[0] + 1.0, o.b[1])).collect();
            let pts_by: Vec<(f32, f32)> = pair.obs.iter().map(|o| (o.b[0], o.b[1] + 1.0)).collect();
            let ba = undistort_points_for_optical_motion(&pts_a, pair.a.timestamp_ms, pair.a.index, params, size);
            let bb = undistort_points_for_optical_motion(&pts_b, pair.b.timestamp_ms, pair.b.index, params, size);
            let bbx = undistort_points_for_optical_motion(&pts_bx, pair.b.timestamp_ms, pair.b.index, params, size);
            let bby = undistort_points_for_optical_motion(&pts_by, pair.b.timestamp_ms, pair.b.index, params, size);
            let gyro = params.gyro.read();
            let orient = |t: f64| orientation(&self.vision, &gyro, t);
            let mut out = Vec::with_capacity(pair.obs.len());
            for (i, o) in pair.obs.iter().enumerate() {
                let (Some(a), Some(b), Some(bx), Some(by)) = (
                    ba.get(i).copied().flatten(),
                    bb.get(i).copied().flatten(),
                    bbx.get(i).copied().flatten(),
                    bby.get(i).copied().flatten(),
                ) else { continue };
                let pos_a = if horizontal { o.a[0] } else { o.a[1] };
                let pos_b = if horizontal { o.b[0] } else { o.b[1] };
                let ta = pair.a.start_ms + pair.a.per_px_ms * pos_a as f64;
                let tb = pair.b.start_ms + pair.b.per_px_ms * pos_b as f64;
                let m = (orient(tb).inverse() * orient(ta)).to_rotation_matrix().into_inner();
                let (va, vb) = (to_quat_frame(a), to_quat_frame(b));
                let (vbx, vby) = (to_quat_frame(bx), to_quat_frame(by));
                let p = m * va;
                let band = ((pos_a / track_rows) * BANDS as f32).floor().clamp(0.0, (BANDS - 1) as f32) as u8;
                out.push(Derived {
                    id: o.id,
                    seq: pair.seq,
                    band,
                    frame: pair.b.index,
                    uv: [o.b[0] / size.0.max(1) as f32, o.b[1] / size.1.max(1) as f32],
                    jx: vbx - vb,
                    jy: vby - vb,
                    p,
                    r: vb - p,
                    ta_ms: ta,
                    tb_ms: tb,
                });
            }
            out
        }).collect();
        per_pair.into_iter().flatten().collect()
    }

    /// Measures spatial motion left after the same per-band global rotation used by the optical gyro solver.
    /// The input is already high-passed along each persistent track, so slow translation/parallax is not turned
    /// into a warp. A local source-projection Jacobian maps the remaining ray error back to tracked pixels,
    /// carrying lens distortion, camera mesh, IBIS/OIS and breathing geometry into the grid.
    fn measure_local(
        &mut self,
        derived: &[Derived],
        rhp: &[Option<Vector3<f64>>],
        groups: &HashMap<(usize, u8), Vec<usize>>,
        fits: &[(usize, u8, BandMeasurement)],
    ) {
        let mut local_by_seq: HashMap<usize, (usize, Vec<([f32; 2], [f64; 2])>)> = HashMap::new();
        for (seq, band, m) in fits {
            let Some(idx) = groups.get(&(*seq, *band)) else { continue };
            let Some(&first) = idx.first() else { continue };
            let entry = local_by_seq.entry(*seq).or_insert_with(|| (derived[first].frame, Vec::new()));
            for &i in idx {
                let d = &derived[i];
                let Some(hp) = rhp.get(i).copied().flatten() else { continue };
                // Small-angle rotation measured for this readout band. What remains is spatial motion only.
                let local_ray = hp - m.rho.cross(&d.p);
                let Some([mut dx, mut dy]) = ray_delta_to_pixels(local_ray, d.jx, d.jy) else { continue };
                dx /= self.track_size.0.max(1) as f64;
                dy /= self.track_size.1.max(1) as f64;
                if !dx.is_finite() || !dy.is_finite() { continue; }
                // Fail closed on catastrophic tracks/cuts before they reach the spatial robustifier.
                let mag = (dx * dx + dy * dy).sqrt();
                if mag > 0.012 {
                    let k = 0.012 / mag;
                    dx *= k;
                    dy *= k;
                }
                entry.1.push((d.uv, [dx, dy]));
            }
        }
        let mut seqs: Vec<_> = local_by_seq.into_iter().collect();
        seqs.sort_by_key(|(seq, _)| *seq);
        for (_, (frame, local)) in seqs {
            if let Some(grid) = spatial_grid(&local) {
                self.local_steps.push(LocalStep { frame, grid });
            }
        }
    }

    /// Takes the slow part out of each track's residuals: a local quadratic fit (Savitzky-Golay, the fit of the
    /// window's edge at the track's ends), which is where the parallax lives
    fn high_pass(&mut self, derived: &[Derived]) -> Vec<Option<Vector3<f64>>> {
        let mut order: Vec<usize> = (0..derived.len()).collect();
        order.sort_unstable_by_key(|&i| (derived[i].id, derived[i].seq));
        let mut out = vec![None; derived.len()];
        let mut s = 0;
        while s < order.len() {
            let mut e = s + 1;
            while e < order.len() && derived[order[e]].id == derived[order[s]].id && derived[order[e]].seq == derived[order[e - 1]].seq + 1 { e += 1; }
            let len = e - s;
            if len >= HP_MIN {
                let l = { let l = len.min(HP_MAX); if l % 2 == 0 { l - 1 } else { l } };
                let proj = self.sg_cache.entry(l).or_insert_with(|| sg_projection(l));
                for k in 0..len {
                    let w0 = (k as isize - (l / 2) as isize).clamp(0, (len - l) as isize) as usize;
                    let pos = k - w0;
                    let mut smooth = Vector3::zeros();
                    for j in 0..l { smooth += derived[order[s + w0 + j]].r * proj[(pos, j)]; }
                    out[order[s + k]] = Some(derived[order[s + k]].r - smooth);
                }
            }
            s = e;
        }
        out
    }
}

/// The orientation the image is compared against at a moment of the video: the motion data's, or for a file without
/// any the one chained from the frames (along the geodesic between them, as `optical_correction::densify` has it)
fn orientation(vision: &Option<TimeQuat>, gyro: &GyroSource, t_ms: f64) -> UnitQuaternion<f64> {
    let Some(keys) = vision else { return gyro.org_quat_at_timestamp(t_ms) };
    let t = (t_ms * 1000.0).round() as i64;
    match (keys.range(..=t).next_back(), keys.range(t..).next()) {
        (Some((&a, qa)), Some((&b, qb))) => if b == a { *qa } else { qa.slerp(qb, (t - a) as f64 / (b - a) as f64) },
        (Some((_, q)), None) | (None, Some((_, q))) => *q,
        (None, None) => UnitQuaternion::identity(),
    }
}

/// Least-squares pixel displacement whose local source-projection Jacobian produces ray_delta.
fn ray_delta_to_pixels(ray_delta: Vector3<f64>, jx: Vector3<f64>, jy: Vector3<f64>) -> Option<[f64; 2]> {
    let a11 = jx.dot(&jx);
    let a12 = jx.dot(&jy);
    let a22 = jy.dot(&jy);
    let b1 = jx.dot(&ray_delta);
    let b2 = jy.dot(&ray_delta);
    let det = a11 * a22 - a12 * a12;
    if !det.is_finite() || det <= 1e-16 { return None; }
    let dx = (b1 * a22 - b2 * a12) / det;
    let dy = (a11 * b2 - a12 * b1) / det;
    (dx.is_finite() && dy.is_finite()).then_some([dx, dy])
}

fn spatial_grid(points: &[([f32; 2], [f64; 2])]) -> Option<Vec<[f32; 2]>> {
    if points.len() < MIN_BAND_POINTS * 2 { return None; }
    let mut grid = vec![[0.0f32; 2]; OPTICAL_GRID * OPTICAL_GRID];
    let radius2 = 0.30f64.powi(2);
    let sigma2 = 0.14f64.powi(2);
    let mut supported = 0usize;
    for gy in 0..OPTICAL_GRID {
        for gx in 0..OPTICAL_GRID {
            let u = gx as f64 / (OPTICAL_GRID - 1) as f64;
            let v = gy as f64 / (OPTICAL_GRID - 1) as f64;
            let mut candidates = Vec::new();
            for &(uv, d) in points {
                let r2 = (uv[0] as f64 - u).powi(2) + (uv[1] as f64 - v).powi(2);
                if r2 <= radius2 {
                    candidates.push((d, (-0.5 * r2 / sigma2).exp()));
                }
            }
            if candidates.len() < 6 { continue; }
            let sw = candidates.iter().map(|x| x.1).sum::<f64>();
            if sw <= 0.0 { continue; }
            let mean = [
                candidates.iter().map(|x| x.0[0] * x.1).sum::<f64>() / sw,
                candidates.iter().map(|x| x.0[1] * x.1).sum::<f64>() / sw,
            ];
            let mut errs: Vec<f64> = candidates.iter().map(|x| {
                ((x.0[0] - mean[0]).powi(2) + (x.0[1] - mean[1]).powi(2)).sqrt()
            }).collect();
            errs.sort_by(|a,b| a.total_cmp(b));
            let scale = (1.4826 * errs[errs.len()/2]).max(2e-5);
            let mut sx = 0.0;
            let mut sy = 0.0;
            let mut ww = 0.0;
            for (d, spatial) in candidates {
                let e = ((d[0] - mean[0]).powi(2) + (d[1] - mean[1]).powi(2)).sqrt();
                let robust = 1.0 / (1.0 + (e / (2.5 * scale)).powi(2));
                let wt = spatial * robust;
                sx += d[0] * wt;
                sy += d[1] * wt;
                ww += wt;
            }
            if ww > 1.5 {
                let support = (ww / 12.0).min(1.0);
                grid[gy * OPTICAL_GRID + gx] = [(sx / ww * support) as f32, (sy / ww * support) as f32];
                supported += 1;
            }
        }
    }
    if supported < OPTICAL_GRID * OPTICAL_GRID / 3 { return None; }
    let old = grid.clone();
    for gy in 0..OPTICAL_GRID {
        for gx in 0..OPTICAL_GRID {
            let mut x = old[gy * OPTICAL_GRID + gx][0] as f64;
            let mut y = old[gy * OPTICAL_GRID + gx][1] as f64;
            let mut n = 1.0;
            for (dx,dy) in [(-1,0),(1,0),(0,-1),(0,1)] {
                let xx = gx as isize + dx;
                let yy = gy as isize + dy;
                if xx >= 0 && yy >= 0 && xx < OPTICAL_GRID as isize && yy < OPTICAL_GRID as isize {
                    let q = old[yy as usize * OPTICAL_GRID + xx as usize];
                    x += q[0] as f64;
                    y += q[1] as f64;
                    n += 1.0;
                }
            }
            let i = gy * OPTICAL_GRID + gx;
            grid[i][0] = (0.6 * old[i][0] as f64 + 0.4 * x / n) as f32;
            grid[i][1] = (0.6 * old[i][1] as f64 + 0.4 * y / n) as f32;
        }
    }
    Some(grid)
}

fn build_local_residual(steps: &[LocalStep], fps: f64) -> OpticalResidualCorrection {
    if steps.is_empty() || fps <= 0.0 { return OpticalResidualCorrection::default(); }
    let mut steps = steps.to_vec();
    steps.sort_by_key(|s| s.frame);
    steps.dedup_by_key(|s| s.frame);
    let window = (LOCAL_SMOOTH_SECONDS * fps).round().max(3.0) as usize;
    let radius = (window / 2).max(1);
    let mut frames = Vec::new();
    let mut start = 0usize;
    while start < steps.len() {
        let mut end = start + 1;
        while end < steps.len() && steps[end].frame == steps[end - 1].frame + 1 { end += 1; }
        let run = &steps[start..end];
        if run.len() >= 5 {
            let nodes = OPTICAL_GRID * OPTICAL_GRID;
            let mut paths = vec![vec![[0.0f64; 2]; nodes]; run.len()];
            for i in 0..run.len() {
                if i > 0 { paths[i] = paths[i - 1].clone(); }
                for n in 0..nodes {
                    paths[i][n][0] += run[i].grid[n][0] as f64;
                    paths[i][n][1] += run[i].grid[n][1] as f64;
                }
            }
            for i in 0..run.len() {
                let a = i.saturating_sub(radius);
                let b = (i + radius + 1).min(run.len());
                let count = (b - a) as f64;
                let mut corr = vec![[0.0f32; 2]; nodes];
                let mut max_mag = 0.0f64;
                for n in 0..nodes {
                    let sx = (a..b).map(|j| paths[j][n][0]).sum::<f64>() / count;
                    let sy = (a..b).map(|j| paths[j][n][1]).sum::<f64>() / count;
                    let mut cx = sx - paths[i][n][0];
                    let mut cy = sy - paths[i][n][1];
                    let mag = (cx * cx + cy * cy).sqrt();
                    if mag > 0.02 {
                        let k = 0.02 / mag;
                        cx *= k;
                        cy *= k;
                    }
                    max_mag = max_mag.max((cx * cx + cy * cy).sqrt());
                    corr[n] = [cx as f32, cy as f32];
                }
                if max_mag > 1e-6 { frames.push((run[i].frame, corr)); }
            }
        }
        start = end;
    }
    OpticalResidualCorrection::from_normalized_frames(LOCAL_SMOOTH_SECONDS, frames)
}

#[cfg_attr(not(feature = "use-opencv"), allow(dead_code))]
/// The rotation `R` with `b ≈ R·a` for most of the points: Kabsch, reweighted against the ones that disagree
fn robust_rotation(a: &[Vector3<f64>], b: &[Vector3<f64>]) -> Matrix3<f64> {
    let mut w = vec![1.0f64; a.len()];
    let mut r = Matrix3::identity();
    for _ in 0..6 {
        let h: Matrix3<f64> = a.iter().zip(b).zip(&w).map(|((a, b), w)| a * b.transpose() * *w).sum();
        let svd = h.svd(true, true);
        let (Some(u), Some(vt)) = (svd.u, svd.v_t) else { break };
        let d = (vt.transpose() * u.transpose()).determinant().signum();
        r = vt.transpose() * Matrix3::from_diagonal(&Vector3::new(1.0, 1.0, d)) * u.transpose();
        let res: Vec<f64> = a.iter().zip(b).map(|(a, b)| (b - r * a).norm()).collect();
        let mut sorted = res.clone();
        sorted.sort_by(|x, y| x.total_cmp(y));
        let scale = (1.4826 * sorted[sorted.len() / 2]).max(1e-9);
        for (w, e) in w.iter_mut().zip(&res) { *w = 1.0 / (1.0 + (e / (2.5 * scale)).powi(2)); }
    }
    r
}

/// Maps samples of a window of `l` to their least-squares quadratic
fn sg_projection(l: usize) -> DMatrix<f64> {
    let c = (l as f64 - 1.0) / 2.0;
    let x = DMatrix::from_fn(l, 3, |i, j| ((i as f64 - c) / c.max(1.0)).powi(j as i32));
    let xtx = x.transpose() * &x;
    let inv = xtx.try_inverse().unwrap_or_else(|| DMatrix::zeros(3, 3));
    &x * inv * x.transpose()
}

/// One band of one frame pair: the rotation its points moved by beyond the quaternions, `r ≈ ρ × p`, robustly
fn fit_band(derived: &[Derived], rhp: &[Option<Vector3<f64>>], idx: &[usize], sigma_floor: f64, gyro: &GyroSource, vision: &Option<TimeQuat>) -> Option<BandMeasurement> {
    if idx.len() < MIN_BAND_POINTS { return None; }
    let mut w = vec![1.0f64; idx.len()];
    let mut rho = Vector3::zeros();
    let mut h = Matrix3::zeros();
    let mut res = vec![0.0f64; idx.len()];
    for _ in 0..5 {
        h = Matrix3::zeros();
        let mut g = Vector3::zeros();
        for (k, &i) in idx.iter().enumerate() {
            let (p, r) = (derived[i].p, rhp[i]?);
            h += (Matrix3::identity() - p * p.transpose()) * w[k];
            g += p.cross(&r) * w[k];
        }
        rho = h.try_inverse()? * g;
        for (k, &i) in idx.iter().enumerate() {
            res[k] = (rhp[i]? - rho.cross(&derived[i].p)).norm();
        }
        let mut sorted = res.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let scale = (1.4826 * sorted[sorted.len() / 2]).max(1e-9);
        for k in 0..idx.len() { w[k] = 1.0 / (1.0 + (res[k] / (2.5 * scale)).powi(2)); }
    }
    let sw: f64 = w.iter().sum();
    if sw < MIN_BAND_POINTS as f64 * 0.5 { return None; }
    let var = res.iter().zip(&w).map(|(e, w)| w * e * e).sum::<f64>() / sw / 2.0;
    let cov = h.try_inverse()? * var + Matrix3::identity() * sigma_floor * sigma_floor;
    let info = cov.try_inverse()?;

    let ta = idx.iter().zip(&w).map(|(&i, w)| derived[i].ta_ms * w).sum::<f64>() / sw;
    let tb = idx.iter().zip(&w).map(|(&i, w)| derived[i].tb_ms * w).sum::<f64>() / sw;
    let m = (orientation(vision, gyro, tb).inverse() * orientation(vision, gyro, ta)).to_rotation_matrix().into_inner();
    let to_gyro_us = |t: f64| (t - gyro.offset_at_video_timestamp(t)) * 1000.0;
    Some(BandMeasurement { pair: 0, ta_us: to_gyro_us(ta), tb_us: to_gyro_us(tb), rho, info, m })
}


#[cfg(test)]
mod local_residual_tests {
    use super::*;

    fn sinusoid_steps(fps: f64, hz: f64, frames: usize, amp: f32, first_b: usize) -> Vec<LocalStep> {
        let nodes = OPTICAL_GRID * OPTICAL_GRID;
        let mut out = Vec::new();
        let mut prev = 0.0f32;
        for i in 0..frames {
            let t = i as f64 / fps;
            let path = amp * (std::f64::consts::TAU * hz * t).sin() as f32;
            let d = path - prev;
            prev = path;
            out.push(LocalStep { frame: first_b + i, grid: vec![[d, 0.0]; nodes] });
        }
        out
    }

    fn rms_x(r: &OpticalResidualCorrection) -> f64 {
        let mut sum = 0.0;
        let mut n = 0usize;
        for f in &r.frames {
            if let Some(g) = r.normalized_grid(f.frame as usize) {
                for v in g {
                    sum += (v[0] as f64).powi(2);
                    n += 1;
                }
            }
        }
        if n == 0 { 0.0 } else { (sum / n as f64).sqrt() }
    }

    #[test]
    fn temporal_window_is_seconds_and_targets_fast_motion() {
        for fps in [30.0, 60.0, 120.0] {
            let slow = build_local_residual(&sinusoid_steps(fps, 0.25, (fps * 4.0) as usize, 0.003, 1), fps);
            let fast = build_local_residual(&sinusoid_steps(fps, 8.0, (fps * 4.0) as usize, 0.003, 1), fps);
            let rs = rms_x(&slow);
            let rf = rms_x(&fast);
            assert!(rf > rs * 2.0, "fps={fps}: fast={rf} slow={rs}");
        }
    }

    #[test]
    fn pair_increment_is_applied_to_frame_b_not_a() {
        let r = build_local_residual(&sinusoid_steps(60.0, 8.0, 30, 0.003, 17), 60.0);
        assert!(!r.has_frame(16));
        assert!(r.frames.iter().all(|f| f.frame >= 17));
        assert!(r.frames.iter().any(|f| f.frame == 17));
    }


    #[test]
    fn context_checksum_ignores_installed_optical_residual() {
        use crate::gyro_source::{ OpticalCorrection, OpticalResidualCorrection, OPTICAL_GRID };

        let stab = crate::StabilizationManager::default();
        stab.init_from_video_data(1000.0, 30.0, 30, (352, 288));
        stab.set_output_size(352, 288);
        let lens = r#"{
            "name":"optical context test",
            "calibrator_version":"test",
            "calib_dimension":{"w":352,"h":288},
            "orig_dimension":{"w":352,"h":288},
            "fps":30.0,
            "distortion_model":"opencv_standard",
            "fisheye_params":{
                "camera_matrix":[[300.0,0.0,176.0],[0.0,300.0,144.0],[0.0,0.0,1.0]],
                "distortion_coeffs":[0.0,0.0,0.0,0.0,0.0]
            }
        }"#;
        stab.load_lens_profile(lens).expect("lens");

        let before = context_checksum(&measurement_params(&stab));
        let grid = vec![[0.002f32, -0.001f32]; OPTICAL_GRID * OPTICAL_GRID];
        let residual = OpticalResidualCorrection::from_normalized_frames(0.25,
            (0..30).map(|f| (f, grid.clone())).collect());
        {
            let mut gyro = stab.gyro.write();
            gyro.optical_correction = Some(OpticalCorrection { enabled: true, residual, ..Default::default() });
            gyro.optical_correction_applied = true;
        }
        let after = context_checksum(&measurement_params(&stab));
        assert_eq!(before, after, "an installed residual must not fingerprint itself");
    }

    #[test]
    fn spatial_grid_rejects_moving_foreground_outliers() {
        let mut points = Vec::new();
        for y in 0..12 {
            for x in 0..16 {
                let uv = [(x as f32 + 0.5) / 16.0, (y as f32 + 0.5) / 12.0];
                let d = if x < 4 && y > 3 && y < 9 {
                    [0.010, -0.008]
                } else {
                    [0.0012, -0.0007]
                };
                points.push((uv, d));
            }
        }
        let grid = spatial_grid(&points).expect("enough background support");
        let c = grid[(OPTICAL_GRID / 2) * OPTICAL_GRID + OPTICAL_GRID / 2];
        assert!((c[0] as f64 - 0.0012).abs() < 0.0015, "center x={}", c[0]);
        assert!((c[1] as f64 + 0.0007).abs() < 0.0015, "center y={}", c[1]);
    }
}
