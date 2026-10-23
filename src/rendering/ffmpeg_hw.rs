// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use ffmpeg_next::{ ffi, format, codec, encoder };

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::{ CStr, CString };
use std::ptr;
use std::sync::LazyLock;
use std::sync::atomic::{ AtomicU64, Ordering };
use parking_lot::Mutex;

type DeviceType = ffi::AVHWDeviceType;

// FFmpeg's Vulkan hwaccel is opt-in - never let FFmpeg create a Vulkan device unless the user explicitly enabled it
pub fn is_vulkan_enabled() -> bool { gyroflow_core::settings::get_bool("useVulkanEncoder", false) }

#[derive(Debug)]
pub struct HWDevice {
    type_: DeviceType,
    device_ref: *mut ffi::AVBufferRef,
    device_name: Option<String>,

    pub hw_formats: Vec<format::Pixel>,
    pub sw_formats: Vec<format::Pixel>,
    pub min_size: (i32, i32),
    pub max_size: (i32, i32)
}
impl HWDevice {
    pub fn from_type(type_: DeviceType, device_name: Option<&str>) -> Result<Self, super::FFmpegError> {
        log::debug!("HWDevice::from_type {type_:?}, device: {device_name:?}");
        if type_ == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VULKAN && !is_vulkan_enabled() {
            log::debug!("Vulkan hwaccel is disabled");
            return Err(super::FFmpegError::CannotCreateGPUDecoding);
        }
        unsafe {
            let dev = device_name.and_then(|x| if x.is_empty() { None } else { CString::new(x).ok() });

            let mut device_ref = ptr::null_mut();
            let err = ffi::av_hwdevice_ctx_create(&mut device_ref, type_, dev.as_ref().map_or(ptr::null(), |x| x.as_ptr()), ptr::null_mut(), 0);
            if err >= 0 && !device_ref.is_null() {
                Ok(Self {
                    type_,
                    device_name: device_name.map(|x| x.to_string()),
                    device_ref,
                    hw_formats: Vec::new(),
                    sw_formats: Vec::new(),
                    min_size: (0, 0),
                    max_size: (0, 0),
                })
            } else {
                super::append_log(&format!("Failed to create specified HW device: {:?}\n", type_));
                Err(super::FFmpegError::CannotCreateGPUDecoding)
            }
        }
    }

    pub fn add_ref(&self) -> *mut ffi::AVBufferRef {
        unsafe { ffi::av_buffer_ref(self.device_ref) }
    }
    pub fn as_mut_ptr(&self) -> *mut ffi::AVBufferRef { self.device_ref }
    pub fn device_type(&self) -> DeviceType { self.type_ }
    pub fn device_name(&self) -> Option<&str> { self.device_name.as_deref() }
    pub fn name(&self) -> String {
        unsafe {
            let name_ptr = ffi::av_hwdevice_get_type_name(self.type_);
            CStr::from_ptr(name_ptr).to_string_lossy().into()
        }
    }
}
impl Drop for HWDevice {
    fn drop(&mut self) {
        unsafe { ffi::av_buffer_unref(&mut self.device_ref); }
    }
}
unsafe impl Sync for HWDevice { }
unsafe impl Send for HWDevice { }

static DEVICES: LazyLock<Mutex<HashMap<u64, HWDevice>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Whether an encoder works, by its name and its device, see `probe_encoder`. Cleared with the devices
static PROBED_ENCODERS: LazyLock<Mutex<HashMap<(&'static str, u64), bool>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Bumped when the devices are cleared, so that a probe still running on the old ones doesn't keep its result
static DEVICES_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn clear_devices() {
    DEVICES_GENERATION.fetch_add(1, Ordering::SeqCst);
    DEVICES.lock().clear();
    PROBED_ENCODERS.lock().clear();
}
pub fn initialize_ctx(type_: ffi::AVHWDeviceType) {
    let mut devices = DEVICES.lock();
    if let Entry::Vacant(e) = devices.entry(type_ as u64) {
        ::log::debug!("create {:?}", type_);
        if let Ok(dev) = HWDevice::from_type(type_, None) {
            ::log::debug!("created ok {:?}", type_);
            e.insert(dev);
        }
    }
}

pub fn supported_gpu_backends() -> Vec<String> {
    let mut ret = Vec::new();
    let mut hw_type = ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE;
    for _ in 0..20 { // Better 20 than infinity
        unsafe {
            hw_type = ffi::av_hwdevice_iterate_types(hw_type);
            if hw_type == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
                break;
            }
            // returns a pointer to static string, shouldn't be freed
            let name_ptr = ffi::av_hwdevice_get_type_name(hw_type);
            ret.push(CStr::from_ptr(name_ptr).to_string_lossy().into());
        }
    }
    ret
}

