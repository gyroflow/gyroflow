

use qml_video_rs::video_player::MDKPlayer;
use std::sync::Arc;
use crate::core::StabilizationManager;
use crate::core::undistortion::RGBA8;
use cpp::*;

cpp! {{
    struct RustPtr { void *data; };
    #include "src/qt_gpu/qrhi_undistort.cpp"
}}

pub fn init_player(mdkplayer: &mut MDKPlayer, stab: Arc<StabilizationManager<RGBA8>>) {
    cpp!(unsafe [mdkplayer as "MDKPlayer *", stab as "RustPtr"] -> bool as "bool" {
        if (!mdkplayer) return false;
        static std::unique_ptr<QtRHIUndistort> rhiUndistortion = std::make_unique<QtRHIUndistort>();

        auto initCb = [mdkplayer](QSize texSize, QSizeF itemSize) -> bool {
            return rhiUndistortion->init(mdkplayer, texSize, itemSize);
        };
        auto renderCb = [mdkplayer, stab](double timestamp, int32_t frame, bool doRender) -> bool {
            float bg[4];
            uint32_t params_count = rust!(Rust_Controller_RenderRHIParams [stab: Arc<StabilizationManager<RGBA8>> as "RustPtr", bg: *mut f32 as "float *"] -> u32 as "uint32_t" {
                let params = stab.params.read();
                *bg.offset(0) = params.background[0] / 255.0;
                *bg.offset(1) = params.background[1] / 255.0;
                *bg.offset(2) = params.background[2] / 255.0;
                *bg.offset(3) = params.background[3] / 255.0;
                if params.frame_readout_time.abs() > 0.0 {
                    (params.size.1 + 1) as u32
                } else {
                    2
                }
            });
            
            rhiUndistortion->params_buffer.resize(params_count * 12);
            auto ptr = rhiUndistortion->params_buffer.data();
            bool ok = rust!(Rust_Controller_RenderRHI [timestamp: f64 as "double", frame: i32 as "int32_t", stab: Arc<StabilizationManager<RGBA8>> as "RustPtr", ptr: *mut f32 as "float *", params_count: u32 as "uint32_t"] -> bool as "bool" {
                stab.fill_undistortion_data_padded(frame as usize, ptr, params_count as usize * 12)
            });

            return ok && rhiUndistortion->render(mdkplayer, timestamp, frame, ptr, params_count, bg, doRender, nullptr, 0, nullptr, 0);
        };
        auto cleanupCb = [] { rhiUndistortion->cleanup(); };
        mdkplayer->setupGpuCompute(initCb, renderCb, cleanupCb);
    });
}
