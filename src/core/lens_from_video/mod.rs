// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Anton Belkin

//! "Find the lens from the video": the lens profile and the rolling shutter readout time of a clip, from the picture
//! alone, with no calibration target.
//!
//! Points are tracked through a fragment of the clip (`optical_motion::Tracker`), a rough scene is made of them with
//! a lens taken as given (`scene`), and a bundle adjustment with rolling shutter (`bundle`) then fits the poses, the
//! points, the lens and the readout time together. It's used two ways:
//!
//! - **Is this known profile the lens of the clip?** (`check_profile`) The scene is made and settled with the profile,
//!   then the lens is let go for a few steps. On a clip that shakes, the right lens stays where it is (2-6 px over a
//!   2.7K frame) and one that is off by 3% in focal length or belongs to another mode walks away (16-90 px). On a
//!   steady one (a gimbal, a drone, a car mount) a wrong lens stays for those few steps too - 15% off, it has moved
//!   4 px - so a lens that stays is only said to fit once the fragment has shown how it tells (`Telling`): with the
//!   focal length put 5% off either way, the lens either walks away in those steps, and staying meant something; or,
//!   let go until the fit rests, comes back to one place, and the profile is judged by how far it is from there; or
//!   neither, and the fragment has no say. The profiles of the camera in the database (`candidates`) are all tried,
//!   on three fragments of the clip, and the nearest that fits is the one (`find_known`): one fragment's verdict
//!   isn't trusted, and the fragments are weighed by how much of a scene they had.
//! - **What is the lens, then?** (`estimate`) From the nearest lens: the focal length and the readout time first, on a
//!   third of the points, then the centre and the distortion too. About a minute for 300 frames.
//!
//! What it needs, and can't do without: a camera that moves through space while it shakes (walking, swimming), a
//! static scene with detail out to the corners, few things moving on their own. The focal length comes from the
//! parallax: a camera that only turns on the spot has none (`NoScene::NoTranslation`). In-camera stabilization that
//! changes its crop from frame to frame leaves no constant lens to find. And under water behind a flat port the
//! refraction index and the focal length trade off exactly, so the picture can't tell water from air: the caller says
//! which it is, and the lens found is always the lens in air, for `light_refraction_coefficient` to bend.

pub mod lens;
pub mod bundle;
pub mod scene;
pub mod candidates;

pub use lens::{ Lens, LensShift };
pub use bundle::HeldOut;
pub use scene::{ NoScene, SceneInfo };
pub use candidates::{ Candidate, How, candidates };

use std::collections::{ HashMap, HashSet };
use rayon::prelude::*;
use bundle::{ Bundle, FitOptions, Free };
use crate::LensProfile;
use crate::stabilization_params::ReadoutDirection;
use crate::synchronization::optical_motion::Observation;

/// The pixel thresholds below are for a frame this wide (2.7K) and scale with the frame
const REFERENCE_WIDTH: f64 = 2704.0;
/// Scale of the Cauchy loss of the fits, pixels
const LOSS: f64 = 1.5;
/// A known profile fits when, let go for `PROBE_STEPS`, the lens moves less than this over the frame (median, after
/// the best common tilt), nowhere by more than `FIT_MAX`, and the readout by less than `FIT_READOUT` of the frame
/// interval - on a fragment that tells, see `Telling`. Fits loosely: only the median holds
const FIT_MEDIAN: f64 = 15.0;
const FIT_MAX: f64 = 40.0;
const FIT_READOUT: f64 = 0.06;
const PROBE_STEPS: usize = 5;
/// Iterations a let-go fit gets to come to rest in, on a steady fragment
const REST_STEPS: usize = 60;
/// How far off the focal length is put, either way, to see how a fragment tells
const HOLD_OFF: f64 = 0.05;

/// Tracks given to a fit, the shortest of them (frames), and every which one of them is held out
const POINTS: usize = 2000;
const MIN_TRACK: usize = 10;
const HOLD_OUT: usize = 5;
/// A check of a known profile, and the first stage of an estimate, use every n-th of the tracks
const THIN: usize = 3;
/// Frames of a fragment a known profile is checked on, and the most an estimate takes: its time grows faster than
/// the number of frames, and it's no more precise on more
pub const CHECK_FRAMES: usize = 100;
pub const MAX_FRAMES: usize = 300;
/// Width the frames are tracked at, and the points kept alive in each. The positions are scaled to the video's frame.
/// Wider than the optical analysis' 1000 px, which only measures a rotation out of thousands of points: the distortion
/// of a lens is a fraction of a pixel a point, and it shows. On the HERO4 2.7K clip the lens came out 0.7 px from the
/// one the whole clip at full resolution gives, where at 960 px it was 7 px off, with the distortion taking the blame
pub const TRACKED_WIDTH: u32 = 1920;
pub const TRACKED_POINTS: usize = 2500;
/// The fragments see about this many frames a second: a clip at 60 or 120 fps gives every 2nd or 4th frame
const TARGET_FPS: f64 = 30.0;

fn px_scale(size: (usize, usize)) -> f64 { size.0.max(size.1) as f64 / REFERENCE_WIDTH }

/// An error in pixels as the user gets to see it: never an infinity (a fit with nothing held out), and no point
/// reporting more than "hopeless"
fn finite(v: f64) -> f64 { if v.is_finite() { v.min(999.0) } else { 999.0 } }

/// Every which frame of a clip at this frame rate is tracked
pub fn frame_step(fps: f64) -> usize { ((fps / TARGET_FPS).round() as usize).max(1) }

