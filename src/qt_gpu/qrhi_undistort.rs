// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use qml_video_rs::video_player::MDKPlayerWrapper;
use std::sync::Arc;
use crate::core::StabilizationManager;
use crate::core::stabilization::{ KernelParams, RGBA8, distortion_models::DistortionModel };
use cpp::*;
use qmetaobject::{ QSize, QString };

cpp! {{
    struct RustPtr { void *data; };
    #include "src/qt_gpu/qrhi_undistort.cpp"

    static std::unique_ptr<QtRHIUndistort> rhiUndistortion;
}}

pub fn resize_player(stab: Arc<StabilizationManager<RGBA8>>) {
    let player = cpp!(unsafe [] -> *mut MDKPlayerWrapper as "MDKPlayerWrapper *" {
        if (rhiUndistortion && !rhiUndistortion->m_pipeline.isNull() && rhiUndistortion->m_player) {
            return rhiUndistortion->m_player;
        } else {
            return nullptr;
        }
    });
    if !player.is_null() {
        unsafe { init_player(&mut *player, stab); }
    }
}
pub fn init_player(mdkplayer: &mut MDKPlayerWrapper, stab: Arc<StabilizationManager<RGBA8>>) {
    cpp!(unsafe [mdkplayer as "MDKPlayerWrapper *", stab as "RustPtr"] {
        if (!mdkplayer || !mdkplayer->mdkplayer) return;

        auto initCb = [mdkplayer, stab](QSize texSize, QSizeF itemSize) -> bool {
            rhiUndistortion = std::make_unique<QtRHIUndistort>(mdkplayer);

            uint32_t params_size = rust!(Rust_Controller_RenderRHIParamsSize [] -> u32 as "uint32_t" { std::mem::size_of::<KernelParams>() as u32 });

            QSize outputSize = rust!(Rust_Controller_InitRHI [stab: Arc<StabilizationManager<RGBA8>> as "RustPtr"] -> QSize as "QSize" {
                let osize = stab.params.read().output_size;
                QSize { width: osize.0 as u32, height: osize.1 as u32 }
            });
            QString shaderPath = rust!(Rust_Controller_InitRHI2 [stab: Arc<StabilizationManager<RGBA8>> as "RustPtr"] -> QString as "QString" {
                let distortion_model = DistortionModel::from_id(stab.lens.read().distortion_model_id);
                QString::from(distortion_model.glsl_shader_path())
            });
            return rhiUndistortion->init(mdkplayer->mdkplayer, texSize, itemSize, outputSize, shaderPath, params_size);
        };
        auto renderCb = [mdkplayer, stab](double timestamp, int32_t frame, bool doRender) -> bool {
            if (!rhiUndistortion) return false;

            uint32_t matrix_count = rust!(Rust_Controller_RenderRHIParams [stab: Arc<StabilizationManager<RGBA8>> as "RustPtr"] -> u32 as "uint32_t" {
                let params = stab.params.read();
                if params.frame_readout_time.abs() > 0.0 {
                    params.size.1 as u32
                } else {
                    1
                }
            });

            if (rhiUndistortion->matrices.size() < matrix_count * 9) {
                rhiUndistortion->matrices.resize(matrix_count * 9);
            }
            auto mat_ptr     = rhiUndistortion->matrices.data();
            auto params_ptr  = rhiUndistortion->kernel_params.data();
            auto params_size = rhiUndistortion->kernel_params.size();
            bool ok = rust!(Rust_Controller_RenderRHI [timestamp: f64 as "double", frame: i32 as "int32_t", stab: Arc<StabilizationManager<RGBA8>> as "RustPtr", mat_ptr: *mut f32 as "float *", matrix_count: u32 as "uint32_t", params_ptr: *mut u8 as "unsigned char *", params_size: u32 as "uint32_t"] -> bool as "bool" {
                stab.fill_undistortion_data((timestamp * 1_000_000.0) as i64, mat_ptr, matrix_count as usize * 9, params_ptr, params_size as usize)
            });

            return ok && rhiUndistortion->render(mdkplayer->mdkplayer);
        };

        auto cleanupCb = [] { rhiUndistortion.reset(); };

        mdkplayer->mdkplayer->cleanupGpuCompute();
        mdkplayer->mdkplayer->setupGpuCompute(initCb, renderCb, cleanupCb);
    });
}

pub fn deinit_player(mdkplayer: &mut MDKPlayerWrapper) {
    cpp!(unsafe [mdkplayer as "MDKPlayerWrapper *"] {
        if (!mdkplayer || !mdkplayer->mdkplayer) return;
        rhiUndistortion.reset();
        mdkplayer->mdkplayer->cleanupGpuCompute();
        mdkplayer->mdkplayer->setupGpuCompute(nullptr, nullptr, nullptr);
    });
}
