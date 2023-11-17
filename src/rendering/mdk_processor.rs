// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use ffmpeg_next::ffi;
use ffmpeg_next::frame;
use ffmpeg_next::Dictionary;
use super::ffmpeg_video_converter::Converter;
use super::ffmpeg_video::RateControl;
use super::FFmpegError;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::Ordering::SeqCst;
use qmetaobject::QUrl;
use qmetaobject::QString;
use itertools::Itertools;

pub struct MDKProcessor {
    pub mdk: qml_video_rs::video_item::MDKVideoItem,
    format: ffmpeg_next::format::Pixel,
    pub custom_decoder: String,
    org_width: Arc<AtomicI32>,
    org_height: Arc<AtomicI32>,
    url: String,
    pub on_frame_callback: Option<Box<dyn FnMut(i64, &mut frame::Video, Option<&mut frame::Video>, &mut Converter, &mut RateControl) -> Result<(), FFmpegError> + 'static>>,
}
impl Drop for MDKProcessor {
    fn drop(&mut self) {
        gyroflow_core::filesystem::stop_accessing_url(&self.url, false);
    }
}

impl MDKProcessor {
    pub fn from_file(url: &str, decoder_options: Option<Dictionary>) -> Self {
        gyroflow_core::filesystem::start_accessing_url(url, false);

        let mut mdk = qml_video_rs::video_item::MDKVideoItem::default();
        let mut custom_decoder = String::new(); // eg. BRAW:format=rgba64le
        let mut format = ffmpeg_next::format::Pixel::RGBA;
        let filename = gyroflow_core::filesystem::get_filename(url);

        let mut options: String = decoder_options.map(|x| x.into_iter().map(|x| format!("{}={}", x.0, x.1)).join(":")).unwrap_or_default();
        if !options.is_empty() { options.insert(0, ':'); }

        if filename.to_ascii_lowercase().ends_with("braw") {
            let gpu = if *super::GPU_DECODING.read() { "auto" } else { "no" }; // Disable GPU decoding for BRAW
            custom_decoder = format!("BRAW:gpu={}{}", gpu, options);
        }
        if filename.to_ascii_lowercase().ends_with("r3d") {
            format = ffmpeg_next::format::Pixel::BGRA;
            custom_decoder = format!("R3D:gpu=auto{}", options);
        }
        ::log::info!("Custom decoder: {custom_decoder}");

        mdk.setUrl(QUrl::from(QString::from(url)), QString::from(custom_decoder.clone()));
        Self {
            mdk,
            url: url.to_owned(),
            format,
            custom_decoder,
            org_width: Arc::new(AtomicI32::new(-1)),
            org_height: Arc::new(AtomicI32::new(-1)),
            on_frame_callback: None
        }
    }
    pub fn on_frame<F>(&mut self, cb: F) where F: FnMut(i64, &mut frame::Video, Option<&mut frame::Video>, &mut Converter, &mut RateControl) -> Result<(), FFmpegError> + 'static {
        self.on_frame_callback = Some(Box::new(cb));
    }

    pub fn get_org_dimensions(&self) -> Option<(Arc<AtomicI32>, Arc<AtomicI32>)> {
        Some((self.org_width.clone(), self.org_height.clone()))
    }

    pub fn start_decoder_only(&mut self, ranges: Vec<(f64, f64)>, cancel_flag: Arc<AtomicBool>) -> Result<(), FFmpegError> {
        let ranges_ms = ranges.into_iter().map(|(from, to)| (from as usize, to as usize)).collect();
        let mut cb = self.on_frame_callback.take();

        let (tx, rx) = futures_intrusive::channel::shared::oneshot_channel();

        let mut converter = Converter::default();
        let mut ffmpeg_frame = None;
        let format = self.format;
        let self_org_width = self.org_width.clone();
        let self_org_height = self.org_height.clone();
        self.mdk.startProcessing(0, 0, 0, false, &self.custom_decoder, ranges_ms, move |frame_num, timestamp_ms, width, height, org_width, org_height, _fps, _duration_ms, _frame_count, data| {
            if frame_num == -1 || data.is_empty() {
                let _ = tx.send(());
                return true;
            }
            if org_width  > 0 { self_org_width.store(org_width as i32, SeqCst); }
            if org_height > 0 { self_org_height.store(org_height as i32, SeqCst); }

            if let Some(ref mut cb) = cb {
                let timestamp_us = (timestamp_ms * 1000.0).round() as i64;
                if ffmpeg_frame.is_none() {
                    let mut frame = ffmpeg_next::frame::Video::empty();
                    frame.set_format(format);
                    frame.set_width(width);
                    frame.set_height(height);
                    ffmpeg_frame = Some(frame);
                }
                let ffmpeg_frame = ffmpeg_frame.as_mut().unwrap();

                unsafe {
                    (*ffmpeg_frame.as_mut_ptr()).buf[0] = ffi::av_buffer_create(data.as_mut_ptr(), data.len(), Some(noop), std::ptr::null_mut(), 0);
                    (*ffmpeg_frame.as_mut_ptr()).data[0] = data.as_mut_ptr();
                    (*ffmpeg_frame.as_mut_ptr()).linesize[0] = data.len() as i32 / height as i32;
                }
                if let Err(e) = cb(timestamp_us, ffmpeg_frame, None, &mut converter, &mut RateControl::default()) {
                    ::log::error!("mdk_processor error: {:?}", e);
                    return false;
                }
            }
            !cancel_flag.load(Relaxed)
        });

        pollster::block_on(rx.receive());

        std::thread::sleep(std::time::Duration::from_millis(100));

        Ok(())
    }
}

unsafe extern "C" fn noop(_opaque: *mut std::os::raw::c_void, _data: *mut u8) { }