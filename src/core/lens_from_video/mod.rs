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
//!   then the lens is let go for a few steps. The right lens stays where it is (2-6 px over a 2.7K frame); one that is
//!   off by 3% in focal length or belongs to another mode walks away (16-90 px). The profiles of the camera in the
//!   database are tried nearest first (`candidates`), on three fragments of the clip: one fragment's verdict isn't
//!   trusted, the fragments are weighed by how much of a scene they had (`find_known`).
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
pub use candidates::{ Candidate, candidates };

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
/// interval. Fits loosely: only the median holds
const FIT_MEDIAN: f64 = 15.0;
const FIT_MAX: f64 = 40.0;
const FIT_READOUT: f64 = 0.06;
const PROBE_STEPS: usize = 5;
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

/// What was tracked in one fragment of the clip
#[derive(Clone, Debug, Default)]
pub struct Fragment {
    /// The video's frame: the pixels of the lens
    pub size: (usize, usize),
    pub frames: usize,
    /// Time between two of its frames, ms
    pub frame_ms: f64,
    /// When each of its frames was taken, in those intervals from the first: 0, 1, 2... with a jump where a frame of
    /// the video is missing or was dropped (`FragmentBuilder::push_frame`)
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
        while self.times.len() < self.frames { self.times.push(self.times.len() as f64); }
    }

    /// The tracks long enough to count, numbered from 0
    pub fn tracks(&self) -> Tracks {
        let mut count: HashMap<u32, usize> = HashMap::new();
        for t in &self.track { *count.entry(*t).or_insert(0) += 1; }
        let mut ids: Vec<u32> = count.iter().filter(|(_, n)| **n >= MIN_TRACK).map(|(t, _)| *t).collect();
        ids.sort_unstable();
        let index: HashMap<u32, u32> = ids.iter().enumerate().map(|(i, t)| (*t, i as u32)).collect();
        let mut out = Tracks { size: self.size, frames: self.frames, tracks: ids.len(), times: self.times.clone(), ..Default::default() };
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
    /// (0, 0) of the tracker is the middle of the top left pixel. `slot`: which frame of the fragment this is, counted
    /// in the frame steps the caller tracks at - so a frame of the video that is missing or out of order leaves a gap,
    /// and the fit knows the camera moved further between the two frames either side of it
    pub fn push_frame(&mut self, slot: usize, observations: &[Observation]) {
        let f = self.fed;
        self.fed += 1;
        self.fragment.times.push(slot as f64);
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
        self.fragment
    }
}

#[derive(Clone, Debug, Default)]
pub struct Tracks {
    pub size: (usize, usize),
    pub frames: usize,
    pub tracks: usize,
    /// When each frame was taken, see `Fragment::times`
    pub times: Vec<f64>,
    pub track: Vec<u32>,
    pub frame: Vec<u32>,
    pub uv: Vec<[f64; 2]>,
}

/// How a known profile did on one fragment
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct ProfileCheck {
    pub scene: SceneInfo,
    /// How far the lens moved when it was let go, pixels of the frame
    pub moved: LensShift,
    /// The readout time with the profile as it is (the profile's own, or fitted when it has none) and with the lens
    /// let go, in frame intervals of the fragment
    pub readout: f64,
    pub readout_free: f64,
    /// Error of the held-out tracks with the profile as it is, and with the lens let go
    pub error: f64,
    pub error_free: f64,
    pub fits: bool,
    /// The median holds, but a corner moved or the readout did
    pub fits_loosely: bool,
}
impl ProfileCheck {
    /// How much this fragment's verdict counts: a fragment where the camera hardly moved through space, or with few
    /// points, knows little of the lens
    pub fn weight(&self) -> f64 {
        (self.scene.parallax / 1.5).min(1.0) * (self.scene.points as f64 / 500.0).min(1.0)
    }
}

