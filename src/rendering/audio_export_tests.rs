// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

//! Tests for the external audio encoder against real files.
//!
//! Unlike the core tests, which are pure logic, these exercise ffmpeg for real:
//! they create an output file, write an audio stream and check the result by
//! reading the file back. This is what proves the mux works with no fork-local
//! modifications.

use ffmpeg_next::codec;
use gyroflow_core::audio::{AudioTrack, SourceFormat};

use super::audio_export::ExternalAudioEncoder;

fn make_track(seconds: f32, sample_rate: u32, channels: u16) -> AudioTrack {
    let frames = (sample_rate as f32 * seconds) as usize;
    let mut samples = Vec::with_capacity(frames * channels as usize);
    for i in 0..frames {
        let t = i as f32 / sample_rate as f32;
        for ch in 0..channels {
            let freq = 440.0 + 220.0 * ch as f32;
            samples.push(0.5 * (2.0 * std::f32::consts::PI * freq * t).sin());
        }
    }
    AudioTrack {
        path: String::new(),
        samples,
        channels,
        sample_rate,
        source_format: SourceFormat::F32,
        mono_analysis: Vec::new(),
        offset_seconds: 0.0,
        preserve_original_format: true,
    }
}

/// Writes the track to a file and returns the path.
fn write_audio_file(name: &str, track: &AudioTrack, codec_id: codec::Id) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    let path_str = path.to_string_lossy().to_string();

    let mut octx = ffmpeg_next::format::output(&path_str).expect("create output file");

    let mut encoder = ExternalAudioEncoder::new(codec_id, track, &mut octx, 0).expect("create encoder");

    octx.write_header().expect("write header");

    let time_base = octx.stream(0).map(|s| s.time_base()).expect("stream time base");

    encoder.write_all(&track.samples, &mut octx, time_base).expect("write samples");
    encoder.finish(&mut octx, time_base).expect("finish");

    octx.write_trailer().expect("write trailer");

    path
}

/// Reads back the audio stream parameters of the generated file.
fn probe_audio(path: &std::path::Path) -> (codec::Id, u32, u16, f64) {
    let ictx = ffmpeg_next::format::input(&path.to_string_lossy().to_string()).expect("open generated file");
    let stream = ictx
        .streams()
        .best(ffmpeg_next::media::Type::Audio)
        .expect("the file should have an audio stream");

    let params = stream.parameters();
    let ctx = codec::context::Context::from_parameters(params).expect("parameters");
    let audio = ctx.decoder().audio().expect("audio decoder");

    let duration_s = stream.duration() as f64 * f64::from(stream.time_base());

    (audio.codec().map(|c| c.id()).unwrap_or(codec::Id::None), audio.rate(), audio.channels(), duration_s)
}

#[test]
fn writes_pcm_f32le_preserving_rate_and_channels() {
    let track = make_track(1.0, 48000, 2);
    let path = write_audio_file("gyroflow_test_out_f32.mov", &track, codec::Id::PCM_F32LE);

    assert!(path.exists(), "the output file was not created");
    let size = std::fs::metadata(&path).unwrap().len();
    // 1 s of stereo float is ~384 KB of data; the bound leaves room for the container.
    assert!(size > 300_000, "file too small ({size} bytes): the audio was not written");

    let (codec_id, rate, channels, duration) = probe_audio(&path);

    // The core of the feature: float in, float out, no conversion.
    assert_eq!(codec_id, codec::Id::PCM_F32LE, "the output codec is not pcm_f32le");
    assert_eq!(rate, 48000, "the original sample rate was not preserved");
    assert_eq!(channels, 2, "the original channels were not preserved");
    assert!((duration - 1.0).abs() < 0.05, "unexpected duration: {duration}s");

    let _ = std::fs::remove_file(path);
}

#[test]
fn writes_aac_for_already_lossy_sources() {
    // MP3 / AAC / M4A input decodes to SourceFormat::Other, which recommends AAC:
    // the source is already lossy, so there is no bit-exact copy to protect. This
    // exercises a different path from PCM - AAC needs planar float, so the frames
    // go through the resampler.
    let mut track = make_track(1.0, 48000, 2);
    track.source_format = SourceFormat::Other;

    let path = write_audio_file("gyroflow_test_out_aac.mp4", &track, codec::Id::AAC);

    assert!(path.exists(), "the output file was not created");
    let (codec_id, rate, channels, duration) = probe_audio(&path);
    assert_eq!(codec_id, codec::Id::AAC, "the output codec is not aac");
    assert_eq!(rate, 48000, "the sample rate was changed");
    assert_eq!(channels, 2, "the channels were changed");
    // AAC pads the stream with priming samples, so the duration is not exact.
    assert!((duration - 1.0).abs() < 0.15, "unexpected duration: {duration}s");

    let _ = std::fs::remove_file(path);
}

