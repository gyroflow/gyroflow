// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Segments of a video cut from camera recordings, from its segment table (`MTRM`, telemetry-parser's
//! `segment_table`): runs of frames that follow each other in one recording. Where one segment ends and the next one
//! starts, the picture and the motion jump to another moment of the recording, or to another recording, so every
//! segment boundary is a hard boundary: the motion data of each segment is integrated, smoothed and looked up on its
//! own, and nothing is interpolated or filtered across a boundary.
//!
//! The video's frames are in segments by the table. The motion data is too, but not always at the same times: a camera
//! writes a frame's readings into the metadata of the frames after it (Sony's gyroscope readings of a frame are in the
//! next two frames' metadata, DJI's attitude starts before its frame), so the first frames' metadata of a segment holds
//! readings timed before the segment's first frame, and the segment before it has none of its last frames' readings. So
//! a reading belongs to the segment its metadata sample is in, where that sample is the segment's own: its time is in
//! the segment on the file's timeline, where a container times its metadata samples, and its readings are timed with
//! it, give or take a few frames. That doesn't depend on how the video's time maps to the motion data's.
//!
//! A sample that isn't a segment's own holds the readings of all of the clip (RED's, Blackmagic's and MotionCam's IMU).
//! Where its format records each reading's place among the frames (telemetry-parser's `ReadingOrigin::frame`), a reading
//! belongs to the segment of its frame. Otherwise it belongs to the segment it's timed in, on the motion data's
//! timeline. gyroflow puts a video's time `t` at `t / fps_scale` of the motion data (`StabilizationParams::fps_scale`:
//! an off-speed recording's motion data is timed by its capture, and a user can override the frame rate), so where
//! these readings change segments depends on the scale, which is known only once the file is loaded and can change
//! after. The parts' motion data is placed on its timeline with the scale in force (`SegmentRanges`), from what the
//! readings' samples and frames tell, which is kept with the segments (`TrimSegments::readings`)
//!
//! The other metadata of each frame (its time offset, its lens, its distortion mesh, its focus breathing) belongs to the
//! segment of the frame, and is looked up and filled in within it: the metadata samples of each part are kept with the
//! segments (`TrimSegments::metadata_us`), as the lens metadata is keyed by their times
//!
//! The motion data can be another file's than the video's: a telemetry log beside it or loaded for it, another camera's
//! recording. Its readings are at the video's frames, and in the parts as above, only where the video's table names the
//! file as the log the video was trimmed with (telemetry-parser's `Input::readings_at_table_frames`). Otherwise where
//! each part's readings are in it isn't known (a log the video wasn't trimmed with: its source's untrimmed one is one
//! recording, of which every part shows another moment, in any order): the segments are the video's frames' only,
//! each synchronized and looked up on its own in all of the motion data, which is one piece
//! (`TrimSegments::motion_unplaced`)

use std::ops::{ Range, RangeInclusive };
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use telemetry_parser::segment_table::SegmentTable;
use telemetry_parser::util::{ ImuSensor, ReadingOrigin, SampleInfo, UnplacedValue };

/// How far the readings of a metadata sample can be timed from the sample, in frames of its segment: a camera writes
/// the readings of a frame into the metadata of up to the next two frames. Also how far a lookup of a segment's motion
/// data can be from it and still be held at its ends (`SegmentRanges::reach_us`): a segment's first and last frames can
/// have their readings in the segments beside it
const READINGS_REACH_FRAMES: f64 = 3.0;
/// How far a sample's own time can be from its segment's, in µs: the rounding of the sample's time to its track's ticks
/// (a millisecond for a GoPro's)
const SAMPLE_TIME_TOLERANCE_US: f64 = 1000.0;

