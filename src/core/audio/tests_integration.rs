// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

//! Integration tests for the audio module against real files.
//!
//! Unlike the unit tests elsewhere, which use synthetic in-memory signals, these
//! write an actual WAV to disk and run it through the full decoder. They cover
//! the file -> `AudioTrack` -> export buffer path end to end, including the
//! recognition of the IEEE float format (format tag 3).

use std::io::Write;

use super::decode::decode_file;
use super::export::{build_from_trim_ranges, check_format_compatibility, FormatCompatibility};
use super::SourceFormat;

/// Writes a 32-bit float WAV (IEEE, format tag 3) to a temporary file.
///
/// The content is one sine per channel plus a short transient at `click_at_s`,
/// which acts as a visible reference when the trim is checked sample by sample.
fn write_float_wav(name: &str, sample_rate: u32, channels: u16, duration_s: f32, click_at_s: f32) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);

    let frames = (sample_rate as f32 * duration_s) as usize;
    let click_frame = (sample_rate as f32 * click_at_s) as usize;

    let mut samples: Vec<f32> = Vec::with_capacity(frames * channels as usize);
    for i in 0..frames {
        let t = i as f32 / sample_rate as f32;
        for ch in 0..channels {
            let freq = 440.0 + 220.0 * ch as f32;
            let mut value = 0.5 * (2.0 * std::f32::consts::PI * freq * t).sin();
            if i >= click_frame && i - click_frame < 200 {
                value += 0.9;
            }
            samples.push(value);
        }
    }

    let bits: u16 = 32;
    let block_align = channels * bits / 8;
    let byte_rate = sample_rate * block_align as u32;
    let data_len = (samples.len() * 4) as u32;

    let mut file = std::fs::File::create(&path).expect("create test WAV");
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    // 3 = WAVE_FORMAT_IEEE_FLOAT. This is the field that marks the file as float.
    file.write_all(&3u16.to_le_bytes()).unwrap();
    file.write_all(&channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&block_align.to_le_bytes()).unwrap();
    file.write_all(&bits.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_len.to_le_bytes()).unwrap();
    for s in &samples {
        file.write_all(&s.to_le_bytes()).unwrap();
    }

    path
}

/// Converts a local path to the URL the core `filesystem` module expects.
fn as_url(path: &std::path::Path) -> String {
    crate::filesystem::path_to_url(&path.to_string_lossy())
}

#[test]
fn decodes_float_wav_preserving_the_source_format() {
    let path = write_float_wav("gyroflow_test_f32.wav", 48000, 2, 1.0, 0.5);
    let track = decode_file(&as_url(&path)).expect("decode the float WAV");

    // The core of the feature: float must be recognized as float.
    assert_eq!(track.source_format, SourceFormat::F32);
    assert!(track.source_format.is_float());
    assert_eq!(track.sample_rate, 48000);
    assert_eq!(track.channels, 2, "the original channels must be preserved");
    assert_eq!(track.frame_count(), 48000);
    assert!((track.duration_seconds() - 1.0).abs() < 0.001);

    // The analysis mono exists, is separate, and has one value per frame.
    assert_eq!(track.mono_analysis.len(), 48000);
    assert_ne!(track.mono_analysis.len(), track.samples.len(), "the mono must not be the export buffer");

    assert!(track.preserve_original_format);

    let _ = std::fs::remove_file(path);
}

/// Regression: the PCM format comes from `CodecType`, not from `sample_format`.
///
/// Symphonia leaves `sample_format` empty for PCM streams. The first version of
/// this decoder only looked at that field, so a 32-bit float WAV was classified
/// as `Other` - and would have been silently exported as AAC, discarding the
/// headroom without ever telling the user.
#[test]
fn float_wav_must_not_be_classified_as_compressed() {
    let path = write_float_wav("gyroflow_test_regression_float.wav", 48000, 2, 0.2, 0.1);
    let track = decode_file(&as_url(&path)).expect("decode");

    assert_ne!(
        track.source_format,
        SourceFormat::Other,
        "float WAV classified as compressed: it would be exported lossy"
    );
    assert!(track.source_format.is_float());

    // And the practical consequence: the recommended codec must be float.
    assert_eq!(
        super::export::recommended_codec(track.source_format),
        "PCM (f32le)"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn mono_decoding_also_works() {
    let path = write_float_wav("gyroflow_test_f32_mono.wav", 44100, 1, 0.5, 0.25);
    let track = decode_file(&as_url(&path)).expect("decode mono WAV");

    assert_eq!(track.channels, 1);
    assert_eq!(track.sample_rate, 44100);
    assert_eq!(track.source_format, SourceFormat::F32);

    let _ = std::fs::remove_file(path);
}

#[test]
fn missing_file_returns_a_useful_error() {
    let err = decode_file("file:///path/that/does/not/exist.wav").unwrap_err();
    // Matched on the variant rather than the message text, which is localized.
    assert!(matches!(err, super::decode::DecodeError::Open(_)), "unexpected error: {err}");
}

#[test]
fn invalid_file_is_rejected() {
    let path = std::env::temp_dir().join("gyroflow_test_garbage.wav");
    std::fs::write(&path, b"this is not an audio file").unwrap();

    assert!(decode_file(&as_url(&path)).is_err());

    let _ = std::fs::remove_file(path);
}

#[test]
fn full_path_from_file_to_export_buffer() {
    // Realistic scenario: 5 s float WAV, 3 s video, +0.5 s offset and a trim in
    // the middle.
    let path = write_float_wav("gyroflow_test_pipeline.wav", 48000, 2, 5.0, 2.0);
    let mut track = decode_file(&as_url(&path)).expect("decode");
    track.offset_seconds = 0.5;

    let full = build_from_trim_ranges(&track, &[], 3.0);
    assert_eq!(full.len(), (3.0 * 48000.0) as usize * 2, "the buffer must cover the whole video");

    // Trimming [1/3, 2/3] of the 3 s video leaves 1 s of audio.
    let trimmed = build_from_trim_ranges(&track, &[(1.0 / 3.0, 2.0 / 3.0)], 3.0);
    assert_eq!(trimmed.len(), 48000 * 2, "1 s trim at 48 kHz stereo");

    let compat = check_format_compatibility(track.source_format, "mov", true);
    assert_eq!(compat, FormatCompatibility::Preserved { codec: "PCM (f32le)" });

    // The same material in MP4 must be blocked, not converted.
    let compat_mp4 = check_format_compatibility(track.source_format, "mp4", true);
    assert!(!compat_mp4.can_proceed(), "float in MP4 must require a user decision");

    let _ = std::fs::remove_file(path);
}

#[test]
fn waveform_of_a_real_file_has_coherent_peaks() {
    let path = write_float_wav("gyroflow_test_waveform.wav", 48000, 2, 1.0, 0.5);
    let track = decode_file(&as_url(&path)).expect("decode");

    let peaks = track.peaks_for_width(100);
    assert!(peaks.len() <= 100 && !peaks.is_empty());

    // The transient at t=0.5 s must show up as a peak larger than the surrounding
    // sine: it is halfway through the file, so near bucket 50.
    let middle = &peaks[45..55];
    let max_middle = middle.iter().map(|(_, max)| *max).fold(f32::MIN, f32::max);
    let max_start = peaks[..10].iter().map(|(_, max)| *max).fold(f32::MIN, f32::max);
    assert!(max_middle > max_start, "the transient should produce a larger peak: {max_middle} vs {max_start}");

    let _ = std::fs::remove_file(path);
}