pub unsafe fn pix_formats_to_vec(formats: *const ffi::AVPixelFormat) -> Vec<format::Pixel> {
    if formats.is_null() {
        return Vec::new();
    }
    let mut ret = Vec::new();
    for i in 0..100 {
        let p = unsafe { *formats.offset(i) };
        if p == ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            break;
        }
        ret.push(p.into());
    }
    ret
}

pub unsafe fn codec_pix_formats(codec_ctx: *const ffi::AVCodecContext, codec: *const ffi::AVCodec) -> Vec<format::Pixel> {
    unsafe {
        if codec.is_null() && codec_ctx.is_null() {
            return Vec::new();
        }
        let mut formats: *const ffi::AVPixelFormat = ptr::null();
        let mut count: std::os::raw::c_int = 0;
        let ret = ffi::avcodec_get_supported_config(codec_ctx, codec, ffi::AVCodecConfig::AV_CODEC_CONFIG_PIX_FORMAT, 0, (&mut formats as *mut *const ffi::AVPixelFormat).cast(), &mut count);
        if ret < 0 || formats.is_null() || count <= 0 {
            return Vec::new();
        }
        std::slice::from_raw_parts(formats, count as usize).iter().map(|p| (*p).into()).collect()
    }
}

pub fn init_device_for_decoding(index: usize, codec: *const ffi::AVCodec, decoder_ctx: &mut codec::context::Context, device: Option<&str>) -> Result<(usize, ffi::AVHWDeviceType, String, Option<ffi::AVPixelFormat>), super::FFmpegError> {
    let mut decoders = Vec::<(usize, ffi::AVHWDeviceType)>::new();

    for i in 0..20 {
        unsafe {
            let config = ffi::avcodec_get_hw_config(codec, i as i32);
            if !config.is_null() && (*config).device_type != ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
                decoders.push((i, (*config).device_type));
            }
        }
    }
    if let Some(pos) = decoders.iter().position(|(_, type_)| *type_ == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA) {
        // Move CUDA to first position
        let cuda = decoders.remove(pos);
        decoders.insert(0, cuda);
    }
    if is_vulkan_enabled() {
        if let Some(pos) = decoders.iter().position(|(_, type_)| *type_ == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VULKAN) {
            let x = decoders.remove(pos);
            decoders.insert(0, x);
        }
    } else {
        decoders.retain(|(_, type_)| *type_ != ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VULKAN);
    }
    if gyroflow_core::settings::get_bool("useD3D12Encoder", false) {
        if let Some(pos) = decoders.iter().position(|(_, type_)| *type_ == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D12VA) {
            let x = decoders.remove(pos);
            decoders.insert(0, x);
        }
    }

    log::debug!("Available decoders: {decoders:?}");

    for (i, _) in decoders.iter().skip(index) {
        unsafe {
            let config = ffi::avcodec_get_hw_config(codec, *i as i32);
            if config.is_null() {
                ::log::debug!("config null for {}", i);
                continue;
            }
            let type_ = (*config).device_type;
            if type_ == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
                continue;
            }
            if cfg!(target_os = "windows") && type_ == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI {
                continue;
            }
            ::log::debug!("[dec] codec type {:?} {}", type_, i);
            let mut devices = DEVICES.lock();
            let mut device_hash = 0;
            if let Some(dev_name) = device {
                let mut hasher = crc32fast::Hasher::new();
                hasher.update(dev_name.as_bytes());
                device_hash = hasher.finalize() as u64;
            }
            if let Entry::Vacant(e) = devices.entry(type_ as u64 + device_hash) {
                if let Ok(dev) = HWDevice::from_type(type_, device) {
                    e.insert(dev);
                }
            }
            if let Some(dev) = devices.get(&(type_ as u64 + device_hash)) {
                (*decoder_ctx.as_mut_ptr()).hw_device_ctx = dev.add_ref();
                return Ok((*i, type_, dev.name(), Some((*config).pix_fmt)));
            }
        }
    }
    Ok((0, ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE, String::new(), None))
}

