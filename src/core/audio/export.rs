// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

//! Assembly of the audio buffer to be embedded in the exported video.
//!
//! This file holds pure logic only: trimming, shifting and silence padding, all in
//! `f32` and with no ffmpeg dependency. Encoding and muxing live in
//! `src/rendering/audio_export.rs`, in the root crate, because `gyroflow-core` does
//! not depend on ffmpeg and should not start to.
//!
//! # Core rule
//!
//! `t_audio = t_video + offset`. A video range `[t_in, t_out]` maps to the audio
//! range `[t_in + offset, t_out + offset]`.
//!
//! The returned buffer always covers exactly the duration of the requested video
//! range. Where the audio does not reach - before the start or after the end of the
//! track - the space is filled with silence. Returning a shorter buffer would
//! misalign the mux.
//!
//! # Format preservation
//!
//! Nothing here quantizes: samples stay in `f32` from decode to encode, which is
//! the only place conversion to the output format happens. See
//! [`SourceFormat`](super::SourceFormat) and [`recommended_codec`].

use super::{AudioTrack, SourceFormat};

/// Output codec recommended to preserve a given source format.
///
/// The names are exactly those accepted by the codec selector in
/// `src/rendering/mod.rs:250-258`.
pub fn recommended_codec(format: SourceFormat) -> &'static str {
    match format {
        // Float needs float: converting to integer loses the headroom above 0 dBFS
        // that 32-bit float material carries, which is precisely why recorders like
        // the DJI Mic use this format.
        SourceFormat::F32 => "PCM (f32le)",
        SourceFormat::S32 | SourceFormat::S24 => "PCM (s24le)",
        SourceFormat::S16 => "PCM (s16le)",
        SourceFormat::U8 => "PCM (s16le)",
        // The source is already lossy; there is nothing to preserve bit for bit.
        SourceFormat::Other => "AAC",
    }
}

/// Whether a container accepts the given audio codec.
///
/// This check exists to prevent a silent conversion: when float does not fit the
/// chosen container the user must be warned and decide, instead of receiving a
/// downgraded file.
///
/// `extension` is compared without the dot and case-insensitively.
pub fn container_supports_codec(extension: &str, codec: &str) -> bool {
    let ext = extension.trim_start_matches('.').to_ascii_lowercase();
    match codec {
        // PCM float exists in MOV and MKV. MP4 does not support it in practice: the
        // standard defines no mapping for pcm_f32le and players will not play it.
        "PCM (f32le)" => matches!(ext.as_str(), "mov" | "mkv"),
        // Integer PCM is accepted in MOV/MKV; in MP4 it is possible but rare and
        // poorly supported, so we treat it as incompatible rather than produce
        // files the user cannot open.
        "PCM (s16le)" | "PCM (s16be)" | "PCM (s24le)" | "PCM (s24be)" => {
            matches!(ext.as_str(), "mov" | "mkv" | "wav")
        }
        // AAC fits any of the containers Gyroflow exports.
        "AAC" => matches!(ext.as_str(), "mp4" | "mov" | "mkv" | "m4a"),
        _ => false,
    }
}

/// Result of the compatibility check between source, codec and container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatCompatibility {
    /// The source format will be preserved losslessly.
    Preserved {
        /// Codec that will be used for the output.
        codec: &'static str,
    },
    /// The chosen container cannot hold the source format.
    ///
    /// Do not convert silently. Present the options to the user: change the
    /// container, or explicitly accept the loss.
    ContainerMismatch {
        /// Codec that would preserve the material.
        wanted_codec: &'static str,
        /// Extension chosen for the video, which cannot hold it.
        extension: String,
        /// Extension suggested as an alternative.
        suggested_extension: &'static str,
    },
    /// The user explicitly turned preservation off.
    DowngradeAccepted {
        /// Default codec of the container.
        codec: &'static str,
    },
}

impl FormatCompatibility {
    /// Whether the export can proceed without user intervention.
    pub fn can_proceed(&self) -> bool {
        !matches!(self, Self::ContainerMismatch { .. })
    }
}

