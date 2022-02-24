// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#[cfg(feature = "use-opencl")]
pub mod opencl;
pub mod wgpu;

pub fn initialize_contexts() -> Option<String> {
    let wgpu = std::panic::catch_unwind(|| {
        wgpu::WgpuWrapper::initialize_context()
    });
    match wgpu {
        Ok(Some(name)) => { return Some(name); },
        Ok(None) => { log::error!("wgpu init error"); },
        Err(e) => { log::error!("wgpu init error: {:?}", e); }
    }
    #[cfg(feature = "use-opencl")]
    {
        let cl = std::panic::catch_unwind(|| {
            opencl::OclWrapper::initialize_context()
        });
        match cl {
            Ok(Ok(name)) => { return Some(name); },
            Ok(Err(e)) => { log::error!("OpenCL error: {:?}", e); },
            Err(e) => { log::error!("OpenCL error: {:?}", e); }
        }
    }


    None
}