/// `find_encoder_that_works`, or the last of `encoders` when none does, to fail with "Encoder not found" when it's not in the build (eg. libx264 before the GPL codecs are downloaded)
pub fn find_working_encoder(encoders: &[(&'static str, bool)], device: Option<&str>) -> (&'static str, bool, Option<DeviceType>) {
    if encoders.is_empty() { return ("", false, None); } // TODO: should be Result<>

    find_encoder_that_works(encoders, device).unwrap_or_else(|| {
        let x = encoders.last().unwrap();
        (x.0, x.1, None)
    })
}

/// The first of `encoders` in the build that works here. A software one always does, a hardware one when its device can be created,
/// and when it can be opened for those whose device doesn't tell, see `needs_probe`
pub fn find_encoder_that_works(encoders: &[(&'static str, bool)], device: Option<&str>) -> Option<(&'static str, bool, Option<DeviceType>)> {
    let mut device_hash = 0;
    if let Some(dev_name) = device {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(dev_name.as_bytes());
        device_hash = hasher.finalize() as u64;
    }

    for x in encoders {
        if let Some(enc) = encoder::find_by_name(x.0) {
            if !x.1 { return Some((x.0, x.1, None)); } // If not HW encoder

            if let Some(type_) = encoder_device(&enc, x.0, device, device_hash) {
                if !needs_probe(x.0) || probe_encoder(&enc, x.0, type_, device, device_hash) {
                    return Some((x.0, x.1, Some(type_)));
                }
            }
        } else {
            super::append_log(&format!("Codec not found: {:?}\n", x.0));
        }
    }
    None
}

/// The device of the first hardware config of the encoder that can be created, with its frame constraints read
fn encoder_device(enc: &codec::codec::Codec, name: &str, device: Option<&str>, device_hash: u64) -> Option<DeviceType> {
    for i in 0..20 {
        unsafe {
            let type_ = if !name.contains("videotoolbox") {
                let config = ffi::avcodec_get_hw_config(enc.as_ptr(), i);
                if config.is_null() {
                    println!("config is null {}", name);
                    break;
                }
                let type_ = (*config).device_type;
                ::log::debug!("[enc] codec type {:?} {}, for: {}", type_, i, name);
                let mut devices = DEVICES.lock();
                if let Entry::Vacant(e) = devices.entry(type_ as u64 + device_hash) {
                    ::log::debug!("create {:?}", type_);
                    if let Ok(dev) = HWDevice::from_type(type_, device) {
                        ::log::debug!("created ok {:?}", type_);
                        e.insert(dev);
                    }
                }
                type_
            } else {
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX
            };
            let mut devices = DEVICES.lock();
            if let Some(dev) = devices.get_mut(&(type_ as u64 + device_hash)) {
                let mut constraints = ffi::av_hwdevice_get_hwframe_constraints(dev.as_mut_ptr(), ptr::null());
                if !constraints.is_null() {
                    dev.hw_formats = pix_formats_to_vec((*constraints).valid_hw_formats);
                    dev.sw_formats = pix_formats_to_vec((*constraints).valid_sw_formats);
                    dev.min_size = ((*constraints).min_width, (*constraints).min_height);
                    dev.max_size = ((*constraints).max_width, (*constraints).max_height);

                    log::debug!("HW formats: {:?}", &dev.hw_formats);
                    log::debug!("SW formats: {:?}", &dev.sw_formats);

                    ffi::av_hwframe_constraints_free(&mut constraints);
                }
                return Some(dev.device_type());
            }
        }
    }
    None
}

/// Whether it takes opening the encoder to know if it works here. Every GPU of a vendor has the device of its encoders,
/// but only the newer ones encode AV1: NVIDIA RTX 40, AMD Radeon RX 7000, Intel Arc and Core Ultra, and later
pub fn needs_probe(encoder_name: &str) -> bool {
    encoder_name.starts_with("av1_")
}

/// Options the encoder needs, in the render and in `probe_encoder`: on Android MediaCodec is used through the NDK
pub fn encoder_required_options(encoder_name: &str) -> &'static [(&'static str, &'static str)] {
    if encoder_name.contains("mediacodec") {
        &[("ndk_codec", "1"), ("ndk_async", "1")]
    } else {
        &[]
    }
}

/// Opens the encoder for a 1080p video on the device, like the render does. A GPU without the codec fails here.
/// The result is kept until the devices are cleared
fn probe_encoder(enc: &codec::codec::Codec, name: &'static str, type_: DeviceType, device: Option<&str>, device_hash: u64) -> bool {
    let key = (name, type_ as u64 + device_hash);
    if let Some(&works) = PROBED_ENCODERS.lock().get(&key) { return works; }
    let generation = DEVICES_GENERATION.load(Ordering::SeqCst);

    // The errors of an encoder that doesn't work are expected, they'd only confuse the log shown when a render fails
    super::PROBING_ENCODER.set(true);
    let result = (|| -> Result<(), ffmpeg_next::Error> {
        let size = (1920, 1080);
        let mut encoder = codec::context::Context::new_with_codec(*enc).encoder().video()?;
        encoder.set_width(size.0);
        encoder.set_height(size.1);
        encoder.set_format(format::Pixel::NV12);
        encoder.set_frame_rate(Some(ffmpeg_next::Rational::new(30, 1)));
        encoder.set_time_base(ffmpeg_next::Rational::new(1, 30));
        encoder.set_bit_rate(20 * 1024 * 1024);
        // Vulkan, D3D12 and VAAPI encoders take only frames on the device
        let hw_frames_only = unsafe { codec_pix_formats(ptr::null(), enc.as_ptr()) }.iter().all(|x| is_hardware_format((*x).into()));
        // Without its frames, an encoder that takes only frames on the device can't encode
        initialize_hwframes_context(unsafe { encoder.as_mut_ptr() }, ptr::null_mut(), type_, format::Pixel::NV12.into(), size, hw_frames_only, device)?;
        let mut options = ffmpeg_next::Dictionary::new();
        for (k, v) in encoder_required_options(name) {
            options.set(k, v);
        }
        encoder.open_with(options)?;
        Ok(())
    })();
    super::PROBING_ENCODER.set(false);

    match &result {
        Ok(_)  => log::info!("Encoder {name} works on {type_:?}"),
        Err(e) => log::info!("Encoder {name} doesn't work on {type_:?}: {e:?}"),
    }
    let mut probed = PROBED_ENCODERS.lock();
    if DEVICES_GENERATION.load(Ordering::SeqCst) == generation {
        probed.insert(key, result.is_ok());
    }
    result.is_ok()
}

pub unsafe fn get_transfer_formats_from_gpu(frame: *mut ffi::AVFrame) -> Vec<format::Pixel> {
    unsafe {
        let mut formats = ptr::null_mut();
        if !frame.is_null() && !(*frame).hw_frames_ctx.is_null() {
            ffi::av_hwframe_transfer_get_formats((*frame).hw_frames_ctx, ffi::AVHWFrameTransferDirection::AV_HWFRAME_TRANSFER_DIRECTION_FROM, &mut formats, 0);
        }
        if formats.is_null() {
            Vec::new()
        } else {
            pix_formats_to_vec(formats)
        }
    }
}
pub unsafe fn get_transfer_formats_to_gpu(frame: *mut ffi::AVFrame) -> Vec<format::Pixel> {
    unsafe {
        let mut formats = ptr::null_mut();
        if !frame.is_null() && !(*frame).hw_frames_ctx.is_null() {
            ffi::av_hwframe_transfer_get_formats((*frame).hw_frames_ctx, ffi::AVHWFrameTransferDirection::AV_HWFRAME_TRANSFER_DIRECTION_TO, &mut formats, 0);
        }
        if formats.is_null() {
            Vec::new()
        } else {
            pix_formats_to_vec(formats)
        }
    }
}

pub fn is_hardware_format(format: ffi::AVPixelFormat) -> bool {
    format == ffi::AVPixelFormat::AV_PIX_FMT_CUDA ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_DXVA2_VLD ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_VDPAU ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_D3D11 ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_D3D11VA_VLD ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_D3D12 ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_MEDIACODEC ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_OPENCL ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_QSV ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_MMAL ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_VULKAN ||
    format == ffi::AVPixelFormat::AV_PIX_FMT_VAAPI
}

pub fn initialize_hwframes_context(encoder_ctx: *mut ffi::AVCodecContext, _frame_ctx: *mut ffi::AVFrame, type_: DeviceType, pixel_format: ffi::AVPixelFormat, size: (u32, u32), init_hwframes: bool, device_name: Option<&str>) -> Result<(), ffmpeg_next::Error> {
    let mut devices = DEVICES.lock();
    let mut device_hash = 0;
    if let Some(dev_name) = device_name {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(dev_name.as_bytes());
        device_hash = hasher.finalize() as u64;
    }
    if let Some(dev) = devices.get_mut(&(type_ as u64 + device_hash)) {
        unsafe {
            if (*encoder_ctx).hw_device_ctx.is_null() && type_ != ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_QSV {
                (*encoder_ctx).hw_device_ctx = dev.add_ref();
                log::debug!("Setting hw_device_ctx {:?}", (*encoder_ctx).hw_device_ctx);
            }

            if init_hwframes {
                if dev.sw_formats.is_empty() && !(*encoder_ctx).codec.is_null() {
                    dev.sw_formats = codec_pix_formats(encoder_ctx, (*encoder_ctx).codec);
                    log::debug!("Setting codec formats: {:?}", dev.sw_formats);
                }

                if !dev.hw_formats.is_empty() {
                    let target_format: ffi::AVPixelFormat = {
                        if !dev.sw_formats.contains(&pixel_format.into()) {
                            log::warn!("Encoder doesn't support the desired pixel format ({:?})\n", pixel_format);
                            log::debug!("dev.sw_formats: {:?}", &dev.sw_formats);
                            let formats = get_transfer_formats_to_gpu(_frame_ctx);
                            if formats.is_empty() {
                                super::append_log(&format!("No frame transfer formats. Desired format: {:?}\n", pixel_format));
                                ffi::AVPixelFormat::AV_PIX_FMT_NONE
                            } else if formats.contains(&pixel_format.into()) {
                                pixel_format
                            } else {
                                // Just pick the first format.
                                // TODO: this should maybe take into consideration if the frame is 8 bit or more
                                format::Pixel::into(*formats.first().unwrap())
                            }
                        } else {
                            pixel_format
                        }
                    };
                    log::debug!("target_format: {:?}", &target_format);

                    if target_format != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                        let hw_format = *dev.hw_formats.first().unwrap(); // Safe because we check !is_empty() above

                        if (*encoder_ctx).hw_frames_ctx.is_null() {
                            let mut hw_frames_ref = ffi::av_hwframe_ctx_alloc(dev.as_mut_ptr());
                            if hw_frames_ref.is_null() {
                                super::append_log(&format!("Failed to create GPU frame context {:?}.\n", type_));
                                return Err(ffmpeg_next::Error::Other { errno: ffmpeg_next::util::error::ENOMEM });
                            }
                            (*encoder_ctx).hw_frames_ctx = ffi::av_buffer_ref(hw_frames_ref);
                            ffi::av_buffer_unref(&mut hw_frames_ref);
                        } else {
                            log::debug!("hwframes already exists");
                        }
                        let frames_ctx_ref = (*encoder_ctx).hw_frames_ctx;

                        let frames_ctx = (*frames_ctx_ref).data as *mut ffi::AVHWFramesContext;
                        if (*frames_ctx).format    == ffi::AVPixelFormat::AV_PIX_FMT_NONE { (*frames_ctx).format    = hw_format.into(); }
                        if (*frames_ctx).sw_format == ffi::AVPixelFormat::AV_PIX_FMT_NONE { (*frames_ctx).sw_format = target_format; }
                        (*frames_ctx).width     = size.0 as i32;
                        (*frames_ctx).height    = size.1 as i32;
                        if type_ == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_QSV || type_ == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI {
                            (*frames_ctx).initial_pool_size = 20;
                        }

                        let err = ffi::av_hwframe_ctx_init(frames_ctx_ref);
                        if err < 0 {
                            super::append_log(&format!("Failed to initialize frame context. Error code: {}\n", err));
                            // Through the encoder's field, which it nulls: the encoder would use the freed context, and free it again
                            ffi::av_buffer_unref(&mut (*encoder_ctx).hw_frames_ctx);
                            return Err(err.into());
                        } else {
                            log::debug!("inited hwframe ctx");
                        }
                        log::debug!("frames_ctx.format: {:?}", &(*frames_ctx).format);
                        log::debug!("frames_ctx.sw_format: {:?}", &(*frames_ctx).sw_format);
                        (*encoder_ctx).pix_fmt = (*frames_ctx).format;
                    }
                }

            }
            return Ok(());
        }
    } else {
        log::warn!("DEVICES didn't have {:?}", type_);
    }
    Ok(())
}

pub fn find_best_matching_codec(codec: format::Pixel, supported: &[format::Pixel]) -> Option<format::Pixel> {
    if supported.is_empty() { return None; }

    if supported.contains(&codec) { return Some(codec); }

    let pairs = vec![
        (format::Pixel::P210LE, format::Pixel::YUV422P10LE),
        (format::Pixel::P010LE, format::Pixel::YUV420P10LE),
        (format::Pixel::NV12,   format::Pixel::YUV420P),
        (format::Pixel::NV21,   format::Pixel::YUV420P),
    ];
    for (a, b) in pairs {
        if codec == a && supported.contains(&b) { return Some(b); }
        if codec == b && supported.contains(&a) { return Some(a); }
    }

    super::append_log(&format!("No matching codec, we need {:?} and supported are: {:?}\n", codec, supported));

    None
}

// pub fn get_supported_pixel_formats(name: &str) -> Vec<ffi::AVPixelFormat> {
//     if let Some(mut codec) = encoder::find_by_name(name) {
//         unsafe {
//             pix_formats_to_vec((*codec.as_mut_ptr()).pix_fmts)
//         }
//     } else {
//         Vec::new()
//     }
// }
