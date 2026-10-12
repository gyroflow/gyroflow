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
//! nowhere.
//!
//! The tracks are kept with what was measured from them (`Tracks`): the user can take out the ones on something that
//! moves on its own and that the robust fit took for the scene, and the rest is measured again without going over the
//! video again (`OpticalMeasurements::remeasure`)

pub mod solver;
mod odometry;
#[cfg(feature = "use-opencv")]
mod tracker;

use std::collections::{ BTreeSet, HashMap, HashSet };
use std::ops::{ ControlFlow, RangeInclusive };
use std::sync::{ Arc, atomic::{ AtomicBool, AtomicU64, Ordering::{ Relaxed, SeqCst } } };
use nalgebra::{ DMatrix, Matrix3, Rotation3, UnitQuaternion, Vector3 };
use rayon::prelude::*;

use crate::StabilizationManager;
use crate::gyro_source::{ GyroSource, OpticalCorrection, OpticalCorrectionSettings, TimeQuat, TrackingMethod, optical_correction::{ self, Fnv } };
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
/// Tracks in fewer frame pairs than this count for nothing and aren't shown: too short for the correction's high-pass,
/// and mostly points lost again right away, to motion blur, an occlusion, the edge of the frame. The motion measured for
/// a file without motion data leaves them out too, see `Measuring::chain_vision`
const MIN_TRACK_PAIRS: u32 = HP_MIN as u32;
/// Floor of the uncertainty of one band's rotation, in pixels of the tracked frame
const SIGMA_FLOOR_PX: f64 = 0.01;
/// The tracked positions are measured and kept in fixed point, this many steps a pixel of the tracked frame: their error
/// (std. dev. 0.0006 px) is far below `SIGMA_FLOOR_PX` and the tracking's own, and moved the band measurements by under
/// a hundredth of their own uncertainty. See `Tracks`
const POSITION_STEPS: f32 = 512.0;
/// Frames `Tracks` codes together: what looking at one of them decodes
const BLOCK_FRAMES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observation {
    pub id: u32,
    pub a: [f32; 2],
    pub b: [f32; 2],
}

fn to_fixed(v: f32) -> i32 { (v * POSITION_STEPS).round() as i32 }
fn from_fixed(v: i32) -> f32 { v as f32 / POSITION_STEPS }
fn point_from_fixed(q: [i32; 2]) -> [f32; 2] { [from_fixed(q[0]), from_fixed(q[1])] }

#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    index: usize,
    timestamp_ms: f64,
    /// Time of the first tracked row (or column), and per tracked pixel along the readout: `FrameTransform::at_timestamp`
    start_ms: f64,
    per_px_ms: f64,
    /// Middle of the readout: the frame's own time
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
    /// Which of its pair's observations it is
    k: u32,
    band: u8,
    /// Where the quaternions put the point in the second frame, and how far off that was (quaternion frame)
    p: Vector3<f64>,
    r: Vector3<f64>,
    ta_ms: f64,
    tb_ms: f64,
}

/// What the image measured: kept (in memory) so a change of the settings refits the correction in a fraction of a
/// second, and taking tracks out measures it again in a few, instead of another pass over the video
#[derive(Default)]
pub struct OpticalMeasurements {
    pub bands: Vec<BandMeasurement>,
    pub scaled_fps: f64,
    /// Of the quaternions they were measured against, see `OpticalCorrection::quats_checksum`
    pub quats_checksum: u64,
    /// And of the rest they were measured with, see `context_checksum`
    pub context_checksum: u64,
    /// For a file without motion data, the orientation measured against, see `OpticalCorrection::video_base`
    pub video_base: Vec<(i64, [f32; 4])>,
    /// How the points were followed
    pub tracking: TrackingMethod,
    pub frames: usize,
    pub measured_frames: usize,
    /// `StabilizationManager::optical_generation` when the analysis started: they're only for what was loaded then
    pub generation: u64,
    /// What was tracked, for "Show tracked points" and for measuring again without some of it
    pub tracks: Arc<Tracks>,
    /// The tracks left out of these measurements
    pub excluded: Arc<HashSet<u32>>,
    /// What became of each point of each frame pair of `tracks`, in the same order
    pub states: PackedStates,
}

/// What became of each point of each frame pair, in the order of the pairs and of their observations: `PointState` in
/// two bits, but `Removed` - that's the tracks left out, which say it themselves
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct PackedStates {
    bits: Vec<u8>,
    len: usize,
}
impl PackedStates {
    /// `n` more, as `Unused`
    fn push_unused(&mut self, n: usize) {
        self.len += n;
        // Two set bits are `Unused`, which also the bits past the end of the last byte stay
        self.bits.resize(self.len.div_ceil(4), 0xff);
    }
    fn set(&mut self, i: usize, state: PointState) {
        let code = match state { PointState::Used => 0, PointState::Downweighted => 1, PointState::Rejected => 2, _ => 3 };
        let (byte, shift) = (i / 4, i % 4 * 2);
        self.bits[byte] = (self.bits[byte] & !(3 << shift)) | (code << shift);
    }
    fn get(&self, i: usize) -> Option<PointState> {
        (i < self.len).then(|| match (self.bits[i / 4] >> (i % 4 * 2)) & 3 {
            0 => PointState::Used,
            1 => PointState::Downweighted,
            2 => PointState::Rejected,
            _ => PointState::Unused,
        })
    }
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
}

/// What became of a tracked point, for "Show tracked points"
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PointState {
    /// Its band of rows was measured with it
    Used,
    /// ... with less weight: it moved differently from most of its band (something moving, a reflection, parallax the
    /// high-pass didn't take out)
    Downweighted,
    /// ... next to none
    Rejected,
    /// Not measured: its track was too short for the high-pass, or its band had too few points
    Unused,
    /// Its track was taken out by the user
    Removed,
}
impl PointState {
    /// From its weight in the robust fit of its band (Cauchy: 0.5 at 2.5 σ, 0.1 at 7.5 σ)
    fn from_weight(w: f64) -> Self {
        if w >= 0.5 { Self::Used } else if w >= 0.1 { Self::Downweighted } else { Self::Rejected }
    }
}

/// Where a track starts: the frame, numbered as `frame_at_timestamp` numbers them, and the point there, relative to the
/// frame in 16 bits. The ids the tracker gives its tracks only mean something within one analysis, but another analysis
/// of the same frames, from the same first one, tracks the same points and starts them at the same places: what a track
/// the user took out is remembered by, in the project too (`StabilizationManager::optical_removed_tracks`). In JSON
/// `[frame, x, y]`
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct TrackStart(pub u32, pub u16, pub u16);

