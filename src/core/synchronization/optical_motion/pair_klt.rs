// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! KLT between any two frames, for the synchronization's PyrLK (`synchronization::OFPyrLK`): the corners of a frame as
//! good_features_to_track picks them, and points followed from one frame into another - the tracker's KLT (`tracker.rs`)
//! for pairs. On the GPU when there is one (`gpu_klt`), with OpenCV without, or from the call after the GPU failed. A
//! build without OpenCV has the GPU's KLT only.
//! The synchronization finds the corners of its frames and follows them into the next ones on a thread pool, in any
//! order: one `GpuKlt` takes one frame or one pair at a time. A pair goes up as its two frames, the second one tracked
//! from the first, which doesn't go up again when it's the frame that went up last (after its corners, or the pair before)

use std::sync::mpsc;
use parking_lot::Mutex;
use super::{ gpu_klt::GpuKlt, tracker::{ self, KltDevice } };

/// Without OpenCV there's no KLT but the GPU's
const NO_GPU: &str = "Synchronization with KLT needs a GPU in this build";

pub struct PairKlt {
    /// Room for this many points to track on the GPU
    max_points: usize,
    state: Mutex<State>,
}

struct State {
    /// The GPU's adapter, looked for and its device made on a thread from the start (see `tracker::find_gpu`), or none:
    /// the CPU is selected, there's no GPU, it couldn't be set up or it failed
    finding: Option<mpsc::Receiver<Result<wgpu::Adapter, String>>>,
    adapter: Option<wgpu::Adapter>,
    /// KLT on it, set up for the frame size, and the frame that went up last (its timestamp)
    gpu: Option<GpuKlt>,
    last: Option<i64>,
}

impl State {
    /// On the CPU from here on with OpenCV, without it not at all
    fn without_gpu(&mut self, why: String) {
        if cfg!(feature = "use-opencv") { log::warn!("Synchronization: KLT on the CPU, {why}"); } else { log::error!("{NO_GPU}: {why}"); }
        self.adapter = None;
        self.gpu = None;
        self.last = None;
    }
}

impl PairKlt {
    pub fn new(device: KltDevice, max_points: usize) -> Self {
        let finding = match device {
            KltDevice::Gpu(selected) => Some(tracker::find_gpu(selected)),
            KltDevice::Cpu => {
                if !cfg!(feature = "use-opencv") { log::error!("{NO_GPU}, and the CPU is selected for processing"); }
                None
            },
        };
        Self { max_points, state: Mutex::new(State { finding, adapter: None, gpu: None, last: None }) }
    }

    /// The strongest corners of a frame (`w` x `h`, rows `stride` bytes apart, told from the other frames by its
    /// timestamp, see `track`), picked as good_features_to_track picks them from the tracker's corner strengths: above
    /// `quality` times the strongest, each at least `min_distance` px from the stronger ones, at most `max`
    pub fn corners(&self, timestamp_us: i64, pixels: &[u8], (w, h): (u32, u32), stride: usize, max: usize, quality: f64, min_distance: f64) -> Result<Vec<[f32; 2]>, String> {
        if pixels.len() < stride * h as usize || stride < w as usize { return Err("not a whole frame".into()); }
        let on_gpu = self.on_gpu(w, h, stride, |gpu, last| {
            gpu.submit(pixels, std::iter::empty())?;
            gpu.finish()?;
            *last = Some(timestamp_us);
            Ok(std::mem::take(&mut gpu.candidates))
        });
        let mut candidates = match on_gpu {
            Some(candidates) => candidates,
            None => Self::candidates_on_cpu(pixels, w, h, stride)?,
        };
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        Ok(pick(&candidates, w as i32, h as i32, max, quality, min_distance))
    }

    /// Where each of `points` of frame `a` went in frame `b` (both `w` x `h`, rows `stride` bytes apart, told apart by
    /// their timestamps), by pyramidal Lucas-Kanade: None where it lost the point, or where the point tracked back didn't
    /// come within `fb_max` px of where it started
    pub fn track(&self, a: (i64, &[u8]), b: (i64, &[u8]), (w, h): (u32, u32), stride: usize, points: &[[f32; 2]], fb_max: f32) -> Result<Vec<Option<[f32; 2]>>, String> {
        if a.1.len() < stride * h as usize || b.1.len() < stride * h as usize || stride < w as usize { return Err("not a whole frame".into()); }
        if points.is_empty() { return Ok(Vec::new()); }
        if points.len() > self.max_points { return Err(format!("{} points to track, room for {}", points.len(), self.max_points)); }
        let on_gpu = self.on_gpu(w, h, stride, |gpu, last| {
            if *last != Some(a.0) {
                gpu.submit(a.1, std::iter::empty())?;
                gpu.finish()?;
            }
            gpu.submit(b.1, points.iter().copied())?;
            gpu.finish()?;
            *last = Some(b.0);
            Ok(tracker::checked(points, &gpu.fwd, &gpu.bwd, fb_max))
        });
        if let Some(moved) = on_gpu { return Ok(moved); }
        #[cfg(feature = "use-opencv")]
        {
            let frame = |pixels| tracker::frame_mat(pixels, stride, w as i32, h as i32);
            (|| tracker::Tracker::klt(&frame(a.1)?, &frame(b.1)?, points, fb_max))().map_err(tracker::cv)
        }
        #[cfg(not(feature = "use-opencv"))]
        Err(NO_GPU.into())
    }