/// Picks the output codec, or detects that the container cannot hold the material.
///
/// `extension` is the output extension chosen for the video - that is what rules,
/// since the audio is embedded in it.
///
/// The suggested alternative is always `.mov`: it is the container already used by
/// ProRes, DNxHD and CineForm in the export selector, and the one Gyroflow's own
/// interface recommends when codec and container do not match
/// (`src/ui/App.qml:729`).
pub fn check_format_compatibility(
    source_format: SourceFormat,
    extension: &str,
    preserve_original_format: bool,
) -> FormatCompatibility {
    if !preserve_original_format {
        return FormatCompatibility::DowngradeAccepted { codec: "AAC" };
    }

    let wanted = recommended_codec(source_format);
    if container_supports_codec(extension, wanted) {
        FormatCompatibility::Preserved { codec: wanted }
    } else {
        FormatCompatibility::ContainerMismatch {
            wanted_codec: wanted,
            extension: extension.trim_start_matches('.').to_ascii_lowercase(),
            suggested_extension: "mov",
        }
    }
}

/// Converts a video timestamp to the corresponding audio frame index.
///
/// Rounding happens only once, on the offset-adjusted time. Converting in two
/// steps (time -> video frame -> sample) would accumulate half-frame error, which
/// is 16 ms at 30 fps - audible.
/// A negative result means the video starts before the audio; the gap is silence.
fn video_time_to_audio_frame(t_video_s: f64, offset_s: f64, sample_rate: u32) -> i64 {
    ((t_video_s + offset_s) * sample_rate as f64).round() as i64
}

/// Builds the audio buffer for one video range.
///
/// - `track` is the imported track, with the original channels preserved.
/// - `t_in_s` / `t_out_s` delimit the video range, in seconds.
///
/// Returns interleaved `f32` samples covering exactly `t_out_s - t_in_s` seconds,
/// with silence where the track does not reach.
pub fn build_segment(track: &AudioTrack, t_in_s: f64, t_out_s: f64) -> Vec<f32> {
    let channels = track.channels as usize;
    if channels == 0 || track.sample_rate == 0 || t_out_s <= t_in_s {
        return Vec::new();
    }

    let sample_rate = track.sample_rate;
    let offset = track.offset_seconds;

    // Frame count the video range requires. This is the size the buffer will have,
    // regardless of what the track covers.
    let wanted_frames = (((t_out_s - t_in_s) * sample_rate as f64).round() as i64).max(0) as usize;
    if wanted_frames == 0 {
        return Vec::new();
    }

    let start_frame = video_time_to_audio_frame(t_in_s, offset, sample_rate);
    let available_frames = track.frame_count() as i64;

    let mut out = vec![0.0f32; wanted_frames * channels];

    // Intersection between the requested range and what the track actually covers.
    let copy_from = start_frame.max(0);
    let copy_to = (start_frame + wanted_frames as i64).min(available_frames);
    if copy_to <= copy_from {
        // No overlap: the whole range is silence.
        return out;
    }

    // Where inside the output buffer the copied range starts. Positive when the
    // audio comes in after the video start (positive offset).
    let dest_frame_offset = (copy_from - start_frame).max(0) as usize;

    let src_start = copy_from as usize * channels;
    let src_end = copy_to as usize * channels;
    let dst_start = dest_frame_offset * channels;
    let dst_end = dst_start + (src_end - src_start);

    out[dst_start..dst_end].copy_from_slice(&track.samples[src_start..src_end]);
    out
}

/// Builds the buffer for several video ranges, concatenated in the given order.
///
/// Used when the export has multiple trim ranges: each range is mapped
/// independently and the buffers are spliced, mirroring what the video does.
///
/// Times are in seconds. For the core's `trim_ranges`, which are normalized from 0
/// to 1, use [`build_from_trim_ranges`].
pub fn build_segments(track: &AudioTrack, ranges: &[(f64, f64)]) -> Vec<f32> {
    if ranges.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (t_in, t_out) in ranges {
        out.extend_from_slice(&build_segment(track, *t_in, *t_out));
    }
    out
}

