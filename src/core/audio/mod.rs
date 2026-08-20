// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

//! External audio synchronization with the gyroscope.
//!
//! Imports an audio track recorded by a separate device (DJI Mic and similar),
//! allows aligning it to the video, and prepares the buffer to be embedded in
//! the export.
//!
//! # Analysis audio is not export audio
//!
//! [`AudioTrack`] holds two representations of the same file, and mixing them up
//! is the most expensive mistake possible here:
//!
//! - [`AudioTrack::samples`] is the export audio: all original channels, original
//!   sample rate, kept in `f32` so no intermediate stage quantizes it. The source
//!   format is recorded in [`AudioTrack::source_format`] so the final encode can
//!   reconstruct the file losslessly.
//! - [`AudioTrack::mono_analysis`] is the analysis audio: a throwaway mono downmix
//!   that exists only for the auto-sync cross-correlation. It must never be
//!   exported.
//!
//! The whole pipeline (offset, silence, trim) works in `f32`; conversion to the
//! output format happens only once, at encode time.

pub mod decode;
pub mod export;
pub mod features;
pub mod sync;
pub mod waveform;

#[cfg(test)]
mod tests_integration;

use std::fmt;

/// Source file format, kept so the material can be re-encoded losslessly.
///
/// Audio is held in `f32` in memory regardless of this; the enum records where it
/// came from, so the export can pick a codec that does not degrade it (float
/// stays float, and the bit depth of integer PCM is never lowered).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceFormat {
    /// IEEE 32-bit float (WAV format tag 3), the DJI Mic format.
    ///
    /// Also the default: an empty [`AudioTrack`] should not suggest there is a
    /// lower-precision format to preserve.
    #[default]
    F32,
    /// Signed 32-bit integer PCM.
    S32,
    /// Signed 24-bit integer PCM.
    S24,
    /// Signed 16-bit integer PCM.
    S16,
    /// Unsigned 8-bit integer PCM.
    U8,
    /// Lossy source (AAC, MP3...) or an unmapped format.
    ///
    /// There is no original format to preserve bit for bit; the export can choose
    /// freely, since the loss happened before the file reached us.
    Other,
}

impl SourceFormat {
    /// Nominal bit depth of the source format.
    pub fn bits(self) -> u32 {
        match self {
            Self::F32 | Self::S32 => 32,
            Self::S24 => 24,
            Self::S16 => 16,
            Self::U8 => 8,
            Self::Other => 0,
        }
    }

    /// Whether the source is floating point.
    ///
    /// This is what makes the export require `pcm_f32le`: silently converting
    /// float to integer or to a lossy codec would discard headroom the user was
    /// never warned about.
    pub fn is_float(self) -> bool {
        matches!(self, Self::F32)
    }

    /// Stable identifier, for the interface to look up a translated label and
    /// for serialization. Never shown to the user as-is.
    pub fn key(self) -> &'static str {
        match self {
            Self::F32   => "f32",
            Self::S32   => "s32",
            Self::S24   => "s24",
            Self::S16   => "s16",
            Self::U8    => "u8",
            Self::Other => "compressed",
        }
    }

    /// Short English label, for logs and for `Display`. The interface must use
    /// [`Self::key`] and translate it instead.
    pub fn label(self) -> &'static str {
        match self {
            Self::F32 => "32-bit float",
            Self::S32 => "32-bit int",
            Self::S24 => "24-bit int",
            Self::S16 => "16-bit int",
            Self::U8 => "8-bit",
            Self::Other => "compressed",
        }
    }
}

impl fmt::Display for SourceFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// An external audio track imported by the user.
///
/// See the module documentation for the distinction between the export fields and
/// the analysis ones.
#[derive(Debug, Clone, Default)]
pub struct AudioTrack {
    /// Source file path in [`crate::filesystem`] URL form - this is what gets
    /// serialized into the `.gyroflow` and used to reload the audio when the
    /// project is reopened.
    pub path: String,

    // ---- Export data (original format preserved) ----
    /// Samples of all channels, interleaved (`[c0_s0, c1_s0, c0_s1, ...]`), kept
    /// in `f32`.
    ///
    /// The interleaving follows symphonia's own convention and matches what the
    /// output encoder expects, avoiding a needless transposition.
    pub samples: Vec<f32>,
    /// Channel count of the original file (not 1 when the source is stereo).
    pub channels: u16,
    /// Sample rate of the original file, in Hz.
    pub sample_rate: u32,
    /// Source format, for lossless re-encoding.
    pub source_format: SourceFormat,

    // ---- Analysis/sync data (throwaway) ----
    /// Mono downmix used exclusively by the auto-sync correlation.
    ///
    /// Must never be written to the exported file.
    pub mono_analysis: Vec<f32>,

    /// Shift applied to the audio, in seconds: `t_audio = t_video + offset`.
    pub offset_seconds: f64,

    /// Whether the source format must be preserved bit for bit on export.
    ///
    /// Defaults to `true`. Only the user can turn this off, and only explicitly,
    /// aware of the loss.
    pub preserve_original_format: bool,
}

impl AudioTrack {
    /// Number of audio frames (a frame holds one sample per channel).
    pub fn frame_count(&self) -> usize {
        if self.channels == 0 {
            return 0;
        }
        self.samples.len() / self.channels as usize
    }

    /// Track duration in seconds.
    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frame_count() as f64 / self.sample_rate as f64
    }

    /// Whether the track has no audio loaded.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Readable format summary for logs, for example
    /// `"48000 Hz, 2 ch, 32-bit float"`. The interface builds its own from the
    /// individual fields, so the units and the format label can be translated.
    pub fn format_summary(&self) -> String {
        format!("{} Hz, {} ch, {}", self.sample_rate, self.channels, self.source_format)
    }
}
