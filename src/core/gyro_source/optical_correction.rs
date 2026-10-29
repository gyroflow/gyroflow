// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! The motion data correction measured from the video itself ("Analyze image optically", see
//! `synchronization::optical_motion`).
//!
//! Some cameras record motion data that is right at low frequencies and wrong above a few Hz: a hard-mounted DJI O4
//! picks up motor vibration as rotation in two of its axes and misses part of the real one, and nothing in the
//! metadata says so. The image does. The correction is the small rotation that makes the quaternions agree with it,
//! `q' = q * exp(δ(t))`, with δ in the quaternions' own frame on the motion data timeline, stored as a uniform cubic
//! B-spline. It's measured against one particular set of quaternions, so it only applies to those: anything that
//! changes them (another integration method, a filter, the IMU orientation) makes it stale, see `quats_checksum`. And
//! against one particular picture of them - which pixel was read out when, and which ray it came from: another sync,
//! lens profile or rolling shutter time makes it stale too, see `context_checksum`
//!
//! A video cut from recordings is corrected part by part (`OpticalCorrection::parts`): each part's correction is fitted
//! to the measurements of its own frames alone and applies to its own motion data alone, as the part trimmed on its own
//! would have it. A spline over a boundary would carry one moment's correction into another's frames

use std::ops::Range;
use nalgebra::{ UnitQuaternion, Vector3 };
use super::TimeQuat;
use crate::stabilization::{ ComputeParams, FrameTransform };
use crate::synchronization::optical_motion::{ self, OpticalMeasurements, solver::{ self, BandMeasurement } };

/// What only the user can judge: nothing in the metadata says how far the motion data can be trusted
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct OpticalCorrectionSettings {
    /// How far the correction may take the motion data from what it says, 0 to 1. Low keeps it to the small, fast
    /// errors of a vibrating gyro; high lets the image override the motion data down to slow motion and by degrees - a
    /// gyro whose gain collapses during a roll, glitches - at the cost of following the image's own mistakes too
    pub strength: f64,
    /// One correction per frame instead of one per band of rows: it can't follow what changes within a frame
    /// (vibration), but doesn't need every band of rows measured either - for a global shutter, too little texture
    /// to track in parts of the frame, or a rolling shutter time that's wrong or unknown
    pub per_frame: bool,
}
impl Default for OpticalCorrectionSettings {
    fn default() -> Self { Self { strength: 0.5, per_frame: false } }
}

/// How the analysis follows the points from one frame to the next. Unlike the settings above it takes another
/// analysis: what it changes is what's measured
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackingMethod {
    /// Pyramidal Lucas-Kanade at corners: the most accurate where there's texture to track
    #[default]
    Klt,
    /// Dense optical flow (DIS), sampled at the same points: a flow everywhere, also where corners are few and weak
    /// (sky, water, fog), at a few times the time
    Dis,
    /// Dense optical flow from gmflow (AI, Unimatch), sampled like DIS: a flow everywhere that holds up through large
    /// motion and weak detail, but coarse (the model sees 576x320) and slow, practical on a GPU. Only in builds with
    /// the `use-burn` feature, see `available`
    Gmflow,
}
impl TrackingMethod {
    /// None for a name it doesn't know, or for a method this build doesn't have
    pub fn from_name(name: &str) -> Option<Self> {
        let method = match name.to_ascii_lowercase().as_str() { "klt" => Self::Klt, "dis" => Self::Dis, "gmflow" => Self::Gmflow, _ => return None };
        method.available().then_some(method)
    }
    pub fn name(&self) -> &'static str {
        match self { Self::Klt => "klt", Self::Dis => "dis", Self::Gmflow => "gmflow" }
    }
    /// Whether this build can track with it
    pub fn available(&self) -> bool {
        match self { Self::Klt | Self::Dis => true, Self::Gmflow => cfg!(feature = "use-burn") }
    }
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct OpticalCorrection {
    /// Applied at all. Switched off rather than dropped, so the analysis survives toggling it
    pub enabled: bool,
    /// The settings it was fitted with
    pub settings: OpticalCorrectionSettings,
    /// Motion data timestamp of the first control point, microseconds
    pub start_us: f64,
    /// Distance between the control points, microseconds
    pub spacing_us: f64,
    /// Control points of δ: rotation vectors in the quaternions' frame, radians. Stored quantized: six of them per
    /// frame make a long clip's worth heavy for a project file
    #[serde(with = "quantized")]
    pub coeffs: Vec<[f32; 3]>,
    /// `checksum` of the quaternions the correction was measured against
    pub quats_checksum: u64,
    /// And of everything else it was measured with, see `synchronization::optical_motion::context_checksum`
    pub context_checksum: u64,
    /// For a file without motion data: the rotation the analysis measured between the frames, chained - the orientation
    /// the correction sits on. One per frame, (timestamp µs, w x y z); `integrate` uses it densified, see `base_quats`
    pub video_base: Vec<(i64, [f32; 4])>,
    /// How the analysis followed the points. Only to tell the user it changed since: the correction still applies, it's
    /// just measured the other way
    pub tracking: TrackingMethod,

    /// Frames the analysis tracked, and how many of them yielded a measurement
    pub frames: usize,
    pub measured_frames: usize,
    /// Rms of the correction where it was measured, degrees
    pub rms_deg: f64,
    /// For a video cut from recordings, the correction of each of its parts that has one (`solve_parts`), in order: a
    /// spline of its own on `coeffs`. Empty for a video of one piece, whose spline is all of `coeffs` from `start_us`
    pub parts: Vec<CorrectionPart>,
}

