// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#[cfg(feature = "use-opencl")]
pub mod opencl;
pub mod wgpu;

pub fn initialize_contexts() -> Option<String> {
    #[cfg(feature = "use-opencl")]
    {
        let cl = std::panic::catch_unwind(|| {
            opencl::OclWrapper::initialize_context()
        });
        match cl {
            Ok(Ok(name)) => { return Some(name); },
            Ok(Err(e)) => { log::error!("OpenCL error: {:?}", e); },
            Err(e) => {
                if let Some(s) = e.downcast_ref::<&str>() {
                    log::error!("Failed to initialize OpenCL {}", s);
                } else if let Some(s) = e.downcast_ref::<String>() {
                    log::error!("Failed to initialize OpenCL {}", s);
                } else {
                    log::error!("Failed to initialize OpenCL {:?}", e);
                }
            }
        }
    }

    let wgpu = std::panic::catch_unwind(|| {
        wgpu::WgpuWrapper::initialize_context()
    });
    match wgpu {
        Ok(Some(name)) => { return Some(name); },
        Ok(None) => { log::error!("wgpu init error"); },
        Err(e) => {
            if let Some(s) = e.downcast_ref::<&str>() {
                log::error!("Failed to initialize wgpu {}", s);
            } else if let Some(s) = e.downcast_ref::<String>() {
                log::error!("Failed to initialize wgpu {}", s);
            } else {
                log::error!("Failed to initialize wgpu {:?}", e);
            }
        }
    }

    None
}
