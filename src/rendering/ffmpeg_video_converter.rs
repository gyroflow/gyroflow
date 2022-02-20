// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use ffmpeg_next::{ ffi, format, frame, software };
use crate::rendering::FFmpegError;

#[derive(Default)]
pub struct Converter {
    pub convert_to: Option<software::scaling::Context>,
    pub convert_from: Option<software::scaling::Context>,
    pub sw_frame_converted: Option<frame::Video>,
    pub sw_frame_converted_out: Option<frame::Video>,
}
impl<'a> Converter {
    pub fn convert_pixel_format<F>(&mut self, frame: &mut frame::Video, out_frame: &mut frame::Video, format: format::Pixel, mut cb: F) -> Result<(), FFmpegError> where F: FnMut(&mut frame::Video, &mut frame::Video) + 'a {
        if frame.format() != format {
            if self.sw_frame_converted.is_none() {
                self.sw_frame_converted = Some(frame::Video::new(format, frame.width(), frame.height()));
                //self.convert_from = Some(software::converter((frame.width(), frame.height()), frame.format(), format)?);
                self.convert_from = Some(software::scaling::Context::get(
                    frame.format(), // input
                    frame.width(),
                    frame.height(),
                    format, // output
                    frame.width(),
                    frame.height(),
                    software::scaling::flag::Flags::LANCZOS,
                )?);
            }

            if self.sw_frame_converted_out.is_none() {
                self.sw_frame_converted_out = Some(frame::Video::new(format, out_frame.width(), out_frame.height()));
                //self.convert_to = Some(software::converter((out_frame.width(), out_frame.height()), format, out_frame.format())?);
                self.convert_to = Some(software::scaling::Context::get(
                    format, // input
                    out_frame.width(),
                    out_frame.height(),
                    out_frame.format(), // output
                    out_frame.width(),
                    out_frame.height(),
                    software::scaling::flag::Flags::LANCZOS,
                )?);
            }

            let sw_frame_converted = self.sw_frame_converted.as_mut().ok_or(FFmpegError::FrameEmpty)?;
            let sw_frame_converted_out = self.sw_frame_converted_out.as_mut().ok_or(FFmpegError::FrameEmpty)?;
            let convert_from = self.convert_from.as_mut().ok_or(FFmpegError::ConverterEmpty)?;
            let convert_to = self.convert_to.as_mut().ok_or(FFmpegError::ConverterEmpty)?;

            convert_from.run(frame, sw_frame_converted)?;

            cb(sw_frame_converted, sw_frame_converted_out);
            
            convert_to.run(sw_frame_converted_out, out_frame)?;
        } else {
            cb(frame, out_frame);
        }
        Ok(())
    }

    // Scale is only used for autosync
    pub fn scale(&mut self, frame: &mut frame::Video, format: format::Pixel, width: u32, height: u32) -> Result<frame::Video, FFmpegError> {
        if frame.width() != width || frame.height() != height || frame.format() != format {
            if self.sw_frame_converted.is_none() {
                self.sw_frame_converted = Some(frame::Video::new(format, width, height));
                self.convert_to = Some(
                    software::scaling::Context::get(
                        frame.format(), frame.width(), frame.height(), format, width, height, software::scaling::Flags::BILINEAR,
                    )?
                );
            }

            let sw_frame_converted = self.sw_frame_converted.as_mut().ok_or(FFmpegError::FrameEmpty)?;
            let convert_to = self.convert_to.as_mut().ok_or(FFmpegError::ConverterEmpty)?;

            convert_to.run(frame, sw_frame_converted)?;

            Ok(unsafe { frame::Video::wrap(ffi::av_frame_clone(sw_frame_converted.as_ptr())) })
        } else {
            Ok(unsafe { frame::Video::wrap(ffi::av_frame_clone(frame.as_ptr())) })
        }
    }
}
