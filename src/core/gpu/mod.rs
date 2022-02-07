// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#[cfg(feature = "use-opencl")]
pub mod opencl;
pub mod wgpu;

pub fn initialize_contexts() -> Option<String> {
    #[cfg(feature = "use-opencl")]
    match opencl::OclWrapper::initialize_context() {
        Ok(name) => { return Some(name); },
        Err(e) => { log::error!("OpenCL error: {:?}", e); }
    }

    match wgpu::WgpuWrapper::initialize_context() {
        Some(name) => { return Some(name); },
        None => { log::error!("WGPU init error"); }
    }

    None
}