/// The fragments of the automatic attempt, in the clip's milliseconds: its start (past the first second, often
/// shaken by the hand on the button), its middle and about 80% of it, `CHECK_FRAMES` tracked frames each. Fewer for
/// a clip too short for three
pub fn automatic_fragments(duration_ms: f64, fps: f64) -> Vec<(f64, f64)> {
    if !(fps > 0.0) || !(duration_ms > 0.0) { return Vec::new(); }
    let len = CHECK_FRAMES as f64 * frame_step(fps) as f64 * 1000.0 / fps;
    if duration_ms <= len * 1.2 { return vec![(0.0, duration_ms)]; }
    let last = duration_ms - len;
    let mut out: Vec<(f64, f64)> = Vec::new();
    for start in [1000f64.min(duration_ms * 0.05), duration_ms * 0.5 - len / 2.0, duration_ms * 0.8 - len / 2.0] {
        let start = start.clamp(0.0, last);
        if out.last().is_none_or(|x| start >= x.1) { out.push((start, start + len)); }
    }
    out
}

/// A fragment of this many tracked frames from `start_ms` (the playhead), or as near it as the clip allows: the whole
/// clip, when it's shorter
pub fn fragment_from(start_ms: f64, duration_ms: f64, fps: f64, frames: usize) -> (f64, f64) {
    if !(fps > 0.0) || !(duration_ms > 0.0) { return (0.0, duration_ms.max(0.0)); }
    let len = (frames as f64 * frame_step(fps) as f64 * 1000.0 / fps).min(duration_ms);
    let start = start_ms.clamp(0.0, duration_ms - len);
    (start, start + len)
}

/// Which of these fragments of the clip (ms) have nothing in common with the stretch an estimate was fitted to: the
/// ones it can be judged on. A lens stays put on the frames it was fitted to, right or not
pub fn apart_from(fragments: &[(f64, f64)], fitted: (f64, f64)) -> Vec<usize> {
    (0..fragments.len()).filter(|i| fragments[*i].1 <= fitted.0 || fragments[*i].0 >= fitted.1).collect()
}

/// What was tracked in one fragment of the clip
#[derive(Clone, Debug, Default)]
pub struct Fragment {
    /// The video's frame: the pixels of the lens
    pub size: (usize, usize),
    pub frames: usize,
    /// Time between two of its frames, ms: the median of what's between the frames it has
    pub frame_ms: f64,
    /// When each of its frames was taken, ms: the video's own times (`FragmentBuilder::push_frame`), uneven where its
    /// frame rate varies or a frame is missing
    times: Vec<f64>,
    /// Every sighting of every track
    track: Vec<u32>,
    frame: Vec<u32>,
    uv: Vec<[f64; 2]>,
}

impl Fragment {
    pub fn new(size: (usize, usize), frame_ms: f64) -> Self { Self { size, frame_ms, ..Default::default() } }

    pub fn is_empty(&self) -> bool { self.track.is_empty() }

    /// A point of a track in a frame of the fragment, in pixels of the video's frame
    pub fn push(&mut self, track: u32, frame: usize, x: f64, y: f64) {
        self.track.push(track); self.frame.push(frame as u32); self.uv.push([x, y]);
        self.frames = self.frames.max(frame + 1);
        while self.times.len() < self.frames { self.times.push(self.times.len() as f64 * self.frame_ms); }
    }

    /// The tracks long enough to count, numbered from 0
    pub fn tracks(&self) -> Tracks {
        let mut count: HashMap<u32, usize> = HashMap::new();
        for t in &self.track { *count.entry(*t).or_insert(0) += 1; }
        let mut ids: Vec<u32> = count.iter().filter(|(_, n)| **n >= MIN_TRACK).map(|(t, _)| *t).collect();
        ids.sort_unstable();
        let index: HashMap<u32, u32> = ids.iter().enumerate().map(|(i, t)| (*t, i as u32)).collect();
        // The fit counts time in frame intervals from the first frame
        let (first, interval) = (self.times.first().copied().unwrap_or(0.0), if self.frame_ms > 0.0 { self.frame_ms } else { 1.0 });
        let mut out = Tracks { size: self.size, frames: self.frames, tracks: ids.len(), times: self.times.iter().map(|t| (t - first) / interval).collect(), ..Default::default() };
        for i in 0..self.track.len() {
            if let Some(k) = index.get(&self.track[i]) {
                out.track.push(*k); out.frame.push(self.frame[i]); out.uv.push(self.uv[i]);
            }
        }
        out
    }
}

/// Collects a fragment from what `optical_motion::Tracker` says of each frame fed to it
pub struct FragmentBuilder {
    fragment: Fragment,
    /// Pixels of the video's frame per pixel of the tracked one
    scale: (f64, f64),
    /// The tracks seen in the frame before
    alive: HashSet<u32>,
    fed: usize,
}
impl FragmentBuilder {
    pub fn new(size: (usize, usize), tracked_size: (u32, u32), frame_ms: f64) -> Self {
        Self {
            fragment: Fragment::new(size, frame_ms),
            scale: (size.0 as f64 / tracked_size.0.max(1) as f64, size.1 as f64 / tracked_size.1.max(1) as f64),
            alive: HashSet::new(), fed: 0,
        }
    }
    pub fn frames(&self) -> usize { self.fed }

    /// The observations of the next frame: where each point was in the frame before, and where it is now. The pixel
    /// (0, 0) of the tracker is the middle of the top left pixel. `time_ms`: when the frame was taken, by the video's
    /// own timestamps - so the fit knows how far the camera moved between two frames whatever the frame rate does,
    /// and across a frame that is missing
    pub fn push_frame(&mut self, time_ms: f64, observations: &[Observation]) {
        let f = self.fed;
        self.fed += 1;
        self.fragment.times.push(time_ms);
        if f == 0 { return; }
        let place = |p: [f32; 2], s: (f64, f64)| ((p[0] as f64 + 0.5) * s.0 - 0.5, (p[1] as f64 + 0.5) * s.1 - 0.5);
        let mut alive = HashSet::with_capacity(observations.len());
        for o in observations {
            let id = o.id;
            if !self.alive.contains(&id) {
                let (x, y) = place(o.a, self.scale);
                self.fragment.push(id, f - 1, x, y);
            }
            let (x, y) = place(o.b, self.scale);
            self.fragment.push(id, f, x, y);
            alive.insert(id);
        }
        self.alive = alive;
    }
    pub fn finish(mut self) -> Fragment {
        self.fragment.frames = self.fed;
        self.fragment.times.truncate(self.fed);
        // The frame interval is what's between most of its frames: the median, the lower of the two in the middle
        let mut steps: Vec<f64> = self.fragment.times.windows(2).map(|t| t[1] - t[0]).filter(|d| *d > 0.0).collect();
        if !steps.is_empty() {
            steps.sort_by(|a, b| a.total_cmp(b));
            self.fragment.frame_ms = steps[(steps.len() - 1) / 2];
        }
        self.fragment
    }
}