fn to_u16(v: f32) -> u16 { (v.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16 }

/// What measuring the tracks takes besides them: kept with them, to measure them again
#[derive(Clone, Default)]
struct Setup {
    /// Its `gyro` is a copy of the motion data without any optical correction: that's what the correction is measured against
    params: ComputeParams,
    horizontal_readout: bool,
    /// Of the frames tracked, and the focal length in their pixels
    track_size: (u32, u32),
    focal_px: f64,
    /// No motion data: the motion is measured from the frames too, see `Measuring::vision`
    from_video: bool,
}

/// Every frame pair an analysis tracked, kept with what it measured: to show the points, and to measure again without
/// the tracks the user took out. The robust fit of each band of rows tells the scene from what moves on its own as long
/// as the scene is most of the band; where it isn't (a large vehicle, a crowd, water, the drone's own propellers) only
/// the user can tell.
///
/// In memory only (a project keeps the correction), and compact: an analysis keeps over a thousand points in every frame,
/// which as frame pairs of floats took over 20 bytes a point and a frame - a gigabyte for ten minutes at 60 fps. Here
/// every frame keeps its points once, in fixed point (`POSITION_STEPS`, also what the analysis measures), each coded
/// against where it was predicted to be: moved on from the frame before as it moved last, and by what the frame's points
/// moved by beyond that, the camera's own acceleration (their median). What's left is the shake of the point itself, a
/// byte or two a coordinate: about 4 bytes a point and a frame on the clips tried, everything included - a fifth, 200 MB
/// for those ten minutes. In blocks of `BLOCK_FRAMES` that decode on their own: showing a frame decodes one of them,
/// measuring again all of them in turn
#[derive(Default)]
pub struct Tracks {
    setup: Setup,
    entries: Vec<Entry>,
    /// `BLOCK_FRAMES` entries each, see `TracksBuilder::code`
    blocks: Vec<Box<[u8]>>,
    /// The tracks that count, `MIN_TRACK_PAIRS` pairs and longer, by id: the ones shown, and taken out
    long: Vec<TrackInfo>,
    /// ... by where they start (indices into `long`)
    by_start: Vec<u32>,
    /// The block decoded last: playing or stepping through the frames looks at one after another of the same block
    cache: parking_lot::Mutex<Option<(usize, Arc<Vec<CodedFrame>>)>>,
}

/// A frame of `Tracks`
#[derive(Clone, Copy, Debug)]
struct Entry {
    frame: Frame,
    /// How many of its points came from the frame before: its first ones, which are the observations of the pair ending
    /// here, in their order. 0 when no pair ends here
    continued: u32,
    /// Where the states of that pair start, see `PackedStates`
    states_from: u64,
}

#[derive(Clone, Copy, Debug)]
struct TrackInfo {
    id: u32,
    start: TrackStart,
}

/// The points of a frame, in fixed point (`POSITION_STEPS`), as `Tracks` codes them and decoding gives them back: first
/// the ones that came from the frame before, in its order, then the ones new here
#[derive(Clone, Default, Debug, PartialEq)]
struct CodedFrame {
    ids: Vec<u32>,
    pos: Vec<[i32; 2]>,
    /// How far each moved since the frame before, 0 for the new ones: what the next frame is predicted from
    vel: Vec<[i32; 2]>,
    /// Which go on into the next frame
    next: Vec<bool>,
}

/// Unsigned LEB128, and signed values zigzag-coded into it: 7 bits a byte, so the small values most of the coded data
/// is take one or two
mod varint {
    pub fn put(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 { out.push(v as u8 | 0x80); v >>= 7; }
        out.push(v as u8);
    }
    pub fn put_signed(out: &mut Vec<u8>, v: i64) { put(out, ((v << 1) ^ (v >> 63)) as u64); }
    pub fn put_bits(out: &mut Vec<u8>, bits: &[bool]) {
        out.extend(bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |byte, (i, b)| byte | ((*b as u8) << i))));
    }
    pub struct Reader<'a> { data: &'a [u8], at: usize }
    impl<'a> Reader<'a> {
        pub fn new(data: &'a [u8]) -> Self { Self { data, at: 0 } }
        pub fn get(&mut self) -> u64 {
            let (mut v, mut shift) = (0u64, 0);
            loop {
                let byte = self.data[self.at];
                self.at += 1;
                v |= ((byte & 0x7f) as u64) << shift;
                if byte < 0x80 { return v; }
                shift += 7;
            }
        }
        pub fn get_signed(&mut self) -> i64 { let v = self.get(); ((v >> 1) as i64) ^ -((v & 1) as i64) }
        pub fn bits(&mut self, n: usize) -> Vec<bool> {
            let bytes = &self.data[self.at..self.at + n.div_ceil(8)];
            self.at += bytes.len();
            (0..n).map(|i| bytes[i / 8] >> (i % 8) & 1 == 1).collect()
        }
    }
}

/// A frame on its own: ids and positions each from the one before (the first from 0)
fn code_absolute(out: &mut Vec<u8>, f: &CodedFrame) {
    varint::put(out, f.ids.len() as u64);
    let (mut id, mut p) = (0i64, [0i64; 2]);
    for (i, q) in f.ids.iter().zip(&f.pos) {
        varint::put_signed(out, *i as i64 - id);
        id = *i as i64;
        for c in 0..2 { varint::put_signed(out, q[c] as i64 - p[c]); p[c] = q[c] as i64; }
    }
    varint::put_bits(out, &f.next);
}
fn decode_absolute(r: &mut varint::Reader) -> CodedFrame {
    let n = r.get() as usize;
    let mut f = CodedFrame { ids: Vec::with_capacity(n), pos: Vec::with_capacity(n), vel: vec![[0, 0]; n], next: Vec::new() };
    let (mut id, mut p) = (0i64, [0i64; 2]);
    for _ in 0..n {
        id += r.get_signed();
        for v in &mut p { *v += r.get_signed(); }
        f.ids.push(id as u32);
        f.pos.push([p[0] as i32, p[1] as i32]);
    }
    f.next = r.bits(n);
    f
}
/// A frame against the one before (`prev`), which its first points went on from: each of those as how far it is from
/// where it was predicted - where it was in `prev`, moved on as it moved there - beyond what all of them are (the median,
/// the camera's own acceleration). The new points as in `code_absolute`, from the last of those
fn code_relative(out: &mut Vec<u8>, prev: &CodedFrame, f: &CodedFrame) {
    let from = prev.next.iter().enumerate().filter(|(_, n)| **n).map(|(j, _)| j);
    let residual: Vec<[i64; 2]> = from.zip(&f.pos).map(|(j, q)| [0, 1].map(|c| q[c] as i64 - prev.pos[j][c] as i64 - prev.vel[j][c] as i64)).collect();
    let continued = residual.len();
    let common = [0, 1].map(|c| {
        let mut v: Vec<i64> = residual.iter().map(|r| r[c]).collect();
        let half = v.len() / 2;
        if v.is_empty() { 0 } else { *v.select_nth_unstable(half).1 }
    });
    for c in common { varint::put_signed(out, c); }
    for r in &residual { for c in 0..2 { varint::put_signed(out, r[c] - common[c]); } }
    varint::put(out, (f.ids.len() - continued) as u64);
    let (mut id, mut p) = (f.ids[..continued].last().copied().unwrap_or_default() as i64, [0i64; 2]);
    for (i, q) in f.ids[continued..].iter().zip(&f.pos[continued..]) {
        varint::put_signed(out, *i as i64 - id);
        id = *i as i64;
        for c in 0..2 { varint::put_signed(out, q[c] as i64 - p[c]); p[c] = q[c] as i64; }
    }
    varint::put_bits(out, &f.next);
}
fn decode_relative(r: &mut varint::Reader, prev: &CodedFrame) -> CodedFrame {
    let common = [r.get_signed(), r.get_signed()];
    let mut f = CodedFrame::default();
    for (j, _) in prev.next.iter().enumerate().filter(|(_, n)| **n) {
        let q = [0, 1].map(|c| (prev.pos[j][c] as i64 + prev.vel[j][c] as i64 + common[c] + r.get_signed()) as i32);
        f.ids.push(prev.ids[j]);
        f.vel.push([q[0] - prev.pos[j][0], q[1] - prev.pos[j][1]]);
        f.pos.push(q);
    }
    let new = r.get() as usize;
    let (mut id, mut p) = (f.ids.last().copied().unwrap_or_default() as i64, [0i64; 2]);
    for _ in 0..new {
        id += r.get_signed();
        for v in &mut p { *v += r.get_signed(); }
        f.ids.push(id as u32);
        f.pos.push([p[0] as i32, p[1] as i32]);
        f.vel.push([0, 0]);
    }
    f.next = r.bits(f.ids.len());
    f
}

