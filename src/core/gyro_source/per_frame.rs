// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Per-frame tables of the file metadata (`FileMetadata::per_frame_time_offsets`, `camera_stab_data`,
//! `mesh_correction.frames`, `lens_breathing`), built while walking the telemetry samples. Every reader looks them
//! up by the video frame (`frame_at_timestamp`), so entry k has to be frame k's whatever the samples before it held:
//! values are put at the frame their sample describes, never appended, and a frame without a value of its own is
//! filled in explicitly once all of them are known.
//!
//! Which frame numbers count and what a table costs are separate. `limit_frames` decides the first, from evidence the
//! frame numbers can't make up themselves: the video's frame count, or without it the gaps a stream can have. A table
//! keeps only the values it's given while the samples are walked (`PerFrame`), whatever frames they're spread over,
//! and is laid out by frame once all of them are in: its storage counted and taken in one fallible allocation
//! (`table`), so the load drops a table the memory can't hold instead of aborting. That holds for every buffer a table
//! needs by frame, not only the first one: a table computed for every frame of a run is laid out in its own storage
//! (`hold_leading`), and a pass over a table's frames collects into storage taken the same way (`collect_table`)

use rayon::iter::{ IndexedParallelIterator, ParallelExtend };

/// The longest run of frames without metadata the frame numbers of a clip of unknown length can have, telemetry-parser's
/// bound on the gaps of a stream (`FrameIndex::MAX_GAP`): 16384 frames, over a minute at 240 fps. The gaps a camera or
/// a producer leaves are the frames whose metadata it couldn't write or read, a few at a time; a number further on is a
/// broken one
pub const MAX_GAP: usize = 1 << 14;

/// How many frames the per-frame tables of a clip can have: a metadata sample numbered at or past it describes no
/// frame. `video_frames` is the video's frame count when it's known, `sample_frames` the frame each sample of the
/// metadata describes (`None` for one that describes none).
///
/// The tables have an entry for every frame up to the last one with a value, so they're as long as the metadata's
/// largest frame number, and those can be broken: one glitch in a producer's frame counter, or one bogus sample time,
/// puts a frame millions of frames past the end of the recording. So the frames that count come from evidence the
/// numbers don't make up themselves:
/// - with the video's frame count, every frame of the video is a real one, with or without a sample of its own, and
///   past it there's to be at least one sample for every frame without one (an estimated count can be a frame short)
/// - without it, the frames the metadata reaches from the clip's first frame through gaps no longer than a stream's
///   can be (`MAX_GAP`), as telemetry-parser bounds the frames of a stream of unknown length: a number past a longer
///   gap describes no frame, and neither does any further one. Metadata for every few frames, or with frames missing,
///   stays whole, and however a sample is numbered, it adds at most `MAX_GAP` frames without a value to the tables
///
/// A healthy file numbers its samples one per frame from the first, within both
pub fn frame_limit(video_frames: Option<usize>, sample_frames: &[Option<usize>]) -> usize {
    match video_frames {
        Some(video_frames) => video_frames.max(sample_frames.len().saturating_mul(2)),
        None => {
            let mut frames: Vec<usize> = sample_frames.iter().flatten().copied().collect();
            frames.sort_unstable();
            let mut reached = 0; // the frames up to the last one reached
            for frame in frames {
                if frame.saturating_sub(reached) > MAX_GAP { break; }
                reached = reached.max(frame.saturating_add(1));
            }
            reached
        }
    }
}

/// Sets the frame of every sample numbered at or past the frames the clip can have (`frame_limit`) to `None`, with one
/// warning for all of them
pub fn limit_frames(video_frames: Option<usize>, sample_frames: &mut [Option<usize>]) {
    let limit = frame_limit(video_frames, sample_frames);
    let mut past = 0;
    for frame in sample_frames.iter_mut().filter(|frame| frame.is_some_and(|frame| frame >= limit)) {
        past += 1;
        *frame = None;
    }
    if past > 0 {
        let samples = sample_frames.len();
        match video_frames {
            Some(n) => log::warn!("{past} of {samples} metadata samples are numbered past the {limit} frames the clip can have (the video has {n}), ignored"),
            None => {
                let after = limit.checked_sub(1).map_or_else(|| "the clip's start".to_owned(), |last| format!("frame {last}"));
                log::warn!("{past} of {samples} metadata samples are numbered past a gap of more than {MAX_GAP} frames after {after}, longer than a gap in the metadata can be when the video's frame count isn't known, ignored");
            }
        }
    }
}

