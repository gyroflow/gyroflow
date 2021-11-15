use ffmpeg_next::{ ffi, codec, decoder, encoder, format, frame, picture, software, Dictionary, Packet, Rational, Error, Frame, rescale::Rescale };

use super::ffmpeg_processor::Status;

#[derive(Default)]
pub struct Converter {
    pub convert_to: Option<software::scaling::Context>,
    pub convert_from: Option<software::scaling::Context>,
    pub sw_frame_converted: Option<frame::Video>,
}
impl<'a> Converter {
    pub fn convert_pixel_format<F>(&mut self, frame: &mut frame::Video, format: format::Pixel, mut cb: F) where F: FnMut(&mut frame::Video) + 'a {
        if frame.format() != format {

            if self.sw_frame_converted.is_none() {
                self.sw_frame_converted = Some(frame::Video::new(format, frame.width(), frame.height()));
                self.convert_to = Some(software::converter((frame.width(), frame.height()), format, frame.format()).unwrap());
                self.convert_from = Some(software::converter((frame.width(), frame.height()), frame.format(), format).unwrap());
            }

            let sw_frame_converted = self.sw_frame_converted.as_mut().unwrap();
            let convert_from = self.convert_from.as_mut().unwrap();
            let convert_to = self.convert_to.as_mut().unwrap();

            convert_from.run(frame, sw_frame_converted).unwrap();

            cb(sw_frame_converted);

            convert_to.run(sw_frame_converted, frame).unwrap();
        } else {
            cb(frame);
        }
    }
    pub fn scale(&mut self, frame: &mut frame::Video, format: format::Pixel, width: u32, height: u32) -> frame::Video {
        if frame.width() != width || frame.height() != height || frame.format() != format {
            if self.sw_frame_converted.is_none() {
                self.sw_frame_converted = Some(frame::Video::new(format, width, height));
                self.convert_to = Some(
                    software::scaling::Context::get(
                        frame.format(), frame.width(), frame.height(), format, width, height, software::scaling::Flags::BILINEAR,
                    ).unwrap()
                );
            }

            let sw_frame_converted = self.sw_frame_converted.as_mut().unwrap();
            let convert_to = self.convert_to.as_mut().unwrap();

            convert_to.run(frame, sw_frame_converted).unwrap();

            unsafe { frame::Video::wrap(ffi::av_frame_clone(sw_frame_converted.as_ptr())) }
        } else {
            unsafe { frame::Video::wrap(ffi::av_frame_clone(frame.as_ptr())) }
        }
    }
}

pub struct FrameBuffers {
    pub sw_frame: frame::Video,
}
impl Default for FrameBuffers {
    fn default() -> Self { Self {
        sw_frame: frame::Video::empty(),
    } }
}

#[derive(Default)]
pub struct VideoTranscoder<'a> {
    pub input_index: usize,
    pub output_index: usize,
    pub decoder: Option<decoder::Video>,
    pub encoder: Option<encoder::video::Video>,

    pub codec_options: Dictionary<'a>,

    pub gpu_pixel_format: Option<ffi::AVPixelFormat>,

    pub decode_only: bool,

    pub converter: Converter,

    pub buffers: FrameBuffers,

    pub on_frame_callback: Option<Box<dyn FnMut(i64, &mut frame::Video, &mut Converter) + 'a>>,

    pub first_frame_ts: Option<i64>
}

impl<'a> VideoTranscoder<'a> {
    fn init_encoder(frame: &mut Frame, decoder: &mut decoder::Video, octx: &mut format::context::Output, hw_format: Option<ffi::AVPixelFormat>, codec_options: Dictionary) -> Result<encoder::video::Video, Error> {
        let global_header = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);
        //let mut ost = octx.add_stream(encoder::find_by_name(&video_params.codec))?;
        let mut ost = octx.stream_mut(0).unwrap();//octx.add_stream(encoder::find_by_name("hevc_nvenc"))?;
        let mut encoder = ost.codec().encoder().video()?;
        encoder.set_height(decoder.height());
        encoder.set_width(decoder.width());
        encoder.set_aspect_ratio(decoder.aspect_ratio());
        encoder.set_format(decoder.format());
        encoder.set_frame_rate(decoder.frame_rate());
        encoder.set_time_base(decoder.frame_rate().unwrap().invert());
        encoder.set_bit_rate(decoder.bit_rate());
        encoder.set_color_range(decoder.color_range());
        encoder.set_colorspace(decoder.color_space());
        unsafe {
            (*encoder.as_mut_ptr()).color_trc = (*decoder.as_ptr()).color_trc;
            (*encoder.as_mut_ptr()).color_primaries = (*decoder.as_ptr()).color_primaries;
        }

