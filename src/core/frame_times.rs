// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! The presentation times of a video's frames, for a video whose frames aren't evenly spaced from its start (a variable
//! frame rate: a phone's, a camera that dropped frames, a file cut from recordings at other rates). Everything per frame
//! (the per-frame metadata, the frame's lens and zoom) is by the frame's ordinal, its place among the frames the video
//! presents, and a decoder gives a frame's presentation time: with evenly spaced frames the ordinal is that time times
//! the frame rate, but with uneven ones it's where the time is in the video's own list of times. The list is the
//! container's as FFmpeg's demuxer reads it (telemetry-parser's `segment_table::video_frame_times`: each sample's
//! decode time and composition offset through the edit list, an edit's first frame at the edit's start), the times a
//! decoder presents the frames at

use telemetry_parser::segment_table::VideoFrameTimes;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FrameTimes {
    /// Ticks per second of the times below, the file's video track's
    timescale: u64,
    /// Where each frame starts, in the file's time
    starts: Vec<i64>,
    /// Where the last frame ends: the end of the video (`of_video`)
    end: i64,
    /// The video's most common frame duration, for times outside of it
    nominal: i64,
}

impl FrameTimes {
    /// The frame times of a video, when they aren't `k` frame durations from 0: `None` for a video of evenly spaced
    /// frames from its start, whose frame `k` is at `k / fps`. A video of evenly spaced frames whose first one is
    /// presented after 0 (a composition offset without an edit list, an empty edit) has them too: the decoders (FFmpeg's,
    /// MDK's) give a frame its presentation time as the file has it, so `round(t · fps)` would number every frame that
    /// many frames late
    pub fn of_video(v: &VideoFrameTimes) -> Option<Self> {
        let even = v.starts.iter().enumerate().all(|(k, t)| *t == k as i64 * v.nominal);
        let last = *v.starts.last()?;
        if even || v.timescale == 0 || v.nominal <= 0 { return None; }
        // A last frame that lasts 0 is one whose duration the file leaves unknown, which the decoders make a nominal
        // frame's (FFmpeg's demuxer gives a packet without a duration one frame of the stream's rate, `demux.c`
        // `compute_pkt_fields`): it's presented, and the video ends, a frame after its start. Ended at its start, the
        // video would have its last frame and the frame after the end at one time, and lookups there the one past the end
        let end = if v.end > last { v.end } else { last + v.nominal };
        Some(Self { timescale: v.timescale, starts: v.starts.clone(), end, nominal: v.nominal })
    }

    pub fn frames(&self) -> usize { self.starts.len() }

    /// The video's frame rate as its file states it, its timescale over its most common frame duration
    pub fn frame_rate(&self) -> f64 { self.timescale as f64 / self.nominal as f64 }

    /// How much later than its place among the frames frame `frame` is presented, ms in the video's time (the file's over
    /// `scale`): the time frames before it last longer than a nominal frame, less what any lasts shorter. An off-speed
    /// Blackmagic RAW recording ends with a frame that lasts until the recording's end, so a video cut from recordings
    /// has that frame wherever its cut puts it, and the frames after it this much after their place
    pub fn held_before_ms(&self, frame: usize, scale: f64) -> f64 {
        let frame = frame.min(self.starts.len().saturating_sub(1));
        let (first, start) = (self.starts.first().copied().unwrap_or(0), self.starts.get(frame).copied().unwrap_or(0));
        self.ms(start - first - frame as i64 * self.nominal) / scale
    }

    /// A time of the file, in ticks, in ms
    fn ms(&self, ticks: i64) -> f64 { ticks as f64 * 1000.0 / self.timescale as f64 }

    /// The time (ms) frame `frame` is presented at, in the video's time: the file's over `scale` (the frame rate scale,
    /// `StabilizationParams::fps_scale`). Frames before the first one or after the last one are a nominal frame duration
    /// apart
    pub fn timestamp_at_frame(&self, frame: i32, scale: f64) -> f64 {
        let n = self.starts.len() as i64;
        let file_ms = match frame as i64 {
            f if f < 0 => self.ms(self.starts[0]) + f as f64 * self.ms(self.nominal),
            f if f >= n => self.ms(self.end) + (f - n) as f64 * self.ms(self.nominal),
            f => self.ms(self.starts[f as usize]),
        };
        file_ms / scale
    }

