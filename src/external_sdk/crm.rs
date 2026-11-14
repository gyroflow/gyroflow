// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

// Canon Cinema RAW Light (.CRM), decoded by MDK's mdk-crm plugin through the Canon SDK.
// The package holds the SDK libraries and its CanonLUTs folder, used for colour-space and HDR conversions.
pub struct CanonSdk { }

impl CanonSdk {
    pub fn is_installed() -> bool {
        if let Ok(path) = super::SDK_PATH.as_ref() {
            let mut path = path.clone();
            path.push("_");
            if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
                return
                    path.with_file_name("CrmSdk.dll").exists() &&
                    path.with_file_name("crxdec.dll").exists() &&
                    path.with_file_name("CrmRsc1.bin").exists() &&
                    path.with_file_name("CanonLUTs").exists();
            } else if cfg!(target_os = "macos") {
                return
                    path.with_file_name("CrmSdk.framework").exists() &&
                    path.with_file_name("CanonLUTs").exists();
            } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                return
                    path.with_file_name("libCrmSdk.so").exists() &&
                    path.with_file_name("libcrxdec.so.2.0").exists() &&
                    path.with_file_name("CrmRsc1.bin").exists() &&
                    path.with_file_name("CanonLUTs").exists();
            }
        }

        // No Canon SDK for this platform, so don't ask for download
        true
    }

    pub fn get_download_url(sdk_base: &str) -> Option<String> {
        let filename = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            "Canon_CRM_SDK_Windows_2.10.0.tar.gz"
        } else if cfg!(target_os = "macos") {
            "Canon_CRM_SDK_MacOS_2.10.0.tar.gz"
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            "Canon_CRM_SDK_Linux_2.10.0.tar.gz"
        } else {
            return None;
        };

        if !sdk_base.is_empty() {
            Some(format!("{}/{}", sdk_base.trim_end_matches('/'), filename))
        } else {
            Some(format!("https://api.gyroflow.xyz/sdk/{}", filename))
        }
    }
}
