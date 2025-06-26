// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::SeqCst;
pub struct REDSdk { }

impl REDSdk {
    pub fn is_installed() -> bool {
        if let Ok(path) = super::SDK_PATH.as_ref() {
            let mut path = path.clone();
            path.push("_");
            if cfg!(target_os = "windows") {
                return
                    path.with_file_name("REDDecoder-x64.dll").exists() &&
                    path.with_file_name("REDR3D-x64.dll").exists() &&
                    path.with_file_name("REDOpenCL-x64.dll").exists() &&
                    path.with_file_name("REDCuda-x64.dll").exists();
            } else if cfg!(target_os = "macos") {
                return
                    path.with_file_name("REDDecoder.dylib").exists() &&
                    path.with_file_name("REDMetal.dylib").exists() &&
                    path.with_file_name("REDOpenCL.dylib").exists() &&
                    path.with_file_name("REDR3D.dylib").exists();
            } else if cfg!(target_os = "linux") {
                return
                    path.with_file_name("REDCuda-x64.so").exists() &&
                    path.with_file_name("REDDecoder-x64.so").exists() &&
                    path.with_file_name("REDOpenCL-x64.so").exists() &&
                    path.with_file_name("REDR3D-x64.so").exists();
            }
        }

        // Platform not supported so don't ask for download
        true
    }

    pub fn get_download_url() -> Option<&'static str> {
        if cfg!(target_os = "windows") {
            Some("https://api.gyroflow.xyz/sdk/RED_SDK_Windows_8.6.0.tar.gz")
        } else if cfg!(target_os = "macos") {
            Some("https://api.gyroflow.xyz/sdk/RED_SDK_MacOS_8.6.0.tar.gz")
        } else if cfg!(target_os = "linux") {
            Some("https://api.gyroflow.xyz/sdk/RED_SDK_Linux_8.6.0.tar.gz")
        } else {
            None
        }
    }

    // Assumes regular filesystem
    pub fn find_redline() -> String {
        let locations = if cfg!(target_os = "windows") {
            vec![
                "C:/Program Files/REDCINE-X PRO One-Off 64-bit/REDline.exe",
                "C:/Program Files/REDCINE-X PRO 64-bit/REDline.exe",
                "REDline.exe",
            ]
        } else if cfg!(target_os = "macos") {
            vec![
                "/Applications/REDCINE-X Professional/REDCINE-X PRO.app/Contents/MacOS/REDline",
                "REDline",
            ]
        } else if cfg!(target_os = "linux") {
            vec!["REDline"]
        } else {
            vec![]
        };

        if let Some(paths) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&paths) {
                let full_path = dir.join("REDline");
                if full_path.is_file() {
                    if let Some(full_path) = full_path.to_str() {
                        return full_path.to_string();
                    }
                }
            }
        }

        for l in locations {
            if let Ok(p) = std::fs::canonicalize(l) {
                if p.exists() {
                    if let Some(p) = p.to_str() {
                        return p.to_string();
                    }
                }
            }
        }

        String::new()
    }

    // Assumes regular filesystem
    pub fn convert_r3d<F: FnMut((f64, String, String))>(url: &str, format: i32, force_primary: bool, gamma: i32, space: i32, additional_params: &str, mut progress: F, cancel_flag: Arc<AtomicBool>) {
        let redline = Self::find_redline();
        if !redline.is_empty() {
            let p = std::path::Path::new(&gyroflow_core::filesystem::url_to_path(url)).to_owned();

            let output_file = p.with_extension("").to_string_lossy().into_owned();

            cancel_flag.store(false, SeqCst);

            use std::process::{ Command, Stdio };
            use std::io::{ BufRead, BufReader, Error, ErrorKind, Result };
            let re_output_name = regex::Regex::new(r#"ProRes Output Filename: (.+?), Codec:"#).unwrap();
            let re_progress    = regex::Regex::new(r#"Export Job frame complete. [0-9]+ ([0-9\.]+)"#).unwrap();

            let result = (|| -> Result<()> {
                let mut cmd = Command::new(redline);
                #[cfg(target_os = "windows")]
                { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); } // CREATE_NO_WINDOW

                cmd
                    .args(["-i", &p.to_string_lossy()])
                    .args(["-o", &output_file])
                    .args(["--format", "201"])
                    .args(["--PRcodec", &format!("{}", format)])
                    .args(["--useMeta", "--metaIgnoreFrameGuide", "--fit", "3"])
                    .args(["--useRMD", "2"])
                    .args(["--gammaCurve", &format!("{}", gamma)])
                    .args(["--colorSpace", &format!("{}", space)]);
                if force_primary {
                    cmd.args(["--primaryDev"]);
                }
                if !additional_params.is_empty() {
                    cmd.args(additional_params.split_whitespace());
                }
                let mut child = cmd
                    .stderr(Stdio::piped())
                    .spawn()?;

                let stderr = child.stderr.take().ok_or_else(|| Error::new(ErrorKind::Other, "Could not capture the command output."))?;

                let reader = BufReader::new(stderr);
                let mut out_filename = None;
                let mut any_progress = false;

                for line in reader.lines().flatten() {
                    if let Some(m) = re_output_name.captures(&line) {
                        out_filename = Some(gyroflow_core::filesystem::path_to_url(m.get(1).unwrap().as_str()));
                    }
                    if let Some(m) = re_progress.captures(&line) {
                        if let Ok(p) = m.get(1).unwrap().as_str().parse::<f64>() {
                            progress((p, String::new(), out_filename.clone().unwrap_or_default()));
                            any_progress = true;
                        }
                    }
                    ::log::debug!("REDline: {}", line);
                    if cancel_flag.load(SeqCst) {
                        child.kill()?;
                        break;
                    }
                }
                if !any_progress || out_filename.is_none() {
                    progress((1.0, "REDline failed to convert the file. See gyroflow.log for full REDline output and error messages.".into(), out_filename.unwrap_or_default()));
                } else {
                    progress((1.0, String::new(), out_filename.unwrap_or_default()));
                }
                Ok(())
            })();
            if let Err(e) = result {
                progress((1.0, format!("An error occured: {:?}", e.to_string()), String::new()))
            }
        }
    }
}
