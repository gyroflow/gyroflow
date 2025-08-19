// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2025 Adrian <adrian.eddy at gmail>

pub struct LegacyNvenc { }

impl LegacyNvenc {
    pub fn is_installed() -> bool {
        use nvml_wrapper::Nvml;
        if let Ok(nvml) = Nvml::init() {
            if let Ok(mut ver) = nvml.sys_driver_version() {
                log::info!("NVIDIA driver version: {ver}");
                if ver.split('.').count() < 3 {
                    ver.push_str(".0");
                }
                let min_version = if cfg!(target_os = "windows") { "551.76.0" } else { "550.54.14" };
                if let Ok(min_version) = semver::Version::parse(min_version) {
                    if let Ok(this_version) = semver::Version::parse(&ver) {
                        if this_version < min_version {
                            log::error!("NVIDIA driver version {ver} is too old, minimum required is {min_version}");
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