/// The parts of a video that are runs of frames of one recording: its segment table's segments, those that follow each
/// other in their source taken together (§8.1 lets a reader merge them). A segment follows the one before in their
/// source when its first frame is the one after that one's last (output frame `out_first_frame + i` is source frame
/// `src_first_frame + i`) and the camera clock runs on between them: this file's clock value `c` is the source's
/// `c + clock_shift`, so the motion data timed by the clock goes on from the one before exactly when both segments'
/// shifts are the same. A segment whose clock was moved against the one before (the source's frames missing between
/// them, or a clock that the cut set to its nominal rate there) starts another part. Each part is the table's segments
/// `range`
pub fn contiguous_parts(table: &SegmentTable) -> Vec<Range<usize>> {
    let mut parts: Vec<Range<usize>> = Vec::new();
    for (r, s) in table.segments.iter().enumerate() {
        let follows = r > 0 && {
            let p = &table.segments[r - 1];
            p.source == s.source && p.src_first_frame + p.frame_count == s.src_first_frame && p.clock_shift == s.clock_shift
        };
        match parts.last_mut() {
            Some(last) if follows => last.end = r + 1,
            _ => parts.push(r..r + 1),
        }
    }
    parts
}

/// A kind of motion data: a series of readings timed on the motion data's timeline
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MotionSeries {
    /// The IMU readings (`FileMetadata::raw_imu`)
    Imu,
    /// The camera's orientations (`FileMetadata::quaternions`)
    Orientations,
}

/// Readings of a series that follow each other in time, all of one part by their metadata samples, or all of samples
/// that aren't a part's own
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReadingRun {
    pub series: MotionSeries,
    /// The first and the last reading's time, µs on the motion data's timeline (`motion_key`)
    pub first_us: i64,
    pub last_us: i64,
    /// The part of their samples, `None` for readings of samples that aren't a part's own: these are of the part they're
    /// timed in (`SegmentRanges`)
    pub part: Option<usize>,
}

/// The segments of a video cut from recordings (its segment table), as the motion data is handled in: the table's
/// segments that follow each other in their source taken together (`contiguous_parts`), each a hard boundary
#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TrimSegments {
    /// The video's segment table
    pub table: SegmentTable,
    /// The table's segments of each part (`contiguous_parts`)
    pub parts: Vec<Range<usize>>,
    /// The parts of the motion data's readings by their metadata samples and frames, which their times don't tell: each
    /// series' readings in time order, in runs of one part. Empty for a video whose readings weren't read with it
    /// (they're all of the part they're timed in)
    pub readings: Vec<ReadingRun>,
    /// The times of each part's metadata samples, the first's and the last's (µs, the file's metadata timeline, which
    /// the lens metadata is keyed by: `FileMetadata::lens_params`, `lens_positions`), `None` for a part without any: a
    /// frame's lens metadata is its part's (`lens_keys_of_frame`). Empty for a video whose samples weren't read with it
    pub metadata_us: Vec<Option<(i64, i64)>>,
    /// The values of the accelerometer and the magnetometer interpolated at a gyroscope reading's time from readings whose
    /// parts aren't all known (`telemetry_parser::util::ImuReadings::unplaced_values`): of the gyroscope reading's part
    /// only where the parts' places on the motion timeline put those readings in it too (`SegmentRanges::cut_values`).
    /// Empty for a video whose readings weren't read with it, or whose readings' parts are all known
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unplaced: Vec<UnplacedValue<usize>>,
    /// Whether the motion data isn't known to be at the video's frames: another file's (a telemetry log, another
    /// camera's recording) that the video's table doesn't name as the log the video was trimmed with. The parts are
    /// then the video's frames' only: each is synchronized and looked up on its own, in all of the motion data, which
    /// is integrated, filtered and smoothed as one piece (`SegmentRanges::whole`)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub motion_unplaced: bool,
}

impl TrimSegments {
    /// The segments of a video whose table is `table`, its motion data read from `samples`: `series` are the motion
    /// data's readings of each kind, each with its time (µs, `motion_key`) and where it came from (its sample, an index
    /// in `samples`, and its frame where the format records it)
    pub fn new(table: SegmentTable, samples: &[SampleInfo], series: &[(MotionSeries, &[(i64, ReadingOrigin)])]) -> Self {
        let parts = contiguous_parts(&table);
        let mut segments = Self { table, parts, readings: Vec::new(), metadata_us: Vec::new(), unplaced: Vec::new(), motion_unplaced: false };
        let sample_part = segments.sample_parts(samples, series);
        // Each series' readings in time order, in runs of one part
        let mut runs: Vec<ReadingRun> = Vec::new();
        for &(kind, readings) in series {
            let mut by_time: Vec<(i64, Option<usize>)> = readings.iter().map(|(t, origin)| (*t, segments.part_of_reading(&sample_part, origin))).collect();
            by_time.sort_by_key(|x| x.0);
            for (t, part) in by_time {
                match runs.last_mut() {
                    Some(run) if run.series == kind && run.part == part => run.last_us = t,
                    _ => runs.push(ReadingRun { series: kind, first_us: t, last_us: t, part }),
                }
            }
        }
        let own = || runs.iter().filter_map(|run| Some((run.part?, run)));
        if own().any(|(p, a)| own().any(|(q, b)| q > p && b.first_us < a.last_us)) {
            log::warn!("The motion data of the video's segments overlaps in time: where it does, it's taken for the later segment's");
        }
        segments.readings = runs;
        segments
    }

