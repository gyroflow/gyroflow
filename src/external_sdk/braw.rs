// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

pub struct BrawSdk { }

impl BrawSdk {
    pub fn is_installed() -> bool {
        if let Ok(path) = super::SDK_PATH.as_ref() {
            let mut path = path.clone();
            path.push("_");
            if cfg!(target_os = "windows") {
                return
                    path.with_file_name("BlackmagicRawAPI.dll").exists() &&
                    path.with_file_name("DecoderCUDA.dll").exists() &&
                    path.with_file_name("DecoderOpenCL.dll").exists() &&
                    path.with_file_name("InstructionSetServicesAVX.dll").exists() &&
                    path.with_file_name("InstructionSetServicesAVX2.dll").exists();
            } else if cfg!(target_os = "macos") {
                return path.with_file_name("BlackmagicRawAPI.framework").exists();
            } else if cfg!(target_os = "linux") {
                return
                    path.with_file_name("libBlackmagicRawAPI.so").exists() &&
                    path.with_file_name("libDecoderCUDA.so").exists() &&
                    path.with_file_name("libDecoderOpenCL.so").exists() &&
                    path.with_file_name("libInstructionSetServicesAVX.so").exists() &&
                    path.with_file_name("libInstructionSetServicesAVX2.so").exists();
            }
        }

        // Platform not supported so don't ask for download
        true
    }

    pub fn get_download_url() -> Option<&'static str> {
        if cfg!(target_os = "windows") {
            Some("https://api.gyroflow.xyz/sdk/Blackmagic_RAW_SDK_Windows_5.0.0.tar.gz")
        } else if cfg!(target_os = "macos") {
            Some("https://api.gyroflow.xyz/sdk/Blackmagic_RAW_SDK_MacOS_5.0.0.tar.gz")
        } else if cfg!(target_os = "linux") {
            Some("https://api.gyroflow.xyz/sdk/Blackmagic_RAW_SDK_Linux_5.0.0.tar.gz")
        } else {
            None
        }
    }
}
