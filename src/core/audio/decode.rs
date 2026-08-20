// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

//! Decoding of the external audio file.
//!
//! # Why symphonia and not the project's ffmpeg
//!
//! Gyroflow already links ffmpeg, but only in the root crate: `gyroflow-core` does
//! not have `ffmpeg-next` among its dependencies and is meant to stay that way -
//! the core is pure logic, buildable and testable without the video toolchain.
//! Since the decode lives in `src/core/audio/`, using ffmpeg would drag that whole
//! dependency into the core.
//!
//! `symphonia` solves the case without that cost: it is pure Rust, was already in
//! `Cargo.lock` (transitively, via rodio) and covers WAV/PCM including IEEE 32-bit
//! float, which is the DJI Mic format and the central requirement here. The
//! enabled features also cover AAC/MP4, MP3 and FLAC, so the user can import the
//! `.m4a` that some recorders produce.

use symphonia::core::audio::{AudioBufferRef, SampleBuffer};
use symphonia::core::codecs::{CodecType, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::sample::SampleFormat;

use super::{AudioTrack, SourceFormat};

/// Adapts [`crate::filesystem::FileWrapper`] to the interface symphonia expects.
///
/// `FileWrapper` already provides `Read + Seek` and knows the file size; only the
/// `MediaSource` declarations are missing. Going through the wrapper (instead of
/// opening a `std::fs::File`) is what makes the import work on Android, where
/// files arrive as `content://`.
struct FileWrapperSource(crate::filesystem::FileWrapper);

impl std::io::Read for FileWrapperSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(&mut self.0, buf)
    }
}
impl std::io::Seek for FileWrapperSource {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        std::io::Seek::seek(&mut self.0, pos)
    }
}
impl MediaSource for FileWrapperSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.0.size as u64)
    }
}

/// Failures while importing an audio file.
///
/// The messages are written to be shown to the user as-is.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("Could not open the audio file: {0}")]
    Open(String),

    #[error("Unrecognized audio format or corrupted file: {0}")]
    UnsupportedFormat(String),

    #[error("The file contains no decodable audio track")]
    NoAudioTrack,

    #[error("The file does not report the sample rate of the audio track")]
    MissingSampleRate,

    #[error("Failed to decode the audio: {0}")]
    Decode(String),

    #[error("The audio track is empty")]
    Empty,
}

/// Derives the source format from the codec type.
///
/// For PCM, symphonia does not fill in `sample_format` in the stream parameters -
/// the information is in the `CodecType`. Ignoring that would classify a 32-bit
/// float WAV as `Other` and export it lossily. This was a real bug, caught by the
/// tests against an actual file.
fn map_codec_type(codec: CodecType) -> Option<SourceFormat> {
    use symphonia::core::codecs::*;
    match codec {
        CODEC_TYPE_PCM_F32LE | CODEC_TYPE_PCM_F32BE | CODEC_TYPE_PCM_F64LE | CODEC_TYPE_PCM_F64BE
        | CODEC_TYPE_PCM_F32LE_PLANAR | CODEC_TYPE_PCM_F32BE_PLANAR
        | CODEC_TYPE_PCM_F64LE_PLANAR | CODEC_TYPE_PCM_F64BE_PLANAR => Some(SourceFormat::F32),

        CODEC_TYPE_PCM_S32LE | CODEC_TYPE_PCM_S32BE | CODEC_TYPE_PCM_U32LE | CODEC_TYPE_PCM_U32BE
        | CODEC_TYPE_PCM_S32LE_PLANAR | CODEC_TYPE_PCM_S32BE_PLANAR
        | CODEC_TYPE_PCM_U32LE_PLANAR | CODEC_TYPE_PCM_U32BE_PLANAR => Some(SourceFormat::S32),

        CODEC_TYPE_PCM_S24LE | CODEC_TYPE_PCM_S24BE | CODEC_TYPE_PCM_U24LE | CODEC_TYPE_PCM_U24BE
        | CODEC_TYPE_PCM_S24LE_PLANAR | CODEC_TYPE_PCM_S24BE_PLANAR
        | CODEC_TYPE_PCM_U24LE_PLANAR | CODEC_TYPE_PCM_U24BE_PLANAR => Some(SourceFormat::S24),

        CODEC_TYPE_PCM_S16LE | CODEC_TYPE_PCM_S16BE | CODEC_TYPE_PCM_U16LE | CODEC_TYPE_PCM_U16BE
        | CODEC_TYPE_PCM_S16LE_PLANAR | CODEC_TYPE_PCM_S16BE_PLANAR
        | CODEC_TYPE_PCM_U16LE_PLANAR | CODEC_TYPE_PCM_U16BE_PLANAR => Some(SourceFormat::S16),

        CODEC_TYPE_PCM_S8 | CODEC_TYPE_PCM_U8
        | CODEC_TYPE_PCM_S8_PLANAR | CODEC_TYPE_PCM_U8_PLANAR => Some(SourceFormat::U8),

        // Not PCM: probably a lossy codec, handled by the caller.
        _ => None,
    }
}