/// Builds the audio buffer from the project's trim ranges.
///
/// `trim_ranges` are the
/// [`StabilizationParams::trim_ranges`](crate::stabilization_params::StabilizationParams::trim_ranges),
/// normalized from 0 to 1 over the video duration - not in seconds.
///
/// An empty list means "no trim": the audio covers the whole video.
pub fn build_from_trim_ranges(track: &AudioTrack, trim_ranges: &[(f64, f64)], video_duration_s: f64) -> Vec<f32> {
    if video_duration_s <= 0.0 {
        return Vec::new();
    }

    if trim_ranges.is_empty() {
        return build_segment(track, 0.0, video_duration_s);
    }

    let in_seconds: Vec<(f64, f64)> = trim_ranges
        .iter()
        .map(|(from, to)| (from * video_duration_s, to * video_duration_s))
        .collect();

    build_segments(track, &in_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic track: `frames` mono frames whose value equals their index.
    ///
    /// That way each sample identifies the instant it came from, and a wrong trim
    /// shows up as a shifted value instead of going unnoticed.
    fn track(frames: usize, sample_rate: u32, offset_seconds: f64) -> AudioTrack {
        AudioTrack {
            path: String::new(),
            samples: (0..frames).map(|i| i as f32).collect(),
            channels: 1,
            sample_rate,
            source_format: SourceFormat::F32,
            mono_analysis: Vec::new(),
            offset_seconds,
            preserve_original_format: true,
        }
    }

    #[test]
    fn trim_without_offset_picks_the_right_range() {
        // 100 frames at 100 Hz = 1 s. Range [0.1, 0.2] -> frames 10..20.
        let t = track(100, 100, 0.0);
        let seg = build_segment(&t, 0.1, 0.2);
        assert_eq!(seg.len(), 10);
        assert_eq!(seg[0], 10.0);
        assert_eq!(seg[9], 19.0);
    }

    #[test]
    fn positive_offset_advances_the_audio() {
        // offset=+0.1 s: the video range [0.0, 0.1] pulls audio from [0.1, 0.2] ->
        // frames 10..20.
        let t = track(100, 100, 0.1);
        let seg = build_segment(&t, 0.0, 0.1);
        assert_eq!(seg[0], 10.0);
    }

    #[test]
    fn negative_offset_pads_silence_at_the_start() {
        // offset=-0.05 s: the video at [0.0, 0.1] asks for audio from -0.05 s, which
        // does not exist. The first 5 frames are silence.
        let t = track(100, 100, -0.05);
        let seg = build_segment(&t, 0.0, 0.1);
        assert_eq!(seg.len(), 10);
        assert_eq!(&seg[0..5], &[0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(seg[5], 0.0); // first real frame of the track
        assert_eq!(seg[6], 1.0);
    }

    #[test]
    fn range_past_the_end_becomes_silence() {
        let t = track(100, 100, 0.0);
        // The track is 1 s long; asking for [1.5, 1.6] has no overlap at all.
        let seg = build_segment(&t, 1.5, 1.6);
        assert_eq!(seg.len(), 10);
        assert!(seg.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn buffer_covers_the_requested_duration_even_past_the_track() {
        let t = track(100, 100, 0.0);
        // Asks for 0.5 s starting at 0.8 s: the track ends at 1.0 s.
        let seg = build_segment(&t, 0.8, 1.3);
        // The size follows the video range, not what is left of the audio.
        assert_eq!(seg.len(), 50);
        assert_eq!(seg[0], 80.0);
        assert_eq!(seg[19], 99.0);
        assert!(seg[20..].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn stereo_preserves_the_interleaving() {
        let mut t = track(0, 100, 0.0);
        t.channels = 2;
        // [L0,R0, L1,R1, L2,R2] with L=index and R=index+100.
        t.samples = vec![0.0, 100.0, 1.0, 101.0, 2.0, 102.0];
        let seg = build_segment(&t, 0.01, 0.03);
        assert_eq!(seg, vec![1.0, 101.0, 2.0, 102.0]);
    }

    #[test]
    fn multiple_segments_are_concatenated_in_order() {
        let t = track(100, 100, 0.0);
        let seg = build_segments(&t, &[(0.0, 0.05), (0.5, 0.55)]);
        assert_eq!(seg.len(), 10);
        assert_eq!(seg[0], 0.0);
        assert_eq!(seg[5], 50.0);
    }

    #[test]
    fn normalized_trim_ranges_become_seconds() {
        // 1000 frames at 1000 Hz = 1 s of audio, for a 1 s video.
        let mut t = track(1000, 1000, 0.0);
        t.samples = (0..1000).map(|i| i as f32).collect();

        // Normalized range [0.25, 0.5] of a 1 s video = [0.25 s, 0.5 s].
        let seg = build_from_trim_ranges(&t, &[(0.25, 0.5)], 1.0);
        assert_eq!(seg.len(), 250);
        assert_eq!(seg[0], 250.0);
        assert_eq!(seg[249], 499.0);
    }

    #[test]
    fn without_trim_the_audio_covers_the_whole_video() {
        let t = track(1000, 1000, 0.0);
        let seg = build_from_trim_ranges(&t, &[], 1.0);
        assert_eq!(seg.len(), 1000);
    }

    #[test]
    fn multiple_trim_ranges_are_concatenated() {
        let mut t = track(1000, 1000, 0.0);
        t.samples = (0..1000).map(|i| i as f32).collect();

        let seg = build_from_trim_ranges(&t, &[(0.0, 0.1), (0.9, 1.0)], 1.0);
        assert_eq!(seg.len(), 200);
        assert_eq!(seg[0], 0.0);      // start of the first range
        assert_eq!(seg[100], 900.0);  // start of the second, right after the splice
    }

    #[test]
    fn trim_with_offset_shifts_the_audio_along() {
        let mut t = track(1000, 1000, 0.1); // +100 ms
        t.samples = (0..1000).map(|i| i as f32).collect();

        // Video at [0.0, 0.1] with offset +0.1 s -> audio at [0.1, 0.2].
        let seg = build_from_trim_ranges(&t, &[(0.0, 0.1)], 1.0);
        assert_eq!(seg[0], 100.0);
    }

    #[test]
    fn float_requires_a_float_codec() {
        assert_eq!(recommended_codec(SourceFormat::F32), "PCM (f32le)");
    }

    #[test]
    fn lossy_sources_get_aac_and_fit_anywhere() {
        // MP3, M4A and AAC input decode to Other: there is no bit-exact copy to
        // protect, so AAC is both the recommendation and compatible with every
        // container Gyroflow exports.
        assert_eq!(recommended_codec(SourceFormat::Other), "AAC");
        for ext in ["mp4", "mov", "mkv"] {
            assert!(container_supports_codec(ext, "AAC"), "AAC should fit in .{ext}");
        }
        let status = check_format_compatibility(SourceFormat::Other, "mp4", true);
        assert!(matches!(status, FormatCompatibility::Preserved { .. }),
                "a lossy source in mp4 must not be reported as a conflict: {status:?}");
    }

    #[test]
    fn float_does_not_fit_in_mp4() {
        assert!(!container_supports_codec("mp4", "PCM (f32le)"));
        assert!(container_supports_codec("mov", "PCM (f32le)"));
        assert!(container_supports_codec(".MOV", "PCM (f32le)"));
        assert!(container_supports_codec("mkv", "PCM (f32le)"));
    }

    #[test]
    fn float_mp4_conflict_is_flagged_and_not_converted() {
        let r = check_format_compatibility(SourceFormat::F32, "mp4", true);
        assert!(!r.can_proceed());
        match r {
            FormatCompatibility::ContainerMismatch { wanted_codec, suggested_extension, .. } => {
                assert_eq!(wanted_codec, "PCM (f32le)");
                assert_eq!(suggested_extension, "mov");
            }
            _ => panic!("float in MP4 must be flagged as incompatible"),
        }
    }

    #[test]
    fn float_in_mov_is_preserved() {
        let r = check_format_compatibility(SourceFormat::F32, "mov", true);
        assert!(r.can_proceed());
        assert_eq!(r, FormatCompatibility::Preserved { codec: "PCM (f32le)" });
    }

    #[test]
    fn explicit_downgrade_is_respected() {
        let r = check_format_compatibility(SourceFormat::F32, "mp4", false);
        assert!(r.can_proceed());
        assert_eq!(r, FormatCompatibility::DowngradeAccepted { codec: "AAC" });
    }
}