    /// The part of each of `samples` that is one part's own, `None` for the others: its time is in that part, on the
    /// file's timeline, and the readings of `series` that came from it (as in `new`) are timed with it
    pub fn sample_parts(&self, samples: &[SampleInfo], series: &[(MotionSeries, &[(i64, ReadingOrigin)])]) -> Vec<Option<usize>> {
        let n = self.len();
        let this = self;
        let us = |ms: f64| ms * 1000.0;
        let frame_us = |r: usize| { let v = this.video_us(r); (v.end - v.start) as f64 / this.frames(r).len() as f64 };
        // The part's span with `margin` µs on either side, the file's ends open
        let span = |r: usize, margin: f64| {
            let v = this.video_us(r);
            (if r == 0 { f64::NEG_INFINITY } else { v.start as f64 - margin }, if r + 1 == n { f64::INFINITY } else { v.end as f64 + margin })
        };
        let sample_span = |s: &SampleInfo| (us(s.timestamp_ms), us(s.timestamp_ms + s.duration_ms.max(0.0)));
        // The part of each sample, if it's one of a part: its time in that part, on the file's timeline …
        let mut sample_part: Vec<Option<usize>> = samples.iter().map(|s| {
            let (t0, t1) = sample_span(s);
            let r = this.segment_of_video_us(((t0 + t1) / 2.0).round() as i64);
            let (lo, hi) = span(r, SAMPLE_TIME_TOLERANCE_US);
            (t0 >= lo && t1 <= hi).then_some(r)
        }).collect();
        // … and its readings timed with it. A sample whose readings are timed farther from it holds those of more than
        // its own moment, whatever its own time: a clip-wide one, its readings timed by their own clock (which an
        // off-speed recording runs at its capture rate)
        for (_, readings) in series {
            for &(t, ReadingOrigin { sample, .. }) in *readings {
                if let Some(Some(r)) = sample_part.get(sample).copied() {
                    let (t0, t1) = sample_span(&samples[sample]);
                    let reach = READINGS_REACH_FRAMES * frame_us(r);
                    if !(t0 - reach..=t1 + reach).contains(&(t as f64)) { sample_part[sample] = None; }
                }
            }
        }
        sample_part
    }

    /// The part of a reading that came from `origin` (`ReadingOrigin`): its frame's, where its format records it, or
    /// its sample's where that's one part's own (`sample_parts`), `None` for a reading of the part it's timed in
    pub fn part_of_reading(&self, sample_parts: &[Option<usize>], origin: &ReadingOrigin) -> Option<usize> {
        match origin.frame {
            Some(frame) if !self.is_empty() => Some(self.segment_of_frame(usize::try_from(frame).unwrap_or(usize::MAX))),
            _ => sample_parts.get(origin.sample).copied().flatten(),
        }
    }

    /// The keys (µs) of the lens metadata that frame `frame` takes its own from: those of its part's metadata samples
    /// (`metadata_us`), the file's ends open (the first part's from the start, the last part's to the end), none for a
    /// part without any; all of them for a video of one part, or segments read without their samples
    pub fn lens_keys_of_frame(&self, frame: usize) -> RangeInclusive<i64> {
        if self.metadata_us.is_empty() || self.len() < 2 { return i64::MIN..=i64::MAX; }
        let r = self.segment_of_frame(frame);
        match self.metadata_us.get(r).copied().flatten() {
            Some((first, last)) => (if r == 0 { i64::MIN } else { first })..=(if r + 1 == self.len() { i64::MAX } else { last }),
            // An empty range: no key is in it
            None => RangeInclusive::new(1, 0),
        }
    }

