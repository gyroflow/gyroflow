use ffmpeg_next::{ ffi, encoder, Stream, Error };

use std::collections::HashMap;
use std::ffi::CStr;
use parking_lot::Mutex;

type DeviceType = ffi::AVHWDeviceType;

#[derive(Debug)]
pub struct HWDevice {
    type_: DeviceType,
    device_ref: *mut ffi::AVBufferRef,

    pub hw_formats: Vec<ffi::AVPixelFormat>,
    pub sw_formats: Vec<ffi::AVPixelFormat>,
    pub min_size: (i32, i32),
    pub max_size: (i32, i32)
}
impl HWDevice {
    pub fn from_type(type_: DeviceType) -> Result<Self, ()> {
        unsafe {
            let mut device_ref = std::ptr::null_mut();
            let err = ffi::av_hwdevice_ctx_create(&mut device_ref, type_, std::ptr::null(), std::ptr::null_mut(), 0);
            if err >= 0 && !device_ref.is_null() {
                Ok(Self {
                    type_,
                    device_ref,
                    hw_formats: Vec::new(),
                    sw_formats: Vec::new(),
                    min_size: (0, 0),
                    max_size: (0, 0),
                })
            } else {
                super::append_log(&format!("Failed to create specified HW device: {:?}\n", type_));
                Err(())
            }
        }
    }