/// Is this the lens of the fragment? `readout`: the profile's own, in frame intervals of the fragment, when it has one.
/// The lens carries the refraction index to check it with
pub fn check_profile(tracks: &Tracks, lens: &Lens, readout: Option<f64>, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<ProfileCheck, NoScene> {
    let s = px_scale(tracks.size);
    let loss = LOSS * s;
    let (scene, info) = scene::rough_scene(tracks, lens, POINTS, HOLD_OUT)?;
    let mut bundle = Bundle::new(scene);
    bundle.select_every(THIN);
    // The poses and the points settle with the lens as it is, and the readout time is found if the profile has none
    bundle.fit(*lens, readout.unwrap_or(0.0), &FitOptions::new(Free::NONE, 4, 4.0 * loss), cancelled);
    let (_, tau, _) = bundle.fit(*lens, readout.unwrap_or(0.0), &FitOptions::new(if readout.is_some() { Free::NONE } else { Free::READOUT }, 12, loss).tolerance(0.5 * s), cancelled);
    if bundle.parallax(lens, tau) < scene::MIN_PARALLAX_FITTED { return Err(NoScene::NoTranslation); }
    let as_it_is = bundle.check(lens, tau, loss);
    // Then the lens is let go
    let (free, tau_free, _) = bundle.fit(*lens, tau, &FitOptions::new(Free::ALL, PROBE_STEPS, loss).tolerance(0.05 * s), cancelled);
    let let_go = bundle.check(&free, tau_free, loss);
    let moved = lens.shift_after_tilt(&free, tracks.size);
    let fits_loosely = moved.median < FIT_MEDIAN * s;
    Ok(ProfileCheck {
        scene: info, moved, readout: tau, readout_free: tau_free, error: as_it_is.rms / s, error_free: let_go.rms / s,
        fits: fits_loosely && moved.max < FIT_MAX * s && (tau_free - tau).abs() < FIT_READOUT,
        fits_loosely,
    })
}

/// How well a lens fits the clip, over the fragments: the one measure both a known profile and an estimate are judged by,
/// so the two can be put side by side
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct Match {
    /// "good": it holds where the clip has a scene; "loose": it holds over the frame but a corner or the readout time
    /// moves; "poor": it walks away when let go
    pub verdict: Verdict,
    /// Fragments it holds on, and those that had a scene at all
    pub fragments_fitting: usize,
    pub fragments: usize,
    /// How far the lens moved when it was let go, pixels of the video, over the fragments (weighted): the median over
    /// the frame, and the largest in a corner. It holds under `limit` (`FIT_MEDIAN` brought to the frame's size)
    pub moved: f64,
    pub moved_corner: f64,
    pub limit: f64,
    /// Readout time it was measured to have, ms
    pub readout_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict { Good, Loose, #[default] Poor }

/// A lens against every fragment, in one medium
#[derive(Clone, Debug, serde::Serialize)]
pub struct MediumResult {
    pub refraction: f64,
    /// One per fragment; none where no scene could be made
    pub checks: Vec<Option<ProfileCheck>>,
    /// Weight of the fragments it fits, fits loosely, and of all that had a scene
    pub fitting: f64,
    pub fitting_loosely: f64,
    pub total: f64,
    /// Error of the held-out tracks with it as it is, over the fragments, at 2.7K
    pub error: f64,
    #[serde(rename = "match")]
    pub matching: Match,
}
impl MediumResult {
    fn new(refraction: f64, checks: Vec<Option<ProfileCheck>>, fragments: &[Fragment], scale: f64) -> Self {
        let weight = |c: &ProfileCheck| c.weight();
        let total: f64 = checks.iter().flatten().map(weight).sum();
        let fitting: f64 = checks.iter().flatten().filter(|c| c.fits).map(weight).sum();
        let fitting_loosely: f64 = checks.iter().flatten().filter(|c| c.fits_loosely).map(weight).sum();
        let mean = |f: &dyn Fn(&ProfileCheck) -> f64| if total > 0.0 { checks.iter().flatten().map(|c| f(c) * weight(c)).sum::<f64>() / total } else { f64::INFINITY };
        let readout_ms = weighted_median(checks.iter().zip(fragments).filter_map(|(c, f)| c.filter(|c| c.fits_loosely).map(|c| (c.readout * f.frame_ms, c.weight()))).collect());
        let holds = |w: f64| w >= 0.5 && w >= total * 2.0 / 3.0;
        Self {
            refraction,
            error: mean(&|c| c.error),
            matching: Match {
                verdict: if holds(fitting) { Verdict::Good } else if holds(fitting_loosely) { Verdict::Loose } else { Verdict::Poor },
                fragments_fitting: checks.iter().flatten().filter(|c| c.fits).count(),
                fragments: checks.iter().flatten().count(),
                moved: finite(mean(&|c| c.moved.median)),
                moved_corner: finite(mean(&|c| c.moved.corner)),
                limit: FIT_MEDIAN * scale,
                readout_ms,
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
    pub tried: Vec<CandidateResult>,
    /// The first one that fits, in `tried`
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
        self.accepted.or_else(|| self.tried.iter().enumerate().filter(|(_, x)| x.result().total > 0.0)
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
/// corner, 96% when sure. Returns the medium, and whether the clip was sure of it
fn choose_medium(media: &[MediumResult]) -> (usize, bool) {
    if media.len() < 2 { return (0, true); }
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
        let w = checks.iter().map(|c| c.weight()).sum::<f64>() / checks.len() as f64;
        votes[best] += w;
        voters += 1;
        if first_vote.is_some_and(|v| v != best) { agree = false; }
        first_vote.get_or_insert(best);
        if checks[second].moved.corner > 1.5 * checks[best].moved.corner && checks[best].error <= checks[second].error { sure_voters += 1; }
    }
    if voters == 0 {
        // No fragment had a scene in both: the one that holds best, unsure
        let best = (0..media.len()).max_by(|a, b| media[*a].fitting.total_cmp(&media[*b].fitting).then(media[*b].matching.moved.total_cmp(&media[*a].matching.moved))).unwrap_or(0);
        return (best, false);
    }
    let best = (0..media.len()).max_by(|a, b| votes[*a].total_cmp(&votes[*b])).unwrap_or(0);
    (best, agree && sure_voters == voters)
}

/// Tries the known profiles in their order on every fragment, in each of `media` (refraction indices: 1 in air, 1.33
/// under water - both when nobody knows), and stops at the first one that fits: in the medium the clip votes for, the
/// fragments it fits weigh at least two thirds of those that had a scene, and at least half a good fragment in all.
/// `progress` gets the fraction done
pub fn find_known(fragments: &[Fragment], candidates: &[Candidate], media: &[f64], cancelled: &(dyn Fn() -> bool + Sync), progress: &(dyn Fn(f64) + Sync)) -> KnownProfiles {
    let tracks: Vec<Tracks> = fragments.iter().map(|f| f.tracks()).collect();
    let scale = fragments.first().map(|f| px_scale(f.size)).unwrap_or(1.0);
    let media: Vec<f64> = if media.is_empty() { vec![1.0] } else { media.to_vec() };
    let mut out = KnownProfiles::default();
    let mut with_scene = vec![false; fragments.len()];
    let mut no_translation = 0;
    for (n, candidate) in candidates.iter().enumerate() {
        if cancelled() { break; }
        // Every fragment in every medium at once
        let jobs: Vec<(usize, usize)> = (0..media.len()).flat_map(|m| (0..fragments.len()).map(move |f| (m, f))).collect();
        let results: Vec<Result<ProfileCheck, NoScene>> = jobs.par_iter().map(|&(m, f)| {
            let readout = candidate.readout_ms.filter(|_| fragments[f].frame_ms > 0.0).map(|ms| ms / fragments[f].frame_ms);
            check_profile(&tracks[f], &candidate.lens.with_refraction(media[m]), readout, cancelled)
        }).collect();
        if cancelled() { break; }
        no_translation += results.iter().filter(|r| matches!(r, Err(NoScene::NoTranslation))).count();
        let mut per_medium: Vec<Vec<Option<ProfileCheck>>> = vec![Vec::new(); media.len()];
        for ((m, f), r) in jobs.iter().zip(results) {
            if r.is_ok() { with_scene[*f] = true; }
            if let Err(e) = &r { log::debug!("Lens from video: {} in {}: fragment {f}: {e:?}", candidate.name, media[*m]); }
            per_medium[*m].push(r.ok());
        }
        let results: Vec<MediumResult> = per_medium.into_iter().zip(&media).map(|(checks, &n)| MediumResult::new(n, checks, fragments, scale)).collect();
        let (chosen, medium_sure) = choose_medium(&results);
        let accepted = results[chosen].matching.verdict == Verdict::Good;
        for r in &results {
            log::info!("Lens from video: {} [{}] - {}", candidate.name, if r.refraction != 1.0 { "water" } else { "air" }, r.checks.iter().map(|c| match c {
                Some(c) => format!("moved {:.1}/{:.1} corner {:.1} px, readout {:.3} -> {:.3}, error {:.2} -> {:.2}, weight {:.2}{}", c.moved.median, c.moved.max, c.moved.corner, c.readout, c.readout_free, c.error, c.error_free, c.weight(), if c.fits { " FITS" } else { "" }),
                None => "no scene".into(),
            }).collect::<Vec<_>>().join(" | "));
        }
        if media.len() > 1 { log::info!("Lens from video: {} - shot in {}{}", candidate.name, if results[chosen].refraction != 1.0 { "water" } else { "air" }, if medium_sure { "" } else { " (not sure)" }); }
        out.tried.push(CandidateResult { name: candidate.name.clone(), id: candidate.id.clone(), media: results, chosen, medium_sure, accepted });
        progress((n + 1) as f64 / candidates.len() as f64);
        if accepted { out.accepted = Some(n); break; }
    }
    out.fragments_with_scene = with_scene.iter().filter(|x| **x).count();
    out.no_translation = out.fragments_with_scene == 0 && no_translation > 0;
    out
}

/// How well a lens found some other way (an estimate) fits these fragments, in this medium: what `find_known` says of
/// a known profile, for the two to be compared
pub fn check_lens(fragments: &[Fragment], lens: &Lens, readout_ms: Option<f64>, cancelled: &(dyn Fn() -> bool + Sync)) -> Match {
    let candidate = Candidate { name: String::new(), id: String::new(), lens: Lens { refraction: 1.0, ..*lens }, readout_ms, loadable: false, is_loaded: false, profile: None, rank: (0, 0, String::new(), String::new()) };
    let known = find_known(fragments, &[candidate], &[lens.refraction], cancelled, &|_| ());
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
    let before = bundle.check(start, 0.0, loss);
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
    let (lens, tau, _) = bundle.fit(lens1, tau1, &FitOptions::new(Free::ALL, 60, loss).tolerance(0.05 * s), cancelled);
    let found = bundle.check(&lens, tau, loss);
    progress(0.8);
    // The lens is trusted only as far as the fragment resists a wrong focal length: the distortion is kept, it would
    // soak the change up
    let state = bundle.state();
    let mut firmness = f64::INFINITY;
    for scale in [0.9, 1.1] {
        if cancelled() { break; }
        let mut off = lens; off.fx *= scale; off.fy *= scale;
        let (off, tau_off, _) = bundle.fit(off, tau, &FitOptions::new(Free::CENTRE_READOUT, 12, loss).tolerance(0.3 * s), cancelled);
        firmness = firmness.min(bundle.check(&off, tau_off, loss).rms / found.rms - 1.0);
        bundle.restore(&state);
    }
    progress(1.0);
    Ok(Estimate {
        lens, readout: tau, readout_ms: tau * frame_ms, scene: info,
        // Capped, and never an infinity: these go to the user through json, which has no number for one
        error_before: finite(before.rms / s), error: finite(found.rms / s), within_1px: found.within_1px,
        firmness: if firmness.is_finite() { firmness } else { 0.0 },
    })
}

/// A Gyroflow lens profile of a lens found here: the lens in air, with the readout time measured (ms, negative for a
/// sensor read from the bottom up), marked as estimated from the video (`LensProfile::estimated_from_video`) so it isn't
/// taken for a calibration. `calibrated_by`: who made it, as for any profile. `like`: the profile it came from or the one
/// that was loaded, for the names of the camera and the lens
pub fn profile_of(lens: &Lens, readout_ms: Option<f64>, size: (usize, usize), fps: f64, under_water: bool, camera: (&str, &str), like: Option<&LensProfile>, calibrated_by: &str, note: &str) -> LensProfile {
    let mut p = Lens { refraction: 1.0, ..*lens }.to_profile(size);
    if let Some(like) = like {
        p.camera_brand = like.camera_brand.clone();
        p.camera_model = like.camera_model.clone();
        p.lens_model = like.lens_model.clone();
        p.camera_setting = like.camera_setting.clone();
        p.crop_factor = like.crop_factor;
    }
    if !camera.0.is_empty() { p.camera_brand = camera.0.to_string(); }
    if !camera.1.is_empty() { p.camera_model = camera.1.to_string(); }
    p.fps = fps;
    p.calibrated_by = calibrated_by.to_string();
    p.estimated_from_video = true;
    p.note = format!("{note}{}", if under_water { ", shot under water (this is the lens in air)" } else { "" });
    if let Some(ms) = readout_ms.filter(|ms| ms.abs() > 0.05) {
        p.frame_readout_time = Some(ms.abs());
        p.frame_readout_direction = Some(if ms < 0.0 { ReadoutDirection::BottomToTop } else { ReadoutDirection::TopToBottom });
    }
    p.init();
    p.finalize();
    p
}

/// The profile of an estimate
pub fn profile(estimate: &Estimate, size: (usize, usize), fps: f64, under_water: bool, camera: (&str, &str), like: Option<&LensProfile>, calibrated_by: &str) -> LensProfile {
    let note = format!("Estimated from the video: {} frames, error {:.2} px", estimate.scene.frames, estimate.error);
    let mut p = profile_of(&estimate.lens, Some(estimate.readout_ms), size, fps, under_water, camera, like, calibrated_by, &note);
    p.fisheye_params.RMS_error = estimate.error * px_scale(size);
    p.num_images = estimate.scene.frames;
    p.finalize();
    p
}

/// Why an estimate is not to be trusted, for the user; empty when nothing is wrong. A lens whose focal length the
/// fragment doesn't hold is the one real trap: the fit then has an answer and no reason for it
pub fn doubts(estimate: &Estimate) -> String {
    let mut out: Vec<&str> = Vec::new();
    // Firmness is only a flag where it's negative - the fragment would rather have a focal length 10% off. As a
    // threshold it threw good clips away: where the tracking noise is a pixel or two, a wrong focal length doesn't
    // show as a worse fit, however right the answer is. What tells a good answer from a bad one is other fragments
    // agreeing with it, which is what checking the profile over the clip does
    if estimate.firmness < 0.0 { out.push("this fragment doesn't hold its focal length (the camera has to move through space, not only turn)"); }
    if estimate.scene.parallax < 0.7 { out.push("the camera barely moved through space in it"); }
    if estimate.error > 2.5 { out.push("the tracks don't fit the result well (things moving on their own, motion blur, or a crop that changes from frame to frame because of in-camera stabilization)"); }
    if estimate.scene.points < 400 { out.push("there was little in the picture to track"); }
    out.join("; ")
}

impl LensProfile {
    /// Made by "Find the lens from the video", not calibrated on a target
    pub fn is_estimated_from_footage(&self) -> bool { self.estimated_from_video }
}

#[cfg(test)]
mod tests;
