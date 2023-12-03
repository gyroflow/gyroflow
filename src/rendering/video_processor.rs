// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use super::*;
use super::mdk_processor::*;
use super::ffmpeg_video::RateControl;
use ffmpeg_next::{ frame, Dictionary };
use std::{ sync::{ Arc, atomic::{ AtomicI32, AtomicBool } }, rc::Rc, cell::RefCell };

pub enum Processor<'a> {
    Ffmpeg(FfmpegProcessor<'a>),
    Mdk(MDKProcessor)
}
pub struct VideoProcessor<'a> {
    inner: Processor<'a>
}

impl<'a> VideoProcessor<'a> {
    pub fn from_file(base: &'a gyroflow_core::filesystem::EngineBase, url: &str, gpu_decoding: bool, gpu_decoder_index: usize, decoder_options: Option<Dictionary>) -> Result<Self, FFmpegError> {
        let filename = gyroflow_core::filesystem::get_filename(url);
        if filename.to_lowercase().ends_with(".braw") || filename.to_lowercase().ends_with(".r3d") {
            Ok(Self { inner: Processor::Mdk(MDKProcessor::from_file(url, decoder_options)) })
        } else {
            Ok(Self { inner: Processor::Ffmpeg(FfmpegProcessor::from_file(base, url, gpu_decoding, gpu_decoder_index, decoder_options)?) })
        }
    }

    pub fn get_org_dimensions(&self) -> Option<(Arc<AtomicI32>, Arc<AtomicI32>)> {
        match &self.inner {
            Processor::Ffmpeg(_) => None,
            Processor::Mdk(x) => x.get_org_dimensions(),
        }
    }
    pub fn get_video_info(url: &str) -> Result<crate::rendering::ffmpeg_processor::VideoInfo, ffmpeg_next::Error> {
        let filename = gyroflow_core::filesystem::get_filename(url);
        if filename.to_lowercase().ends_with(".braw") || filename.to_lowercase().ends_with(".r3d") {
            let mut mdk = MDKProcessor::from_file(url, None);

            let (tx, rx) = futures_intrusive::channel::shared::oneshot_channel();

            let info = Rc::new(RefCell::new(crate::rendering::ffmpeg_processor::VideoInfo::default()));

            let info2 = info.clone();
            mdk.mdk.startProcessing(0, 0, 0, false, &mdk.custom_decoder, vec![], move |frame_num, _, _, _, org_width, org_height, fps, duration_ms, frame_count, data| {
                if fps > 0.0 && org_width > 0 {
                    let mut info2 = info2.borrow_mut();
                    info2.duration_ms = duration_ms;
                    info2.frame_count = frame_count as usize;
                    info2.fps = fps;
                    info2.width = org_width;
                    info2.height = org_height;
                }
                if frame_num == -1 || data.is_empty() {
                    let _ = tx.send(());
                    return true;
                }
                false
            });
            pollster::block_on(rx.receive());
            info.borrow_mut().rotation = mdk.mdk.getRotation();

            std::thread::sleep(std::time::Duration::from_millis(100));

            let info = info.borrow().clone();
            Ok(info)
        } else {
            FfmpegProcessor::get_video_info(url)
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