/// Maps the format reported by symphonia to the enum we store.
///
/// `bits_per_sample` is consulted because a 24-bit WAV is usually delivered as
/// `S32` with `bits_per_sample = 24`: keeping that distinction avoids exporting as
/// 32 bits material that was born with 24.
fn map_source_format(sample_format: Option<SampleFormat>, bits_per_sample: Option<u32>) -> SourceFormat {
    match sample_format {
        Some(SampleFormat::F32) | Some(SampleFormat::F64) => SourceFormat::F32,
        Some(SampleFormat::S32) | Some(SampleFormat::U32) => match bits_per_sample {
            Some(24) => SourceFormat::S24,
            _ => SourceFormat::S32,
        },
        Some(SampleFormat::S24) | Some(SampleFormat::U24) => SourceFormat::S24,
        Some(SampleFormat::S16) | Some(SampleFormat::U16) => SourceFormat::S16,
        Some(SampleFormat::S8) | Some(SampleFormat::U8) => SourceFormat::U8,
        // Lossy codecs (AAC, MP3) do not expose sample_format: the loss already
        // happened before the file reached us, so there is no source format to
        // preserve bit for bit.
        None => SourceFormat::Other,
    }
}

/// Builds the mono downmix used only by the sync analysis.
///
/// A plain average across channels is enough here: the auto-sync correlates energy
/// envelopes, which are insensitive to phase and absolute gain.
///
/// This buffer must never be exported - see the module documentation.
fn downmix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let channels = channels as usize;
    let frames = interleaved.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for frame in 0..frames {
        let base = frame * channels;
        let sum: f32 = interleaved[base..base + channels].iter().sum();
        mono.push(sum / channels as f32);
    }
    mono
}

