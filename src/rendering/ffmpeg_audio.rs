// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>, Maik <myco at gmx>

use ffmpeg_next::{ ffi, codec, format, decoder, encoder, frame, Packet, Rescale, Rational, Error, format::context::Output, channel_layout::ChannelLayout};
use super::audio_resampler::AudioResampler;

pub struct AudioTranscoder {
    pub ost_index: usize,
    pub decoder: decoder::Audio,
    pub encoder: encoder::Audio,
    pub first_frame_ts: Option<i64>,
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

        let channel_layout = codec.channel_layouts().map_or(ChannelLayout::STEREO, |cls| cls.best(decoder.channel_layout().channels()));

        if global {
            encoder.set_flags(codec::flag::Flags::GLOBAL_HEADER);
        }

        encoder.set_rate(decoder.rate() as i32);
        encoder.set_channel_layout(channel_layout);
        encoder.set_channels(channel_layout.channels());
        encoder.set_format(codec.formats().expect("unknown supported formats").next().unwrap());
        encoder.set_bit_rate(decoder.bit_rate().min(320000));
        encoder.set_max_bit_rate(decoder.max_bit_rate().min(320000));

        encoder.set_time_base((1, decoder.rate() as i32));
        output.set_time_base((1, decoder.rate() as i32));

        let encoder = encoder.open_as(codec)?;
        output.set_parameters(&encoder);

        let mut in_channel_layout = decoder.channel_layout();
        if in_channel_layout.is_empty() {
            in_channel_layout = ChannelLayout::STEREO;
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
            first_frame_ts: None
        })
    }

    pub fn receive_and_process_decoded_frames(&mut self, octx: &mut Output, ost_time_base: Rational, start_ms: Option<f64>) -> Result<(), Error> {
        let mut frame = frame::Audio::empty();
        
        while self.decoder.receive_frame(&mut frame).is_ok() {
            
            if let Some(mut ts) = frame.timestamp() {
                let timestamp_us = ts.rescale(self.decoder.time_base(), (1, 1000000));
                let timestamp_ms = timestamp_us as f64 / 1000.0;
                
                if start_ms.is_none() || timestamp_ms >= start_ms.unwrap() {
                    if self.first_frame_ts.is_none() {
                        self.first_frame_ts = frame.timestamp();
                    }
                    ts -= self.first_frame_ts.unwrap();
                    
                    frame.set_pts(Some(ts));

                    self.resampler.new_frame(&mut frame)?;
                    while let Some(out_frame) = self.resampler.run() {
                        self.encoder.send_frame(&out_frame)?;
                        self.receive_and_process_encoded_packets(octx, ost_time_base)?;
                    }
                }
            }
        }
        Ok(())
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

    pub fn flush(&mut self, octx: &mut Output, ost_time_base: Rational, start_ms: Option<f64>) -> Result<(), Error> {
        self.decoder.send_eof()?;
        self.receive_and_process_decoded_frames(octx, ost_time_base, start_ms)?;

        if let Some(out_frame) = self.resampler.flush() {
            self.encoder.send_frame(&out_frame)?;
        }

        self.encoder.send_eof()?;
        self.receive_and_process_encoded_packets(octx, ost_time_base)?;
        Ok(())
    }
}