/// Builds `Tracks` from the frame pairs as they're tracked
#[derive(Default)]
struct TracksBuilder {
    entries: Vec<Entry>,
    blocks: Vec<Box<[u8]>>,
    block: Vec<u8>,
    /// The frame coded last, which the next one goes on from
    last: Option<CodedFrame>,
    /// The second frame of the last pair, until the next pair says which points start in it
    pending: Option<(Frame, Vec<u32>, Vec<[i32; 2]>)>,
    pairs: usize,
    observations: u64,
    /// The tracks still going: where they started, and how many pairs they're in so far
    live: HashMap<u32, (TrackStart, u32)>,
    long: Vec<TrackInfo>,
}

impl TracksBuilder {
    /// The next pair the tracker gave (between the frames `a` and `b` of `size` pixels), as it's kept and measured: its
    /// positions in fixed point, and its observations in the order of the points of `a`, see `Entry::continued`
    #[cfg_attr(not(any(test, feature = "use-opencv")), allow(dead_code))]
    fn push(&mut self, a: Frame, b: Frame, mut obs: Vec<Observation>, size: (u32, u32)) -> Pair {
        // The tracker's order already, but it's what the order of the new points of a frame is
        obs.sort_unstable_by_key(|o| o.id);
        let fixed = |p: [f32; 2]| [to_fixed(p[0]), to_fixed(p[1])];
        // The pair's first frame, whole now that it's known which points start in it: the ones that came there from
        // the frame before, if it's the second frame of the last pair, and the new ones
        let (mut ids, mut pos) = match self.pending.take() {
            Some((frame, ids, pos)) if frame.index == a.index => (ids, pos),
            Some(pending) => { self.end_frame(pending); (Vec::new(), Vec::new()) },
            None => (Vec::new(), Vec::new()),
        };
        let continued = ids.len();
        let index: HashMap<u32, usize> = obs.iter().enumerate().map(|(k, o)| (o.id, k)).collect();
        let came: HashSet<u32> = ids.iter().copied().collect();
        let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
        for o in obs.iter().filter(|o| !came.contains(&o.id)) {
            let q = fixed(o.a);
            ids.push(o.id);
            pos.push(q);
            self.live.insert(o.id, (TrackStart(a.index as u32, to_u16(from_fixed(q[0]) / w), to_u16(from_fixed(q[1]) / h)), 0));
        }
        let next: Vec<bool> = ids.iter().map(|id| index.contains_key(id)).collect();
        let mut kept = Vec::with_capacity(obs.len());
        let (mut b_ids, mut b_pos) = (Vec::with_capacity(obs.len()), Vec::with_capacity(obs.len()));
        for (j, id) in ids.iter().enumerate() {
            if !next[j] { self.end_track(*id); continue; }
            let q = fixed(obs[index[id]].b);
            kept.push(Observation { id: *id, a: point_from_fixed(pos[j]), b: point_from_fixed(q) });
            b_ids.push(*id);
            b_pos.push(q);
            if let Some(t) = self.live.get_mut(id) { t.1 += 1; }
        }
        self.code(a, ids, pos, next, continued);
        self.pending = Some((b, b_ids, b_pos));
        self.pairs += 1;
        Pair { seq: self.pairs - 1, a, b, obs: kept }
    }

    /// The second frame of a pair no other pair goes on from: all its points end there
    fn end_frame(&mut self, (frame, ids, pos): (Frame, Vec<u32>, Vec<[i32; 2]>)) {
        for id in &ids { self.end_track(*id); }
        let n = ids.len();
        self.code(frame, ids, pos, vec![false; n], n);
    }

    fn end_track(&mut self, id: u32) {
        if let Some((start, pairs)) = self.live.remove(&id) {
            if pairs >= MIN_TRACK_PAIRS { self.long.push(TrackInfo { id, start }); }
        }
    }

    /// Codes the next frame: against the one before when `continued` of its points came from there, on its own
    /// otherwise. A block starts with the frame before it on its own too (when the block's first frame goes on from it),
    /// as what the first one is coded against, so the block decodes without the one before
    fn code(&mut self, frame: Frame, ids: Vec<u32>, pos: Vec<[i32; 2]>, next: Vec<bool>, continued: usize) {
        let starts_block = self.entries.len() % BLOCK_FRAMES == 0;
        if starts_block && !self.block.is_empty() {
            self.blocks.push(std::mem::take(&mut self.block).into_boxed_slice());
        }
        let prev = self.last.take().filter(|_| continued > 0).map(|p| if starts_block {
            // How it moved there isn't in the block
            CodedFrame { vel: vec![[0, 0]; p.ids.len()], ..p }
        } else { p });
        if starts_block {
            varint::put(&mut self.block, prev.is_some() as u64);
            if let Some(p) = &prev { code_absolute(&mut self.block, p); }
        }
        let vel = match &prev {
            Some(p) => {
                let from = p.next.iter().enumerate().filter(|(_, n)| **n).map(|(j, _)| j);
                let mut vel: Vec<[i32; 2]> = from.zip(&pos).map(|(j, q)| [q[0] - p.pos[j][0], q[1] - p.pos[j][1]]).collect();
                debug_assert_eq!(vel.len(), continued);
                vel.resize(ids.len(), [0, 0]);
                vel
            },
            None => vec![[0, 0]; ids.len()],
        };
        let f = CodedFrame { ids, pos, vel, next };
        match &prev {
            Some(p) => code_relative(&mut self.block, p, &f),
            None => code_absolute(&mut self.block, &f),
        }
        self.entries.push(Entry { frame, continued: continued as u32, states_from: self.observations });
        self.observations += continued as u64;
        self.last = Some(f);
    }

    fn finish(mut self, setup: Setup) -> Tracks {
        if let Some(pending) = self.pending.take() { self.end_frame(pending); }
        let live: Vec<u32> = self.live.keys().copied().collect();
        for id in live { self.end_track(id); }
        if !self.block.is_empty() { self.blocks.push(std::mem::take(&mut self.block).into_boxed_slice()); }
        self.long.sort_unstable_by_key(|t| t.id);
        let mut by_start: Vec<u32> = (0..self.long.len() as u32).collect();
        by_start.sort_unstable_by_key(|i| self.long[*i as usize].start);
        Tracks { setup, entries: self.entries, blocks: self.blocks, long: self.long, by_start, cache: Default::default() }
    }
}

impl Tracks {
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// The memory it takes, bytes
    pub fn size(&self) -> usize {
        self.blocks.iter().map(|b| b.len()).sum::<usize>() + self.entries.len() * std::mem::size_of::<Entry>() +
            self.long.len() * (std::mem::size_of::<TrackInfo>() + 4)
    }

    /// Long enough to count and to be shown, see `MIN_TRACK_PAIRS`
    fn track(&self, id: u32) -> Option<&TrackInfo> {
        self.long.binary_search_by_key(&id, |t| t.id).ok().map(|i| &self.long[i])
    }

    pub fn start_of(&self, id: u32) -> Option<TrackStart> { self.track(id).map(|t| t.start) }