/// Decodes an audio file into an [`AudioTrack`].
///
/// `url` is a [`crate::filesystem`] URL, not a raw path, so the import also works
/// on Android, where access goes through `content://` rather than the filesystem.
///
/// Samples are kept as interleaved `f32` preserving all original channels, and the
/// source format is recorded so the export can reconstruct the file losslessly.
pub fn decode_file(url: &str) -> Result<AudioTrack, DecodeError> {
    // `FileWrapper` implements Read + Seek and handles the descriptor lifetime on
    // Android (see filesystem/mod.rs:119-144).
    let file = crate::filesystem::open_file(url, false, false)
        .map_err(|e| DecodeError::Open(e.to_string()))?;
    let stream = MediaSourceStream::new(Box::new(FileWrapperSource(file)), Default::default());

    // The extension is only a hint; symphonia confirms it from the actual content.
    let filename = crate::filesystem::get_filename(url);
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(&filename).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| DecodeError::UnsupportedFormat(e.to_string()))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(DecodeError::NoAudioTrack)?;

    let track_id = track.id;
    let params = track.codec_params.clone();

    let sample_rate = params.sample_rate.ok_or(DecodeError::MissingSampleRate)?;

    // For PCM the format comes from the codec type; sample_format is usually
    // empty. Only when the codec is not PCM (AAC, MP3...) do we fall back to it.
    let source_format = map_codec_type(params.codec)
        .unwrap_or_else(|| map_source_format(params.sample_format, params.bits_per_sample));

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| DecodeError::UnsupportedFormat(e.to_string()))?;

    // Reserve based on `n_frames` when the container reports the duration, avoiding
    // dozens of reallocations on a long file.
    let mut samples: Vec<f32> = Vec::with_capacity(
        params.n_frames.unwrap_or(0).saturating_mul(params.channels.map_or(2, |c| c.count() as u64)) as usize,
    );
    let mut channels: u16 = params.channels.map_or(0, |c| c.count() as u16);
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // End of stream: `ResetRequired` and the IO `UnexpectedEof` are the two
            // normal ways for the read to finish.
            Err(SymphoniaError::ResetRequired) => break,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(DecodeError::Decode(e.to_string())),
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                append_samples(&decoded, &mut sample_buf, &mut samples, &mut channels);
            }
            // Isolated corrupted packets do not invalidate the whole file;
            // symphonia recommends carrying on in these two cases.
            Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
            Err(e) => return Err(DecodeError::Decode(e.to_string())),
        }
    }

    if samples.is_empty() || channels == 0 {
        return Err(DecodeError::Empty);
    }

    let mono_analysis = downmix_to_mono(&samples, channels);

    Ok(AudioTrack {
        path: url.to_string(),
        samples,
        channels,
        sample_rate,
        source_format,
        mono_analysis,
        offset_seconds: 0.0,
        preserve_original_format: true,
    })
}

/// Converts a decoded buffer to interleaved `f32` and accumulates it.
///
/// `SampleBuffer::copy_interleaved_ref` handles every [`AudioBufferRef`] variant,
/// so no ten-arm match is needed - and the `f32` conversion is the same one we
/// would write by hand.
fn append_samples(
    decoded: &AudioBufferRef,
    sample_buf: &mut Option<SampleBuffer<f32>>,
    out: &mut Vec<f32>,
    channels: &mut u16,
) {
    let spec = *decoded.spec();
    if *channels == 0 {
        *channels = spec.channels.count() as u16;
    }

    // The buffer is recreated when the capacity is not enough: some containers
    // deliver a first packet smaller than the following ones.
    let needs_new = match sample_buf {
        Some(buf) => buf.capacity() < decoded.capacity(),
        None => true,
    };
    if needs_new {
        *sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
    }

    if let Some(buf) = sample_buf {
        buf.copy_interleaved_ref(decoded.clone());
        out.extend_from_slice(buf.samples());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_the_channels() {
        // Two interleaved channels: [L=1.0, R=0.0] and [L=0.0, R=1.0].
        let stereo = vec![1.0, 0.0, 0.0, 1.0];
        let mono = downmix_to_mono(&stereo, 2);
        assert_eq!(mono, vec![0.5, 0.5]);
    }

    #[test]
    fn downmix_of_mono_is_identity() {
        let mono_in = vec![0.1, -0.2, 0.3];
        assert_eq!(downmix_to_mono(&mono_in, 1), mono_in);
    }

    #[test]
    fn float_is_recognized_as_float() {
        assert!(map_source_format(Some(SampleFormat::F32), Some(32)).is_float());
        assert!(map_source_format(Some(SampleFormat::F64), Some(64)).is_float());
    }

    #[test]
    fn wav_24_bit_does_not_become_32() {
        // A 24-bit WAV is usually delivered as S32 with bits_per_sample=24.
        assert_eq!(map_source_format(Some(SampleFormat::S32), Some(24)), SourceFormat::S24);
        assert_eq!(map_source_format(Some(SampleFormat::S32), Some(32)), SourceFormat::S32);
    }

    #[test]
    fn lossy_codec_has_no_format_to_preserve() {
        assert_eq!(map_source_format(None, None), SourceFormat::Other);
        assert!(!map_source_format(None, None).is_float());
    }
}