    pub fn add_ref(&self) -> *mut ffi::AVBufferRef {
        unsafe { ffi::av_buffer_ref(self.device_ref) }
    }
    pub fn as_mut_ptr(&self) -> *mut ffi::AVBufferRef { self.device_ref }
    pub fn device_type(&self) -> DeviceType { self.type_ }
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

lazy_static::lazy_static! {
    static ref DEVICES: Mutex<HashMap<DeviceType, HWDevice>> = Mutex::new(HashMap::new());
}

pub fn supported_gpu_backends() -> Vec<String> {
    let mut ret = Vec::new();
    let mut hw_type = ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE;
    for _ in 0..100 { // Better 100 than infinity
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

pub unsafe fn pix_formats_to_vec(formats: *const ffi::AVPixelFormat) -> Vec<ffi::AVPixelFormat> {
    let mut ret = Vec::new();
    for i in 0..100 {
        let p = *formats.offset(i);
        if p == ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            break;
        }
        ret.push(p);
    }
    ret
}

pub fn init_device_for_decoding(codec: *mut ffi::AVCodec, stream: &mut Stream) -> Result<(ffi::AVHWDeviceType, String, Option<ffi::AVPixelFormat>), Error> {
    for i in 0..100 {
        unsafe {
            let config = ffi::avcodec_get_hw_config(codec, i);
            if config.is_null() {
                break;
            }
            let type_ = (*config).device_type;
            ::log::debug!("codec type {:?} {}", type_, i);
            let mut devices = DEVICES.lock();
            if !devices.contains_key(&type_) {
                if let Ok(dev) = HWDevice::from_type(type_) {
                    devices.insert(type_, dev);
                }
            }
            if let Some(dev) = devices.get(&type_) {
                let mut decoder_ctx = stream.codec().decoder();
                (*decoder_ctx.as_mut_ptr()).hw_device_ctx = dev.add_ref();
                return Ok((type_, dev.name(), Some((*config).pix_fmt)));
            }
        }
    }
    Ok((ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE, String::new(), None))
}

pub fn find_working_encoder(encoders: &Vec<(&'static str, bool)>) -> (&'static str, bool, Option<DeviceType>) {
    if encoders.is_empty() { return ("", false, None); } // TODO: should be Result<>

    for x in encoders {
        if let Some(mut enc) = encoder::find_by_name(x.0) {
            for i in 0..100 {
                unsafe {
                    let config = ffi::avcodec_get_hw_config(enc.as_mut_ptr(), i);
                    if config.is_null() {
                        break;
                    }
                    let type_ = (*config).device_type;
                    ::log::debug!("codec type {:?} {}", type_, i);
                    let mut devices = DEVICES.lock();
                    if !devices.contains_key(&type_) {
                        ::log::debug!("create {:?}", type_);
                        if let Ok(dev) = HWDevice::from_type(type_) {
                            ::log::debug!("created ok {:?}", type_);
                            devices.insert(type_, dev);
                        }
                    }
                    if let Some(dev) = devices.get_mut(&type_) {
                        let mut constraints = ffi::av_hwdevice_get_hwframe_constraints(dev.as_mut_ptr(), std::ptr::null());
                        if !constraints.is_null() {
                            dev.hw_formats = pix_formats_to_vec((*constraints).valid_hw_formats);
                            dev.sw_formats = pix_formats_to_vec((*constraints).valid_sw_formats);
                            dev.min_size = ((*constraints).min_width as i32, (*constraints).min_height as i32);
                            dev.max_size = ((*constraints).max_width as i32, (*constraints).max_height as i32);
                
                            ffi::av_hwframe_constraints_free(&mut constraints);
                        }
                        return (x.0, x.1, Some(dev.device_type()));
                    }
                }
            }
        }
    }
    let x = encoders.last().unwrap();
    return (x.0, x.1, None);
}

pub unsafe fn get_transfer_formats_from_gpu(frame: *mut ffi::AVFrame) -> Vec<ffi::AVPixelFormat> {
    let mut formats = std::ptr::null_mut();
    if !frame.is_null() && !(*frame).hw_frames_ctx.is_null() {
        ffi::av_hwframe_transfer_get_formats((*frame).hw_frames_ctx, ffi::AVHWFrameTransferDirection::AV_HWFRAME_TRANSFER_DIRECTION_FROM, &mut formats, 0);
    }
    if formats.is_null() {
        Vec::new()
    } else {
        pix_formats_to_vec(formats)
    }
}
pub unsafe fn get_transfer_formats_to_gpu(frame: *mut ffi::AVFrame) -> Vec<ffi::AVPixelFormat> {
    let mut formats = std::ptr::null_mut();
    if !frame.is_null() && !(*frame).hw_frames_ctx.is_null() {
        ffi::av_hwframe_transfer_get_formats((*frame).hw_frames_ctx, ffi::AVHWFrameTransferDirection::AV_HWFRAME_TRANSFER_DIRECTION_TO, &mut formats, 0);
    }
    if formats.is_null() {
        Vec::new()
    } else {
        pix_formats_to_vec(formats)
    }
}

pub fn initialize_hwframes_context(encoder_ctx: *mut ffi::AVCodecContext, _frame_ctx: *mut ffi::AVFrame, type_: DeviceType, pixel_format: ffi::AVPixelFormat, size: (u32, u32)) -> Result<(), ()> {
    let devices = DEVICES.lock();
    if let Some(dev) = devices.get(&type_) {
        unsafe {                
            let mut hw_frames_ref = ffi::av_hwframe_ctx_alloc(dev.as_mut_ptr());
            if hw_frames_ref.is_null() {
                super::append_log(&format!("Failed to create GPU frame context {:?}.\n", type_));
                return Err(());
            }

            if !dev.hw_formats.is_empty() {
                let target_format = {
                    if !dev.sw_formats.contains(&pixel_format) {
                        log::warn!("Encoder doesn't support the desired pixel format ({:?})\n", pixel_format);
                        log::debug!("dev.sw_formats: {:?}", &dev.sw_formats);
                        let formats = get_transfer_formats_to_gpu(_frame_ctx);
                        if formats.is_empty() {
                            super::append_log(&format!("No frame transfer formats. Desired format: {:?}\n", pixel_format));
                            ffi::AVPixelFormat::AV_PIX_FMT_NONE
                        } else if formats.contains(&pixel_format) {
                            pixel_format
                        } else {
                            // Just pick the first format.
                            // TODO: this should maybe take into consideration if the frame is 8 bit or more
                            *formats.first().unwrap()
                        }
                    } else {
                        pixel_format
                    }
                };
                log::debug!("target_format: {:?}", &target_format);

                if target_format != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                    let hw_format = *dev.hw_formats.first().unwrap(); // Safe because we check !is_empty() above
                    let mut frames_ctx = (*hw_frames_ref).data as *mut ffi::AVHWFramesContext;
                    (*frames_ctx).format    = hw_format;
                    (*frames_ctx).sw_format = target_format;
                    (*frames_ctx).width     = size.0 as i32;
                    (*frames_ctx).height    = size.1 as i32;
                    (*frames_ctx).initial_pool_size = 20;
                    
                    let err = ffi::av_hwframe_ctx_init(hw_frames_ref);
                    if err < 0 {
                        super::append_log(&format!("Failed to initialize frame context. Error code: {}\n", err));
                        ffi::av_buffer_unref(&mut hw_frames_ref);
                        return Err(());
                    }
                    (*encoder_ctx).hw_frames_ctx = ffi::av_buffer_ref(hw_frames_ref);
                    (*encoder_ctx).pix_fmt = hw_format;
                
                    ffi::av_buffer_unref(&mut hw_frames_ref);
                }
            }
        }
    }
    Ok(())
}

pub fn find_best_matching_codec(codec: ffi::AVPixelFormat, supported: &[ffi::AVPixelFormat]) -> ffi::AVPixelFormat {
    if supported.is_empty() || supported.contains(&codec) { return ffi::AVPixelFormat::AV_PIX_FMT_NONE; }

    if codec == ffi::AVPixelFormat::AV_PIX_FMT_P010LE && supported.contains(&ffi::AVPixelFormat::AV_PIX_FMT_YUV420P10LE) { return ffi::AVPixelFormat::AV_PIX_FMT_YUV420P10LE; }
    if codec == ffi::AVPixelFormat::AV_PIX_FMT_NV12   && supported.contains(&ffi::AVPixelFormat::AV_PIX_FMT_YUV420P)     { return ffi::AVPixelFormat::AV_PIX_FMT_YUV420P; }

    super::append_log(&format!("No matching codec, we need {:?} and supported are: {:?}\n", codec, supported));

    *supported.first().unwrap()
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
