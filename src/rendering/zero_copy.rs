// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

#![allow(unused_variables)]
#![allow(unused_mut)]

use ffmpeg_next::format::Pixel;
use ffmpeg_next::frame::Video;
use gyroflow_core::gpu::{ BufferDescription, BufferSource };

#[derive(Default)]
pub struct RenderGlobals {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    pub tex_cache: Option<mac_ffi::MetalTextureCache>
}

pub fn map_hardware_format(format: Pixel, frame: &Video) -> Option<Pixel> {
    match format {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        Pixel::VIDEOTOOLBOX => {
            let pix_fmt = unsafe { mac_ffi::CVPixelBufferGetPixelFormatType((*frame.as_ptr()).data[3] as mac_ffi::CVPixelBufferRef) };
            let pix_fmt_bytes = pix_fmt.to_be_bytes();
            match &pix_fmt_bytes {
                b"BGRA" => Some(Pixel::BGRA),    // kCVPixelFormatType_32BGRA                        | 32 bit BGRA
                b"xf20" => Some(Pixel::P010LE),  // kCVPixelFormatType_420YpCbCr10BiPlanarFullRange  | 2 plane YCbCr10 4:2:0, each 10 bits in the MSBs of 16bits, full-range (Y range 0-1023)
                b"x420" => Some(Pixel::P010LE),  // kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange | 2 plane YCbCr10 4:2:0, each 10 bits in the MSBs of 16bits, video-range (luma=[64,940] chroma=[64,960])
                b"420f" => Some(Pixel::NV12),    // kCVPixelFormatType_420YpCbCr8BiPlanarFullRange   | Bi-Planar Component Y'CbCr 8-bit 4:2:0, full-range (luma=[0,255] chroma=[1,255]).  baseAddr points to a big-endian CVPlanarPixelBufferInfo_YCbCrBiPlanar struct
                b"420v" => Some(Pixel::NV12),    // kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange  | Bi-Planar Component Y'CbCr 8-bit 4:2:0, video-range (luma=[16,235] chroma=[16,240]).  baseAddr points to a big-endian CVPlanarPixelBufferInfo_YCbCrBiPlanar struct
                b"y420" => Some(Pixel::YUV420P), // kCVPixelFormatType_420YpCbCr8Planar              | Planar Component Y'CbCr 8-bit 4:2:0.  baseAddr points to a big-endian CVPlanarPixelBufferInfo_YCbCrPlanar struct
                b"f420" => Some(Pixel::YUV420P), // kCVPixelFormatType_420YpCbCr8PlanarFullRange     | Planar Component Y'CbCr 8-bit 4:2:0, full range.  baseAddr points to a big-endian CVPlanarPixelBufferInfo_YCbCrPlanar struct
                b"xf22" => Some(Pixel::P210LE),  // kCVPixelFormatType_422YpCbCr10BiPlanarFullRange  | 2 plane YCbCr10 4:2:2, each 10 bits in the MSBs of 16bits, full-range (Y range 0-1023)
                b"x422" => Some(Pixel::P210LE),  // kCVPixelFormatType_422YpCbCr10BiPlanarVideoRange | 2 plane YCbCr10 4:2:2, each 10 bits in the MSBs of 16bits, video-range (luma=[64,940] chroma=[64,960])
                b"sv22" => Some(Pixel::P216LE),  // kCVPixelFormatType_422YpCbCr16BiPlanarVideoRange |
                b"2vuy" => Some(Pixel::UYVY422), // kCVPixelFormatType_422YpCbCr8                    | Component Y'CbCr 8-bit 4:2:2, ordered Cb Y'0 Cr Y'1
                b"422f" => Some(Pixel::NV16),    // kCVPixelFormatType_422YpCbCr8BiPlanarFullRange   |
                b"422v" => Some(Pixel::NV16),    // kCVPixelFormatType_422YpCbCr8BiPlanarVideoRange  |
                b"y416" => Some(Pixel::AYUV64),  // kCVPixelFormatType_4444AYpCbCr16                 | Component Y'CbCrA 16-bit 4:4:4:4, ordered A Y' Cb Cr, full range alpha, video range Y'CbCr, 16-bit little-endian samples.
                b"xf44" => Some(Pixel::P410LE),  // kCVPixelFormatType_444YpCbCr10BiPlanarFullRange  | 2 plane YCbCr10 4:4:4, each 10 bits in the MSBs of 16bits, full-range (Y range 0-1023)
                b"x444" => Some(Pixel::P410LE),  // kCVPixelFormatType_444YpCbCr10BiPlanarVideoRange | 2 plane YCbCr10 4:4:4, each 10 bits in the MSBs of 16bits, video-range (luma=[64,940] chroma=[64,960])
                b"sv44" => Some(Pixel::P416LE),  // kCVPixelFormatType_444YpCbCr16BiPlanarVideoRange |
                b"444f" => Some(Pixel::NV24),    // kCVPixelFormatType_444YpCbCr8BiPlanarFullRange   |
                b"444v" => Some(Pixel::NV24),    // kCVPixelFormatType_444YpCbCr8BiPlanarVideoRange  |
                _ => {  log::error!("Unknown VT pixel format: {pix_fmt:08x}"); None }
            }
        },
        _ => None
    }
}