/// The correction of one part of a video cut from recordings (`OpticalCorrection::parts`): the motion data it applies to
/// (its timestamps, µs: the part's, `SegmentRanges::motion`), and its spline, `count` control points of
/// `OpticalCorrection::coeffs` from `first`, the first of them at `start_us`
#[derive(Default, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CorrectionPart {
    pub motion_us: (i64, i64),
    pub start_us: f64,
    pub first: usize,
    pub count: usize,
}

impl OpticalCorrection {
    /// δ at a motion data timestamp. Zero outside of the analyzed range: there the motion data is used as it is. In a
    /// video cut from recordings, the spline of the part whose motion data has the timestamp (`parts`), zero in a part
    /// without one
    pub fn at(&self, timestamp_us: f64) -> Vector3<f64> {
        if self.coeffs.is_empty() || self.spacing_us <= 0.0 { return Vector3::zeros(); }
        if self.parts.is_empty() { return spline_at(&self.coeffs, self.start_us, self.spacing_us, timestamp_us); }
        let t = timestamp_us.floor() as i64;
        match self.parts.iter().find(|p| t >= p.motion_us.0 && t < p.motion_us.1) {
            Some(p) => spline_at(self.coeffs.get(p.first..p.first + p.count).unwrap_or_default(), p.start_us, self.spacing_us, timestamp_us),
            None => Vector3::zeros(),
        }
    }

    /// The orientation of a file without motion data, as the renderer gets it: `video_base` at 1 kHz
    pub fn base_quats(&self) -> TimeQuat { base_quats(&self.video_base) }
    /// `base_quats` of a video cut from recordings, part by part as each part's range trimmed alone has it: each part's
    /// frames at 1 kHz on the grid of whole ms from the part's start (`parts`: each part's motion data, its timestamps
    /// in µs, and where the part starts on that timeline). Nothing between two parts' frames: the next part's frame is
    /// of another moment
    pub fn base_quats_in(&self, parts: &[(Range<i64>, i64)]) -> TimeQuat {
        let sparse = sparse_base(&self.video_base);
        let mut out = TimeQuat::new();
        for (motion, origin_us) in parts {
            out.append(&mut densify_from(&sparse.range(motion.clone()).map(|(t, q)| (*t, *q)).collect(), *origin_us));
        }
        out
    }

    /// Whether it was measured on (uncorrected) quaternions of this `checksum`, in this context (`context_checksum`)
    pub fn measured_on(&self, quats_checksum: u64, context_checksum: u64) -> bool {
        (!self.coeffs.is_empty() || self.video_base.len() >= 2)
            && self.quats_checksum == quats_checksum && self.context_checksum == context_checksum
    }

    pub fn apply(&self, quats: &mut TimeQuat) {
        for (ts, q) in quats.iter_mut() {
            let d = self.at(*ts as f64);
            if d != Vector3::zeros() {
                *q *= UnitQuaternion::from_scaled_axis(d);
            }
        }
    }