/// The storage of a table laid out by frame, `frames` entries of it: counted with checked arithmetic and taken in one
/// fallible allocation. `None`, with an error naming the table (`what`), when the memory can't hold it: the table is
/// dropped then, and the load goes on without it
pub fn table<U>(frames: usize, what: &str) -> Option<Vec<U>> {
    let mut storage = Vec::new();
    reserve(&mut storage, frames, what).then_some(storage)
}

/// Room in a table's storage for `more` frames past the ones it has, taken like `table` takes it: `false`, with its
/// error, when the memory can't hold them. Nothing is allocated when the storage already has the room
fn reserve<U>(storage: &mut Vec<U>, more: usize, what: &str) -> bool {
    match storage.try_reserve_exact(more) {
        Ok(()) => true,
        Err(e) => {
            log::error!("{what}: a table of {} frames of {} bytes each can't be allocated ({e}), dropped", storage.len().saturating_add(more), std::mem::size_of::<U>());
            false
        }
    }
}

/// A table laid out by frame whose frames from `first` on all have a value of their own (`values`, frame `first + i`
/// at `i`), with the frames before `first` filled in: they take the first value, as `PerFrame::hold` fills in the
/// frames before its first value. Their storage is reserved like `table` takes it, and is already there when the
/// table's storage was taken for all of its frames: the values aren't moved to other storage, so the table never
/// needs a second buffer of its size. `None` when the memory can't hold it
pub fn hold_leading<T: Clone>(mut values: Vec<T>, first: usize, what: &str) -> Option<Vec<T>> {
    let Some(value) = values.first().filter(|_| first > 0).cloned() else { return Some(values) };
    if !reserve(&mut values, first, what) { return None; }
    // Put after the values and turned to the front, in place
    values.extend(std::iter::repeat_n(value, first));
    values.rotate_right(first);
    report_filled(what, first, values.len());
    Some(values)
}

/// A pass over the frames of a table, one value for each of them (`values`, in frame order), collected into storage
/// taken like `table` takes it. `None` when the memory can't hold it
pub fn collect_table<T>(values: impl ExactSizeIterator<Item = T>, what: &str) -> Option<Vec<T>> {
    let mut out = table(values.len(), what)?;
    out.extend(values);
    Some(out)
}

/// `collect_table` of a pass computed in parallel
pub fn par_collect_table<T: Send>(values: impl IndexedParallelIterator<Item = T>, what: &str) -> Option<Vec<T>> {
    let mut out = table(values.len(), what)?;
    out.par_extend(values);
    Some(out)
}

/// The warning for the frames of a table filled in from the frames around them, `missing` of its `frames`
fn report_filled(what: &str, missing: usize, frames: usize) {
    log::warn!("{what}: {missing} of {frames} frames have none, filled in from the frames around them");
}

/// Values by video frame, as the samples give them: only the frames that have one take storage, whatever frames
/// they're spread over, until the table is laid out by frame (`hold`, `interpolate`, `into_options`). The frame
/// numbers are bounded by the caller (`limit_frames`)
pub struct PerFrame<T> {
    /// What the table is, for its messages
    what: &'static str,
    /// (frame, value) in the order they were set, by frame and without a repeated frame while `ordered`
    values: Vec<(usize, T)>,
    /// Whether `values` is by frame: it stays so while the samples come in frame order, and is sorted once otherwise
    ordered: bool,
    duplicates: usize,
}

