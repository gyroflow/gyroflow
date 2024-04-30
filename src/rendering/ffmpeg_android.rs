// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use ndk::media::image_reader::*;
use ndk::hardware_buffer::*;
use ndk::native_window::*;
use jni::{ objects::{ GlobalRef, JObject }, JavaVM };
use std::ffi::*;
use ffmpeg_next::ffi;
use std::sync::mpsc::{ Receiver, channel };

extern "C" {
    fn av_mediacodec_release_buffer(buffer: *mut c_void, render: c_int) -> c_int;
    fn av_jni_get_java_vm(log_ctx: *mut c_void) -> *mut c_void;
}

// Keep in sync with https://github.com/FFmpeg/FFmpeg/blob/master/libavutil/hwcontext_mediacodec.h
#[repr(C)]
struct AVMediaCodecDeviceContext {
    surface: *mut c_void,
    native_window: *mut c_void,
    create_window: c_int
}

struct AndroidHWBuffer(HardwareBuffer);
unsafe impl Send for AndroidHWBuffer {}

pub struct AndroidHWHandles {
    image_reader: ImageReader,
    window: NativeWindow,
    surface: GlobalRef,
    pub receiver: Option<Receiver<AndroidHWBuffer>>
}

impl AndroidHWHandles {
    pub fn init_with_context(decoder_ctx: &mut ffmpeg_next::codec::context::Context) -> Result<Self, &'static str> { // TODO error type
        let mut image_reader = ImageReader::new_with_usage(1, 1, ImageFormat::YUV_420_888, HardwareBufferUsage::GPU_SAMPLED_IMAGE, 10).unwrap(); // TODO: unwrap

        let (sender, receiver) = channel();

        image_reader.set_image_listener(Box::new(move |image_reader| {
            match &mut image_reader.acquire_next_image() {
                Ok(AcquireResult::Image(image)) => {
                    let timestamp = std::time::Duration::from_nanos(image.timestamp().unwrap() as u64);

                    // TODO import AHardwareBuffer to wgpu Vulkan
                    // The problem with the hardware buffer is that it can be in arbitrary vendor-specific pixel format
                    // One way to tackle that is to use vk::SamplerYcbcrConversion from Vulkan and draw over Vulkan RGB8 texture, but it's a lot of work ( https://github.com/korejan/ALVR/blob/master/alvr/experiments/client/src/video_decoder/mediacodec.rs )
                    // Another way is to draw RGBA in an OpenGL context using Surface https://docs.rs/ndk/latest/ndk/surface_texture/struct.SurfaceTexture.html and update_tex_image
                    // We may need to enable Gl backend in wgpu for that
                    let hw_buffer = image.hardware_buffer().unwrap();
                    ::log::debug!("AHardwareBuffer: {:?} ({}x{} {:?} {:?})", hw_buffer.as_ptr(), image.width().unwrap(), image.height().unwrap(), image.format().unwrap(), timestamp);

                    sender.send(AndroidHWBuffer(hw_buffer)).unwrap();
                }
                Ok(AcquireResult::NoBufferAvailable) => { ::log::warn!("acquire_next_image: NoBufferAvailable"); }
                Ok(AcquireResult::MaxImagesAcquired) => { ::log::warn!("acquire_next_image: MaxImagesAcquired"); }
                Err(e) => { ::log::error!("acquire_next_image error {e:?}"); }
            }
        })).unwrap();
        image_reader.set_buffer_removed_listener(Box::new(|_, _| {
            log::debug!("buffer removed");
        })).unwrap();

        let window;
        let surface;
        unsafe {
            let vm = JavaVM::from_raw(av_jni_get_java_vm(std::ptr::null_mut()) as *mut _).unwrap(); // TODO: unwrap
            let env = vm.attach_current_thread().unwrap(); // TODO: unwrap

            let hw_device_ctx = (*decoder_ctx.as_mut_ptr()).hw_device_ctx;
            {
                if hw_device_ctx.is_null() { return Err("hw_device_ctx is null"); }
            }

            let av_hw_device_ctx = (*hw_device_ctx).data as *mut ffi::AVHWDeviceContext;
            {
                if av_hw_device_ctx.is_null() { return Err("av_hw_device_ctx is null"); }
                if (*av_hw_device_ctx).type_ != ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_MEDIACODEC { return Err("av_hw_device_ctx->type is not MediaCodec"); }
            }

            let media_codec_device_ctx = (*av_hw_device_ctx).hwctx as *mut AVMediaCodecDeviceContext;
            {
                if media_codec_device_ctx.is_null() { return Err("media_codec_device_ctx is null"); }
            }

            window = image_reader.window().unwrap(); // TODO: unwrap
            surface = env.new_global_ref(JObject::from_raw(window.to_surface(env.get_raw()))).unwrap(); // TODO: unwrap

            (*media_codec_device_ctx).surface = surface.as_raw() as *mut _;
        }