    pub fn hash_into(&self, hasher: &mut impl std::hash::Hasher) {
        hasher.write_u8(self.enabled as u8);
        hasher.write_u64(self.settings.strength.to_bits());
        hasher.write_u8(self.settings.per_frame as u8);
        hasher.write_u64(self.quats_checksum);
        hasher.write_u64(self.context_checksum);
        hasher.write_u64(self.start_us.to_bits());
        hasher.write_u64(self.spacing_us.to_bits());
        hasher.write_usize(self.video_base.len());
        for (t, q) in self.video_base.iter().step_by((self.video_base.len() / 256).max(1)) {
            hasher.write_i64(*t); hasher.write_u32(q[0].to_bits()); hasher.write_u32(q[1].to_bits()); hasher.write_u32(q[2].to_bits());
        }
        hasher.write_usize(self.coeffs.len());
        for c in self.coeffs.iter().step_by((self.coeffs.len() / 512).max(1)) {
            hasher.write_u32(c[0].to_bits()); hasher.write_u32(c[1].to_bits()); hasher.write_u32(c[2].to_bits());
        }
        for p in &self.parts {
            hasher.write_i64(p.motion_us.0); hasher.write_i64(p.motion_us.1); hasher.write_u64(p.start_us.to_bits()); hasher.write_usize(p.first); hasher.write_usize(p.count);
        }
    }
}

/// The uniform cubic B-spline of the control points `coeffs` (the first at `start_us`, `spacing_us` apart) at a motion
/// data timestamp, zero outside of them
fn spline_at(coeffs: &[[f32; 3]], start_us: f64, spacing_us: f64, timestamp_us: f64) -> Vector3<f64> {
    let u = (timestamp_us - start_us) / spacing_us;
    let (s, w) = bspline_weights(u);
    let mut v = Vector3::zeros();
    for (i, w) in w.iter().enumerate() {
        let k = s + i as i64 - 1;
        if k >= 0 && (k as usize) < coeffs.len() {
            let c = coeffs[k as usize];
            v += Vector3::new(c[0] as f64, c[1] as f64, c[2] as f64) * *w;
        }
    }
    v
}

/// The parts of a video cut from recordings (`GyroSource::segment_ranges`), as the measurements of an analysis see them
/// (`solve_parts`): each part's motion data, and the motion data times each of its frames is read out over
#[derive(Clone, Debug)]
pub struct MeasuredParts {
    parts: Vec<MeasuredPart>,
}

/// A part of `MeasuredParts`: its motion data's timestamps (µs), and its frames' readouts, each from its first row's
/// time to its last one's (µs, the motion data's), in order
#[derive(Clone, Debug)]
struct MeasuredPart {
    motion: Range<i64>,
    readouts: Vec<(f64, f64)>,
}

impl MeasuredParts {
    /// How far a band's time can be off the span of its frame's readout, µs: the rounding of the times to µs
    const TOLERANCE_US: f64 = 1.0;

    /// The parts of the video `params` measure (`optical_motion::measurement_params`), `None` for a video of one piece.
    /// A frame is read out over its rows as the analysis and the render have it (`FrameTransform::at_timestamp`): from
    /// its time with its per-frame offset less half its readout time to that plus half of it, in the motion data's time
    /// less its part's sync offset (`GyroSource::lookup_for_frame`). Takes the `gyro` lock
    pub fn of(params: &ComputeParams) -> Option<Self> {
        let gyro = params.gyro.read();
        // Motion data of one piece (`SegmentRanges::whole`) is measured as one
        let segments = gyro.segment_ranges().filter(|s| !s.whole)?;
        let md = gyro.file_metadata.read();
        let parts = (0..segments.len()).map(|r| {
            let frames = segments.frames(r);
            let motion_us = |t_ms: f64| (t_ms - gyro.offset_at_video_timestamp_in(t_ms, Some(r))) * 1000.0;
            let spans = frames.map(|f| {
                let timestamp_ms = params.timestamp_at_frame(f as i32);
                let readout = FrameTransform::get_frame_readout_time(params, false, timestamp_ms, &md);
                let mid = timestamp_ms + md.frame_time_offset(f);
                let (a, b) = (motion_us(mid - readout / 2.0), motion_us(mid + readout / 2.0));
                (a.min(b) - Self::TOLERANCE_US, a.max(b) + Self::TOLERANCE_US)
            }).collect::<Vec<_>>();
            let mut readouts = spans;
            readouts.sort_by(|a, b| a.0.total_cmp(&b.0));
            MeasuredPart { motion: segments.motion[r].clone(), readouts }
        }).collect();
        Some(Self { parts })
    }

    /// The part a band measured between two frames is of: the one both of whose frames' readouts have its time in it.
    /// `None` for a band no part's frames tell, or two parts' (a junction where the parts' sync differs), which no part's
    /// correction can take
    fn part_of(&self, band: &BandMeasurement) -> Option<usize> {
        let read_in = |spans: &[(f64, f64)], t: f64| spans.get(spans.partition_point(|s| s.0 <= t).wrapping_sub(1)).is_some_and(|s| t <= s.1);
        let mut parts = self.parts.iter().enumerate().filter(|(_, p)| read_in(&p.readouts, band.ta_us) && read_in(&p.readouts, band.tb_us)).map(|(r, _)| r);
        match (parts.next(), parts.next()) {
            (Some(r), None) => Some(r),
            _ => None,
        }
    }
}