        if global_header {
            encoder.set_flags(codec::Flags::GLOBAL_HEADER);
        }

        unsafe {
            if !(*decoder.as_mut_ptr()).hw_device_ctx.is_null() && hw_format.is_some() {
                let hw_ctx = (*decoder.as_mut_ptr()).hw_device_ctx;
                
                let mut hw_frames_ref = ffi::av_hwframe_ctx_alloc(hw_ctx);
                if hw_frames_ref.is_null() {
                    eprintln!("Failed to create GPU frame context.");
                    return Err(Error::Unknown);
                }

                let mut formats = std::ptr::null_mut();
                if !(*frame.as_mut_ptr()).hw_frames_ctx.is_null() {
                    ffi::av_hwframe_transfer_get_formats((*frame.as_mut_ptr()).hw_frames_ctx, ffi::AVHWFrameTransferDirection::AV_HWFRAME_TRANSFER_DIRECTION_FROM, &mut formats, 0);
                }
                let sw_format = if formats.is_null() {
                    eprintln!("No frame transfer formats.");
                    ffi::AVPixelFormat::AV_PIX_FMT_NONE
                    //return Err(Error::Unknown);
                } else {
                    *formats // Just get the first one
                };
                // for i in 0..100 {
                //     let mut p = *formats.offset(i);
                //     dbg!(p);
                //     if p == ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                //         break;
                //     }
                // }
                if sw_format != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                    let mut frames_ctx = (*hw_frames_ref).data as *mut ffi::AVHWFramesContext;
                    (*frames_ctx).format    = hw_format.unwrap(); // Safe because we check is_some() above
                    (*frames_ctx).sw_format = sw_format;
                    (*frames_ctx).width     = decoder.width() as i32;
                    (*frames_ctx).height    = decoder.height() as i32;
                    (*frames_ctx).initial_pool_size = 20;
                    
                    let err = ffi::av_hwframe_ctx_init(hw_frames_ref);
                    if err < 0 {
                        eprintln!("Failed to initialize frame context. Error code: {}", err);
                        ffi::av_buffer_unref(&mut hw_frames_ref);
                        return Err(Error::from(err));
                    }
                    (*encoder.as_mut_ptr()).hw_frames_ctx = ffi::av_buffer_ref(hw_frames_ref);
                    (*encoder.as_mut_ptr()).pix_fmt = hw_format.unwrap(); // Safe because we check is_some() above
                
                    ffi::av_buffer_unref(&mut hw_frames_ref);
                }
            }
        }
        encoder.open_with(codec_options)?;
        encoder = ost.codec().encoder().video()?;
        ost.set_parameters(encoder);
        
        ost.codec().encoder().video()
    }
    
    pub fn receive_and_process_video_frames(&mut self, mut octx: Option<&mut format::context::Output>, ost_time_bases: &mut Vec<Rational>, end_ms: Option<usize>) -> Result<Status, Error> {
        let mut status = Status::Continue;
        
        let mut decoder = self.decoder.as_mut().unwrap();
        
        let mut frame = frame::Video::empty();
        let mut sw_frame = &mut self.buffers.sw_frame;
        let mut hw_frame = frame::Video::empty();
        
        while decoder.receive_frame(&mut frame).is_ok() {

            if !self.decode_only && self.encoder.is_none() {
                let octx = octx.as_deref_mut().unwrap();
                self.encoder = Some(Self::init_encoder(&mut frame, &mut decoder, octx, self.gpu_pixel_format, self.codec_options.to_owned())?);   

                octx.write_header()?;
                //format::context::output::dump(&octx, 0, Some(&output_path));
        
                for (ost_index, _) in octx.streams().enumerate() {
                    ost_time_bases[ost_index] = octx.stream(ost_index as _).ok_or(Error::StreamNotFound)?.time_base();
                }
            }

            if self.first_frame_ts.is_none() {
                self.first_frame_ts = frame.timestamp();
            }

            if let Some(mut ts) = frame.timestamp() {
                let timestamp_us = ts.rescale(decoder.time_base(), (1, 1000000));
                ts -= self.first_frame_ts.unwrap();

                if ts >= 0 {
                    if end_ms.is_some() && timestamp_us / 1000 > end_ms.unwrap() as i64 {
                        status = Status::Finish;
                        break;
                    }

                    let timestamp = Some(ts);

                    // TODO: add more hardware formats
                    if frame.format() == format::Pixel::CUDA || 
                       frame.format() == format::Pixel::DXVA2_VLD || 
                       //frame.format() == format::Pixel::VAAPI || 
                       frame.format() == format::Pixel::D3D11VA_VLD || 
                       frame.format() == format::Pixel::VIDEOTOOLBOX || 
                       frame.format() == format::Pixel::MEDIACODEC || 
                       frame.format() == format::Pixel::QSV || 
                       frame.format() == format::Pixel::MMAL || 
                       frame.format() == format::Pixel::D3D11 {
                        unsafe {
                            // retrieve data from GPU to CPU
                            let err = ffi::av_hwframe_transfer_data(sw_frame.as_mut_ptr(), frame.as_mut_ptr(), 0);
                            if err < 0 {
                                eprintln!("Error transferring the data to system memory");
                                break; // TODO: return Err?
                            }
                            sw_frame.set_pts(frame.timestamp());

                            // Process frame
                            if let Some(ref mut cb) = self.on_frame_callback {
                                cb(timestamp_us, &mut sw_frame, &mut self.converter);
                            }

                            if !self.decode_only {
                                // TODO if encoder is GPU
                                let encoder = self.encoder.as_mut().unwrap();
                                // Upload back to GPU
                                let err = ffi::av_hwframe_get_buffer((*encoder.as_mut_ptr()).hw_frames_ctx, hw_frame.as_mut_ptr(), 0);
                                if err < 0 {
                                    eprintln!("Error code: {}.", err);
                                    break;
                                }
                                if (*hw_frame.as_mut_ptr()).hw_frames_ctx.is_null() {
                                    eprintln!("empty frame context");
                                    break;
                                }
                                let err = ffi::av_hwframe_transfer_data(hw_frame.as_mut_ptr(), sw_frame.as_mut_ptr(), 0);
                                if err < 0 {
                                    eprintln!("Error transferring the data to system memory");
                                    break;
                                }
                                hw_frame.set_pts(timestamp);
                                hw_frame.set_kind(picture::Type::None);
                                hw_frame.set_color_primaries(frame.color_primaries());
                                hw_frame.set_color_range(frame.color_range());
                                hw_frame.set_color_space(frame.color_space());
                                hw_frame.set_color_transfer_characteristic(frame.color_transfer_characteristic());
                                encoder.send_frame(&hw_frame).unwrap();
                            }
                        }
                    } else {
                        dbg!(frame.format());

                        let mut sw_frame = frame.clone(); // TODO this can probably be done without cloning, but using frame directly was causing weird artifacts. Maybe need to reset some properties?
                        sw_frame.set_pts(frame.timestamp());

                        if let Some(ref mut cb) = self.on_frame_callback {
                            cb(timestamp_us, &mut sw_frame, &mut self.converter);
                        }

                        if !self.decode_only {
                            let encoder = self.encoder.as_mut().unwrap();
                            sw_frame.set_pts(timestamp);
                            sw_frame.set_kind(picture::Type::None);
                            sw_frame.set_color_primaries(frame.color_primaries());
                            sw_frame.set_color_range(frame.color_range());
                            sw_frame.set_color_space(frame.color_space());
                            sw_frame.set_color_transfer_characteristic(frame.color_transfer_characteristic());
                            encoder.send_frame(&sw_frame).unwrap();
                        }
                    }
                }
            }
        }

        if !self.decode_only && self.encoder.is_some() {
            let ost_time_base = ost_time_bases[self.output_index];
            let octx = octx.as_deref_mut().unwrap();
            self.receive_and_process_encoded_packets(octx, ost_time_base);
        }

        Ok(status)
    }

    pub fn receive_and_process_encoded_packets(&mut self, octx: &mut format::context::Output, ost_time_base: Rational) {
        if !self.decode_only {
            let time_base = self.decoder.as_ref().unwrap().time_base();
            let mut encoded = Packet::empty();
            while self.encoder.as_mut().unwrap().receive_packet(&mut encoded).is_ok() {
                encoded.set_stream(self.output_index);
                encoded.rescale_ts(time_base, ost_time_base);
                encoded.write_interleaved(octx).unwrap();
            }
        }
    }
}
