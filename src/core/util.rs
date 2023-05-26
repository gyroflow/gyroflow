// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use std::fs::File;
use std::io::Result;

#[no_mangle]
pub static NvOptimusEnablement: i32 = 1;
#[no_mangle]
pub static AmdPowerXpressRequestHighPerformance: i32 = 1;

pub fn get_video_metadata(filepath: &str) -> Result<telemetry_parser::util::VideoMetadata> {
    let extensions = ["mp4", "mov", "braw", "insv", "360"];
    if !extensions.into_iter().any(|ext| filepath.to_ascii_lowercase().ends_with(ext)) {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let mut stream = File::open(&filepath)?;
    let filesize = stream.metadata()?.len() as usize;
    telemetry_parser::util::get_video_metadata(&mut stream, filesize)
}

pub fn compress_to_base91<T>(value: &T) -> Option<String>
where T: serde::Serialize {
    use std::io::Write;

    let data = bincode::serialize(value).ok()?;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(&data).ok()?;
    let compressed = e.finish().ok()?;

    String::from_utf8(base91::slice_encode(&compressed)).ok()
}

pub fn decompress_from_base91(base91: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    if base91.is_empty() { return None; }

    let compressed = base91::slice_decode(base91.as_bytes());
    let mut e = flate2::read::ZlibDecoder::new(&compressed[..]);

    let mut decompressed = Vec::new();
    e.read_to_end(&mut decompressed).ok()?;
    Some(decompressed)
}

pub fn path_to_str(path: &std::path::Path) -> String {
    path.to_string_lossy().replace("\\", "/")
}


use std::collections::BTreeMap;
pub trait MapClosest<V> {
    fn get_closest(&self, key: &i64, max_diff: i64) -> Option<&V>;
}
impl<V> MapClosest<V> for BTreeMap<i64, V> {
    fn get_closest(&self, key: &i64, max_diff: i64) -> Option<&V> {
        if self.is_empty() { return None; };
        if self.contains_key(key) { return self.get(key); };

        let r1 = self.range(..key);
        let mut r2 = self.range(key..);

        let f = r1.last();
        let b = r2.next();
        let bd = (key - b.map(|v| *v.0).unwrap_or(-99999)).abs();
        let fd = (key - f.map(|v| *v.0).unwrap_or(-99999)).abs();

        if b.is_some() && bd < max_diff && bd < fd {
            Some(b.unwrap().1)
        } else if f.is_some() && fd < max_diff && fd < bd {
            Some(f.unwrap().1)
        } else {
            None
        }
    }
}
pub fn merge_json(a: &mut serde_json::Value, b: &serde_json::Value) {
    use serde_json::Value;
    match (a, b) {
        (Value::Object(ref mut a), &Value::Object(ref b)) => {
            for (k, v) in b {
                merge_json(a.entry(k).or_insert(Value::Null), v);
            }
        }
        (Value::Array(ref mut a), &Value::Array(ref b)) => {
            a.extend(b.clone());
        }
        (Value::Array(ref mut a), &Value::Object(ref b)) => {
            a.extend([Value::Object(b.clone())]);
        }
        (a, b) => {
            *a = b.clone();
        }
    }
}

pub fn get_setting(key: &str) -> Option<String> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::System::Registry::*;
        use windows::Win32::Foundation::NO_ERROR;
        use windows::core::PCSTR;
        let mut hkey = HKEY::default();
        if RegOpenKeyExA(HKEY_CURRENT_USER, PCSTR::from_raw("Software\\Gyroflow\\Gyroflow\0".as_ptr()), 0, KEY_READ, &mut hkey) == NO_ERROR {
            let key = format!("{}\0", key);
            let key = PCSTR::from_raw(key.as_ptr());
            let mut size: u32 = 0;
            let mut typ = REG_VALUE_TYPE::default();
            if RegQueryValueExA(hkey, key, None, None, None, Some(&mut size)) == NO_ERROR {
                if size > 0 {
                    let mut buf: Vec<u8> = vec![0u8; size as usize];
                    if RegQueryValueExA(hkey, key, None, Some(&mut typ), Some(buf.as_mut_ptr()), Some(&mut size)) == NO_ERROR {
                        if typ == REG_SZ {
                            if let Ok(v) = std::str::from_utf8(&buf[..size as usize - 1]) {
                                RegCloseKey(hkey);
                                return Some(v.to_owned());
                            }
                        }
                    }
                }
            }
            RegCloseKey(hkey);
        }
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    unsafe {
        use core_foundation_sys::{ base::*, string::*, propertylist::* };
        extern "C" {
            pub fn CFPreferencesCopyValue(key: CFStringRef, applicationID: CFStringRef, userName: CFStringRef, hostName: CFStringRef) -> CFPropertyListRef;
        }
        unsafe fn cfstr(v: &str) -> CFStringRef {
            CFStringCreateWithBytes(kCFAllocatorDefault, v.as_ptr(), v.len() as CFIndex, kCFStringEncodingUTF8, false as Boolean)
        }
        let key = cfstr(key);
        let app = cfstr("com.gyroflow-xyz.Gyroflow");
        let user = cfstr("kCFPreferencesCurrentUser");
        let host = cfstr("kCFPreferencesAnyHost");
        let ret = CFPreferencesCopyValue(key, app, user, host);
        CFRelease(key as CFTypeRef);
        CFRelease(app as CFTypeRef);
        CFRelease(user as CFTypeRef);
        CFRelease(host as CFTypeRef);
        if !ret.is_null() {
            let typ = CFGetTypeID(ret);
            if typ == CFStringGetTypeID() {
                let c_string = CFStringGetCStringPtr(ret as CFStringRef, kCFStringEncodingUTF8);
                if !c_string.is_null() {
                    let v = std::ffi::CStr::from_ptr(c_string).to_string_lossy().to_string();
                    CFRelease(ret as CFTypeRef);
                    return Some(v);
                } else {
                    CFRelease(ret as CFTypeRef);
                }
            } else {
                CFRelease(ret as CFTypeRef);
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let key = format!("{}=", key);
        if let Ok(home) = std::env::var("HOME") {
            if let Ok(contents) = std::fs::read_to_string(format!("{home}/.config/Gyroflow/Gyroflow.conf")) {
                for line in contents.lines()  {
                    if line.starts_with(&key) {
                        return Some((&line[key.len()..]).trim().trim_matches('"').replace("\\\"", "\"").to_string());
                    }
                }
            }
        }
    }
    None
}

/*
pub fn rename_calib_videos() {
    use telemetry_parser::Input;
    use walkdir::WalkDir;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use crate::CameraIdentifier;
    WalkDir::new("G:/clips/calibration/GoPro/Hero11/").into_iter().for_each(|e| {
        if let Ok(entry) = e {
            let f_name = entry.path().to_string_lossy().replace('\\', "/");
            if f_name.ends_with(".MP4") {
                let (w, h, fps, _dur) = get_video_metadata(&f_name).unwrap();
                let mut stream = File::open(&f_name).unwrap();
                let filesize = stream.metadata().unwrap().len() as usize;

                let input = Input::from_stream(&mut stream, filesize, &f_name, |_|(), Arc::new(AtomicBool::new(false))).unwrap();

                let camera_identifier = CameraIdentifier::from_telemetry_parser(&input, w as usize, h as usize, fps);
                if let Ok(id) = camera_identifier {
                    let mut add = 0;
                    let mut adds = String::new();
                    loop {
                        let path = std::path::Path::new(&f_name).with_file_name(format!("{}{}.mp4", id.identifier, adds));
                        if path.exists() {
                            add += 1;
                            adds = format!(" - {}", add);
                            continue;
                        }
                        let _ = std::fs::rename(std::path::Path::new(&f_name), path);
                        break;
                    }
                    println!("{}: {}", f_name, id.identifier);
                } else {
                    println!("ERROR UNKNOWN ID {}", f_name);
                }
            }
        }
    });
}
*/