    /// How many parts the video has (`parts`)
    pub fn len(&self) -> usize { self.parts.len() }
    pub fn is_empty(&self) -> bool { self.parts.is_empty() }

    /// The frames of part `r`
    pub fn frames(&self, r: usize) -> Range<usize> {
        let (first, last) = (&self.table.segments[self.parts[r].start], &self.table.segments[self.parts[r].end - 1]);
        first.out_first_frame as usize..last.out_end_frame() as usize
    }
    /// The video time of part `r`, µs of the file
    pub fn video_us(&self, r: usize) -> Range<i64> {
        let (first, last) = (&self.table.segments[self.parts[r].start], &self.table.segments[self.parts[r].end - 1]);
        first.out_start_us..last.out_end_us()
    }
    /// The recording part `r` is of, in the table
    pub fn source(&self, r: usize) -> &telemetry_parser::segment_table::TableSource {
        self.table.source_of(&self.table.segments[self.parts[r].start])
    }
    /// The part of the table's segment `segment`
    pub fn part_of_segment(&self, segment: usize) -> usize {
        self.parts.partition_point(|p| p.end <= segment).min(self.parts.len().saturating_sub(1))
    }
    /// The part of the video's frame `frame` (past the last frame, the last part)
    pub fn segment_of_frame(&self, frame: usize) -> usize {
        self.part_of_segment(self.table.segment_of_frame(frame as u64))
    }
    /// The part of the video's time `t_us`, µs of the file
    pub fn segment_of_video_us(&self, t_us: i64) -> usize {
        self.part_of_segment(self.table.segment_of_time_us(t_us))
    }
}

/// Where the parts of a video cut from recordings (`TrimSegments`) are on the video's timeline and on its motion data's
/// at a frame rate scale (`StabilizationParams::fps_scale`: the file's time `t` is the motion data's `t / scale`), as
/// the motion data is handled and looked up in (`GyroSource::segment_ranges`)
#[derive(Clone, Debug, Default)]
pub struct SegmentRanges {
    /// The first frame of each part
    pub first_frames: Vec<usize>,
    /// The frame after the last one
    pub end_frame: usize,
    /// Where each part starts in the video's time (the file's over the frame rate scale), µs
    pub video_starts_us: Vec<i64>,
    /// Where the video ends in its time, µs
    pub video_end_us: i64,
    /// The motion data of each part, its timestamps (µs): from its first reading (the first part's from the start) to
    /// the next part's first one (the last part's to the end)
    pub motion: Vec<Range<i64>>,
    /// How long each part's motion data lasts, ms, as the whole clip's is taken (`segment_duration_ms`)
    pub durations_ms: Vec<f64>,
    /// The IMU readings (their keys, `motion_key`) whose accelerometer or magnetometer value is interpolated between
    /// readings of another part than theirs, as the parts are placed here (`TrimSegments::unplaced`): their part has no
    /// reading of the sensor on that side, so they have no value of it, as a reading of a known part has none there
    pub cut_values: Vec<(i64, ImuSensor)>,
    /// Whether the motion data is one piece, every part's (`motion` is all of it for each): it isn't known to be at the
    /// video's frames (`TrimSegments::motion_unplaced`), so each part reads it where its sync puts its frames
    pub whole: bool,
    /// Whether a lookup past its part's motion data was reported (`GyroSource::lookup_for_frame`)
    pub reported: Arc<AtomicBool>,
}