#[derive(Clone, Debug, Default)]
pub struct Tracks {
    pub size: (usize, usize),
    pub frames: usize,
    pub tracks: usize,
    /// When each frame was taken, in frame intervals from the first: 0, 1, 2... where the frames are even
    pub times: Vec<f64>,
    pub track: Vec<u32>,
    pub frame: Vec<u32>,
    pub uv: Vec<[f64; 2]>,
}

/// How a known profile did on one fragment
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct ProfileCheck {
    pub scene: SceneInfo,
    /// How far the lens is from where the fragment would have it, pixels of the frame: how far it moved in the quick
    /// steps when it was let go - or, on a steady fragment, how far it is from the fragment's own lens (`Telling`)
    pub moved: LensShift,
    /// The readout time with the profile as it is (the profile's own, or fitted when it has none) and with the lens
    /// let go, in frame intervals of the fragment
    pub readout: f64,
    pub readout_free: f64,
    /// Error of the held-out tracks with the profile as it is, and with the lens let go, px at 2.7K
    pub error: f64,
    pub error_free: f64,
    pub fits: bool,
    /// The median holds, but a corner moved or the readout did
    pub fits_loosely: bool,
    /// Whether the fragment can tell a lens from one near it (`Telling`): one that can't has no say, whatever stayed
    /// put on it. `None`: the lens was past the limit in the quick steps, and nothing more was asked
    pub holds: Option<bool>,
    /// It was judged against the fragment's lens at rest, not by the quick steps (`Telling::AtRest`)
    pub at_rest: bool,
}
impl ProfileCheck {
    /// How much this fragment's verdict counts: a fragment where the camera hardly moved through space, or with few
    /// points, knows little of the lens
    pub fn weight(&self) -> f64 {
        (self.scene.parallax / 1.5).min(1.0) * (self.scene.points as f64 / 500.0).min(1.0)
    }
    /// It has a say: it wasn't found unable to tell
    pub fn votes(&self) -> bool { self.holds != Some(false) }
}

/// How a fragment tells the lens it was shot with from one near it. A lens that stays put when it's let go for a few
/// steps is the right one only where a wrong one wouldn't have stayed too - so before a lens is said to fit, the
/// fragment is asked, with that lens' focal length put `HOLD_OFF` off either way. It's asked once, of the first lens
/// that stays put on it: this is the fragment's own doing, how the camera moved in it
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Telling {
    /// It shakes: a lens that far off is past the limit within the quick steps, as one 4% off is on a hand-held
    /// clip (35 px). Staying put through them is the test, as it has been checked on real footage
    Quick,
    /// It's steady - a gimbal, a drone, a car mount - and pulls on a wrong lens slowly: one 15% off has moved 4 px in
    /// the quick steps. But let go until the fit rests, lenses end up in one place, and put off from there come back
    /// to it: the fragment's own lens, and its readout time. A lens fits when it's within the limit of that
    AtRest(Lens, f64),
    /// Neither: lenses stay where they're put, the right one and the wrong one (a camera that never turns). It has
    /// no say
    Mute,
}

/// The lens let go on a fragment: where it went
struct LetGo {
    info: SceneInfo,
    bundle: Bundle,
    options: FitOptions,
    /// The readout time with the lens as given, and the held-out tracks' error with it
    tau: f64,
    as_it_is: HeldOut,
    free: Lens,
    tau_free: f64,
    /// How far it has moved from the lens as given
    moved: LensShift,
    at_rest: bool,
}
impl LetGo {
    /// On from the quick steps, until the fit rests
    fn rest(&mut self, lens: &Lens, size: (usize, usize), cancelled: &(dyn Fn() -> bool + Sync)) -> Result<(), NoScene> {
        if self.at_rest { return Ok(()); }
        let (free, tau_free, rest) = self.bundle.fit(self.free, self.tau_free, &FitOptions { iterations: REST_STEPS, ..self.options }, cancelled);
        if !rest.cost.is_finite() || !free.is_finite() || !tau_free.is_finite() { return Err(NoScene::NoVerdict); }
        self.free = free; self.tau_free = tau_free; self.at_rest = rest.settled;
        self.moved = lens.shift_after_tilt(&self.free, size);
        Ok(())
    }
}