    /// The tracks that start where one of `starts` does
    pub fn ids_of<'a>(&self, starts: impl IntoIterator<Item = &'a TrackStart>) -> HashSet<u32> {
        starts.into_iter().filter_map(|s| {
            let i = self.by_start.binary_search_by(|i| self.long[*i as usize].start.cmp(s)).ok()?;
            Some(self.long[self.by_start[i] as usize].id)
        }).collect()
    }

    fn track_size(&self) -> (f32, f32) {
        (self.setup.track_size.0.max(1) as f32, self.setup.track_size.1.max(1) as f32)
    }

    /// The frames of a block, and the frame before it when the block's first frame goes on from it
    fn decode(&self, block: usize) -> (Option<CodedFrame>, Vec<CodedFrame>) {
        let mut r = varint::Reader::new(&self.blocks[block]);
        let before = (r.get() == 1).then(|| decode_absolute(&mut r));
        let entries = &self.entries[block * BLOCK_FRAMES..((block + 1) * BLOCK_FRAMES).min(self.entries.len())];
        let mut frames: Vec<CodedFrame> = Vec::with_capacity(entries.len());
        for e in entries {
            let f = match frames.last().or(before.as_ref()) {
                Some(prev) if e.continued > 0 => decode_relative(&mut r, prev),
                _ => decode_absolute(&mut r),
            };
            debug_assert!(f.ids.len() >= e.continued as usize);
            frames.push(f);
        }
        (before, frames)
    }

    /// `decode`, through `cache`
    fn decoded(&self, block: usize) -> Arc<Vec<CodedFrame>> {
        if let Some((b, frames)) = &*self.cache.lock() {
            if *b == block { return frames.clone(); }
        }
        let frames = Arc::new(self.decode(block).1);
        *self.cache.lock() = Some((block, frames.clone()));
        frames
    }

    /// The points of the entry `e` (decoded: `f`), relative to the frame, but the short tracks': their track, and where
    /// their state is (`PackedStates`): in the pair starting there for the ones that go on, else in the pair ending there
    fn points_of(&self, e: usize, f: &CodedFrame) -> Vec<FramePoint> {
        let entry = &self.entries[e];
        let next_from = self.entries.get(e + 1).filter(|n| n.continued > 0).map(|n| n.states_from);
        let (w, h) = self.track_size();
        let mut rank = 0;
        let mut out = Vec::with_capacity(f.ids.len());
        for (j, (id, q)) in f.ids.iter().zip(&f.pos).enumerate() {
            let state = if f.next[j] {
                rank += 1;
                next_from.map(|s| s + rank - 1)
            } else {
                (j < entry.continued as usize).then_some(entry.states_from + j as u64)
            };
            if self.track(*id).is_none() { continue; }
            let p = point_from_fixed(*q);
            out.push(FramePoint { id: *id, pos: (p[0] / w, p[1] / h), state: state.map(|s| s as usize) });
        }
        out
    }

    /// The points in the frame `index`, see `points_of`
    fn frame_points(&self, index: usize) -> Vec<FramePoint> {
        let Ok(e) = self.entries.binary_search_by_key(&index, |e| e.frame.index) else { return Vec::new() };
        let frames = self.decoded(e / BLOCK_FRAMES);
        self.points_of(e, &frames[e % BLOCK_FRAMES])
    }

    /// The points of the tracks `keep` in each of `frames`, placed by `place` (a frame and its points relative to it in,
    /// where they are out, None for nowhere): as tracked, or in the stabilized picture (`to_output`). A block at a time,
    /// in parallel; None once `cancelled`
    fn placed_points(&self, frames: RangeInclusive<usize>, keep: impl Fn(u32) -> bool + Sync, place: impl Fn(usize, &[(f32, f32)]) -> Vec<Option<(f32, f32)>> + Sync, cancelled: impl Fn() -> bool + Sync) -> Option<Vec<(usize, Vec<(u32, f32, f32)>)>> {
        let first = self.entries.partition_point(|e| e.frame.index < *frames.start());
        let end = self.entries.partition_point(|e| e.frame.index <= *frames.end());
        if first >= end { return Some(Vec::new()); }
        let per_block: Option<Vec<Vec<(usize, Vec<(u32, f32, f32)>)>>> = (first / BLOCK_FRAMES..=(end - 1) / BLOCK_FRAMES).into_par_iter().map(|block| {
            if cancelled() { return None; }
            let decoded = self.decode(block).1;
            let from = block * BLOCK_FRAMES;
            Some(decoded.iter().enumerate().map(|(i, f)| (from + i, f)).filter(|(e, _)| (first..end).contains(e)).map(|(e, f)| {
                let points: Vec<FramePoint> = self.points_of(e, f).into_iter().filter(|p| keep(p.id)).collect();
                let index = self.entries[e].frame.index;
                let placed = place(index, &points.iter().map(|p| p.pos).collect::<Vec<_>>());
                (index, points.iter().zip(placed).filter_map(|(p, at)| at.map(|(x, y)| (p.id, x, y))).collect())
            }).collect())
        }).collect();
        Some(per_block?.into_iter().flatten().collect())
    }

    /// The tracks (but `skip`) with a point within `rect` (x0, y0, x1, y1, relative to the frame) in any of `frames`,
    /// placed by `place`, see `placed_points`. None once `cancelled`
    pub fn in_rect(&self, frames: RangeInclusive<usize>, rect: [f32; 4], skip: &HashSet<u32>, place: impl Fn(usize, &[(f32, f32)]) -> Vec<Option<(f32, f32)>> + Sync, cancelled: impl Fn() -> bool + Sync) -> Option<Vec<u32>> {
        let inside = |x: f32, y: f32| x >= rect[0] && x <= rect[2] && y >= rect[1] && y <= rect[3];
        let ids: BTreeSet<u32> = self.placed_points(frames, |id| !skip.contains(&id), place, cancelled)?.into_iter()
            .flat_map(|(_, points)| points.into_iter().filter(|(_, x, y)| inside(*x, *y)).map(|(id, ..)| id))
            .collect();
        Some(ids.into_iter().collect())
    }

    /// Where the tracks `ids` are in `frames`, placed by `place` (see `placed_points`): per track, its frames in order and
    /// where it is in each
    pub fn paths(&self, ids: &HashSet<u32>, frames: RangeInclusive<usize>, place: impl Fn(usize, &[(f32, f32)]) -> Vec<Option<(f32, f32)>> + Sync) -> HashMap<u32, Vec<(usize, f32, f32)>> {
        let mut out: HashMap<u32, Vec<(usize, f32, f32)>> = HashMap::new();
        for (f, points) in self.placed_points(frames, |id| ids.contains(&id), place, || false).unwrap_or_default() {
            for (id, x, y) in points { out.entry(id).or_default().push((f, x, y)); }
        }
        out
    }

    /// Every frame pair, in order, as the analysis measured it, until `f` breaks
    fn for_each_pair(&self, mut f: impl FnMut(Pair) -> ControlFlow<()>) {
        let mut seq = 0;
        for block in 0..self.blocks.len() {
            let (before, frames) = self.decode(block);
            let from = block * BLOCK_FRAMES;
            for (i, b) in frames.iter().enumerate() {
                if self.entries[from + i].continued == 0 { continue; }
                let Some(a) = (if i == 0 { before.as_ref() } else { frames.get(i - 1) }) else { continue };
                let from_a = a.next.iter().enumerate().filter(|(_, n)| **n).map(|(j, _)| j);
                let obs = from_a.zip(b.ids.iter().zip(&b.pos)).map(|(j, (id, q))| {
                    debug_assert_eq!(a.ids[j], *id);
                    Observation { id: *id, a: point_from_fixed(a.pos[j]), b: point_from_fixed(*q) }
                }).collect();
                if f(Pair { seq, a: self.entries[from + i - 1].frame, b: self.entries[from + i].frame, obs }).is_break() { return; }
                seq += 1;
            }
        }
    }
}

/// Points placed where they were tracked, relative to the frame: see `Tracks::placed_points`
pub fn as_tracked(_: usize, points: &[(f32, f32)]) -> Vec<Option<(f32, f32)>> { points.iter().map(|p| Some(*p)).collect() }

/// A point in one frame, see `Tracks::points_of`
struct FramePoint {
    id: u32,
    pos: (f32, f32),
    /// Where in `PackedStates` what became of it is
    state: Option<usize>,
}