    /// `op` on the GPU's KLT set up for the frame size, with the frame that went up last to it. None when there's no GPU
    /// to use, and from the first time it fails
    fn on_gpu<T>(&self, w: u32, h: u32, stride: usize, op: impl FnOnce(&mut GpuKlt, &mut Option<i64>) -> Result<T, String>) -> Option<T> {
        let mut state = self.state.lock();
        let state = &mut *state;
        if let Some(finding) = state.finding.take() {
            match tracker::found_gpu(finding) {
                Ok(adapter) => state.adapter = Some(adapter),
                Err(e) => state.without_gpu(e),
            }
        }
        if state.gpu.as_ref().is_some_and(|g| g.size != (w, h) || g.stride != stride as u32) {
            state.gpu = None;
            state.last = None;
        }
        if state.gpu.is_none() {
            match GpuKlt::new(state.adapter.as_ref()?, w, h, stride as u32, self.max_points) {
                Ok(gpu) => {
                    log::info!("Synchronization: KLT on {}", gpu.name());
                    state.gpu = Some(gpu);
                },
                Err(e) => {
                    state.without_gpu(format!("the GPU couldn't be set up for it: {e}"));
                    return None;
                }
            }
        }
        let done = op(state.gpu.as_mut()?, &mut state.last);
        done.map_err(|e| state.without_gpu(format!("the GPU failed: {e}"))).ok()
    }

    /// The candidate corners (`tracker::local_maxima`) in OpenCV's corner strengths, as the GPU finds them
    #[cfg(feature = "use-opencv")]
    fn candidates_on_cpu(pixels: &[u8], w: u32, h: u32, stride: usize) -> Result<Vec<u64>, String> {
        use opencv::prelude::*;
        let eig = tracker::frame_mat(pixels, stride, w as i32, h as i32).and_then(|frame| tracker::corner_strengths(&frame)).map_err(tracker::cv)?;
        Ok(tracker::local_maxima(eig.data_typed::<f32>().map_err(tracker::cv)?, w as i32, h as i32))
    }
    #[cfg(not(feature = "use-opencv"))]
    fn candidates_on_cpu(_pixels: &[u8], _w: u32, _h: u32, _stride: usize) -> Result<Vec<u64>, String> {
        Err(NO_GPU.into())
    }
}

/// good_features_to_track's choice among the candidate corners (`tracker::local_maxima`, the strongest first): above
/// `quality` times the strongest, each at least `min_distance` from the ones picked before it, at most `max`
fn pick(candidates: &[u64], w: i32, h: i32, max: usize, quality: f64, min_distance: f64) -> Vec<[f32; 2]> {
    let strength = |k: u64| f32::from_bits((k >> 32) as u32) as f64;
    let Some(&strongest) = candidates.first() else { return Vec::new() };
    let threshold = quality * strength(strongest);
    let mut free = vec![true; (w * h) as usize];
    let mut out = Vec::with_capacity(max);
    for &k in candidates {
        if out.len() >= max || strength(k) <= threshold { break; }
        let (x, y) = ((k & 0xffff) as i32, (k >> 16 & 0xffff) as i32);
        if !free[(y * w + x) as usize] { continue; }
        tracker::clear_disc(&mut free, w, h, x as f64, y as f64, min_distance);
        out.push([x as f32, y as f32]);
    }
    out
}

#[cfg(all(test, feature = "use-opencv"))]
mod tests {
    use super::*;
    use opencv::{ core::{ Mat, Point2f, Vector }, prelude::* };
    use super::super::tracker::tests::{ W, H, texture, moved, apply, invert, gpu, median };

    const MAX: usize = 200;
    const SIZE: (u32, u32) = (W as u32, H as u32);