impl SegmentRanges {
    /// The parts of `segments` with the motion data of `md`, at the frame rate scale `scale`: where the readings change
    /// parts by their samples (`TrimSegments::readings`), and the others where the video's time puts the parts on the
    /// motion data's timeline
    pub fn new(segments: &TrimSegments, md: &super::FileMetadata, scale: f64) -> Self {
        let n = segments.len();
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let on_motion = |file_us: i64| (file_us as f64 / scale).round() as i64;
        let video_starts_us: Vec<i64> = (0..n).map(|r| on_motion(segments.video_us(r).start)).collect();
        let video_end_us = n.checked_sub(1).map_or(0, |r| on_motion(segments.video_us(r).end));
        // The first reading of a series at or after `from` and not after `to`
        let first = |series: MotionSeries, from: i64, to: i64| -> Option<i64> {
            if from > to { return None; }
            match series {
                MotionSeries::Imu => md.raw_imu.get(md.raw_imu.partition_point(|x| motion_key(x.timestamp_ms) < from)).map(|x| motion_key(x.timestamp_ms)),
                MotionSeries::Orientations => md.quaternions.range(from..).next().map(|(k, _)| *k),
            }.filter(|t| *t <= to)
        };
        // The runs of a series are its readings' when it has the readings they start and end with: a series taken out,
        // or replaced by motion measured from the video, has none of their parts. Its readings are then all of the
        // parts they're timed in, as those of a series without runs
        let mut own_first: Vec<Option<i64>> = vec![None; n];
        let mut by_time: Vec<(MotionSeries, i64, i64)> = Vec::new();
        for series in [MotionSeries::Imu, MotionSeries::Orientations] {
            let runs: Vec<&ReadingRun> = segments.readings.iter().filter(|run| run.series == series).collect();
            if runs.is_empty() || !runs.iter().all(|run| first(series, run.first_us, run.first_us).is_some() && first(series, run.last_us, run.last_us).is_some()) {
                by_time.push((series, i64::MIN, i64::MAX));
                continue;
            }
            for run in runs {
                match run.part.filter(|p| *p < n) {
                    Some(p) => own_first[p] = Some(own_first[p].map_or(run.first_us, |t| t.min(run.first_us))),
                    None => by_time.push((series, run.first_us, run.last_us)),
                }
            }
        }
        // A part's motion starts with its first reading, or a later part's, and never before the part before it. The
        // readings of a part's samples are its own; the others are of the part whose frames are captured nearest them, on
        // the motion data's timeline: from between the last frame of the part before and the part's own first frame as
        // they're captured, where the motion of a part without readings of its own starts too (motion measured from the
        // video, keyed by its frames' capture). A frame is captured at its time in the video with its per-frame offset
        // (`frame_time_offset`), which can be more than a frame after it (Sony's) or before it (Insta360's, half the
        // exposure and more), so the last frames of the part before can be captured after this part's video starts, and
        // this part's first frame before it, its rows reading the readings there. Halfway, so a frame's motion, keyed by
        // its capture to the µs, is its own part's however its time was rounded
        let captured_from = |r: usize| {
            let first = segments.frames(r).start;
            let frame_us = (video_starts_us[r] - video_starts_us[r - 1]) as f64 / segments.frames(r - 1).len().max(1) as f64;
            let own = video_starts_us[r] as f64 + md.frame_time_offset(first) * 1000.0;
            let before = video_starts_us[r] as f64 - frame_us + md.frame_time_offset(first.saturating_sub(1)) * 1000.0;
            ((own + before) / 2.0).round() as i64
        };
        let mut starts = vec![i64::MIN; n];
        let mut later_own: Option<i64> = None;
        for r in (1..n).rev() {
            later_own = match (later_own, own_first[r]) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) };
            let timed = by_time.iter().filter_map(|&(series, from, to)| first(series, from.max(captured_from(r)), to)).min();
            starts[r] = match (later_own, timed) { (Some(a), Some(b)) => a.min(b), (a, b) => a.or(b).unwrap_or(captured_from(r)) };
        }
        for r in 2..n { starts[r] = starts[r].max(starts[r - 1]); }
        // Motion data that isn't known to be at the video's frames is one piece, all of it every part's
        let whole = segments.motion_unplaced && (!md.raw_imu.is_empty() || !md.quaternions.is_empty());
        let motion: Vec<Range<i64>> = if whole {
            vec![i64::MIN..i64::MAX; n]
        } else {
            (0..n).map(|r| starts[r]..starts.get(r + 1).copied().unwrap_or(i64::MAX)).collect()
        };
        let durations_ms = (0..n).map(|r| { let v = segments.video_us(r); segment_duration_ms(md, (v.end - v.start) as f64 / 1000.0 / scale, motion[r].clone()) }).collect();
        // A reading of no known part is of the part its time is in (the readings are keyed by their time as the IMU's
        // are, `motion_key` of the time in ms)
        let part_at = |t_us: i64, part: Option<usize>| part.filter(|p| *p < n).unwrap_or_else(|| {
            let key = motion_key(t_us as f64 / 1000.0);
            motion.partition_point(|m| m.start <= key).saturating_sub(1)
        });
        let cut_values = segments.unplaced.iter().filter(|v| {
            if whole { return false; }
            let p = part_at(v.at_us, v.part);
            part_at(v.before.0, v.before.1) != p || part_at(v.after.0, v.after.1) != p
        }).map(|v| (motion_key(v.at_us as f64 / 1000.0), v.sensor)).collect();
        let ranges = Self {
            first_frames: (0..n).map(|r| segments.frames(r).start).collect(),
            end_frame: n.checked_sub(1).map_or(0, |r| segments.frames(r).end),
            video_starts_us,
            video_end_us,
            motion,
            durations_ms,
            cut_values,
            whole,
            reported: Default::default(),
        };
        if n > 1 {
            log::debug!("Video segments at the frame rate scale {scale}: {}", (0..n).map(|r| format!("frames from {}, video from {} µs, motion data from {} µs", ranges.first_frames[r], ranges.video_starts_us[r], ranges.motion[r].start)).collect::<Vec<_>>().join("; "));
        }
        ranges
    }

    pub fn len(&self) -> usize { self.first_frames.len() }
    pub fn is_empty(&self) -> bool { self.first_frames.is_empty() }
    /// The part of the video's frame `frame`
    pub fn of_frame(&self, frame: usize) -> usize { self.first_frames.partition_point(|f| *f <= frame).saturating_sub(1) }
    /// The part of the video's time `t_us` (µs, over the frame rate scale)
    pub fn of_video_us(&self, t_us: i64) -> usize { self.video_starts_us.partition_point(|t| *t <= t_us).saturating_sub(1) }
    /// The part of the motion data's time `t_us`, µs
    pub fn of_motion_us(&self, t_us: i64) -> usize { self.motion.partition_point(|r| r.start <= t_us).saturating_sub(1) }
    /// The frames of part `r`
    pub fn frames(&self, r: usize) -> Range<usize> {
        self.first_frames[r]..self.first_frames.get(r + 1).copied().unwrap_or(self.end_frame)
    }
    /// Part `r`'s time in the video (µs, over the frame rate scale)
    pub fn video_us(&self, r: usize) -> Range<i64> {
        self.video_starts_us[r]..self.video_starts_us.get(r + 1).copied().unwrap_or(self.video_end_us)
    }
    /// A frame of part `r` in the video's time, µs: the part's time over its frames
    pub fn frame_us(&self, r: usize) -> f64 {
        let v = self.video_us(r);
        (v.end - v.start) as f64 / self.frames(r).len().max(1) as f64
    }
    /// The video's time of frame `frame` (µs, over the frame rate scale), as its part's frames are spaced over the
    /// part's time: exactly for the part's first frame
    pub fn video_us_of_frame(&self, frame: usize) -> i64 {
        let r = self.of_frame(frame);
        self.video_starts_us[r] + (frame.saturating_sub(self.first_frames[r]) as f64 * self.frame_us(r)).round() as i64
    }
    /// How far past the ends of part `r`'s motion data a lookup of it is held at them, µs: as far as its first and last
    /// frames' readings can be from them (`READINGS_REACH_FRAMES`)
    pub fn reach_us(&self, r: usize) -> f64 {
        READINGS_REACH_FRAMES * self.frame_us(r)
    }
    /// Whether the motion data is in the same pieces as with `other`, each as long (`motion`, `durations_ms`), and with
    /// the same values cut (`cut_values`): what the integration and the IMU filters run on one by one
    /// (`GyroSource::integrate`)
    pub fn same_pieces(&self, other: &Self) -> bool {
        self.motion == other.motion && self.durations_ms == other.durations_ms && self.cut_values == other.cut_values && self.whole == other.whole
    }
    /// A checksum of what the parts are
    pub fn hash_into(&self, hasher: &mut impl std::hash::Hasher) {
        hasher.write_usize(self.len());
        for r in 0..self.len() {
            hasher.write_usize(self.first_frames[r]);
            hasher.write_i64(self.video_starts_us[r]);
            hasher.write_i64(self.motion[r].start);
        }
        hasher.write_usize(self.end_frame);
        hasher.write_i64(self.video_end_us);
        hasher.write_u8(self.whole as u8);
    }
}

