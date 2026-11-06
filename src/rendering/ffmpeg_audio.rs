// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>, Maik <myco at gmx>

use ffmpeg_next::{ ffi, codec, format, decoder, encoder, frame, Packet, Rescale, Rational, Error, format::context::Output, channel_layout::ChannelLayout};
use super::audio_resampler::AudioResampler;
use super::ffmpeg_processor::FrameTimestamps;

pub struct AudioTranscoder {
    pub ist_index: usize,
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
            ist_index: ist.index(),
            ost_index,
            decoder,
            encoder,
            resampler,
        })
    }

    pub fn receive_and_process_decoded_frames(&mut self, octx: &mut Output, ost_time_base: Rational, start_ms: Option<f64>, frame_ts: &mut FrameTimestamps) -> Result<(), Error> {
        let mut frame = frame::Audio::empty();

        while self.decoder.receive_frame(&mut frame).is_ok() {
            let timestamp_us = frame.timestamp().map(|ts| ts.rescale(self.decoder.time_base(), (1, 1000000)));
            let rate = frame.rate() as i32;
            let duration_us = if rate > 0 { (frame.samples() as i64).rescale((1, rate), (1, 1000000)) } else { 0 };

            if let Some(placement) = frame_ts.place(self.ist_index, timestamp_us, duration_us, rate > 0, start_ms) {
                if placement.skip_us > 0 || placement.duration_us < duration_us {
                    // Cut to the samples that are written
                    let samples = frame.samples();
                    let skip = (placement.skip_us.rescale((1, 1000000), (1, rate)).max(0) as usize).min(samples);
                    let count = (placement.duration_us.rescale((1, 1000000), (1, rate)).max(0) as usize).min(samples - skip);
                    if count == 0 {
                        continue;
                    }
                    unsafe { Self::cut_frame(&mut frame, skip, count); }
                }
                frame.set_pts(Some(placement.ts_us.rescale((1, 1000000), self.decoder.time_base())));

                self.resampler.new_frame(&mut frame)?;
                while let Some(out_frame) = self.resampler.run() {
                    self.encoder.send_frame(out_frame)?;
                    self.receive_and_process_encoded_packets(octx, ost_time_base)?;
                }
            }
        }
        Ok(())
    }

    /// Keeps `count` samples of a decoded frame from `skip` on, by moving its data pointers
    unsafe fn cut_frame(frame: &mut frame::Audio, skip: usize, count: usize) {
        let (planes, offset) = if frame.is_planar() {
            (frame.channels() as usize, skip * frame.format().bytes())
        } else {
            (1, skip * frame.format().bytes() * frame.channels() as usize)
        };
        unsafe {
            let frame = frame.as_mut_ptr();
            for i in 0..planes {
                let plane = (*frame).extended_data.add(i);
                *plane = (*plane).add(offset);
                if i < (*frame).data.len() {
                    (*frame).data[i] = *plane;
                }
            }
            (*frame).nb_samples = count as i32;
        }
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

    pub fn flush(&mut self, octx: &mut Output, ost_time_base: Rational, start_ms: Option<f64>, frame_ts: &mut FrameTimestamps) -> Result<(), Error> {
        self.decoder.send_eof()?;
        self.receive_and_process_decoded_frames(octx, ost_time_base, start_ms, frame_ts)?;

        if let Some(out_frame) = self.resampler.flush() {
            self.encoder.send_frame(out_frame)?;
        }

        self.encoder.send_eof()?;
        self.receive_and_process_encoded_packets(octx, ost_time_base)?;
        Ok(())
    }
}