pub fn get_plane_size(frame: &Video, plane: usize) -> (usize, usize) {
    match frame.format() {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        Pixel::VIDEOTOOLBOX => {
            let ptr = unsafe { (*frame.as_ptr()).data[3] as mac_ffi::CVPixelBufferRef };
            (
                unsafe { mac_ffi::CVPixelBufferGetWidthOfPlane(ptr, plane as u64) } as usize,
                unsafe { mac_ffi::CVPixelBufferGetHeightOfPlane(ptr, plane as u64) } as usize
            )
        },
        _ => {
            (
                frame.plane_width(plane) as usize,
                frame.plane_height(plane) as usize
            )
        }
    }
}

pub fn get_plane_buffer<'a>(frame: &'a mut Video, size: (usize, usize), plane_index: usize, render_globals: &mut RenderGlobals, wgpu_format: Option<gyroflow_core::WgpuTextureFormat>) -> BufferDescription<'a> {
    match frame.format() {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        Pixel::VIDEOTOOLBOX => {
            let cache = render_globals.tex_cache.get_or_insert_with(|| mac_ffi::MetalTextureCache::new().unwrap()); // TODO: unwrap

            let f = gyroflow_core::gpu::wgpu_interop_metal::format_wgpu_to_metal(wgpu_format.unwrap()); // TODO: unwrap

            let ftex = cache.get_texture_for_plane(frame, size, f, plane_index, false);

            BufferDescription {
                size: (size.0, size.1, size.0), // TODO: stride
                data: BufferSource::Metal { texture: ftex, command_queue: std::ptr::null_mut() },
                texture_copy: true,
                ..Default::default()
            }
        },
        // Pixel::D3D11 => {
            //let mut ptr1 = [0u8; 8];
            //unsafe { ptr1.copy_from_slice(buffer, 8); }

            /*if (in_frame_data->hw_frames_ctx) {
                AVHWFramesContext *frames_ctx = (AVHWFramesContext *)inlink->hw_frames_ctx->data;
                AVBufferRef *device_ref = frames_ctx->device_ref;
            } else if (ctx->hw_device_ctx) {
                AVBufferRef *device_ref = ctx->hw_device_ctx;
            }

            AVHWDeviceContext *device_ctx   = (AVHWDeviceContext *)device_ref->data;
            AVD3D11VADeviceContext *device_hwctx = device_ctx->hwctx;

            ID3D11Device *d3d11_device = device_hwctx->device;*/
        // },
        _ => {
            BufferDescription {
                size: (frame.plane_width (plane_index) as usize,
                       frame.plane_height(plane_index) as usize,
                       frame.stride      (plane_index)),
                data: BufferSource::Cpu { buffer: frame.data_mut(plane_index) },
                ..Default::default()
            }
        }
    }
}