/// Where points of the input frame `index` (relative to it) are in the stabilized picture `params` render (relative to
/// it): the render's own point path, each point at its own row's time, with the zoom and the lens correction. None
/// where the lens has no image of a point. `params` the right way up, like `measurement_params`, for upright positions
pub fn to_output(params: &ComputeParams, index: usize, points: &[(f32, f32)]) -> Vec<Option<(f32, f32)>> {
    if points.is_empty() { return Vec::new(); }
    let ts = crate::timestamp_at_frame(index as i32, params.scaled_fps);
    let (w, h) = (params.width as f32, params.height as f32);
    let px: Vec<(f32, f32)> = points.iter().map(|p| (p.0 * w, p.1 * h)).collect();
    let amount = params.keyframes.value_at_video_timestamp(&crate::KeyframeType::LensCorrectionStrength, ts).unwrap_or(params.lens_correction_amount);
    let (ow, oh) = (params.output_width.max(1) as f32, params.output_height.max(1) as f32);
    crate::stabilization::undistort_points_with_rolling_shutter(&px, ts, Some(index), params, amount, true, false).into_iter()
        .map(|p| (p.0.is_finite() && p.1.is_finite() && p.0 > -99999.0).then(|| (p.0 / ow, p.1 / oh)))
        .collect()
}

/// The solver's parameters for the user's settings. The strength moves the hand-over to the motion data (the ridge,
/// relative to what the image measured) over three decades, from only the fast errors of a vibrating gyro up to the
/// image overriding the motion data down to about a tenth of a Hz. The correction works per row unless asked for one
/// per frame (`OpticalCorrectionSettings::per_frame`): both kinds of damage seen (vibration, a gyro whose gain
/// collapses during a roll) change within a frame, and one correction per frame did worse on every clip tried. The
/// same band measurements serve both, each at its own rows' times: per frame only spaces the control points further
pub fn solver_params(settings: &OpticalCorrectionSettings, scaled_fps: f64) -> SolverParams {
    let per_frame = if settings.per_frame { 1.0 } else { BANDS as f64 };
    SolverParams {
        spacing_us: 1_000_000.0 / scaled_fps.max(1.0) / per_frame,
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
        tracking: m.tracking,
        frames: m.frames,
        measured_frames: m.measured_frames,
        rms_deg,
    })
}

