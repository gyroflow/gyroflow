// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

//! Encoding and muxing of the external audio track.
//!
//! Unlike [`super::ffmpeg_audio`], which transcodes streams that already exist
//! in the input file and copies their parameters from a `format::stream::Stream`,
//! there is no input stream here - the audio comes from another file, already
//! decoded in memory - so the parameters come from the [`AudioTrack`].
//!
//! Samples arrive as `f32` from [`gyroflow_core::audio::export`] and are
//! converted only once, to whatever format the encoder asks for. With
//! `pcm_f32le` and an encoder that accepts `F32` there is no conversion at all,
//! so the original bits reach the output intact.

use ffmpeg_next::{ codec, encoder, format, frame, Error, Rational };
use ffmpeg_next::channel_layout::ChannelLayout;
use ffmpeg_next::format::context::Output;

use gyroflow_core::audio::AudioTrack;

use super::audio_resampler::AudioResampler;

/// Writes an in-memory audio buffer as a new stream of the output file.
pub struct ExternalAudioEncoder {
    ost_index: usize,
    encoder: encoder::Audio,
    /// `None` when the encoder already accepts interleaved `f32` - the
    /// `pcm_f32le` case, where any conversion would be gratuitous loss.
    resampler: Option<AudioResampler>,
    /// Samples per channel that each frame carries to the encoder.
    frame_size: usize,
    sample_rate: u32,
    channels: u16,
    /// Frames sent so far, used to compute the PTS.
    frames_written: i64,
}

impl ExternalAudioEncoder {
    /// Creates the output stream and configures the encoder from the track.
    ///
    /// `codec_id` must come from
    /// [`gyroflow_core::audio::export::recommended_codec`], already validated
    /// against the container - getting here with a codec the container does not
    /// accept would produce an unreadable file.
    pub fn new(codec_id: codec::Id, track: &AudioTrack, octx: &mut Output, ost_index: usize) -> Result<Self, Error> {
        let codec = encoder::find(codec_id).ok_or(Error::EncoderNotFound)?.audio()?;
        let global = octx.format().flags().contains(format::flag::Flags::GLOBAL_HEADER);

        let channels = track.channels.max(1) as i32;
        let channel_layout = codec
            .channel_layouts()
            .map_or(ChannelLayout::default(channels), |cls| cls.best(channels));

        let mut output = octx.add_stream(codec)?;
        let ctx = unsafe {
            codec::context::Context::wrap(ffmpeg_next::ffi::avcodec_alloc_context3(codec.as_ptr()), None)
        };
        let mut encoder = ctx.encoder().audio()?;

        if global {
            encoder.set_flags(codec::flag::Flags::GLOBAL_HEADER);
        }

        // Sample rate and channels come from the original file, with no
        // resampling: the material must be preserved as it was recorded.
        encoder.set_rate(track.sample_rate as i32);
        encoder.set_channel_layout(channel_layout);

        // Prefer F32 among the formats the codec accepts - that is what avoids
        // converting float audio.
        let target_format = codec
            .formats()
            .and_then(|mut formats| {
                let all: Vec<_> = formats.by_ref().collect();
                all.iter()
                    .find(|f| matches!(f, format::Sample::F32(_)))
                    .copied()
                    .or_else(|| all.first().copied())
            })
            .ok_or(Error::EncoderNotFound)?;
        encoder.set_format(target_format);

        encoder.set_time_base((1, track.sample_rate as i32));
        output.set_time_base((1, track.sample_rate as i32));

        let encoder = encoder.open_as(codec)?;
        output.set_parameters(&encoder);

        // PCM accepts any frame size and reports 0; use a fixed block in that
        // case, to avoid emitting thousands of tiny packets.
        let frame_size = if encoder.frame_size() > 0 { encoder.frame_size() as usize } else { 1024 };

        // The resampler is only used when the encoder doesn't accept interleaved f32.
        let source_format = format::Sample::F32(format::sample::Type::Packed);
        let resampler = if encoder.format() == source_format && encoder.rate() == track.sample_rate {
            None
        } else {
            Some(AudioResampler::new(
                (source_format, channel_layout, track.sample_rate),
                (encoder.format(), encoder.channel_layout(), encoder.rate()),
                frame_size,
            )?)
        };

        Ok(Self {
            ost_index,
            encoder,
            resampler,
            frame_size,
            sample_rate: track.sample_rate,
            channels: track.channels.max(1),
            frames_written: 0,
        })
    }