#[test]
fn aac_output_is_not_silent() {
    // A lossy encode still has to carry the signal: a file of the right size
    // full of silence would pass every check above.
    let mut track = make_track(1.0, 48000, 1);
    track.source_format = SourceFormat::Other;

    let path = write_audio_file("gyroflow_test_out_aac_mono.mp4", &track, codec::Id::AAC);
    let size = std::fs::metadata(&path).unwrap().len();
    // 1 s of mono AAC is a few KB; silence compresses to almost nothing.
    assert!(size > 3_000, "file too small ({size} bytes): the audio was probably silent");

    let (codec_id, _, channels, _) = probe_audio(&path);
    assert_eq!(codec_id, codec::Id::AAC);
    assert_eq!(channels, 1, "mono was not preserved");

    let _ = std::fs::remove_file(path);
}

#[test]
fn writes_at_44100_hz_without_resampling() {
    // Sample rates other than 48 kHz must also pass through untouched.
    let track = make_track(0.5, 44100, 2);
    let path = write_audio_file("gyroflow_test_out_44k.mov", &track, codec::Id::PCM_F32LE);

    let (codec_id, rate, channels, _) = probe_audio(&path);
    assert_eq!(codec_id, codec::Id::PCM_F32LE);
    assert_eq!(rate, 44100, "the sample rate was changed");
    assert_eq!(channels, 2);

    let _ = std::fs::remove_file(path);
}

#[test]
fn writes_mono() {
    let track = make_track(0.5, 48000, 1);
    let path = write_audio_file("gyroflow_test_out_mono.mov", &track, codec::Id::PCM_F32LE);

    let (_, rate, channels, _) = probe_audio(&path);
    assert_eq!(rate, 48000);
    assert_eq!(channels, 1, "mono turned into something else");

    let _ = std::fs::remove_file(path);
}

/// Drains the decoded frames, converting to interleaved `f32`.
///
/// `frame.plane::<f32>(0)` is sized in **frames**, not samples: in stereo it
/// returns half the values. Hence the data is read from the raw buffer, the same
/// way the encoder does.
fn collect_frames(decoder: &mut ffmpeg_next::decoder::Audio, out: &mut Vec<f32>) {
    let mut frame = ffmpeg_next::frame::Audio::empty();
    while decoder.receive_frame(&mut frame).is_ok() {
        let values = frame.samples() * frame.channels() as usize;
        let bytes = frame.data(0);
        let floats = unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const f32, values.min(bytes.len() / 4)) };
        out.extend_from_slice(floats);
    }
}

#[test]
fn audio_content_survives_the_encode() {
    // Writes a track and reads it back sample by sample: with pcm_f32le the path
    // is lossless, so the values must come back practically identical.
    let track = make_track(0.2, 48000, 2);
    let path = write_audio_file("gyroflow_test_roundtrip.mov", &track, codec::Id::PCM_F32LE);

    let mut ictx = ffmpeg_next::format::input(&path.to_string_lossy().to_string()).expect("open");
    let stream_index = ictx
        .streams()
        .best(ffmpeg_next::media::Type::Audio)
        .expect("audio stream")
        .index();

    let params = ictx.stream(stream_index).unwrap().parameters();
    let ctx = codec::context::Context::from_parameters(params).unwrap();
    let mut decoder = ctx.decoder().audio().unwrap();

    let mut decoded: Vec<f32> = Vec::new();
    let packets: Vec<_> = ictx.packets().filter_map(|(s, p)| (s.index() == stream_index).then_some(p)).collect();
    for packet in packets {
        decoder.send_packet(&packet).unwrap();
        collect_frames(&mut decoder, &mut decoded);
    }
    decoder.send_eof().unwrap();
    collect_frames(&mut decoder, &mut decoded);

    assert!(!decoded.is_empty(), "nothing was decoded back");

    // Compare the beginning: a conversion to integer would produce an error far
    // larger than this bound.
    let compare = decoded.len().min(track.samples.len()).min(4800);
    let max_error = (0..compare)
        .map(|i| (decoded[i] - track.samples[i]).abs())
        .fold(0.0f32, f32::max);
    assert!(max_error < 1e-6, "the audio was altered along the way: max error {max_error}");

    let _ = std::fs::remove_file(path);
}