impl OpticalMeasurements {
    /// Measures the same tracks again, without the ones `excluded`: everything the analysis does after the tracking, a
    /// small part of its time. `cancelled` is asked every so often, and stops it ("Cancelled")
    pub fn remeasure(&self, excluded: HashSet<u32>, cancelled: impl Fn() -> bool) -> Result<OpticalMeasurements, String> {
        let mut m = Measuring::new(self.tracks.setup.clone(), Arc::new(excluded));
        let mut stopped = false;
        self.tracks.for_each_pair(|pair| {
            let n = pair.seq + 1;
            m.push(pair);
            stopped = n % CHUNK == 0 && cancelled();
            if stopped { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
        });
        if stopped { return Err("Cancelled".into()); }
        m.process(true);
        if cancelled() { return Err("Cancelled".into()); }
        if m.measurements.is_empty() { return Err("Not enough of the image could be tracked".into()); }
        let (_, measured) = m.finish(self.quats_checksum);
        Ok(OpticalMeasurements {
            bands: measured.bands,
            scaled_fps: self.scaled_fps,
            quats_checksum: measured.quats_checksum,
            context_checksum: self.context_checksum,
            video_base: measured.video_base,
            tracking: self.tracking,
            frames: self.frames,
            measured_frames: measured.measured_pairs,
            generation: self.generation,
            tracks: self.tracks.clone(),
            excluded: measured.excluded,
            states: measured.states,
        })
    }

    /// The points tracked in the frame `index` (numbered as `frame_at_timestamp` numbers them): their track, where they
    /// are relative to the frame (0 to 1), and what became of them
    pub fn points_at(&self, index: usize) -> Vec<(u32, f32, f32, PointState)> {
        self.tracks.frame_points(index).into_iter().map(|p| {
            let state = if self.excluded.contains(&p.id) { PointState::Removed } else {
                p.state.and_then(|s| self.states.get(s)).unwrap_or(PointState::Unused)
            };
            (p.id, p.pos.0, p.pos.1, state)
        }).collect()
    }
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
    fps_scale: Option<f64>,
    scaled_fps: f64,
    quats_checksum: u64,
    context_checksum: u64,
    #[cfg(feature = "use-opencv")]
    tracker: tracker::Tracker,
    tracking: TrackingMethod,
    last: Option<Frame>,
    /// Every frame pair tracked so far, kept, see `Tracks`
    tracks: TracksBuilder,
    measuring: Measuring,
    frames: usize,
    total_frames: usize,
    /// The part of the clip analyzed: the trim ranges, in the file's own milliseconds
    ranges_ms: Vec<(f64, f64)>,
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
            let from_video = !gyro.file_metadata.read().has_motion();
            if gyro.optical_correction.take().is_some() {
                gyro.integrate();
            }
            if from_video {
                gyro.quaternions.clear();
            } else if gyro.quaternions.len() < 2 {
                return Err("No motion data to correct".into());
            }
            let quats_checksum = optical_correction::checksum(&gyro.quaternions);
            params.gyro = Arc::new(parking_lot::RwLock::new(gyro));
            let tracking = *stab.optical_tracking.read();
            let context_checksum = context_checksum(&params);

            let (fps_scale, scaled_fps, horizontal_readout, ranges_ms, total_frames) = {
                let p = stab.params.read();
                // Only what's going to be exported: the trim ranges, or the whole clip without any
                let ranges: Vec<(f64, f64)> = if p.trim_ranges.is_empty() { vec![(0.0, 1.0)] } else { p.trim_ranges.clone() };
                let ranges_ms = ranges.iter().map(|(a, b)| (a * p.duration_ms, b * p.duration_ms)).collect::<Vec<_>>();
                let total_frames = ranges.iter().map(|(a, b)| ((b - a) * p.frame_count as f64).round() as usize).sum();
                (p.fps_scale, p.get_scaled_fps(), p.frame_readout_direction.is_horizontal(), ranges_ms, total_frames)
            };
            let setup = Setup { params, horizontal_readout, track_size: (0, 0), focal_px: 0.0, from_video };
            Ok(Self {
                fps_scale, scaled_fps, quats_checksum, context_checksum,
                tracker: tracker::Tracker::new(MAX_POINTS, tracking).map_err(|e| format!("OpenCV error: {e:?}"))?,
                tracking,
                last: None,
                tracks: Default::default(),
                measuring: Measuring::new(setup, Default::default()),
                frames: 0,
                total_frames,
                ranges_ms,
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

        let setup = &mut self.measuring.setup;
        if setup.track_size != (width, height) {
            setup.track_size = (width, height);
            let (k, ..) = FrameTransform::get_lens_data_at_timestamp(&setup.params, ts_ms, false);
            setup.focal_px = k[(0, 0)] * width as f64 / setup.params.width.max(1) as f64;
        }
        let frame = self.measuring.frame(index, ts_ms);
        let continuous = self.last.map(|l| index == l.index + 1).unwrap_or(false);

        #[cfg(feature = "use-opencv")]
        {
            if !continuous { self.tracker.reset(); }
            let obs = self.tracker.track(width, height, stride, pixels).map_err(|e| format!("OpenCV error: {e:?}"))?;
            if continuous && !obs.is_empty() {
                if let Some(a) = self.last {
                    // Measured as it's kept: measuring the kept tracks again gives the very same
                    let pair = self.tracks.push(a, frame, obs, self.measuring.setup.track_size);
                    self.measuring.push(pair);
                }
            }
        }
        #[cfg(not(feature = "use-opencv"))]
        { let _ = (continuous, stride, pixels); }

        self.last = Some(frame);
        self.frames += 1;
        Ok(())
    }

    /// Measures everything tracked so far, however recent
    pub fn flush(&mut self) { self.measuring.process(true); }

    /// The band measurements taken so far: all of them after `flush`
    pub fn measurements(&self) -> &[BandMeasurement] { &self.measuring.measurements }

    /// Measures what's left. The correction is then `solve`d from the measurements, as many times as the settings change
    pub fn finish(mut self) -> Result<OpticalMeasurements, String> {
        if self.is_cancelled() { return Err("Cancelled".into()); }
        self.measuring.process(true);
        if self.is_cancelled() { return Err("Cancelled".into()); }
        if self.measuring.measurements.is_empty() { return Err("Not enough of the image could be tracked".into()); }
        let points = self.tracks.observations;
        let (setup, measured) = self.measuring.finish(self.quats_checksum);
        let tracks = self.tracks.finish(setup);
        ::log::info!("Optical analysis: {} frames, {} measured pairs, {} band measurements, {} tracked points kept in {:.1} MB{}", self.frames, measured.measured_pairs, measured.bands.len(),
            points, tracks.size() as f64 / 1e6, if tracks.setup.from_video { ", motion from the video" } else { "" });
        Ok(OpticalMeasurements {
            bands: measured.bands,
            scaled_fps: self.scaled_fps,
            quats_checksum: measured.quats_checksum,
            context_checksum: self.context_checksum,
            video_base: measured.video_base,
            tracking: self.tracking,
            frames: self.frames,
            measured_frames: measured.measured_pairs,
            generation: self.generation.1,
            tracks: Arc::new(tracks),
            excluded: measured.excluded,
            states: measured.states,
        })
    }
}

/// What `Measuring` measured
struct Measured {
    bands: Vec<BandMeasurement>,
    quats_checksum: u64,
    video_base: Vec<(i64, [f32; 4])>,
    measured_pairs: usize,
    excluded: Arc<HashSet<u32>>,
    states: PackedStates,
}

/// Turns tracked frame pairs into band measurements as they come: everything the analysis does after the tracking, and
/// all that measuring the same tracks again without some of them takes. Holds only the pairs it still looks at
struct Measuring {
    setup: Setup,
    /// Tracks left out: none of their points count
    excluded: Arc<HashSet<u32>>,
    /// The pairs from `window_from` on: as far back as the high-pass and the chained rotations still look
    window: Vec<Pair>,
    window_from: usize,
    /// Pairs so far
    pairs: usize,
    /// A file without motion data: the rotation between each two frames, measured from their tracks and chained, at the
    /// frames' own times. What the rest of the analysis compares the image against instead of the quaternions
    vision: Option<TimeQuat>,
    /// What measures those rotations: the translation and the tracks' depths it keeps from one frame pair to the next
    odometry: odometry::VisualOdometry,
    /// The pairs the rotations were measured for so far: they wait until it's known which of their tracks are too short
    /// to count, `MIN_TRACK_PAIRS` - 1 pairs later
    chained: usize,
    /// ... with too few long tracks, from all of them
    chained_with_short: usize,
    /// The frame pairs each track was in so far, for as long as the rotations aren't chained past it
    lengths: HashMap<u32, u32>,
    /// The first pair the next measurement looks at: as far back as the high-pass reaches
    held_from: usize,
    measured_upto: usize,
    measurements: Vec<BandMeasurement>,
    measured_pairs: usize,
    /// What became of each point of each pair measured, for "Show tracked points", and where each pair's start
    states: PackedStates,
    states_from: Vec<u64>,
    observations: u64,
    sg_cache: HashMap<usize, DMatrix<f64>>,
}

impl Measuring {
    fn new(setup: Setup, excluded: Arc<HashSet<u32>>) -> Self {
        let vision = setup.from_video.then(TimeQuat::new);
        Self {
            setup, excluded, vision,
            window: Vec::new(),
            window_from: 0,
            pairs: 0,
            odometry: Default::default(),
            chained: 0,
            chained_with_short: 0,
            lengths: HashMap::new(),
            held_from: 0,
            measured_upto: 0,
            measurements: Vec::new(),
            measured_pairs: 0,
            states: Default::default(),
            states_from: Vec::new(),
            observations: 0,
            sg_cache: HashMap::new(),
        }
    }

    fn frame(&self, index: usize, timestamp_ms: f64) -> Frame {
        let s = &self.setup;
        frame_timing(&s.params, index, timestamp_ms, if s.horizontal_readout { s.track_size.0 } else { s.track_size.1 })
    }

    fn pair(&self, seq: usize) -> &Pair { &self.window[seq - self.window_from] }

    /// The next pair tracked
    fn push(&mut self, pair: Pair) {
        debug_assert_eq!(pair.seq, self.pairs);
        self.states_from.push(self.observations);
        self.observations += pair.obs.len() as u64;
        if self.vision.is_some() {
            for o in &pair.obs { *self.lengths.entry(o.id).or_insert(0) += 1; }
        }
        self.window.push(pair);
        self.pairs += 1;
        // A track in a pair is as long as it'll get or long enough `MIN_TRACK_PAIRS` - 1 pairs later
        while self.vision.is_some() && self.chained + MIN_TRACK_PAIRS as usize <= self.pairs {
            self.chain_vision();
        }
        self.process(false);
    }

    /// The pairs ready to be measured: all of them with the motion data to measure against, the ones the rotation was
    /// measured for without
    fn ready(&self) -> usize {
        if self.vision.is_some() { self.chained } else { self.pairs }
    }

    /// What it measured, and its setup back. For a file without motion data, with the orientation measured against and
    /// the checksum of the quaternions `integrate` makes of it, otherwise `quats_checksum`
    fn finish(self, quats_checksum: u64) -> (Setup, Measured) {
        if self.vision.is_some() { ::log::info!("Optical analysis: the rotation of {} frame pairs measured, {} of them with short tracks too", self.chained, self.chained_with_short); }
        let (quats_checksum, video_base) = match &self.vision {
            Some(keys) => {
                let base: Vec<(i64, [f32; 4])> = keys.iter().map(|(t, q)| (*t, [q.w as f32, q.i as f32, q.j as f32, q.k as f32])).collect();
                (optical_correction::checksum(&optical_correction::base_quats(&base)), base)
            },
            None => (quats_checksum, Vec::new()),
        };
        (self.setup, Measured {
            bands: self.measurements,
            quats_checksum,
            video_base,
            measured_pairs: self.measured_pairs,
            excluded: self.excluded,
            states: self.states,
        })
    }

    /// Unit bearings of tracked points of a frame, in the quaternions' frame
    fn bearings(&self, pts: &[(f32, f32)], frame: &Frame) -> Vec<Option<Vector3<f64>>> {
        undistort_points_for_optical_motion(pts, frame.timestamp_ms, frame.index, &self.setup.params, self.setup.track_size)
            .into_iter()
            .map(|b| b.map(to_quat_frame))
            .collect()
    }

    /// Extends the orientation of a file without motion data by the next frame pair, by the rotation between its
    /// frames `odometry` finds - the translation's parallax told apart from it. Only the slow part of this has to be
    /// right, and only roughly - what's left of a drift ends up in what the stabilization smooths away - since the
    /// correction measured against it takes care of everything faster, per row
    fn chain_vision(&mut self) {
        let seq = self.chained;
        self.chained += 1;
        let pair = self.pair(seq);
        let (frame_a, frame_b) = (pair.a, pair.b);
        let (a, b) = (frame_a.mid_ms, frame_b.mid_ms);
        let mut obs: Vec<Observation> = pair.obs.iter().filter(|o| !self.excluded.contains(&o.id)).copied().collect();
        // Not the short tracks, unless there's too little else: a pair measured from no points at all is a pair the
        // orientation doesn't turn over, which in a fast turn is all of the turn
        let long: Vec<Observation> = obs.iter().copied().filter(|o| self.lengths.get(&o.id).is_some_and(|n| *n >= MIN_TRACK_PAIRS)).collect();
        if long.len() >= MIN_BAND_POINTS * 2 { obs = long; } else { self.chained_with_short += 1; }
        // The tracks that end in this pair are chained as far as they go: their lengths aren't needed any more
        if seq + 1 < self.pairs {
            let next: HashSet<u32> = self.pair(seq + 1).obs.iter().map(|o| o.id).collect();
            let ended: Vec<u32> = self.pair(seq).obs.iter().map(|o| o.id).filter(|id| !next.contains(id)).collect();
            for id in ended { self.lengths.remove(&id); }
        }
        let pts_a: Vec<(f32, f32)> = obs.iter().map(|o| (o.a[0], o.a[1])).collect();
        let pts_b: Vec<(f32, f32)> = obs.iter().map(|o| (o.b[0], o.b[1])).collect();
        let (ba, bb) = (self.bearings(&pts_a, &frame_a), self.bearings(&pts_b, &frame_b));
        let mut va = Vec::with_capacity(obs.len());
        let mut vb = Vec::with_capacity(obs.len());
        let mut ids = Vec::with_capacity(obs.len());
        for ((o, x), y) in obs.iter().zip(ba).zip(bb) {
            if let (Some(x), Some(y)) = (x, y) { va.push(x); vb.push(y); ids.push(o.id); }
        }
        let m = if va.len() >= MIN_BAND_POINTS {
            let m0 = robust_rotation(&va, &vb);
            self.odometry.step(&va, &vb, &ids, m0, 1.0 / self.setup.focal_px.max(1.0))
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
        // At the end, every track is as long as it gets
        while last_call && self.vision.is_some() && self.chained < self.pairs {
            self.chain_vision();
        }
        let ready = self.ready();
        let end = if last_call { ready } else { ready.saturating_sub(HP_MAX) };
        if end <= self.measured_upto || (!last_call && end < self.measured_upto + CHUNK) { return; }

        // Only what the orientation is known for: past that, the residuals would be the whole motion
        let derived = self.derive(&self.window[self.held_from - self.window_from..ready - self.window_from]);
        let rhp = self.high_pass(&derived);

        // (seq, band) -> the points
        let mut groups: HashMap<(usize, u8), Vec<usize>> = HashMap::new();
        for (i, d) in derived.iter().enumerate() {
            if d.seq >= self.measured_upto && d.seq < end && rhp[i].is_some() {
                groups.entry((d.seq, d.band)).or_default().push(i);
            }
        }
        let floor = SIGMA_FLOOR_PX / self.setup.focal_px.max(1.0);
        let gyro = self.setup.params.gyro.clone();
        let vision = &self.vision;
        let fits: Vec<(usize, BandMeasurement, &Vec<usize>, Vec<f64>)> = groups.par_iter().filter_map(|(&(seq, _), idx)| {
            let gyro = gyro.read();
            fit_band(&derived, &rhp, idx, floor, &gyro, vision).map(|(mut m, w)| { m.pair = seq; (seq, m, idx, w) })
        }).collect();

        // What became of each point, for "Show tracked points"
        debug_assert_eq!(self.states.len() as u64, self.states_from.get(self.measured_upto).copied().unwrap_or(self.observations));
        for seq in self.measured_upto..end {
            let n = self.pair(seq).obs.len();
            self.states.push_unused(n);
        }
        for (_, _, idx, w) in &fits {
            for (&i, &w) in idx.iter().zip(w) {
                let d = &derived[i];
                self.states.set(self.states_from[d.seq] as usize + d.k as usize, PointState::from_weight(w));
            }
        }

        let mut ms: Vec<(usize, BandMeasurement)> = fits.into_iter().map(|(seq, m, ..)| (seq, m)).collect();
        ms.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.ta_us.total_cmp(&b.1.ta_us)));
        let mut seqs: Vec<usize> = ms.iter().map(|(s, _)| *s).collect();
        seqs.sort_unstable();
        seqs.dedup();
        self.measured_pairs += seqs.len();
        self.measurements.extend(ms.into_iter().map(|(_, m)| m));

        self.measured_upto = end;
        self.held_from = end.saturating_sub(HP_MAX);
        // What nothing looks at any more (the chained rotations are past `held_from` too, see `ready`). In chunks, as the
        // pairs after it move down
        if self.held_from - self.window_from >= CHUNK {
            self.window.drain(..self.held_from - self.window_from);
            self.window_from = self.held_from;
        }
    }

    /// Residuals of every point of `pairs` (but the tracks left out), against the quaternions
    fn derive(&self, pairs: &[Pair]) -> Vec<Derived> {
        let params = &self.setup.params;
        let size = self.setup.track_size;
        let horizontal = self.setup.horizontal_readout;
        let track_rows = if horizontal { size.0 } else { size.1 }.max(1) as f32;
        let (vision, excluded) = (&self.vision, &self.excluded);
        let per_pair: Vec<Vec<Derived>> = pairs.par_iter().map(|pair| {
            let obs: Vec<(usize, &Observation)> = pair.obs.iter().enumerate().filter(|(_, o)| !excluded.contains(&o.id)).collect();
            let pts_a: Vec<(f32, f32)> = obs.iter().map(|(_, o)| (o.a[0], o.a[1])).collect();
            let pts_b: Vec<(f32, f32)> = obs.iter().map(|(_, o)| (o.b[0], o.b[1])).collect();
            let ba = undistort_points_for_optical_motion(&pts_a, pair.a.timestamp_ms, pair.a.index, params, size);
            let bb = undistort_points_for_optical_motion(&pts_b, pair.b.timestamp_ms, pair.b.index, params, size);
            let gyro = params.gyro.read();
            let orient = |t: f64| orientation(vision, &gyro, t);
            let mut out = Vec::with_capacity(obs.len());
            for (i, (k, o)) in obs.iter().enumerate() {
                let (Some(a), Some(b)) = (ba.get(i).copied().flatten(), bb.get(i).copied().flatten()) else { continue };
                let pos_a = if horizontal { o.a[0] } else { o.a[1] };
                let pos_b = if horizontal { o.b[0] } else { o.b[1] };
                let ta = pair.a.start_ms + pair.a.per_px_ms * pos_a as f64;
                let tb = pair.b.start_ms + pair.b.per_px_ms * pos_b as f64;
                let m = (orient(tb).inverse() * orient(ta)).to_rotation_matrix().into_inner();
                let (va, vb) = (to_quat_frame(a), to_quat_frame(b));
                let p = m * va;
                let band = ((pos_a / track_rows) * BANDS as f32).floor().clamp(0.0, (BANDS - 1) as f32) as u8;
                out.push(Derived { id: o.id, seq: pair.seq, k: *k as u32, band, p, r: vb - p, ta_ms: ta, tb_ms: tb });
            }
            out
        }).collect();
        per_pair.into_iter().flatten().collect()
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

/// One band of one frame pair: the rotation its points moved by beyond the quaternions, `r ≈ ρ × p`, robustly, and
/// the weight each point ended up with in it
fn fit_band(derived: &[Derived], rhp: &[Option<Vector3<f64>>], idx: &[usize], sigma_floor: f64, gyro: &GyroSource, vision: &Option<TimeQuat>) -> Option<(BandMeasurement, Vec<f64>)> {
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
    Some((BandMeasurement { pair: 0, ta_us: to_gyro_us(ta), tb_us: to_gyro_us(tb), rho, info, m }, w))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (u32, u32) = (960, 540);

    /// What a tracker gives over `pairs` frame pairs: points drifting with a shake and a little noise, some lost in every
    /// pair and topped up with new tracks, and the frame `gap` missing, after which it starts over. Its order: by track
    fn tracked(pairs: usize, gap: usize) -> Vec<(Frame, Frame, Vec<Observation>)> {
        let frame = |index: usize| Frame { index, timestamp_ms: index as f64 * 20.0, start_ms: index as f64 * 20.0 - 5.0, per_px_ms: 0.01, mid_ms: index as f64 * 20.0 };
        let mut seed = 0x2545f4914f6cdd1du64;
        let mut random = move || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; (seed >> 11) as f32 / (1u64 << 53) as f32 };
        let (mut points, mut id): (Vec<(u32, [f32; 2])>, u32) = (Vec::new(), 0);
        let mut out = Vec::new();
        let mut index = 0;
        while out.len() < pairs {
            if index == gap { index += 1; points.clear(); }
            while points.len() < 300 { points.push((id, [3.0 + random() * 950.0, 3.0 + random() * 530.0])); id += 1; }
            let shake = [(index as f32 * 0.7).sin() * 3.0 + 1.5, (index as f32 * 1.3).cos() * 2.0];
            let mut obs = Vec::new();
            for (i, a) in &points {
                let b = [a[0] + shake[0] + random() * 0.2, a[1] + shake[1] + random() * 0.2];
                if random() < 0.03 || b[0] < 3.0 || b[1] < 3.0 || b[0] > 956.0 || b[1] > 536.0 { continue; }
                obs.push(Observation { id: *i, a: *a, b });
            }
            points = obs.iter().map(|o| (o.id, o.b)).collect();
            out.push((frame(index), frame(index + 1), obs));
            index += 1;
        }
        out
    }

    fn kept(pairs: usize, gap: usize) -> (Vec<Pair>, Tracks) {
        let mut builder = TracksBuilder::default();
        let measured = tracked(pairs, gap).into_iter().map(|(a, b, obs)| builder.push(a, b, obs, SIZE)).collect();
        (measured, builder.finish(Setup { track_size: SIZE, ..Default::default() }))
    }

    #[test]
    fn kept_tracks_give_back_the_pairs_measured() {
        let original = tracked(300, 140);
        let (measured, tracks) = kept(300, 140);
        // What's measured is what was tracked, in fixed point
        for ((_, _, obs), pair) in original.iter().zip(&measured) {
            assert_eq!(obs.len(), pair.obs.len());
            for (o, m) in obs.iter().zip(&pair.obs) {
                assert_eq!(o.id, m.id);
                for (x, y) in o.a.iter().chain(&o.b).zip(m.a.iter().chain(&m.b)) {
                    assert!((x - y).abs() <= 0.5 / POSITION_STEPS, "{x} kept as {y}");
                }
            }
        }
        // ... and exactly what measuring the kept tracks again gets, across blocks and the gap
        assert!(tracks.blocks.len() > 3);
        let mut again = Vec::new();
        tracks.for_each_pair(|pair| { again.push(pair); ControlFlow::Continue(()) });
        assert_eq!(again.len(), measured.len());
        for (a, m) in again.iter().zip(&measured) {
            assert_eq!((a.seq, a.a, a.b), (m.seq, m.a, m.b));
            assert_eq!(a.obs, m.obs);
        }
        let points: usize = measured.iter().map(|p| p.obs.len()).sum();
        assert!(tracks.size() < points * 5, "{} bytes for {points} points", tracks.size());
    }

    #[test]
    fn every_frame_shows_its_long_tracks_where_they_are() {
        let (measured, tracks) = kept(200, 90);
        let mut lengths: HashMap<u32, u32> = HashMap::new();
        for o in measured.iter().flat_map(|p| &p.obs) { *lengths.entry(o.id).or_default() += 1; }
        let long = |id: u32| lengths[&id] >= MIN_TRACK_PAIRS;
        // Each point in a frame: in the pair starting there, or - lost right after - the one ending there
        let mut expected: HashMap<usize, Vec<(u32, (f32, f32), usize)>> = HashMap::new();
        let mut from = 0;
        let relative = |p: [f32; 2]| (p[0] / SIZE.0 as f32, p[1] / SIZE.1 as f32);
        for (s, pair) in measured.iter().enumerate() {
            let next: HashSet<u32> = measured.get(s + 1).filter(|n| n.a.index == pair.b.index).map(|n| n.obs.iter().map(|o| o.id).collect()).unwrap_or_default();
            for (k, o) in pair.obs.iter().enumerate().filter(|(_, o)| long(o.id)) {
                expected.entry(pair.a.index).or_default().push((o.id, relative(o.a), from + k));
                if !next.contains(&o.id) { expected.entry(pair.b.index).or_default().push((o.id, relative(o.b), from + k)); }
            }
            from += pair.obs.len();
        }
        let shown: usize = expected.values().map(|v| v.len()).sum();
        assert!(shown > 10_000);
        for index in 0..=210 {
            let mut got: Vec<(u32, (f32, f32), usize)> = tracks.frame_points(index).into_iter().map(|p| (p.id, p.pos, p.state.unwrap())).collect();
            let mut want = expected.remove(&index).unwrap_or_default();
            got.sort_by_key(|p| p.0);
            want.sort_by_key(|p| p.0);
            assert_eq!(got, want, "frame {index}");
        }
    }

    #[test]
    fn long_tracks_are_found_by_where_they_start() {
        let (measured, tracks) = kept(150, 1000);
        let ids: BTreeSet<u32> = measured.iter().flat_map(|p| p.obs.iter().map(|o| o.id)).collect();
        let (mut long, mut short) = (0, 0);
        for id in ids {
            match tracks.start_of(id) {
                Some(start) => { long += 1; assert_eq!(tracks.ids_of([start].iter()), HashSet::from([id])); },
                None => short += 1,
            }
        }
        assert!(long > 100 && short > 100, "{long} long, {short} short");
    }

    #[test]
    fn a_box_over_every_frame_finds_what_each_frame_shows_and_stops_when_asked() {
        let (_, tracks) = kept(250, 120);
        let rect = [0.2, 0.3, 0.45, 0.6];
        let mut want = BTreeSet::new();
        for index in 0..=260 {
            for p in tracks.frame_points(index) {
                if p.pos.0 >= rect[0] && p.pos.0 <= rect[2] && p.pos.1 >= rect[1] && p.pos.1 <= rect[3] { want.insert(p.id); }
            }
        }
        let got = tracks.in_rect(0..=usize::MAX, rect, &HashSet::new(), as_tracked, || false).unwrap();
        assert_eq!(got, want.into_iter().collect::<Vec<_>>());
        assert_eq!(tracks.in_rect(0..=usize::MAX, rect, &HashSet::new(), as_tracked, || true), None);
    }

    #[test]
    fn packed_states_keep_each_point_apart() {
        let mut states = PackedStates::default();
        states.push_unused(7);
        states.set(0, PointState::Used);
        states.set(5, PointState::Rejected);
        states.push_unused(3);
        states.set(9, PointState::Downweighted);
        let got: Vec<Option<PointState>> = (0..11).map(|i| states.get(i)).collect();
        use PointState::*;
        assert_eq!(got, [Some(Used), Some(Unused), Some(Unused), Some(Unused), Some(Unused), Some(Rejected), Some(Unused), Some(Unused), Some(Unused), Some(Downweighted), None]);
    }
}