    /// Index of the audio stream in the output file.
    ///
    /// The processor's `ost_time_bases` vector is sized by the input streams and
    /// has no slot for this index, which corresponds to none of them, so the
    /// stream's time base has to be looked up at write time.
    pub fn stream_index(&self) -> usize {
        self.ost_index
    }

    /// Encodes the whole buffer and writes the packets to the output file.
    ///
    /// `samples` are interleaved `f32` samples, already trimmed and aligned by
    /// [`gyroflow_core::audio::export::build_segment`].
    pub fn write_all(&mut self, samples: &[f32], octx: &mut Output, ost_time_base: Rational) -> Result<(), Error> {
        let channels = self.channels as usize;
        if channels == 0 || samples.is_empty() {
            return Ok(());
        }

        let total_frames = samples.len() / channels;
        let mut pos = 0usize;

        while pos < total_frames {
            let this_frame = (total_frames - pos).min(self.frame_size);

            let mut input = frame::Audio::new(
                format::Sample::F32(format::sample::Type::Packed),
                this_frame,
                self.encoder.channel_layout(),
            );
            input.set_rate(self.sample_rate);
            input.set_pts(Some(self.frames_written));

            // A Packed `frame::Audio` stores all channels interleaved in plane 0,
            // but `plane_mut::<f32>(0)` returns a slice sized in frames, not in
            // samples - in stereo it holds half the values we need to write.
            // Hence the access through the raw data buffer.
            let src = &samples[pos * channels..(pos + this_frame) * channels];
            let dst = input.data_mut(0);
            let byte_len = std::mem::size_of_val(src);
            debug_assert!(dst.len() >= byte_len, "plane smaller than expected: {} < {byte_len}", dst.len());
            dst[..byte_len].copy_from_slice(unsafe {
                // `f32` has no padding nor invariants, so reinterpreting it as
                // bytes is safe, and is what ffmpeg expects to receive.
                std::slice::from_raw_parts(src.as_ptr() as *const u8, byte_len)
            });

            match &mut self.resampler {
                Some(resampler) => {
                    resampler.new_frame(&mut input)?;
                    while let Some(out_frame) = resampler.run() {
                        self.encoder.send_frame(out_frame)?;
                    }
                }
                // Lossless path: the buffer goes straight to the encoder.
                None => {
                    self.encoder.send_frame(&input)?;
                }
            }

            self.receive_packets(octx, ost_time_base)?;

            self.frames_written += this_frame as i64;
            pos += this_frame;
        }

        Ok(())
    }

    /// Flushes the encoder at the end of the export.
    pub fn finish(&mut self, octx: &mut Output, ost_time_base: Rational) -> Result<(), Error> {
        // Drain whatever is left in the resampler before closing the encoder.
        if let Some(resampler) = &mut self.resampler {
            if let Some(out_frame) = resampler.flush() {
                self.encoder.send_frame(out_frame)?;
            }
        }
        self.encoder.send_eof()?;
        self.receive_packets(octx, ost_time_base)
    }

    /// Drains the ready packets and writes them with the correct timestamp.
    fn receive_packets(&mut self, octx: &mut Output, ost_time_base: Rational) -> Result<(), Error> {
        let mut packet = ffmpeg_next::Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.ost_index);
            packet.rescale_ts(self.encoder.time_base(), ost_time_base);
            packet.write_interleaved(octx)?;
        }
        Ok(())
    }
}
