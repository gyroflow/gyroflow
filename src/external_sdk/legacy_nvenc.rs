// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2025 Adrian <adrian.eddy at gmail>

pub struct LegacyNvenc { }

impl LegacyNvenc {
    pub fn is_installed() -> bool {
        use nvml_wrapper::Nvml;
        if let Ok(nvml) = Nvml::init() {
            if let Ok(ver) = nvml.sys_driver_version() {
                log::info!("NVIDIA driver version: {ver}");
                if let Some((major, minor)) = ver.split_once('.') {
                    if let Ok(major) = major.parse::<u32>() {
                        // If driver is older than 522, we need legacy avcodec, because NVENC12 breaks ABI
                        if major < 522 {
                            return false;
                        }
                    }
                }
            }
        }

        // Platform not supported so don't ask for download
        true
    }

    pub fn get_download_url() -> Option<&'static str> {
        if cfg!(target_os = "windows") {
            Some("https://api.gyroflow.xyz/sdk/avcodec-62-nvenc11-windows.tar.gz")
        } else if cfg!(target_os = "linux") {
            Some("https://api.gyroflow.xyz/sdk/avcodec-62-nvenc11-linux.tar.gz")
        } else {
            None
        }
    }
}