/// The scene is made and settled with the lens as given, then the lens is let go for `PROBE_STEPS`. `readout`: the
/// lens' own, in frame intervals, when it has one
fn let_go(tracks: &Tracks, lens: &Lens, readout: Option<f64>, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<LetGo, NoScene> {
    let s = px_scale(tracks.size);
    let loss = LOSS * s;
    let (scene, info) = scene::rough_scene(tracks, lens, POINTS, HOLD_OUT)?;
    let mut bundle = Bundle::new(scene);
    bundle.select_every(THIN);
    // The poses and the points settle with the lens as it is, and the readout time is found if the profile has none
    bundle.fit(*lens, readout.unwrap_or(0.0), &FitOptions::new(Free::NONE, 4, 4.0 * loss), cancelled);
    let (_, tau, _) = bundle.fit(*lens, readout.unwrap_or(0.0), &FitOptions::new(if readout.is_some() { Free::NONE } else { Free::READOUT }, 12, loss).tolerance(0.5 * s), cancelled);
    if bundle.parallax(lens, tau) < scene::MIN_PARALLAX_FITTED { return Err(NoScene::NoTranslation); }
    let as_it_is = bundle.check(lens, tau, loss, s);
    // Then the lens is let go
    let options = FitOptions::new(Free::ALL, PROBE_STEPS, loss).tolerance(0.05 * s);
    let (free, tau_free, probe) = bundle.fit(*lens, tau, &options, cancelled);
    // A lens that took no step hasn't shown that it stays: no step was found, or the cost wasn't a number
    if !probe.sound() || !free.is_finite() || !tau_free.is_finite() { return Err(NoScene::NoVerdict); }
    Ok(LetGo { info, bundle, options, tau, as_it_is, free, tau_free, moved: lens.shift_after_tilt(&free, tracks.size), at_rest: probe.settled })
}

/// Is this the lens of the fragment? `readout`: the profile's own, in frame intervals of the fragment, when it has one.
/// The lens carries the refraction index to check it with
pub fn check_profile(tracks: &Tracks, lens: &Lens, readout: Option<f64>, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<ProfileCheck, NoScene> {
    check_telling(tracks, lens, readout, None, cancelled).map(|x| x.0)
}

/// `check_profile`, with what the fragment has shown of itself to a lens tried before (`Telling`), and what it shows
/// to this one if it wasn't asked yet
fn check_telling(tracks: &Tracks, lens: &Lens, readout: Option<f64>, telling: Option<Telling>, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<(ProfileCheck, Option<Telling>), NoScene> {
    let s = px_scale(tracks.size);
    let limit = FIT_MEDIAN * s;
    let mut go = let_go(tracks, lens, readout, cancelled)?;
    let mut check = ProfileCheck {
        scene: go.info, moved: go.moved, readout: go.tau, readout_free: go.tau_free, error: go.as_it_is.rms, error_free: 0.0,
        fits: false, fits_loosely: false, holds: None, at_rest: false,
    };
    let mut telling = telling;
    // Past the limit already: it's not the lens, and the fragment told as much
    if go.moved.median < limit && !cancelled() {
        // What the fragment shows to this lens, when it wasn't asked before. Kept for the lenses after it only when
        // it's the fragment's doing and not this lens'
        let mut shown = telling;
        if telling.is_none() {
            let off = |scale: f64| Lens { fx: lens.fx * scale, fy: lens.fy * scale, ..*lens };
            let scales = [1.0 + HOLD_OFF, 1.0 / (1.0 + HOLD_OFF)];
            // Would a wrong lens have stayed too?
            if scales.par_iter().all(|scale| let_go(tracks, &off(*scale), readout, cancelled).is_ok_and(|x| x.moved.median >= limit)) {
                telling = Some(Telling::Quick);
                shown = telling;
            } else {
                // It would, this far. Where does it end up, then: that's where the fragment would have the lens -
                // if the fragment has one place for it, which the lens there, put off again, comes back to
                go.rest(lens, tracks.size, cancelled)?;
                check.moved = go.moved;
                check.readout_free = go.tau_free;
                check.at_rest = true;
                if go.at_rest {
                    let own = go.free;
                    let come_back = scales.par_iter().all(|scale| {
                        let start = Lens { fx: own.fx * scale, fy: own.fy * scale, ..own };
                        let_go(tracks, &start, readout, cancelled).and_then(|mut x| x.rest(&start, tracks.size, cancelled).map(|_| x))
                            .is_ok_and(|x| x.at_rest && own.shift_after_tilt(&x.free, tracks.size).median < limit)
                    });
                    telling = Some(if come_back { Telling::AtRest(own, go.tau_free) } else { Telling::Mute });
                    shown = telling;
                } else if go.moved.median < limit {
                    // Still on its way after all the iterations it gets, and still near: nothing to go by, for this
                    // lens. The fragment may yet tell with another
                    shown = Some(Telling::Mute);
                }
                // Else it walked away, only slowly: not the lens
            }
        }
        if let Some(Telling::AtRest(own, tau_own)) = shown {
            check.moved = lens.shift_after_tilt(&own, tracks.size);
            check.readout_free = tau_own;
            check.at_rest = true;
        }
        if shown.is_some() {
            check.holds = Some(shown != Some(Telling::Mute));
            check.fits_loosely = check.moved.median < limit;
            check.fits = check.fits_loosely && check.moved.max < FIT_MAX * s && (check.readout_free - check.readout).abs() < FIT_READOUT;
        }
    }
    check.error_free = go.bundle.check(&go.free, go.tau_free, LOSS * s, s).rms;
    Ok((check, telling))
}

/// How well a lens fits the clip, over the fragments: the one measure both a known profile and an estimate are judged by,
/// so the two can be put side by side
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct Match {
    pub verdict: Verdict,
    /// Fragments it holds on, and those that had a say: a scene, and a way to tell a lens from one near it (`Telling`)
    pub fragments_fitting: usize,
    pub fragments: usize,
    /// Fragments that had a scene and still couldn't tell: the lens stayed, and so would a wrong one
    pub fragments_unable: usize,
    /// How far the lens moved when it was let go, pixels of the video, over the fragments (weighted): the median over
    /// the frame, and the largest in a corner. It holds under `limit` (`FIT_MEDIAN` brought to the frame's size)
    pub moved: f64,
    pub moved_corner: f64,
    pub limit: f64,
    /// The readout time, ms: measured on the clip, or the profile's own when it has one (`readout_measured` false)
    pub readout_ms: Option<f64>,
    pub readout_measured: bool,
}

/// "good": it holds where the clip has a scene, on two parts of it at least; "single": it holds on the one part that
/// had a say - one fragment's word; "loose": it holds over the frame but a corner or the readout time moves; "poor":
/// it walks away when let go, or no part of the clip could tell
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict { Good, Single, Loose, #[default] Poor }

/// A lens against every fragment, in one medium
#[derive(Clone, Debug, serde::Serialize)]
pub struct MediumResult {
    pub refraction: f64,
    /// One per fragment; none where no scene could be made
    pub checks: Vec<Option<ProfileCheck>>,
    /// Weight of the fragments it fits, fits loosely, and of all that had a say
    pub fitting: f64,
    pub fitting_loosely: f64,
    pub total: f64,
    /// Error of the held-out tracks with it as it is, over the fragments, at 2.7K
    pub error: f64,
    #[serde(rename = "match")]
    pub matching: Match,
}
impl MediumResult {
    /// `frame_ms`: of each fragment. `readout_measured`: the lens had no readout time of its own, and the one here was
    /// found on the clip
    fn new(refraction: f64, checks: Vec<Option<ProfileCheck>>, frame_ms: &[f64], scale: f64, readout_measured: bool) -> Self {
        let weight = |c: &ProfileCheck| c.weight();
        let voting = || checks.iter().flatten().filter(|c| c.votes());
        let total: f64 = voting().map(weight).sum();
        let fitting: f64 = voting().filter(|c| c.fits).map(weight).sum();
        let fitting_loosely: f64 = voting().filter(|c| c.fits_loosely).map(weight).sum();
        // Over the fragments with a say; with none, over those that had a scene: how near the lens is, for what it's worth
        let mean = |f: &dyn Fn(&ProfileCheck) -> f64| {
            let all: f64 = checks.iter().flatten().map(weight).sum();
            if total > 0.0 { voting().map(|c| f(c) * weight(c)).sum::<f64>() / total }
            else if all > 0.0 { checks.iter().flatten().map(|c| f(c) * weight(c)).sum::<f64>() / all }
            else { f64::INFINITY }
        };
        let readout_ms = weighted_median(checks.iter().zip(frame_ms).filter_map(|(c, ms)| c.filter(|c| c.fits_loosely && c.votes()).map(|c| (c.readout * ms, c.weight()))).collect());
        let holds = |w: f64| w >= 0.5 && w >= total * 2.0 / 3.0;
        let fragments_fitting = voting().filter(|c| c.fits).count();
        Self {
            refraction,
            error: mean(&|c| c.error),
            matching: Match {
                // One fragment's verdict isn't trusted, however much of a scene it had
                verdict: if holds(fitting) && fragments_fitting >= 2 { Verdict::Good } else if holds(fitting) { Verdict::Single } else if holds(fitting_loosely) { Verdict::Loose } else { Verdict::Poor },
                fragments_fitting,
                fragments: voting().count(),
                fragments_unable: checks.iter().flatten().filter(|c| !c.votes()).count(),
                moved: finite(mean(&|c| c.moved.median)),
                moved_corner: finite(mean(&|c| c.moved.corner)),
                limit: FIT_MEDIAN * scale,
                readout_ms,
                readout_measured,
            },
            checks, fitting, fitting_loosely, total,
        }
    }
}

/// A known profile against every fragment, in each medium tried
#[derive(Clone, Debug, serde::Serialize)]
pub struct CandidateResult {
    pub name: String,
    pub id: String,
    /// One per medium tried (`find_known`'s `media`), and the one the clip was shot in
    pub media: Vec<MediumResult>,
    pub chosen: usize,
    /// Whether the picture says so clearly: every fragment that could tell voted for it, each by a corner that moved
    /// over 1.5 times less than in the other medium and an error that agreed. Always with one medium given
    pub medium_sure: bool,
    pub accepted: bool,
}
impl CandidateResult {
    pub fn result(&self) -> &MediumResult { &self.media[self.chosen] }
    pub fn refraction(&self) -> f64 { self.result().refraction }
    pub fn matching(&self) -> Match { self.result().matching }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct KnownProfiles {
    /// One per candidate, in their order; fewer when the search was cancelled
    pub tried: Vec<CandidateResult>,
    /// The one that fits, in `tried`: the nearest of those that do
    pub accepted: Option<usize>,
    /// Fragments that had a scene for at least one profile
    pub fragments_with_scene: usize,
    /// No fragment had one because the camera didn't move through space
    pub no_translation: bool,
}
impl KnownProfiles {
    /// The profile that fits, or else the one nearest to the lens of the clip - the one whose lens, let go, moved the
    /// least - which is what an estimate starts from
    pub fn best(&self) -> Option<usize> {
        self.accepted.or_else(|| self.tried.iter().enumerate().filter(|(_, x)| x.result().checks.iter().any(|c| c.is_some()))
            .min_by(|a, b| a.1.matching().moved.total_cmp(&b.1.matching().moved)).map(|x| x.0))
    }
}

fn weighted_median(mut v: Vec<(f64, f64)>) -> Option<f64> {
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let half = v.iter().map(|x| x.1).sum::<f64>() / 2.0;
    let mut sum = 0.0;
    for (x, w) in &v { sum += w; if sum >= half { return Some(*x); } }
    None
}

/// Which medium a known lens fits in. With the lens known, refraction is no longer traded for the focal length: a
/// lens tried in the wrong medium is wrong most in the far corners, where the angles are largest. Each fragment that
/// has a scene in both votes, by its weight, for the one its corner moved less in; it's sure of it when the corner
/// moved over 1.5 times less and the held-out tracks agree. On the archive this picked right 91% of the time by the
/// corner, 96% when sure. `current`: the medium in use (in `media`), which a tie goes to - an even vote is no reason
/// to change it. Returns the medium, and whether the clip was sure of it
fn choose_medium(media: &[MediumResult], current: Option<usize>) -> (usize, bool) {
    if media.len() < 2 { return (0, true); }
    // The first of the best, the one in use first: `max_by` would give a tie to the last
    let best_of = |better: &dyn Fn(usize, usize) -> bool| {
        let mut order: Vec<usize> = (0..media.len()).collect();
        if let Some(c) = current.filter(|c| *c < media.len()) { order.swap(0, c); }
        let mut best = order[0];
        for &m in &order[1..] { if better(m, best) { best = m; } }
        best
    };
    let mut votes = vec![0.0f64; media.len()];
    let (mut voters, mut sure_voters, mut agree) = (0, 0, true);
    let mut first_vote: Option<usize> = None;
    for f in 0..media[0].checks.len() {
        let checks: Vec<&ProfileCheck> = media.iter().filter_map(|m| m.checks[f].as_ref()).collect();
        if checks.len() != media.len() { continue; }
        let (best, second) = {
            let mut order: Vec<usize> = (0..checks.len()).collect();
            order.sort_by(|a, b| checks[*a].moved.corner.total_cmp(&checks[*b].moved.corner));
            (order[0], order[1])
        };
        // A corner that moved the same in both is no vote
        if checks[best].moved.corner == checks[second].moved.corner { continue; }
        let w = checks.iter().map(|c| c.weight()).sum::<f64>() / checks.len() as f64;
        votes[best] += w;
        voters += 1;
        if first_vote.is_some_and(|v| v != best) { agree = false; }
        first_vote.get_or_insert(best);
        if checks[second].moved.corner > 1.5 * checks[best].moved.corner && checks[best].error <= checks[second].error { sure_voters += 1; }
    }
    if voters == 0 {
        // No fragment had a scene in both: the one that holds best, unsure
        return (best_of(&|a, b| media[a].fitting > media[b].fitting || (media[a].fitting == media[b].fitting && media[a].matching.moved < media[b].matching.moved)), false);
    }
    let best = best_of(&|a, b| votes[a] > votes[b]);
    (best, agree && sure_voters == voters && votes.iter().filter(|v| **v == votes[best]).count() == 1)
}

/// Tries every known profile on every fragment, in each of `media` (refraction indices: 1 in air, 1.33 under water -
/// both when nobody knows; `current` is the one in use, which a tie goes to), and takes the nearest of those that fit:
/// in the medium the clip votes for, the fragments it fits weigh at least two thirds of those that had a say, and at
/// least half a good fragment in all, two of them at least. The first to fit isn't taken for the lens: a profile a
/// per cent or two off earlier in the list is still within the limit, and the exact one may come after it. So all of
/// them are tried (`candidates` keeps at most 8). `progress` gets the fraction done
pub fn find_known(fragments: &[Fragment], candidates: &[Candidate], media: &[f64], current: f64, cancelled: &(dyn Fn() -> bool + Sync), progress: &(dyn Fn(f64) + Sync)) -> KnownProfiles {
    let tracks: Vec<Tracks> = fragments.iter().map(|f| f.tracks()).collect();
    let frame_ms: Vec<f64> = fragments.iter().map(|f| f.frame_ms).collect();
    let scale = fragments.first().map(|f| px_scale(f.size)).unwrap_or(1.0);
    let media: Vec<f64> = if media.is_empty() { vec![1.0] } else { media.to_vec() };
    let current = media.iter().position(|m| *m == current);
    let mut out = KnownProfiles::default();
    let mut with_scene = vec![false; fragments.len()];
    let mut no_translation = 0;
    // How each fragment tells, in each medium: asked of the first lens that stays put on it, and kept for the others
    let mut telling: Vec<Vec<Option<Telling>>> = vec![vec![None; fragments.len()]; media.len()];
    for (n, candidate) in candidates.iter().enumerate() {
        if cancelled() { break; }
        // Every fragment in every medium at once
        let jobs: Vec<(usize, usize)> = (0..media.len()).flat_map(|m| (0..fragments.len()).map(move |f| (m, f))).collect();
        let results: Vec<Result<(ProfileCheck, Option<Telling>), NoScene>> = jobs.par_iter().map(|&(m, f)| {
            let readout = candidate.readout_ms.filter(|_| frame_ms[f] > 0.0).map(|ms| ms / frame_ms[f]);
            check_telling(&tracks[f], &candidate.lens.with_refraction(media[m]), readout, telling[m][f], cancelled)
        }).collect();
        if cancelled() { break; }
        no_translation += results.iter().filter(|r| matches!(r, Err(NoScene::NoTranslation))).count();
        let mut per_medium: Vec<Vec<Option<ProfileCheck>>> = vec![Vec::new(); media.len()];
        for ((m, f), r) in jobs.iter().zip(results) {
            if r.is_ok() { with_scene[*f] = true; }
            if let Err(e) = &r { log::debug!("Lens from video: {} in {}: fragment {f}: {e:?}", candidate.label(), media[*m]); }
            if let Ok((_, Some(t))) = &r {
                if telling[*m][*f].is_none() { log::info!("Lens from video: fragment {f} [{}] is {}", if media[*m] != 1.0 { "water" } else { "air" }, match t { Telling::Quick => "quick to tell", Telling::AtRest(..) => "steady: told at rest", Telling::Mute => "unable to tell" }); }
                telling[*m][*f] = Some(*t);
            }
            per_medium[*m].push(r.ok().map(|x| x.0));
        }
        let results: Vec<MediumResult> = per_medium.into_iter().zip(&media).map(|(checks, &n)| MediumResult::new(n, checks, &frame_ms, scale, candidate.readout_ms.is_none())).collect();
        let (chosen, medium_sure) = choose_medium(&results, current);
        for r in &results {
            log::info!("Lens from video: {} [{}] - {}", candidate.label(), if r.refraction != 1.0 { "water" } else { "air" }, r.checks.iter().map(|c| match c {
                Some(c) => format!("moved {:.1}/{:.1} corner {:.1} px{}, readout {:.3} -> {:.3}, error {:.2} -> {:.2}, weight {:.2}{}{}", c.moved.median, c.moved.max, c.moved.corner, if c.at_rest { " (at rest)" } else { "" }, c.readout, c.readout_free, c.error, c.error_free, c.weight(), if c.fits { " FITS" } else { "" }, if c.votes() { "" } else { " (no say)" }),
                None => "no scene".into(),
            }).collect::<Vec<_>>().join(" | "));
        }
        if media.len() > 1 { log::info!("Lens from video: {} - shot in {}{}", candidate.label(), if results[chosen].refraction != 1.0 { "water" } else { "air" }, if medium_sure { "" } else { " (not sure)" }); }
        out.tried.push(CandidateResult { name: candidate.name.clone(), id: candidate.id.clone(), media: results, chosen, medium_sure, accepted: false });
        progress((n + 1) as f64 / candidates.len() as f64);
    }
    // The nearest of those that fit
    if !cancelled() {
        out.accepted = (0..out.tried.len()).filter(|n| out.tried[*n].matching().verdict == Verdict::Good)
            .min_by(|a, b| out.tried[*a].matching().moved.total_cmp(&out.tried[*b].matching().moved));
    }
    if let Some(n) = out.accepted { out.tried[n].accepted = true; }
    out.fragments_with_scene = with_scene.iter().filter(|x| **x).count();
    out.no_translation = out.fragments_with_scene == 0 && no_translation > 0;
    out
}

/// How well a lens found some other way (an estimate) fits these fragments, in this medium: what `find_known` says of
/// a known profile, for the two to be compared
pub fn check_lens(fragments: &[Fragment], lens: &Lens, readout_ms: Option<f64>, cancelled: &(dyn Fn() -> bool + Sync)) -> Match {
    let candidate = Candidate { lens: Lens { refraction: 1.0, ..*lens }, readout_ms, ..Candidate::plain("", *lens) };
    let known = find_known(fragments, &[candidate], &[lens.refraction], lens.refraction, cancelled, &|_| ());
    // An estimate's readout time is the one it found, held here: what the fragments say of it isn't a measurement
    known.tried.first().map(|r| r.matching()).unwrap_or_default()
}

/// The lens of a fragment
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Estimate {
    #[serde(skip)]
    pub lens: Lens,
    /// Readout time in frame intervals of the fragment, and in ms: positive from the top of the frame down
    pub readout: f64,
    pub readout_ms: f64,
    pub scene: SceneInfo,
    /// Error of the held-out tracks with the lens it started from (and no readout time) and with the one found,
    /// pixels at 2.7K
    pub error_before: f64,
    pub error: f64,
    pub within_1px: f64,
    /// How firmly the fragment holds the focal length: by how much the error of the held-out tracks grows with it
    /// forced 10% off and everything else fitted again. Near 0, the fragment doesn't know its focal length
    pub firmness: f64,
}

/// Estimates the lens of a fragment, starting from a lens of this camera or near it (with the refraction index to
/// use): the focal length and the readout time first, then the whole lens. `progress` gets the fraction done
pub fn estimate(tracks: &Tracks, start: &Lens, frame_ms: f64, cancelled: &(dyn Fn() -> bool + Sync), progress: &(dyn Fn(f64) + Sync)) -> Result<Estimate, NoScene> {
    let s = px_scale(tracks.size);
    let loss = LOSS * s;
    let (scene, info) = scene::rough_scene(tracks, start, POINTS, HOLD_OUT)?;
    progress(0.05);
    let mut bundle = Bundle::new(scene);
    // The poses and the points settle with the starting lens
    bundle.fit(*start, 0.0, &FitOptions::new(Free::NONE, 4, 4.0 * loss), cancelled);
    let before = bundle.check(start, 0.0, loss, s);
    // What's found is a profile with coefficients: a start without any (a pinhole to the render) goes on as its series
    let start = &start.as_fisheye();
    progress(0.1);
    // Stage 1: one scale for the focal lengths, and the readout time, on a third of the points. It's biased as long as
    // the distortion is someone else's (a scale makes up for it), so it's a start for stage 2 and never a result
    bundle.select_every(THIN);
    let (lens1, tau1, _) = bundle.fit(*start, 0.0, &FitOptions::new(Free::SCALE_READOUT, 30, 2.0 * loss).tolerance(0.5 * s), cancelled);
    if bundle.parallax(&lens1, tau1) < scene::MIN_PARALLAX_FITTED { return Err(NoScene::NoTranslation); }
    bundle.select_every(1);
    bundle.fit(lens1, tau1, &FitOptions::new(Free::NONE, 4, 2.0 * loss).points_only(), cancelled); // The other points catch up
    progress(0.3);
    // Stage 2: the whole lens
    let (lens, tau, whole) = bundle.fit(lens1, tau1, &FitOptions::new(Free::ALL, 60, loss).tolerance(0.05 * s), cancelled);
    // A fit that took no step, or lost its numbers, has found nothing: what it has is where it started
    if !whole.sound() || !lens.is_finite() || !tau.is_finite() { return Err(NoScene::NoVerdict); }
    let found = bundle.check(&lens, tau, loss, s);
    progress(0.8);
    // The lens is trusted only as far as the fragment resists a wrong focal length: the distortion is kept, it would
    // soak the change up
    let state = bundle.state();
    let mut firmness = f64::INFINITY;
    for scale in [0.9, 1.1] {
        if cancelled() { break; }
        let mut off = lens; off.fx *= scale; off.fy *= scale;
        let (off, tau_off, _) = bundle.fit(off, tau, &FitOptions::new(Free::CENTRE_READOUT, 12, loss).tolerance(0.3 * s), cancelled);
        firmness = firmness.min(bundle.check(&off, tau_off, loss, s).rms / found.rms - 1.0);
        bundle.restore(&state);
    }
    progress(1.0);
    Ok(Estimate {
        lens, readout: tau, readout_ms: tau * frame_ms, scene: info,
        // Capped, and never an infinity: these go to the user through json, which has no number for one
        error_before: finite(before.rms), error: finite(found.rms), within_1px: found.within_1px,
        firmness: if firmness.is_finite() { firmness } else { 0.0 },
    })
}

/// A readout time as a profile carries it: its length, and the direction it runs in. `ms`: over the picture's rows,
/// negative for a sensor read from the bottom up; nothing for one too short to be a readout time at all
fn readout_for_profile(ms: Option<f64>) -> Option<(f64, ReadoutDirection)> {
    let ms = ms.filter(|ms| ms.is_finite() && ms.abs() > 0.05)?;
    Some((ms.abs(), if ms < 0.0 { ReadoutDirection::BottomToTop } else { ReadoutDirection::TopToBottom }))
}

/// The names a profile made here goes by: the clip's own camera (`camera`: brand and model, as the file says), and
/// the lens `like` names - the profile the fit started from, or the one that was loaded - when that is a profile of
/// this camera. A profile of another camera has nothing to say of this one's lens, and nor has the setting of
/// another mode of this one: that isn't copied at all
fn name_after(p: &mut LensProfile, camera: (&str, &str), like: Option<&LensProfile>) {
    if let Some(like) = like.filter(|l| candidates::of_camera(camera, l)) {
        p.camera_brand = like.camera_brand.clone();
        p.camera_model = like.camera_model.clone();
        p.lens_model = like.lens_model.clone();
        // The crop factor is the mode's: only that of a profile of this very frame says anything of it
        if like.calib_dimension.w == p.calib_dimension.w && like.calib_dimension.h == p.calib_dimension.h { p.crop_factor = like.crop_factor; }
    }
    if !camera.0.is_empty() { p.camera_brand = camera.0.to_string(); }
    if !camera.1.is_empty() { p.camera_model = camera.1.to_string(); }
}

/// The profile of an estimate: the lens in air, with the readout time measured, marked as estimated from the video
/// (`LensProfile::estimated_from_video`) so it isn't taken for a calibration. `readout_ms`: as the project counts it
/// (`FrameTransform::readout_scale`), negative for a sensor read from the bottom up. `calibrated_by`: who made it, as
/// for any profile. `like`: see `name_after`
pub fn profile(estimate: &Estimate, readout_ms: Option<f64>, size: (usize, usize), fps: f64, under_water: bool, camera: (&str, &str), like: Option<&LensProfile>, calibrated_by: &str) -> LensProfile {
    let mut p = Lens { refraction: 1.0, ..estimate.lens }.to_profile(size);
    name_after(&mut p, camera, like);
    p.fps = fps;
    p.calibrated_by = calibrated_by.to_string();
    p.estimated_from_video = true;
    p.note = format!("Estimated from the video: {} frames, error {:.2} px{}", estimate.scene.frames, estimate.error, if under_water { ", shot under water (this is the lens in air)" } else { "" });
    if let Some((ms, direction)) = readout_for_profile(readout_ms) {
        p.frame_readout_time = Some(ms);
        p.frame_readout_direction = Some(direction);
    }
    p.fisheye_params.RMS_error = estimate.error * px_scale(size);
    p.num_images = estimate.scene.frames;
    p.init();
    p.finalize();
    p
}

/// The profile of a known lens that was brought to the clip's frame from a profile of another size (`How::Scaled`,
/// `How::Cropped`): the same calibration, so still its calibrator's, with a note of where it was brought from.
/// It isn't an estimate. None for a lens that is a profile as it is, or is no profile at all
pub fn brought_profile(candidate: &Candidate, size: (usize, usize), fps: f64) -> Option<LensProfile> {
    let source = candidate.profile.as_ref()?;
    if !matches!(candidate.how, How::Scaled | How::Cropped) { return None; }
    let (sw, sh) = (source.calib_dimension.w, source.calib_dimension.h);
    let mut p = Lens { refraction: 1.0, ..candidate.lens }.to_profile(size);
    p.camera_brand = source.camera_brand.clone();
    p.camera_model = source.camera_model.clone();
    p.lens_model = source.lens_model.clone();
    p.calibrated_by = source.calibrated_by.clone();
    p.fps = fps;
    // A window of the mode is a smaller part of the sensor: the crop factor grows as the window shrinks
    p.crop_factor = source.crop_factor.map(|c| if candidate.how == How::Cropped && size.0 > 0 { c * sw as f64 / size.0 as f64 } else { c });
    p.note = format!("Brought from \"{}\" ({sw}x{sh}): {} to {}x{}", candidate.name, if candidate.how == How::Cropped { "a window of it, pixel for pixel, cropped" } else { "scaled" }, size.0, size.1);
    p.init();
    p.finalize();
    Some(p)
}

/// Why an estimate is not to be trusted, as codes for the UI to put into words; empty when nothing is wrong. A lens
/// whose focal length the fragment doesn't hold is the one real trap: the fit then has an answer and no reason for it
pub fn doubts(estimate: &Estimate) -> Vec<&'static str> {
    let mut out: Vec<&str> = Vec::new();
    // Firmness is only a flag where it's negative - the fragment would rather have a focal length 10% off. As a
    // threshold it threw good clips away: where the tracking noise is a pixel or two, a wrong focal length doesn't
    // show as a worse fit, however right the answer is. What tells a good answer from a bad one is other fragments
    // agreeing with it, which is what checking the profile over the clip does
    if estimate.firmness < 0.0 { out.push("focal_not_held"); }
    if estimate.scene.parallax < 0.7 { out.push("little_parallax"); }
    // Pixels of a 2.7K frame, whatever the frame (`Bundle::check`)
    if estimate.error > 2.5 { out.push("poor_fit"); }
    if estimate.scene.points < 400 { out.push("few_points"); }
    out
}

/// What the render applies to the lens frame by frame from the file's own metadata, and the fit here doesn't model:
/// a recorded digital zoom (the render multiplies the focal length by it), the camera's own focal length or
/// distortion, lens breathing, a sensor or lens that moves to stabilize, a distortion mesh. A known profile would be
/// tried without it and fail on the very clip it's right for; a lens estimated from the picture would have it baked
/// in, and the render would apply it a second time. Such a clip is refused, and this is the code of why
pub fn unmodelled(metadata: &crate::gyro_source::FileMetadata) -> Option<&'static str> {
    if metadata.digital_zoom.is_some_and(|z| z > 0.0 && (z - 1.0).abs() > 1e-6) { return Some("digital_zoom"); }
    if !metadata.lens_positions.is_empty() || metadata.lens_params.values().any(|p| p.has_projection_data()) { return Some("lens_metadata"); }
    if !metadata.lens_breathing.is_empty() { return Some("lens_breathing"); }
    if !metadata.camera_stab_data.is_empty() { return Some("sensor_shift"); }
    if !metadata.mesh_correction.frames.is_empty() { return Some("mesh"); }
    None
}

impl LensProfile {
    /// Made by "Find the lens from the video", not calibrated on a target
    pub fn is_estimated_from_footage(&self) -> bool { self.estimated_from_video }
}

#[cfg(test)]
mod tests;