/// How long a part of a video cut from recordings lasts, `video_ms` in the video, its motion data the timestamps
/// `motion` of `md`'s: as a whole clip's is taken (`GyroSource::refresh_duration`, all of its motion data), the
/// video's, or the span of the camera's orientations or of the IMU readings when they span another one, so that a part
/// is integrated and smoothed as the clip of its range alone is
pub(super) fn segment_duration_ms(md: &super::FileMetadata, video_ms: f64, motion: Range<i64>) -> f64 {
    let mut duration = video_ms;
    let quats = span_duration_ms(md.quaternions.range(motion.clone()).map(|(k, _)| *k as f64 / 1000.0));
    if !md.quaternions.is_empty() && quats > 0.0 && (quats - duration).abs() > 0.01 { duration = quats; }
    let imu = span_duration_ms(imu_slice(&md.raw_imu, &motion).iter().map(|x| x.timestamp_ms));
    if !md.raw_imu.is_empty() && imu > 0.0 && (imu - duration).abs() > 0.01 { duration = imu; }
    duration
}

/// The motion data of a video, split at its segments' boundaries: the timestamps (µs) of each part's data
/// (`SegmentRanges::motion`), `None` for a video of one segment (or none), whose data is one piece
pub fn split_ranges(segments: Option<&SegmentRanges>) -> Option<&[Range<i64>]> {
    segments.filter(|s| s.len() > 1 && !s.whole).map(|s| s.motion.as_slice())
}

