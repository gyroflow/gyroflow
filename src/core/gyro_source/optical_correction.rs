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

use nalgebra::{ UnitQuaternion, Vector3 };
use super::TimeQuat;

/// What only the user can judge: nothing in the metadata says how far the motion data can be trusted
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct OpticalCorrectionSettings {
    /// How far the correction may take the motion data from what it says, 0 to 1. Low keeps it to the small, fast
    /// errors of a vibrating gyro; high lets the image override the motion data down to slow motion and by degrees - a
    /// gyro whose gain collapses during a roll, glitches - at the cost of following the image's own mistakes too
    pub strength: f64,
}
impl Default for OpticalCorrectionSettings {
    fn default() -> Self { Self { strength: 0.5 } }
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

    /// Frames the analysis tracked, and how many of them yielded a measurement
    pub frames: usize,
    pub measured_frames: usize,
    /// Rms of the correction where it was measured, degrees
    pub rms_deg: f64,
}

impl OpticalCorrection {
    /// δ at a motion data timestamp. Zero outside of the analyzed range: there the motion data is used as it is
    pub fn at(&self, timestamp_us: f64) -> Vector3<f64> {
        if self.coeffs.is_empty() || self.spacing_us <= 0.0 { return Vector3::zeros(); }
        let u = (timestamp_us - self.start_us) / self.spacing_us;
        let (s, w) = bspline_weights(u);
        let mut v = Vector3::zeros();
        for (i, w) in w.iter().enumerate() {
            let k = s + i as i64 - 1;
            if k >= 0 && (k as usize) < self.coeffs.len() {
                let c = self.coeffs[k as usize];
                v += Vector3::new(c[0] as f64, c[1] as f64, c[2] as f64) * *w;
            }
        }
        v
    }

    /// The orientation of a file without motion data, as the renderer gets it: `video_base` at 1 kHz
    pub fn base_quats(&self) -> TimeQuat { base_quats(&self.video_base) }

    /// Whether it was measured on (uncorrected) quaternions of this `checksum`, in this context (`context_checksum`)
    pub fn measured_on(&self, quats_checksum: u64, context_checksum: u64) -> bool {
        !self.coeffs.is_empty() && self.quats_checksum == quats_checksum && self.context_checksum == context_checksum
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
    }
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
    densify(&video_base.iter().map(|(t, q)| (*t, UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64)))).collect())
}

/// Orientations sampled at the frames, at 1 kHz: the geodesic between each two, so a lookup between the samples gives
/// what it would have between the frames
pub fn densify(sparse: &TimeQuat) -> TimeQuat {
    let mut out = TimeQuat::new();
    let mut it = sparse.iter().peekable();
    while let Some((&t0, q0)) = it.next() {
        out.insert(t0, *q0);
        if let Some(&(&t1, q1)) = it.peek() {
            let mut t = t0 - t0.rem_euclid(1000) + 1000;
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
    let (Some((&first, &q0)), Some((&last, &q1))) = (quats.first_key_value(), quats.last_key_value()) else { return };
    let end = (duration_ms * 1000.0).round() as i64;
    let mut t = first - first.rem_euclid(1000);
    if t == first { t -= 1000; }
    while t >= 0 { quats.insert(t, q0); t -= 1000; }
    let mut t = last - last.rem_euclid(1000) + 1000;
    while t <= end { quats.insert(t, q1); t += 1000; }
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