    /// The corners: on the CPU exactly good_features_to_track's (of block 7), on the GPU the same but where two strengths
    /// are a rounding apart
    #[test]
    fn corners_as_good_features_to_track() {
        let frame = texture();
        let px = frame.data_bytes().unwrap();
        let mut found = Vector::<Point2f>::new();
        opencv::imgproc::good_features_to_track(&frame, &mut found, MAX as i32, 0.01, 10.0, &Mat::default(), 7, false, 0.04).unwrap();
        let expected: Vec<[f32; 2]> = found.iter().map(|p| [p.x, p.y]).collect();
        assert_eq!(expected.len(), MAX);
        assert_eq!(PairKlt::new(KltDevice::Cpu, MAX).corners(0, px, SIZE, W as usize, MAX, 0.01, 10.0).unwrap(), expected);
        if gpu().is_none() { return; }
        let on_gpu = PairKlt::new(KltDevice::Gpu(None), MAX).corners(0, px, SIZE, W as usize, MAX, 0.01, 10.0).unwrap();
        let same = on_gpu.iter().filter(|p| expected.contains(p)).count();
        eprintln!("corners on the GPU: {same} of {} good_features_to_track's", expected.len());
        assert!(on_gpu.len() == MAX && same * 100 >= MAX * 98, "{same} of {} corners on the GPU are good_features_to_track's", on_gpu.len());
    }

    /// The tracks of a few pairs, in the order the synchronization may ask for them: on the GPU as on the CPU and as the
    /// true motion, and the same from a first frame that went up last as from one that goes up again
    #[test]
    fn pairs_on_gpu_as_on_cpu() {
        if gpu().is_none() { return; }
        let first = texture();
        let frames: Vec<(Mat, [f64; 6])> = (0..4).map(|i| moved(&first, 0.2 * i as f64, (1.3 * i as f64, -0.9 * i as f64))).collect();
        let frame = |i: usize| (i as i64, frames[i].0.data_bytes().unwrap());
        let (on_gpu, on_cpu) = (PairKlt::new(KltDevice::Gpu(None), MAX), PairKlt::new(KltDevice::Cpu, MAX));
        let corners: Vec<Vec<[f32; 2]>> = (0..frames.len()).map(|i| on_gpu.corners(i as i64, frame(i).1, SIZE, W as usize, MAX, 0.01, 10.0).unwrap()).collect();
        let dist = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        let (mut apart, mut off, mut kept) = (Vec::new(), Vec::new(), (0, 0, 0));
        let mut first_12 = None;
        // The first frame went up last for (1, 2) and (2, 3)
        for (a, b) in [(0, 1), (1, 2), (0, 2), (2, 3)] {
            let g = on_gpu.track(frame(a), frame(b), SIZE, W as usize, &corners[a], 1.0).unwrap();
            let c = on_cpu.track(frame(a), frame(b), SIZE, W as usize, &corners[a], 1.0).unwrap();
            let to_b = |p: [f32; 2]| { let t = apply(&frames[b].1, apply(&invert(&frames[a].1), (p[0] as f64, p[1] as f64))); [t.0 as f32, t.1 as f32] };
            for (p, (g, c)) in corners[a].iter().zip(g.iter().zip(&c)) {
                kept = (kept.0 + 1, kept.1 + g.is_some() as usize, kept.2 + c.is_some() as usize);
                if let (Some(g), Some(c)) = (g, c) {
                    apart.push(dist(*g, *c));
                    off.push(dist(*g, to_b(*p)));
                }
            }
            if (a, b) == (1, 2) { first_12 = Some(g); }
        }
        on_gpu.corners(3, frame(3).1, SIZE, W as usize, MAX, 0.01, 10.0).unwrap();
        let again = on_gpu.track(frame(1), frame(2), SIZE, W as usize, &corners[1], 1.0).unwrap();
        assert_eq!(first_12.unwrap(), again, "(1, 2) sent again");
        eprintln!("kept of {}: GPU {}, CPU {}; GPU vs CPU median {} px, max {}; off the true motion median {} px",
            kept.0, kept.1, kept.2, median(apart.clone()), apart.iter().fold(0.0f32, |a, b| a.max(*b)), median(off.clone()));
        assert!(kept.1 * 100 >= kept.0 * 95 && kept.1.abs_diff(kept.2) * 100 <= kept.0, "kept of {}: GPU {}, CPU {}", kept.0, kept.1, kept.2);
        assert!(median(apart.clone()) < 0.001 && apart.iter().all(|d| *d < 0.05), "GPU vs CPU: median {} px", median(apart));
        assert!(median(off.clone()) < 0.02, "off the true motion: median {} px", median(off));
    }
}
