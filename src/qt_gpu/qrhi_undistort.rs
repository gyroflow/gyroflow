// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use gyroflow_core::{ stabilization::ProcessedInfo, gpu::Buffers };
use qml_video_rs::video_player::MDKPlayerWrapper;
use std::sync::Arc;
use crate::core::StabilizationManager;
use crate::core::stabilization::RGBA8;
use cpp::*;
use qmetaobject::{ QSize, QString };

cpp! {{
    #include "src/qt_gpu/qrhi_undistort.cpp"
}}

pub fn render(mdkplayer: &MDKPlayerWrapper, timestamp: f64, width: u32, height: u32, stab: Arc<StabilizationManager>, buffers: &mut Buffers) -> Option<ProcessedInfo> {
    if stab.prevent_recompute.load(std::sync::atomic::Ordering::SeqCst) { return None; }

    let mut timestamp_us = (timestamp * 1000.0).round() as i64;
    let mut output_size = QSize::default();
    let mut shader_path = QString::default();

    if let Some(p) = stab.params.try_read() {
        output_size = QSize { width: p.output_size.0 as u32, height: p.output_size.1 as u32 };
        shader_path = {
            let lens = stab.lens.read();
            let distortion_model = lens.distortion_model.as_deref().unwrap_or("opencv_fisheye");
            let digital_lens = lens.digital_lens.as_ref().map(|x| format!("_{}", x)).unwrap_or_else(|| "".into());

            QString::from(format!(":/src/qt_gpu/compiled/undistort_{}{}.frag.qsb", distortion_model, digital_lens))
        };

        if let Some(scale) = p.fps_scale {
            timestamp_us = (timestamp_us as f64 / scale).round() as i64;
        }
    }

    if let Some(mut undist) = stab.stabilization.try_write() {
        undist.ensure_stab_data_at_timestamp::<RGBA8>(timestamp_us, buffers);
        stab.draw_overlays(&mut undist.drawing, timestamp_us);
    }

    if let Some(undist) = stab.stabilization.try_read() {
        if let Some(itm) = undist.get_undistortion_data(timestamp_us) {
            let params = bytemuck::bytes_of(&itm.kernel_params);
            let params_ptr = params.as_ptr();
            let params_len = params.len() as u32;
            let matrices_ptr = itm.matrices.as_ptr();
            let matrices_len = (itm.matrices.len() * 9 * std::mem::size_of::<f32>()) as u32;
            let canvas = undist.drawing.get_buffer();
            let canvas_ptr = canvas.as_ptr();
            let canvas_len = canvas.len() as u32;

            let canvas_size = undist.drawing.get_size();
            let canvas_size = QSize { width: canvas_size.0 as u32, height: canvas_size.1 as u32 };

            let ok = cpp!(unsafe [mdkplayer as "MDKPlayerWrapper *", output_size as "QSize", shader_path as "QString", width as "uint32_t", height as "uint32_t", params_ptr as "uint8_t*", matrices_ptr as "uint8_t*", canvas_ptr as "uint8_t*", matrices_len as "uint32_t", params_len as "uint32_t", canvas_len as "uint32_t", canvas_size as "QSize"] -> bool as "bool" {
                if (!mdkplayer || !mdkplayer->mdkplayer || shader_path.isEmpty() || output_size.isEmpty()) return false;

                auto rhiUndistortion = static_cast<QtRHIUndistort *>(mdkplayer->mdkplayer->userData());

                if (!QFile::exists(shader_path)) {
                    qDebug2("render") << shader_path << "doesn't exist";
                    delete rhiUndistortion;
                    mdkplayer->mdkplayer->setUserData(nullptr);
                    return false;
                }
                if (output_size.width() < 4 || output_size.height() < 4) {
                    delete rhiUndistortion;
                    mdkplayer->mdkplayer->setUserData(nullptr);
                    return true;
                }

                if (!rhiUndistortion
                || rhiUndistortion->outSize() != output_size
                || rhiUndistortion->texSize() != QSize(width, height)
                || rhiUndistortion->shaderPath() != shader_path
                || rhiUndistortion->itemTexturePtr() != mdkplayer->mdkplayer->rhiTexture()) {
                    delete rhiUndistortion;
                    rhiUndistortion = new QtRHIUndistort();
                    if (!rhiUndistortion->init(mdkplayer->mdkplayer, QSize(width, height), output_size, shader_path, params_len, canvas_size)) {
                        qDebug2("render") << "Failed to initialize";
                        delete rhiUndistortion;
                        mdkplayer->mdkplayer->setUserData(nullptr);
                        return false;
                    }
                    qDebug2("render") << "Initialized" << QSize(width, height) << "->" << output_size << shader_path << rhiUndistortion;
                    mdkplayer->mdkplayer->setUserData(static_cast<void *>(rhiUndistortion));
                    mdkplayer->mdkplayer->setUserDataDestructor([](void *ptr) {
                        delete static_cast<QtRHIUndistort *>(ptr);
                    });
                }

                return rhiUndistortion->render(mdkplayer->mdkplayer, params_ptr, params_len, matrices_ptr, matrices_len, canvas_ptr, canvas_len);
            });
            if ok {
                return Some(ProcessedInfo {
                    fov: itm.fov,
                    minimal_fov: itm.minimal_fov,
                    focal_length: itm.focal_length,
                    backend: "Qt RHI"
                });
            }
        }
    }
    None
}
