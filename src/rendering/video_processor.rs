// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use super::*;
use super::mdk_processor::*;
use super::ffmpeg_video::RateControl;
use ffmpeg_next::{ frame, Dictionary };
use std::sync::{ Arc, atomic::AtomicBool };

pub enum Processor<'a> {
    Ffmpeg(FfmpegProcessor<'a>),
    Mdk(MDKProcessor)
}
pub struct VideoProcessor<'a> {
    inner: Processor<'a>
}

impl<'a> VideoProcessor<'a> {
    pub fn from_file(path: &str, gpu_decoding: bool, gpu_decoder_index: usize, decoder_options: Option<Dictionary>) -> Result<Self, FFmpegError> {
        if path.to_lowercase().ends_with(".braw") {
            Ok(Self { inner: Processor::Mdk(MDKProcessor::from_file(path)) })
        } else {
            Ok(Self { inner: Processor::Ffmpeg(FfmpegProcessor::from_file(path, gpu_decoding, gpu_decoder_index, decoder_options)?) })
        }
    }

    pub fn on_frame<F>(&mut self, cb: F) where F: FnMut(i64, &mut frame::Video, Option<&mut frame::Video>, &mut ffmpeg_video_converter::Converter, &mut RateControl) -> Result<(), FFmpegError> + 'static {
        match &mut self.inner {
            Processor::Ffmpeg(x) => x.on_frame(cb),
            Processor::Mdk(x) => x.on_frame(cb),
        }
    }
    pub fn start_decoder_only(&mut self, ranges: Vec<(f64, f64)>, cancel_flag: Arc<AtomicBool>) -> Result<(), FFmpegError> {
        match &mut self.inner {
            Processor::Ffmpeg(x) => x.start_decoder_only(ranges, cancel_flag),
            Processor::Mdk(x) => x.start_decoder_only(ranges, cancel_flag)
        }
    }
}