impl<T> PerFrame<T> {
    pub fn new(what: &'static str) -> Self {
        Self { what, values: Vec::new(), ordered: true, duplicates: 0 }
    }
    /// The value of `frame`. A second value for the same frame is a sample the parser numbered like another one, and
    /// the first one stays
    pub fn set(&mut self, frame: usize, value: T) {
        match self.values.last() {
            Some((last, _)) if *last == frame => {
                self.duplicates += 1;
                return;
            }
            Some((last, _)) if *last > frame => self.ordered = false,
            _ => {}
        }
        self.values.push((frame, value));
    }
    /// Puts the values in frame order, keeping the first one set for a frame
    fn settle(&mut self) {
        if self.ordered { return; }
        self.values.sort_by_key(|(frame, _)| *frame); // stable: the values of a frame stay in the order they were set
        let set = self.values.len();
        self.values.dedup_by_key(|(frame, _)| *frame);
        self.duplicates += set - self.values.len();
        self.ordered = true;
    }
    /// Whether `frame` has a value
    pub fn has(&self, frame: usize) -> bool {
        if self.ordered {
            self.values.binary_search_by_key(&frame, |(f, _)| *f).is_ok()
        } else {
            self.values.iter().any(|(f, _)| *f == frame)
        }
    }
    /// The values of the frames that have one, in frame order
    pub fn values(&mut self) -> impl Iterator<Item = &T> {
        self.settle();
        self.values.iter().map(|(_, v)| v)
    }
    /// Frames of the table laid out by frame: every frame up to the last one with a value, none when no frame has one
    fn frames(&self) -> usize {
        self.values.last().map_or(0, |(frame, _)| frame + 1)
    }
    /// Logs what didn't fit: frames without a value of their own among the ones that have one, and values that
    /// couldn't be placed
    pub fn report(&mut self) {
        self.settle();
        let (frames, known) = (self.frames(), self.values.len());
        let missing = frames - known;
        if missing > 0 && known > 0 {
            report_filled(self.what, missing, frames);
        }
        if self.duplicates > 0 {
            log::warn!("{}: {} samples describe a frame another sample already did, ignored", self.what, self.duplicates);
        }
    }
    /// By frame, `None` where the frame has no value. Empty when no frame has a value, or when the memory can't hold it
    /// (`table`)
    pub fn into_options(mut self) -> Vec<Option<T>> {
        self.settle();
        let Some(mut out) = table::<Option<T>>(self.frames(), self.what) else { return Vec::new() };
        for (frame, v) in self.values {
            out.resize_with(frame, || None);
            out.push(Some(v));
        }
        out
    }
}

impl<T: Clone> PerFrame<T> {
    /// One value for every frame up to the last one with a value: a frame without one of its own takes the previous
    /// frame's, and the frames before the first value take that one. The values these tables hold (a sensor shift
    /// trajectory, a distortion mesh, a zoom) change little from one frame to the next, so the neighbour's is the
    /// closest the metadata comes to the missing one, where none would be the uncorrected frame popping out of the
    /// sequence. Empty when no frame has a value, or when the memory can't hold it (`table`). A frame's own value is
    /// moved, only the filled in ones are copies
    pub fn hold(mut self) -> Vec<T> {
        self.settle();
        let Some(mut out) = table::<T>(self.frames(), self.what) else { return Vec::new() };
        for (frame, v) in self.values {
            // The frames before this one without a value: the leading ones take this one, the others the one before
            match out.len().checked_sub(1) {
                Some(last) => while out.len() < frame { out.push(out[last].clone()); },
                None => out.extend(std::iter::repeat_n(&v, frame).cloned()),
            }
            out.push(v);
        }
        out
    }
}

impl PerFrame<f64> {
    /// One value for every frame up to the last one with a value: a frame without one of its own is linear between the
    /// nearest frames before and after it that have one, and the frames before the first value take that one. The
    /// values are times (exposure and readout midpoints, clock offsets), which stay constant or drift: the line through
    /// the neighbours is exact for both, and off by less than the step where they jump. Empty when no frame has a
    /// value, or when the memory can't hold it (`table`)
    pub fn interpolate(mut self) -> Vec<f64> {
        self.settle();
        let Some(mut out) = table::<f64>(self.frames(), self.what) else { return Vec::new() };
        let mut prev: Option<(usize, f64)> = None;
        for (j, b) in self.values {
            // The frames before this one without a value, between the one before and this one
            for k in out.len()..j {
                out.push(match prev {
                    Some((p, a)) => a + (b - a) * (k - p) as f64 / (j - p) as f64,
                    None => b,
                });
            }
            out.push(b);
            prev = Some((j, b));
        }
        out
    }
}
