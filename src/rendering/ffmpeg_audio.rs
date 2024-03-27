// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>, Maik <myco at gmx>

use ffmpeg_next::{ ffi, codec, format, decoder, encoder, frame, Packet, Rescale, Rational, Error, format::context::Output, channel_layout::ChannelLayout};
use super::audio_resampler::AudioResampler;
use super::ffmpeg_processor::Status;
use super::ffmpeg_processor::FrameTimestamps;

pub struct AudioTranscoder {
    pub ost_index: usize,
    pub decoder: decoder::Audio,
    pub encoder: encoder::Audio,
    resampler: AudioResampler
}

impl AudioTranscoder {
    pub fn new(codec_id: codec::Id, ist: &format::stream::Stream, octx: &mut Output, ost_index: usize) -> Result<Self, Error> {
        let ctx = codec::context::Context::from_parameters(ist.parameters())?;
        let mut decoder = ctx.decoder().audio()?;
        let codec = encoder::find(codec_id).expect("failed to find encoder").audio()?;
        let global = octx.format().flags().contains(format::flag::Flags::GLOBAL_HEADER);

        decoder.set_parameters(ist.parameters())?;

        let mut output = octx.add_stream(codec)?;
        let ctx = unsafe { codec::context::Context::wrap(ffi::avcodec_alloc_context3(codec.as_ptr()), None) };
        let mut encoder = ctx.encoder().audio()?;

        let channels: i32 = decoder.channels().into();
        let channel_layout = codec.channel_layouts().map_or(ChannelLayout::default(channels), |cls| cls.best(channels));

        if global {
            encoder.set_flags(codec::flag::Flags::GLOBAL_HEADER);
        }

        encoder.set_rate(decoder.rate() as i32);
        encoder.set_channel_layout(channel_layout);
        // encoder.set_channels(channel_layout.channels());
        encoder.set_format(codec.formats().expect("unknown supported formats").next().unwrap());
        encoder.set_bit_rate(decoder.bit_rate().min(320000));
        encoder.set_max_bit_rate(decoder.max_bit_rate().min(320000));

        encoder.set_time_base((1, decoder.rate() as i32));
        output.set_time_base((1, decoder.rate() as i32));

        let encoder = encoder.open_as(codec)?;
        output.set_parameters(&encoder);

        let mut in_channel_layout = decoder.channel_layout();
        if in_channel_layout.is_empty() {
            in_channel_layout = ChannelLayout::default(channels);
        }
        let resampler = AudioResampler::new(
            (decoder.format(), in_channel_layout, decoder.rate()),
            (encoder.format(), encoder.channel_layout(), encoder.rate()),
            1024
        )?;

        Ok(Self {
            ost_index,
            decoder,
            encoder,
            resampler,
        })
    }

    pub fn receive_and_process_decoded_frames(&mut self, octx: &mut Output, ost_time_base: Rational, start_ms: Option<f64>, end_ms: Option<f64>, frame_ts: &mut FrameTimestamps) -> Result<Status, Error> {
        let mut status = Status::Continue;
        let mut frame = frame::Audio::empty();

        while self.decoder.receive_frame(&mut frame).is_ok() {

            if let Some(ts) = frame.timestamp() {
                let timestamp_us = ts.rescale(self.decoder.time_base(), (1, 1000000));
                let timestamp_ms = timestamp_us as f64 / 1000.0;

                if start_ms.is_none() || timestamp_ms >= start_ms.unwrap() {
                    if frame_ts.first.is_none() {
                        frame_ts.first = Some(timestamp_us);
                    }
                    let new_ts = timestamp_us - frame_ts.first.unwrap() + frame_ts.add_audio;
                    if new_ts >= 0 {
                        frame.set_pts(Some(new_ts.rescale((1, 1000000), self.decoder.time_base())));

                        self.resampler.new_frame(&mut frame)?;
                        while let Some(out_frame) = self.resampler.run() {
                            self.encoder.send_frame(out_frame)?;
                            self.receive_and_process_encoded_packets(octx, ost_time_base)?;
                        }
                        if let Some(last_ts) = frame_ts.last_audio {
                            frame_ts.last_duration_audio = new_ts - last_ts;
                        }
                        frame_ts.last_audio = Some(new_ts);
                    }
                }
                if end_ms.is_some() && timestamp_ms > end_ms.unwrap() {
                    status = Status::Finish;
                    break;
                }
            }
        }
        Ok(status)
    }

    pub fn receive_and_process_encoded_packets(&mut self, octx: &mut Output, ost_time_base: Rational) -> Result<(), Error> {
        let mut encoded = Packet::empty();
        while self.encoder.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(self.ost_index);
            encoded.rescale_ts(self.decoder.time_base(), ost_time_base);
            encoded.write_interleaved(octx)?;
        }
        Ok(())
    }

    pub fn flush(&mut self, octx: &mut Output, ost_time_base: Rational, start_ms: Option<f64>, end_ms: Option<f64>, frame_ts: &mut FrameTimestamps) -> Result<(), Error> {
        self.decoder.send_eof()?;
        self.receive_and_process_decoded_frames(octx, ost_time_base, start_ms, end_ms, frame_ts)?;

        if let Some(out_frame) = self.resampler.flush() {
            self.encoder.send_frame(out_frame)?;
        }

        self.encoder.send_eof()?;
        self.receive_and_process_encoded_packets(octx, ost_time_base)?;
        Ok(())
    }
}