/// Fits the correction to the measurements (`optical_motion::solve`), each part of a video cut from recordings on its
/// own: its spline is fitted to the bands measured between its own frames (`MeasuredParts::part_of`), and applies to its
/// own motion data. A part without enough of them measured has no correction, as the same frames trimmed on their own
/// wouldn't have one
pub fn solve_parts(m: &OpticalMeasurements, settings: &OpticalCorrectionSettings, parts: &MeasuredParts) -> Result<OpticalCorrection, String> {
    let params = optical_motion::solver_params(settings, m.scaled_fps);
    let mut bands: Vec<Vec<BandMeasurement>> = vec![Vec::new(); parts.parts.len()];
    let mut left_out = 0;
    for b in &m.bands {
        match parts.part_of(b) {
            Some(r) => bands[r].push(b.clone()),
            None => left_out += 1,
        }
    }
    if left_out > 0 { ::log::info!("Optical correction: {left_out} of {} band measurements aren't of one part's frames, left out", m.bands.len()); }
    let fits = parts.parts.iter().zip(&bands).filter_map(|(MeasuredPart { motion, .. }, bands)| {
        Some((Some(motion.clone()), solver::solve(bands, &params)?, bands.as_slice()))
    }).collect();
    correction_of(m, settings, &params, fits)
}

/// The correction made of splines fitted to the measurements `m` with `params` (`solver::solve`), each with the bands it
/// was fitted to: one fitted to all of them, for all of the motion data (its timestamps `None`, `optical_motion::solve`),
/// or one for each part of a video cut from recordings that has one, with the timestamps (µs) of the part's motion data
/// (`solve_parts`). The one place a correction is put together, so the two can't come to differ in anything but that
pub fn correction_of(m: &OpticalMeasurements, settings: &OpticalCorrectionSettings, params: &solver::SolverParams, fits: Vec<(Option<Range<i64>>, solver::Solution, &[BandMeasurement])>) -> Result<OpticalCorrection, String> {
    let start_us = fits.first().ok_or_else(|| "Not enough of the image could be tracked".to_string())?.1.start_us;
    let (mut coeffs, mut parts, mut sum, mut count) = (Vec::new(), Vec::new(), 0.0, 0usize);
    for (motion, sol, bands) in &fits {
        sum += bands.iter().map(|b| sol.at(b.ta_us, params.spacing_us).norm_squared()).sum::<f64>();
        count += bands.len();
        if let Some(motion) = motion {
            parts.push(CorrectionPart { motion_us: (motion.start, motion.end), start_us: sol.start_us, first: coeffs.len(), count: sol.coeffs.len() });
        }
        coeffs.extend(sol.coeffs.iter().map(|c| [c.x as f32, c.y as f32, c.z as f32]));
    }
    Ok(OpticalCorrection {
        enabled: true,
        settings: *settings,
        start_us,
        spacing_us: params.spacing_us,
        coeffs,
        quats_checksum: m.quats_checksum,
        context_checksum: m.context_checksum,
        video_base: m.video_base.clone(),
        tracking: m.tracking,
        frames: m.frames,
        measured_frames: m.measured_frames,
        rms_deg: (sum / count.max(1) as f64).sqrt().to_degrees(),
        parts,
    })
}

/// Control points as 16-bit integers of a common scale: the full range of the largest one, which leaves the rest a
/// resolution far below a pixel
mod quantized {
    use serde::{ Deserialize, Deserializer, Serialize, Serializer };
    #[derive(Serialize, Deserialize)]
    struct Q { scale: f32, v: Vec<i16> }