    /// The frame presented at the video's time `timestamp_ms` (the file's over `scale`): the one whose time is the nearest,
    /// the later one on a tie, as `round(t · fps)` takes it for evenly spaced frames, where that's within half a nominal
    /// frame. A frame is presented until the next one starts, so a time further from both is the earlier one's: a frame
    /// that lasts for several of the video's frame durations (a Blackmagic RAW clip's frame held for its whole frames,
    /// each of which Blackmagic's SDK counts a frame of the clip and reads none of but the first; a frame before one the
    /// camera dropped) is the frame at each of its times but the half frame before the next one. A time half way is
    /// the later frame's exactly: twice the time is compared with the sum of the two times it's half way between, in
    /// the file's ticks, which a single rounding takes to ms
    pub fn frame_at_timestamp(&self, timestamp_ms: f64, scale: f64) -> i32 {
        let t = timestamp_ms * scale;
        let (first, end, nominal, n) = (self.ms(self.starts[0]), self.ms(self.end), self.ms(self.nominal), self.starts.len());
        if t < first { return ((t - first) / nominal).round() as i32; }
        if t >= end { return (n as f64 + ((t - end) / nominal).round()) as i32; }
        // The last frame starting at or before `t`, or the next one from half way between its start and the later of
        // this one's and a nominal frame before it: when that's nearer, and within half a frame
        let k = self.starts.partition_point(|s| self.ms(*s) <= t).saturating_sub(1);
        let next = self.starts.get(k + 1).copied().unwrap_or(self.end);
        let k = if 2.0 * t >= self.ms(next + self.starts[k].max(next - self.nominal)) { k + 1 } else { k };
        k as i32
    }

    /// The first frame presented at or after the video's time `timestamp_ms` (the file's over `scale`), as `ceil(t · fps)`
    /// takes it for evenly spaced frames
    pub fn first_frame_at_or_after(&self, timestamp_ms: f64, scale: f64) -> i32 {
        let t = timestamp_ms * scale;
        let (first, n) = (self.ms(self.starts[0]), self.starts.len());
        if t <= first { return ((t - first) / self.ms(self.nominal)).ceil() as i32; }
        if t > self.ms(self.starts[n - 1]) { return (n as f64 + ((t - self.ms(self.end)) / self.ms(self.nominal)).ceil().max(0.0)) as i32; }
        self.starts.partition_point(|s| self.ms(*s) < t) as i32
    }

    /// A fingerprint of the times, to the µs, for the keys of everything computed per frame (the zoom, the smoothing, the
    /// focal length curves, the optical correction's context): each frame's lens, zoom and timing is looked up at its
    /// time. FNV-1a (`optical_correction::Fnv`) rather than std's hasher, as the optical correction's context is kept in
    /// project files
    pub fn checksum(&self) -> u64 {
        let mut h = crate::gyro_source::optical_correction::Fnv::default();
        h.eat(self.starts.len() as u64);
        for t in &self.starts { h.eat_rounded(self.ms(*t), 1e3); }
        h.eat_rounded(self.ms(self.end), 1e3);
        h.0
    }
}

#[cfg(test)]
mod tests {
    use super::FrameTimes;
    use telemetry_parser::segment_table::VideoFrameTimes;

    /// 29.97 fps frames at `starts` (ticks of 1/30000 s)
    fn times(starts: &[i64], end: i64) -> Option<FrameTimes> {
        FrameTimes::of_video(&VideoFrameTimes { timescale: 30_000, starts: starts.to_vec(), end, nominal: 1001 })
    }
    fn ms(ticks: i64) -> f64 { ticks as f64 * 1000.0 / 30_000.0 }

    #[test]
    fn evenly_spaced_frames_from_zero_need_no_table() {
        assert!(times(&[0, 1001, 2002], 3003).is_none());
    }

    #[test]
    fn a_first_frame_presented_late_is_frame_zero() {
        // Two frames of composition offset and no edit list: the decoders give the first frame that time
        let t = times(&[2002, 3003, 4004, 5005], 6006).expect("a table");
        for k in 0..4 {
            assert_eq!(t.frame_at_timestamp(ms(2002 + k * 1001), 1.0), k as i32);
            assert_eq!(t.timestamp_at_frame(k as i32, 1.0), ms(2002 + k * 1001));
        }
        assert_eq!(t.frame_at_timestamp(ms(2002) - 0.1, 1.0), 0);
        assert_eq!(t.first_frame_at_or_after(ms(2002) + 0.1, 1.0), 1);
    }

    #[test]
    fn uneven_frames_are_found_by_their_times_at_any_scale() {
        // The third frame dropped
        let t = times(&[0, 1001, 3003, 4004], 5005).expect("a table");
        assert_eq!(t.frame_at_timestamp(ms(3003), 1.0), 2);
        assert_eq!(t.frame_at_timestamp(ms(3003) - 10.0, 1.0), 2);
        assert_eq!(t.frame_at_timestamp(ms(1001) + 10.0, 1.0), 1);
        // The frame rate doubled: the video's time is the file's over 2
        assert_eq!(t.timestamp_at_frame(2, 2.0), ms(3003) / 2.0);
        assert_eq!(t.frame_at_timestamp(ms(3003) / 2.0, 2.0), 2);
    }

    #[test]
    fn the_checksum_tells_the_times_apart() {
        let a = times(&[0, 1001, 3003, 4004], 5005).unwrap();
        assert_eq!(a.checksum(), times(&[0, 1001, 3003, 4004], 5005).unwrap().checksum());
        assert_ne!(a.checksum(), times(&[0, 2002, 3003, 4004], 5005).unwrap().checksum());
    }
}
