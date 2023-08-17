// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

pub struct FfmpegGpl { }

impl FfmpegGpl {
    pub fn is_installed() -> bool {
        if cfg!(any(target_os = "windows", target_os = "macos", target_os = "linux")) {
            let x264 = ffmpeg_next::encoder::find_by_name("libx264");
            let x265 = ffmpeg_next::encoder::find_by_name("libx265");

            return x264.is_some() && x265.is_some();
        }

        // Platform not supported so don't ask for download
        true
    }

    pub fn get_download_url() -> Option<&'static str> {
        if cfg!(target_os = "windows") {
            Some("https://api.gyroflow.xyz/sdk/ffmpeg_gpl_Windows.tar.gz")
        } else if cfg!(target_os = "macos") {
            Some("https://api.gyroflow.xyz/sdk/ffmpeg_gpl_MacOS.tar.gz")
        } else if cfg!(target_os = "linux") {
            Some("https://api.gyroflow.xyz/sdk/ffmpeg_gpl_Linux.tar.gz")
        } else {
            None
        }
    }
}

// https://sourceforge.net/projects/avbuild/files/windows-desktop/ffmpeg-6.0-windows-desktop-vs2022-gpl-lite.7z/download
// https://sourceforge.net/projects/avbuild/files/macOS/ffmpeg-6.0-macOS-gpl-lite.tar.xz/download
// https://sourceforge.net/projects/avbuild/files/linux/ffmpeg-6.0-linux-clang-gpl-lite.tar.xz/download