    pub fn serialize<S: Serializer>(c: &Vec<[f32; 3]>, s: S) -> Result<S::Ok, S::Error> {
        let max = c.iter().flatten().fold(0.0f32, |m, v| m.max(v.abs()));
        let scale = if max > 0.0 { max / i16::MAX as f32 } else { 1.0 };
        Q { scale, v: c.iter().flatten().map(|v| (v / scale).round() as i16).collect() }.serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<[f32; 3]>, D::Error> {
        let q = Q::deserialize(d)?;
        Ok(q.v.chunks_exact(3).map(|c| [c[0] as f32 * q.scale, c[1] as f32 * q.scale, c[2] as f32 * q.scale]).collect())
    }
}

/// Uniform cubic B-spline at the knot coordinate `u`: the segment `s` and the weights of control points `s-1 ..= s+2`
pub fn bspline_weights(u: f64) -> (i64, [f64; 4]) {
    let s = u.floor();
    let f = u - s;
    let g = 1.0 - f;
    (s as i64, [
        g * g * g / 6.0,
        2.0 / 3.0 - f * f + f * f * f / 2.0,
        2.0 / 3.0 - g * g + g * g * g / 2.0,
        f * f * f / 6.0,
    ])
}

/// A `video_base` as the renderer gets it. The analysis fingerprints the base this way too, from the stored single
/// precision, or a rounding would make it look like other motion data
pub fn base_quats(video_base: &[(i64, [f32; 4])]) -> TimeQuat {
    densify(&sparse_base(video_base))
}
/// The orientations of a `video_base`, one at each of its frames
fn sparse_base(video_base: &[(i64, [f32; 4])]) -> TimeQuat {
    video_base.iter().map(|(t, q)| (*t, UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64)))).collect()
}

/// Orientations sampled at the frames, at 1 kHz: the geodesic between each two, so a lookup between the samples gives
/// what it would have between the frames
pub fn densify(sparse: &TimeQuat) -> TimeQuat {
    densify_from(sparse, 0)
}
/// `densify` on the grid of whole ms from `origin_us`
pub fn densify_from(sparse: &TimeQuat, origin_us: i64) -> TimeQuat {
    let mut out = TimeQuat::new();
    let mut it = sparse.iter().peekable();
    while let Some((&t0, q0)) = it.next() {
        out.insert(t0, *q0);
        if let Some(&(&t1, q1)) = it.peek() {
            let mut t = t0 - (t0 - origin_us).rem_euclid(1000) + 1000;
            while t < t1 {
                out.insert(t, q0.slerp(q1, (t - t0) as f64 / (t1 - t0) as f64));
                t += 1000;
            }
        }
    }
    out
}

/// `quats` held at their first and last orientation over the rest of the clip, `0..=duration_ms`, on the grid of
/// `densify`. The smoothing takes the sample rate from how many samples there are per the clip's duration: the motion
/// of an analyzed trim range alone smooths as if sampled at a fraction of its rate - next to not at all
pub fn hold_over_clip(quats: &mut TimeQuat, duration_ms: f64) {
    hold_from(quats, 0);
    hold_to(quats, 0, (duration_ms * 1000.0).round() as i64);
}
/// `quats` held at their first orientation back to `from_us`, on the grid of whole ms from it (`densify_from`)
pub fn hold_from(quats: &mut TimeQuat, from_us: i64) {
    let Some((&first, &q0)) = quats.first_key_value() else { return };
    let mut t = first - (first - from_us).rem_euclid(1000);
    if t == first { t -= 1000; }
    while t >= from_us { quats.insert(t, q0); t -= 1000; }
}
/// `quats` held at their last orientation up to `to_us`, on the grid of whole ms from `origin_us` (`densify_from`)
pub fn hold_to(quats: &mut TimeQuat, origin_us: i64, to_us: i64) {
    let Some((&last, &q1)) = quats.last_key_value() else { return };
    let mut t = last - (last - origin_us).rem_euclid(1000) + 1000;
    while t <= to_us { quats.insert(t, q1); t += 1000; }
}

/// Fingerprint of a set of quaternions. Stored in project files, so it's FNV-1a rather than std's hasher, whose
/// output may change between Rust releases, and it rounds the components, so the last bit of a filter computed on
/// another machine doesn't make a correction stale
pub fn checksum(quats: &TimeQuat) -> u64 {
    let mut h = Fnv::default();
    h.eat(quats.len() as u64);
    for (ts, q) in quats.iter().step_by((quats.len() / 1024).max(1)) {
        h.eat(*ts as u64);
        for v in q.coords.iter() {
            h.eat_rounded(*v, 1e6);
        }
    }
    h.0
}

/// FNV-1a over 64-bit words, for the fingerprints kept in project files
pub struct Fnv(pub u64);
impl Default for Fnv {
    fn default() -> Self { Self(0xcbf29ce484222325) }
}
impl Fnv {
    pub fn eat(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    /// `v` in whole units of `1 / scale`
    pub fn eat_rounded(&mut self, v: f64, scale: f64) {
        self.eat((v * scale).round() as i64 as u64);
    }
}