        Ok(Self {
            image_reader,
            window,
            surface,
            receiver: Some(receiver)
        })
    }
}

pub fn release_frame(frame: &mut ffmpeg_next::frame::Video) {
    if frame.format() == ffmpeg_next::format::pixel::Pixel::MEDIACODEC {
        unsafe {
            let ret = av_mediacodec_release_buffer((*frame.as_mut_ptr()).data[3] as *mut _, 1);
            if ret != 0 {
                log::error!("Failed to release MediaCodec buffer: {ret}");
            }
        }
    }
}


// https://registry.khronos.org/vulkan/specs/1.3-extensions/man/html/VK_ANDROID_external_memory_android_hardware_buffer.html
// https://github.com/yohhoy/heifreader/issues/1#issuecomment-669852293
// https://github.dev/FFmpeg/FFmpeg/blob/13deb775cfccb10bf03789ca9d05e2a3f6131126/libavcodec/mediacodecdec.c#L394
// https://github.dev/qt/qtmultimedia/blob/293d91da3a6979dc64027cc552d0491458daf325/src/plugins/multimedia/ffmpeg/qffmpeghwaccel_mediacodec.cpp#L81
// https://github.com/alvr-org/ALVR/blob/master/alvr/client_core/src/platform/android.rs
// https://github.com/korejan/ALVR/blob/9a18783e358244f90683cf801d29d8601022f504/alvr/experiments/client/src/video_decoder/mediacodec.rs#L610
// https://github.com/0xedward/chromium/blob/master/gpu/command_buffer/service/image_reader_gl_owner.cc
// https://docs.rs/ndk/latest/ndk/hardware_buffer/struct.HardwareBuffer.html
// https://github.com/tmm1/mpv-player/blob/master/video/out/hwdec/hwdec_aimagereader.c
// https://github.com/simul/Teleport/blob/main/AndroidClient/NdkVideoDecoder.cpp#L755
// https://docs.teleportvr.io/reference/video_vulkan_android_import_and_ycbcr.html#video-vulkan-android-import-and-ycbcr

/*
fn vk_format_from_android(android_format: u32) -> vk::Format {
    match android_format {
        AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM => vk::Format::R8G8B8A8_UNORM,
        AHARDWAREBUFFER_FORMAT_R8G8B8X8_UNORM => vk::Format::R8G8B8A8_UNORM,
        AHARDWAREBUFFER_FORMAT_R8G8B8_UNORM => vk::Format::R8G8B8_UNORM,
        AHARDWAREBUFFER_FORMAT_R5G6B5_UNORM => vk::Format::R5G6B5_UNORM_PACK16,
        AHARDWAREBUFFER_FORMAT_R16G16B16A16_FLOAT => vk::Format::R16G16B16A16_SFLOAT,
        AHARDWAREBUFFER_FORMAT_R10G10B10A2_UNORM => vk::Format::A2B10G10R10_UNORM_PACK32,
        HAL_PIXEL_FORMAT_NV12_Y_TILED_INTEL | AHARDWAREBUFFER_FORMAT_Y8Cb8Cr8_420 => {
            vk::Format::G8_B8R8_2PLANE_420_UNORM
        }
        AHARDWAREBUFFER_FORMAT_YCbCr_P010 => {
            vk::Format::G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16
        }
        HAL_PIXEL_FORMAT_YV12 | OMX_COLOR_FormatYUV420Planar => {
            vk::Format::G8_B8_R8_3PLANE_420_UNORM
        }
        AHARDWAREBUFFER_FORMAT_BLOB | _ => vk::Format::UNDEFINED,
    }
}
fn android_format_from_vk(vk_format: vk::Format) -> u32 {
    match vk_format {
        vk::Format::R8G8B8A8_UNORM => AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM,
        vk::Format::R8G8B8_UNORM => AHARDWAREBUFFER_FORMAT_R8G8B8_UNORM,
        vk::Format::R5G6B5_UNORM_PACK16 => AHARDWAREBUFFER_FORMAT_R5G6B5_UNORM,
        vk::Format::R16G16B16A16_SFLOAT => AHARDWAREBUFFER_FORMAT_R16G16B16A16_FLOAT,
        vk::Format::A2B10G10R10_UNORM_PACK32 => AHARDWAREBUFFER_FORMAT_R10G10B10A2_UNORM,
        vk::Format::G8_B8R8_2PLANE_420_UNORM => HAL_PIXEL_FORMAT_NV12_Y_TILED_INTEL,
        vk::Format::G8_B8_R8_3PLANE_420_UNORM => HAL_PIXEL_FORMAT_YV12,
        _ => AHARDWAREBUFFER_FORMAT_BLOB,
    }
}
*/