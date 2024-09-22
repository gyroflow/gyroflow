// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

// TODO: macos sandbox

use zip_extensions::*;
use std::io::{ self, Cursor };
use std::process::Command;
use std::path::Path;

pub fn get_path(typ: &str) -> &'static str {
    if cfg!(target_os = "windows") {
        if typ == "openfx" {
            return "C:/Program Files/Common Files/OFX/Plugins/Gyroflow.ofx.bundle";
        } else if typ == "adobe" {
            return "C:/Program Files/Adobe/Common/Plug-ins/7.0/MediaCore/Gyroflow-Adobe-windows.aex";
        }
    } else if cfg!(target_os = "macos") {
        if typ == "openfx" {
            return "/Library/OFX/Plugins/Gyroflow.ofx.bundle";
        } else if typ == "adobe" {
            return "/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore/Gyroflow.plugin";
        }
    }
    ""
}

#[cfg(target_os = "windows")]
fn query_file_version(path: &str) -> Option<String> {
    use windows::{ Win32::Storage::FileSystem::{ GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW }, core::HSTRING };
    unsafe {
        let hpath = HSTRING::from(path);
        let size = GetFileVersionInfoSizeW(&hpath, None) as usize;
        if size == 0 {
            return None;
        }
        let mut buffer: Vec<u16> = vec![0; size];
        GetFileVersionInfoW(&hpath, 0, buffer.len() as u32, buffer.as_mut_ptr() as _).expect("get file version info failed.");
        let pblock = buffer.as_ptr() as _;
        let lang_id = {
            let mut buffer = std::ptr::null_mut();
            let mut len = 0;
            if VerQueryValueW(pblock, &HSTRING::from("\\VarFileInfo\\Translation"), &mut buffer as _, &mut len).as_bool() {
                let ret = *(buffer as *mut i32);
                ((ret & 0xffff) << 16) + (ret >> 16)
            } else {
                0x040904E4
            }
        };

        unsafe fn file_version_item(pblock: *const std::ffi::c_void, lang_id: i32, version_detail: &str) -> Option<String> {
            let mut buffer = std::ptr::null_mut();
            let mut len = 0;
            let ok = VerQueryValueW(pblock, &HSTRING::from(format!("\\\\StringFileInfo\\\\{lang_id:08x}\\\\{version_detail}")), &mut buffer, &mut len);
            if ok == false || len == 0 {
                return None;
            }
            let raw = std::slice::from_raw_parts(buffer.cast(), len as usize);
            match raw.iter().position(|&c| c == 0) {
                Some(null_pos) => Some(String::from_utf16_lossy(&raw[..null_pos])),
                None => Some(String::from_utf16_lossy(raw)),
            }
        }

        file_version_item(pblock, lang_id, "ProductVersion")
    }
}

fn query_file_version_from_plist(path: &str) -> Option<String> {
    let file = std::fs::read_to_string(path).ok()?;
    let re = regex::Regex::new(r#"<key>CFBundleShortVersionString</key>\s*<string>([^<]+)</string>"#).unwrap();
    let cap = re.captures(&file)?;
    Some(cap.get(1)?.as_str().to_owned())
}

fn copy_files(tempdir: &str, extract_path: &str) -> io::Result<()> {
    let output = if cfg!(target_os = "windows") {
        Command::new("xcopy").args(&[tempdir, extract_path, "/Y", "/E", "/H", "/I"]).output()
    } else if cfg!(target_os = "macos") {
        Command::new("osascript").args(&["-e", &format!("do shell script \"mkdir -p \\\"{extract_path}\\\" ; cp -Rf \\\"{tempdir}\\\" \\\"{extract_path}\\\"\"")]).output()
    } else {
        return Err(io::Error::new(io::ErrorKind::Other, "Unsupported OS"));
    }?;
    // let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        Ok(())
    } else {
        // Retry with elevated privileges
        let status = if cfg!(target_os = "windows") {
            runas::Command::new("xcopy").args(&[tempdir, extract_path, "/Y", "/E", "/H", "/I"]).status()
        } else if cfg!(target_os = "macos") {
            Command::new("osascript").args(&["-e", &format!("do shell script \"mkdir -p \\\"{extract_path}\\\" ; cp -Rf \\\"{tempdir}\\\" \\\"{extract_path}\\\"\" with administrator privileges")]).status()
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "Unsupported OS"));
        }?;

        if status.success() {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "Failed to copy directory with elevated privileges"))
        }
    }
}