/// The key of a motion sample at `ms` in the maps of the motion data (the quaternions'): its time in whole µs, as the
/// integrators and the camera's quaternions are keyed
pub fn motion_key(ms: f64) -> i64 {
    (ms * 1000.0) as i64
}

/// The duration of a piece of motion data whose readings are at `times` (ms, in order), as a clip's is taken from its
/// readings (`GyroSource::load_from_telemetry`): their span, over the readings less one, times the readings
pub fn span_duration_ms(times: impl IntoIterator<Item = f64>) -> f64 {
    let (mut first, mut last, mut n) = (None, 0.0, 0usize);
    for t in times {
        first.get_or_insert(t);
        last = t;
        n += 1;
    }
    match first {
        Some(first) if n > 1 => (last - first) * ((n as f64 + 1.0) / n as f64),
        _ => 0.0,
    }
}

/// `f` of each piece of the motion data (`split_ranges`), the results put together: `f` takes the timestamps (µs) of the
/// piece, and its part (`TrimSegments`), `None` for all of the data, which a video of one piece has
pub fn per_piece(pieces: Option<&[Range<i64>]>, mut f: impl FnMut(Range<i64>, Option<usize>) -> super::TimeQuat) -> super::TimeQuat {
    match pieces {
        None => f(i64::MIN..i64::MAX, None),
        Some(ranges) => {
            let mut out = super::TimeQuat::new();
            for (r, range) in ranges.iter().enumerate() {
                out.append(&mut f(range.clone(), Some(r)));
            }
            out
        }
    }
}

/// The readings of `imu` (in time order) of the timestamps `range`, µs
pub fn imu_slice<'a>(imu: &'a [super::TimeIMU], range: &Range<i64>) -> &'a [super::TimeIMU] {
    let start = imu.partition_point(|x| motion_key(x.timestamp_ms) < range.start);
    let end = imu.partition_point(|x| motion_key(x.timestamp_ms) < range.end);
    &imu[start..end.max(start)]
}

/// The readings of `imu` (in time order) in pieces (`split_ranges`), each with its part, `None` for all of them
pub fn imu_pieces<'a>(imu: &'a mut [super::TimeIMU], pieces: Option<&[Range<i64>]>) -> Vec<(&'a mut [super::TimeIMU], Option<usize>)> {
    let Some(ranges) = pieces else { return vec![(imu, None)] };
    let ends: Vec<usize> = ranges.iter().map(|r| imu.partition_point(|x| motion_key(x.timestamp_ms) < r.end)).collect();
    let mut out = Vec::with_capacity(ranges.len());
    let mut rest = imu;
    let mut taken = 0;
    for (r, end) in ends.into_iter().enumerate() {
        let (piece, tail) = std::mem::take(&mut rest).split_at_mut(end.max(taken) - taken);
        taken += piece.len();
        out.push((piece, Some(r)));
        rest = tail;
    }
    out
}