// -------------------------------------------------------------------
// ------------------------------ macOS ------------------------------
// -------------------------------------------------------------------
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub mod mac_ffi {
    use std::ffi::c_ulonglong;
    use std::ffi::c_void;
    use ffmpeg_next::frame::Video;
    use core_foundation_sys::{ base::{ CFAllocatorRef, CFTypeRef, Boolean }, dictionary::CFDictionaryRef };

    #[derive(Debug, Copy, Clone)]
    pub enum __CVBuffer { }
    pub type CVBufferRef = *mut __CVBuffer;
    pub type CVImageBufferRef = CVBufferRef;
    pub type CVPixelBufferRef = CVImageBufferRef;
    pub type CVMetalTextureRef = CVImageBufferRef;
    pub type CVMetalTextureCacheRef = CFTypeRef;
    pub type CVReturn = i32;
    pub type CVOptionFlags = u64;
    pub type SizeT = c_ulonglong;

    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        pub fn CVMetalTextureCacheCreate(
            allocator: CFAllocatorRef,
            cacheAttributes: CFDictionaryRef,
            metalDevice: *mut metal::MTLDevice,
            textureAttributes: CFDictionaryRef,
            cacheOut: *mut CVMetalTextureCacheRef,
        ) -> CVReturn;
        pub fn CVMetalTextureCacheCreateTextureFromImage(
            allocator: CFAllocatorRef,
            textureCache: CVMetalTextureCacheRef,
            sourceImage: CVImageBufferRef,
            textureAttributes: CFDictionaryRef,
            pixelFormat: metal::MTLPixelFormat,
            width: SizeT,
            height: SizeT,
            planeIndex: SizeT,
            textureOut: *mut CVMetalTextureRef,
        ) -> CVReturn;
        pub fn CVMetalTextureCacheFlush(textureCache: CVMetalTextureCacheRef, options: CVOptionFlags);
        pub fn CVPixelBufferGetWidth(pixelBuffer: CVPixelBufferRef) -> SizeT;
        pub fn CVPixelBufferGetHeight(pixelBuffer: CVPixelBufferRef) -> SizeT;
        pub fn CVPixelBufferGetPixelFormatType(pixelBuffer: CVPixelBufferRef) -> u32;
        pub fn CVPixelBufferGetBaseAddress(pixelBuffer: CVPixelBufferRef) -> *mut c_void;
        pub fn CVPixelBufferGetBytesPerRow(pixelBuffer: CVPixelBufferRef) -> SizeT;
        pub fn CVPixelBufferIsPlanar(pixelBuffer: CVPixelBufferRef) -> Boolean;
        pub fn CVPixelBufferGetPlaneCount(pixelBuffer: CVPixelBufferRef) -> SizeT;
        pub fn CVPixelBufferGetWidthOfPlane(pixelBuffer: CVPixelBufferRef, planeIndex: SizeT) -> SizeT;
        pub fn CVPixelBufferGetHeightOfPlane(pixelBuffer: CVPixelBufferRef, planeIndex: SizeT) -> SizeT;
        pub fn CVMetalTextureGetTexture(image: CVMetalTextureRef) -> *mut metal::MTLTexture;
        pub fn CVMetalTextureIsFlipped(image: CVMetalTextureRef) -> Boolean;
    }

    struct TextureHandle {
        texture_ref: CVMetalTextureRef,
        mtl_texture: *mut metal::MTLTexture
    }
    impl Drop for TextureHandle {
        fn drop(&mut self) {
            //log::debug!("dropping {:?}", self.texture_ref);
            unsafe { core_foundation_sys::base::CFRelease(self.texture_ref as *mut _); }
        }
    }
    pub struct MetalTextureCache {
        cv_cache: CVMetalTextureCacheRef,
        frame_map: lru::LruCache<u64, TextureHandle>
    }

    impl MetalTextureCache {
        pub fn new() -> Option<Self> {
            Self::new_with_device(metal::Device::system_default()?)
        }
        pub fn new_with_device(mtl_device: metal::Device) -> Option<Self> {
            use metal::foreign_types::ForeignType;
            let mut cache: CVMetalTextureCacheRef = std::ptr::null_mut();
            let ret = unsafe { CVMetalTextureCacheCreate(
                std::ptr::null(),
                std::ptr::null(),
                mtl_device.as_ptr() as *mut metal::MTLDevice,
                std::ptr::null(),
                (&mut cache) as *mut *const _ as *mut _
            ) };
            if ret == 0 {
                Some(Self {
                    cv_cache: cache,
                    frame_map: lru::LruCache::new(std::num::NonZeroUsize::new(3*4).unwrap()),
                })
            } else {
                log::error!("Failed to create MetalTextureCache: {:?}", ret);
                None
            }
        }

        pub fn get_texture_for_plane(&mut self, frame: &Video, size: (usize, usize), format: metal::MTLPixelFormat, plane: usize, cache: bool) -> *mut metal::MTLTexture {
            let frame_ptr = unsafe { (*frame.as_ptr()).data[3] as CVPixelBufferRef };
            let key = ((frame_ptr as u64) << 8) | plane as u64;
            if !cache { self.frame_map.pop_entry(&key); }

            self.frame_map.get_or_insert(key, || {
                let mut texture_ref: CVMetalTextureRef = std::ptr::null_mut();

                    //log::debug!("creating texture ref {frame_ptr:?} {plane}");

                    let ret = unsafe { CVMetalTextureCacheCreateTextureFromImage(std::ptr::null(),
                        self.cv_cache,
                        frame_ptr,
                        std::ptr::null(),
                        format,
                        size.0 as u64,
                        size.1 as u64,
                        plane as u64,
                        (&mut texture_ref) as *mut *mut _ as *mut _
                    ) };
                    if ret != 0 {
                        log::error!("Failed to create texture from cache: {:?}", ret);
                    }
                    let mtl_texture = unsafe { CVMetalTextureGetTexture(texture_ref) };
                    TextureHandle {
                        texture_ref,
                        mtl_texture
                    }
            }).mtl_texture
        }
    }
    impl Drop for MetalTextureCache {
        fn drop(&mut self) {
            unsafe { core_foundation_sys::base::CFRelease(self.cv_cache as *mut _); }
        }
    }
}

// -------------------------------------------------------------------
// ------------------------------ macOS ------------------------------
// -------------------------------------------------------------------