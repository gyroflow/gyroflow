mod ffmpeg_audio;
mod ffmpeg_video;
pub mod ffmpeg_processor;

pub use self::ffmpeg_processor::FfmpegProcessor;
use crate::core::{StabilizationManager, undistortion::*};
use ffmpeg_next::format::Pixel;
use ffmpeg_next::frame::Video;
use ffmpeg_next::codec;
use std::sync::{Arc, atomic::AtomicBool};

pub fn match_gpu_encoder(codec: &str, use_gpu: bool, selected_backend: &str) -> &'static str {
    if use_gpu {
        match codec {
            "x264" => match selected_backend {
                "cuda" => "h264_nvenc",
                "qsv"  => "h264_qsv",
                "amf"  => "h264_amf",
                _      => "libx264"
            },
            "x265" => match selected_backend {
                "cuda" => "hevc_nvenc",
                "qsv"  => "hevc_qsv",
                "amf"  => "hevc_amf",
                _      => "libx265"
            },
            "ProRes" => "prores", // TODO
            _        => ""
        }
    } else {
        match codec {
            "x264"   => "libx264",
            "x265"   => "libx265",
            "ProRes" => "prores", // TODO
            _        => ""
        }
    }
}

pub fn render<F>(stab: StabilizationManager, progress: F, video_path: String, codec: String, output_path: String, trim_start: f64, trim_end: f64, output_width: usize, output_height: usize, use_gpu: bool, audio: bool, cancel_flag: Arc<AtomicBool>)
    where F: Fn((f64, usize, usize)) + Send + Sync + Clone
{
    dbg!(FfmpegProcessor::supported_gpu_backends());

    // decoders: h264 h264_qsv h264_cuvid / encoders: libx264 h264_amf h264_nvenc h264_qsv
    // decoders: hevc hevc_qsv hevc_cuvid / encoders: libx265 hevc_amf hevc_nvenc hevc_qsv
    
    let params = stab.params.read();
    let trim_ratio = trim_end - trim_start;
    let total_frame_count = params.frame_count;

    let duration_ms = params.duration_ms;

    let render_duration = params.duration_ms * trim_ratio;
    let render_frame_count = (total_frame_count as f64 * trim_ratio).round() as usize;

    drop(params);

    let mut proc = FfmpegProcessor::from_file(&video_path, use_gpu).unwrap();

    dbg!(&proc.gpu_device);
    proc.video_codec = Some(match_gpu_encoder(&codec, use_gpu, proc.gpu_device.as_ref().unwrap()).to_owned());
    proc.gpu_encoding = use_gpu;
    dbg!(&proc.video_codec);

    if trim_start > 0.0 { proc.start_ms = Some((trim_start * duration_ms) as usize); }
    if trim_end   < 1.0 { proc.end_ms   = Some((trim_end   * duration_ms) as usize); }

    //proc.video.codec_options.set("preset", "medium");

    let start_ms = proc.start_ms.unwrap_or_default();

    if !audio {
        proc.audio_codec = codec::Id::None;
    }

    let mut planes = Vec::<Box<dyn FnMut(usize, &mut [u8], usize, usize, usize)>>::new();

    let progress2 = progress.clone();
    proc.on_frame(move |timestamp_us, input_frame, converter| {
        let absolute_frame_id = ((timestamp_us as f64 / 1000.0 / duration_ms) * total_frame_count as f64).round() as usize;
        let process_frame = ((((timestamp_us as f64 / 1000.0) - start_ms as f64) / render_duration) * render_frame_count as f64).round() as usize + 1;

        macro_rules! create_planes_proc {
            ($planes:ident, $(($t:tt, $w:expr, $h:expr, $s:expr, $yuvi:expr), )*) => {
                $({
                    let bg = {
                        let mut params = stab.params.write();
                        params.size = ($w as usize, $h as usize);
                        params.background
                    };
                    let mut plane = Undistortion::<$t>::default();
                    plane.init_size(<$t as FloatPixel>::from_rgb_color(bg, &$yuvi), ($w as usize, $h as usize), ($s / $t::COUNT) as usize);
                    plane.recompute(&ComputeParams::from_manager(&stab));
                    $planes.push(Box::new(move |frame_id: usize, buffer: &mut [u8], w: usize, h: usize, mut s: usize| {
                        s /= $t::COUNT;
                        let processed = plane.process_pixels(frame_id, w, h, s, bytemuck::cast_slice_mut(buffer));
                        if buffer.as_ptr() as *const u8 != processed as *const u8 { 
                            buffer.copy_from_slice(unsafe { std::slice::from_raw_parts(processed as *mut u8, s*h*std::mem::size_of::<<$t as FloatPixel>::Scalar>()*$t::COUNT) });
                        }
                    }));
                })*
            };
        }

        if planes.is_empty() {
            // Good reference about video formats: https://source.chromium.org/chromium/chromium/src/+/master:media/base/video_frame.cc
            // https://gist.github.com/Jim-Bar/3cbba684a71d1a9d468a6711a6eddbeb
            match input_frame.format() {
                Pixel::NV12 => {
                    create_planes_proc!(planes, 
                        (Luma8, input_frame.plane_width(0), input_frame.plane_height(0), input_frame.stride(0), [0]),
                        (UV8,   input_frame.plane_width(1), input_frame.plane_height(1), input_frame.stride(1), [1,2]),
                    );
                },
                Pixel::NV21 => {
                    create_planes_proc!(planes, 
                        (Luma8, input_frame.plane_width(0), input_frame.plane_height(0), input_frame.stride(0), [0]),
                        (UV8,   input_frame.plane_width(1), input_frame.plane_height(1), input_frame.stride(1), [2,1]),
                    );
                },
                Pixel::P010LE | Pixel::P016LE => {
                    create_planes_proc!(planes, 
                        (Luma16, input_frame.plane_width(0), input_frame.plane_height(0), input_frame.stride(0) / 2/*bytes*/, [0]),
                        (UV16,   input_frame.plane_width(1), input_frame.plane_height(1), input_frame.stride(1) / 2/*bytes*/, [1,2]),
                    );
                },
                Pixel::YUV420P | Pixel::YUVJ420P => {
                    create_planes_proc!(planes, 
                        (Luma8, input_frame.plane_width(0), input_frame.plane_height(0), input_frame.stride(0), [0]),
                        (Luma8, input_frame.plane_width(1), input_frame.plane_height(1), input_frame.stride(1), [1]),
                        (Luma8, input_frame.plane_width(2), input_frame.plane_height(2), input_frame.stride(2), [2]),
                    );
                },
                Pixel::YUV420P10LE | Pixel::YUV420P16LE => {
                    create_planes_proc!(planes, 
                        (Luma16, input_frame.plane_width(0), input_frame.plane_height(0), input_frame.stride(0) / 2/*bytes*/, [0]),
                        (Luma16, input_frame.plane_width(1), input_frame.plane_height(1), input_frame.stride(1) / 2/*bytes*/, [1]),
                        (Luma16, input_frame.plane_width(2), input_frame.plane_height(2), input_frame.stride(2) / 2/*bytes*/, [2]),
                    );
                },
                format => { // All other convert to YUV444P16LE
                    println!("Unknown format {:?}, converting to YUV444P16LE", format);
                    // Go through 4:4:4 because of even plane dimensions
                    converter.convert_pixel_format(input_frame, Pixel::YUV444P16LE, |converted_frame| {
                        create_planes_proc!(planes, 
                            (Luma16, converted_frame.plane_width(0), converted_frame.plane_height(0), converted_frame.stride(0) / 2/*bytes*/, [0]), 
                            (Luma16, converted_frame.plane_width(1), converted_frame.plane_height(1), converted_frame.stride(1) / 2/*bytes*/, [1]), 
                            (Luma16, converted_frame.plane_width(2), converted_frame.plane_height(2), converted_frame.stride(2) / 2/*bytes*/, [2]), 
                        );
                    });
                }
            }
        }
        if planes.is_empty() {
            panic!("Unknown pixel format {:?}", input_frame.format());
        }

        let mut undistort_frame = |frame: &mut Video, bytes_per_scalar: usize| {
            for (i, cb) in planes.iter_mut().enumerate() {
                let w = frame.plane_width(i) as usize;
                let h = frame.plane_height(i) as usize;
                let s = frame.stride(i) as usize / bytes_per_scalar;
        
                let data = frame.data_mut(i);
                
                (*cb)(absolute_frame_id, data, w, h, s);
            }
            progress2((process_frame as f64 / render_frame_count as f64, process_frame, render_frame_count));
        };

        match input_frame.format() {
            Pixel::NV12 | Pixel::NV21 | Pixel::YUV420P | Pixel::YUVJ420P => {
                undistort_frame(input_frame, 1)
            },
            Pixel::P010LE | Pixel::P016LE | Pixel::YUV420P10LE | Pixel::YUV420P16LE => {
                undistort_frame(input_frame, 2)
            },
            _ => {
                converter.convert_pixel_format(input_frame, Pixel::YUV444P16LE, |converted_frame| {
                    undistort_frame(converted_frame, 2);
                });
            }
        }
    });

    proc.render(&output_path, cancel_flag).unwrap(); // TODO errors

    progress((1.0, render_frame_count, render_frame_count));
}
/*
pub fn test() {
    dbg!(FfmpegProcessor::supported_gpu_backends());

    let mut stab = StabilizationManager::default();
    let duration_ms = 15015.0;
    let frame_count = 900;
    let fps = 60000.0/1001.0;
    let video_size = (3840, 2160);

    stab.init_from_video_data("E:/clips/GoPro/rs/C0752.MP4", duration_ms, fps, frame_count, video_size);
    stab.gyro.set_offset(0, -26.0);
    stab.gyro.integration_method = 1;
    stab.gyro.integrate();
    stab.load_lens_profile("E:/clips/GoPro/rs/Sony_A7s3_Tamron_28-200_4k60p.json");
    stab.init_size(video_size.0, video_size.1);
    stab.smoothing_id = 1;
    stab.smoothing_algs[1].as_mut().set_parameter("time_constant", 0.4);
    stab.frame_readout_time = 8.9;
    stab.fov = 1.0;
    stab.background = nalgebra::Vector4::new(0.0, 0.0, 0.0, 0.0);
    stab.recompute_blocking();

    render(
        stab, 
        move |params: (f64, usize, usize)| {
            println!("frame {}/{}", params.1, params.2);
        }, 
        "E:/clips/GoPro/rs/C0752.MP4".into(),
        "x265".into(),
        "E:/clips/GoPro/rs/C0752-test.MP4".into(), 
        0.0,
        1.0,
        video_size.0,
        video_size.1,
        true, 
        true,
        Arc::new(AtomicBool::new(false))
    );
}
// use opencv::core::{Mat, Size, CV_8UC1};
// use std::os::raw::c_void;
        
pub fn test_decode() {
    let mut proc = FfmpegProcessor::from_file("E:/clips/GoPro/rs/C0752.MP4", true).unwrap();

    // TODO: gpu scaling in filters, example here https://github.com/zmwangx/rust-ffmpeg/blob/master/examples/transcode-audio.rs, filter scale_cuvid or scale_npp
    proc.on_frame(move |timestamp_us, input_frame, converter| {
        let small_frame = converter.scale(input_frame, Pixel::GRAY8, 1280, 720);
        println!("ts: {} width: {}", timestamp_us, small_frame.plane_width(0));

        /*let (w, h) = (small_frame.plane_width(0) as i32, small_frame.plane_height(0) as i32);
        let mut bytes = small_frame.data_mut(0);
        let inp = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, bytes.as_mut_ptr() as *mut c_void, w as usize) }.unwrap();
        opencv::imgcodecs::imwrite("D:/test.jpg", &inp, &opencv::types::VectorOfi32::new());*/
        
    });
    let _ = proc.start_decoder_only(vec![
        (100, 2000),
        (3000, 5000),
        (11000, 999999)
    ], Arc::new(AtomicBool::new(false)));
}
*/