pub fn install(typ: &str) -> io::Result<String> {
    let nightly_base = "https://nightly.link/gyroflow/gyroflow-plugins/workflows/release/main/";
    let release_base = "https://github.com/gyroflow/gyroflow-plugins/releases/latest/download/";
    let (download_url, extract_path) = match typ {
        "openfx" => {
            if cfg!(target_os = "windows") {
                if is_nightly() { (format!("{nightly_base}Gyroflow-OpenFX-windows.zip"), "C:/Program Files/Common Files/OFX/Plugins/") }
                else            { (format!("{release_base}Gyroflow-OpenFX-windows.zip"), "C:/Program Files/Common Files/OFX/Plugins/") }
            } else {
                if is_nightly() { (format!("{nightly_base}Gyroflow-OpenFX-macos-zip.zip"), "/Library/OFX/Plugins/") }
                else            { (format!("{release_base}Gyroflow-OpenFX-macos.zip"),     "/Library/OFX/Plugins/") }
            }
        }
        "adobe" => {
            if cfg!(target_os = "windows") {
                if is_nightly() { (format!("{nightly_base}Gyroflow-Adobe-windows.zip"), "C:/Program Files/Adobe/Common/Plug-ins/7.0/MediaCore/") }
                else            { (format!("{release_base}Gyroflow-Adobe-windows.aex"), "C:/Program Files/Adobe/Common/Plug-ins/7.0/MediaCore/Gyroflow-Adobe-windows.aex") }
            } else {
                if is_nightly() { (format!("{nightly_base}Gyroflow-Adobe-macos-zip.zip"), "/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore/") }
                else            { (format!("{release_base}Gyroflow-Adobe-macos.zip"),     "/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore/") }
            }
        }
        _ => unreachable!()
    };

    if let Ok(mut reader) = ureq::get(&download_url).call().map(|x| x.into_reader()) {
        let mut content = Vec::new();
        reader.read_to_end(&mut content)?;

        if download_url.ends_with(".zip") {
            let mut archive = zip::ZipArchive::new(Cursor::new(content))?;
            let mut inner = Vec::new();
            let tempdir = tempfile::tempdir()?;

            if archive.name_for_index(0).map(|x| x.ends_with(".zip")).unwrap_or_default() {
                archive.extract_file_to_memory(0, &mut inner)?;
                let mut archive2 = zip::ZipArchive::new(Cursor::new(inner))?;
                archive2.extract(tempdir.path())?;
            } else {
                archive.extract(tempdir.path())?;
            }
            copy_files(tempdir.path().to_str().unwrap(), &extract_path)?;
        } else {
            let tempfile = tempfile::NamedTempFile::new()?;
            std::fs::write(tempfile.path(), content)?;
            copy_files(tempfile.path().to_str().unwrap(), &extract_path)?;
        }
    }
    detect(typ)
}

fn is_nightly() -> bool {
    crate::util::get_version().contains("(gh") || crate::util::get_version().contains("(dev")
}

pub fn is_nle_installed(typ: &str) -> bool {
    use chrono::{ Datelike, Utc };

    match typ {
        "openfx" => {
            if cfg!(target_os = "windows") {
                Path::new(&format!("C:/Users/{}/AppData/Roaming/Blackmagic Design/DaVinci Resolve", whoami::username())).exists()
            } else {
                Path::new("/Applications/DaVinci Resolve/").exists()
            }
        }
        "adobe" => {
            if cfg!(target_os = "windows") {
                Path::new("C:/Program Files/Adobe/Common/Plug-ins/7.0/MediaCore/").exists()
            } else {
                (2022..(Utc::now().year()+1)).any(|y| {
                    Path::new(&format!("/Applications/Adobe Premiere Pro {y}/")).exists() ||
                    Path::new(&format!("/Applications/Adobe After Effects {y}/")).exists()
                })
            }
        }
        _ => unreachable!()
    }
}

pub fn latest_version() -> Option<String> {
    if is_nightly() {
        let body = ureq::get("https://api.github.com/repos/gyroflow/gyroflow-plugins/actions/runs").call().ok()?.into_string().ok()?;
        let v: serde_json::Value = serde_json::from_str(&body).ok()?;
        for obj in v.get("workflow_runs")?.as_array()? {
            let obj = obj.as_object()?;
            if obj.get("conclusion").and_then(|x| x.as_str()) == Some("success") {
                return Some(format!("{}", obj.get("run_number")?.as_u64()?));
            }
        }
    } else {
        let body = ureq::get("https://api.github.com/repos/gyroflow/gyroflow-plugins/releases").call().ok()?.into_string().ok()?;
        let v: Vec<serde_json::Value> = serde_json::from_str(&body).ok()?;
        for obj in v {
            let obj = obj.as_object()?;
            if obj.get("draft").and_then(|x| x.as_bool()) == Some(false) && obj.get("prerelease").and_then(|x| x.as_bool()) == Some(false) {
                return Some(obj.get("tag_name")?.as_str()?.trim_start_matches('v').to_owned());
            }
        }
    }
    None
}

pub fn detect(typ: &str) -> io::Result<String> {
    let path = get_path(typ);
    #[cfg(target_os = "windows")] {
        if !path.is_empty() && Path::new(path).exists() {
            Ok(query_file_version(&if typ == "openfx" { format!("{path}/Contents/Win64/Gyroflow.ofx") } else { path.to_owned() }).unwrap_or_default())
        } else {
            Ok(String::new())
        }
    }
    #[cfg(target_os = "macos")] {
        if Path::new(path).exists() {
            Ok(query_file_version_from_plist(&format!("{path}/Contents/Info.plist")).unwrap_or_default())
        } else {
            Ok(String::new())
        }
    }
}
