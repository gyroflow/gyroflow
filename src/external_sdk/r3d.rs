// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

pub struct REDSdk { }

impl REDSdk {
    pub fn is_installed() -> bool {
        if let Ok(exe_path) = std::env::current_exe() {
            if cfg!(target_os = "windows") {
                return
                    exe_path.with_file_name("REDDecoder-x64.dll").exists() &&
                    exe_path.with_file_name("REDR3D-x64.dll").exists() &&
                    exe_path.with_file_name("REDOpenCL-x64.dll").exists() &&
                    exe_path.with_file_name("REDCuda-x64.dll").exists();
            } else if cfg!(target_os = "macos") {
                if let Some(parent) = exe_path.parent() {
                    let mut parent = parent.to_path_buf();
                    parent.push("../Frameworks/_");
                    return
                        parent.with_file_name("REDDecoder.dylib").exists() &&
                        parent.with_file_name("REDMetal.dylib").exists() &&
                        parent.with_file_name("REDOpenCL.dylib").exists() &&
                        parent.with_file_name("REDR3D.dylib").exists();
                }
            } else if cfg!(target_os = "linux") {
                if let Some(parent) = exe_path.parent() {
                    let mut lib = parent.to_path_buf();
                    lib.push("lib/_");
                    return
                        lib.with_file_name("REDCuda-x64.so").exists() &&
                        lib.with_file_name("REDDecoder-x64.so").exists() &&
                        lib.with_file_name("REDOpenCL-x64.so").exists() &&
                        lib.with_file_name("REDR3D-x64.so").exists();
                }
            }
        }

        // Platform not supported so don't ask for download
        true
    }

    pub fn get_download_url() -> Option<&'static str> {
        if cfg!(target_os = "windows") {
            Some("https://api.gyroflow.xyz/sdk/RED_SDK_Windows.tar.gz")
        } else if cfg!(target_os = "macos") {
            Some("https://api.gyroflow.xyz/sdk/RED_SDK_MacOS.tar.gz")
        } else if cfg!(target_os = "linux") {
            Some("https://api.gyroflow.xyz/sdk/RED_SDK_Linux.tar.gz")
        } else {
            None
        }
    }
}
