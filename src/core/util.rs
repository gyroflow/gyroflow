// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use std::io::Result;

#[no_mangle]
pub static NvOptimusEnablement: i32 = 1;
#[no_mangle]
pub static AmdPowerXpressRequestHighPerformance: i32 = 1;

pub fn get_video_metadata(url: &str) -> std::result::Result<telemetry_parser::util::VideoMetadata, crate::GyroflowCoreError> {
    let filename = crate::filesystem::get_filename(url);
    let extensions = ["mp4", "mov", "braw", "insv", "360", "mxf"];
    if !extensions.into_iter().any(|ext| filename.to_ascii_lowercase().ends_with(ext)) {
        return Err(crate::GyroflowCoreError::UnsupportedFormat(filename));
    }
    let base = crate::filesystem::get_engine_base();
    let mut file = crate::filesystem::open_file(&base, &url, false)?;
    let filesize = file.size;
    Ok(telemetry_parser::util::get_video_metadata(file.get_file(), filesize)?)
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

pub fn compress_to_base91_cbor<T>(value: &T) -> Option<String>
where T: serde::Serialize {
    use std::io::Write;

    let mut data = Vec::<u8>::new();
    ciborium::into_writer(value, &mut data).ok()?;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(&data).ok()?;
    let compressed = e.finish().ok()?;

    String::from_utf8(base91::slice_encode(&compressed)).ok()
}

pub fn decompress_from_base91_cbor<'de, T>(base91: &str) -> Result<T>
where T: serde::de::DeserializeOwned {
    use std::io::Read;
    if base91.is_empty() { return Err(std::io::ErrorKind::NotFound.into()); }

    let compressed = base91::slice_decode(base91.as_bytes());
    let mut e = flate2::read::ZlibDecoder::new(&compressed[..]);

    let mut decompressed = Vec::new();
    e.read_to_end(&mut decompressed)?;
    ciborium::from_reader(std::io::Cursor::new(decompressed)).map_err(|x| std::io::Error::new(std::io::ErrorKind::Other, format!("{x:?}")))
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
        use windows::core::PCWSTR;

        // If Gyroflow was installed from Microsoft Store, then we need to read from it's sandboxed registry file
        {
            let packages_path = format!("{}/Packages/", std::env::var("LOCALAPPDATA").unwrap_or_default()).replace('\\', "/");
            for e in walkdir::WalkDir::new(&packages_path).into_iter() {
                if let Ok(entry) = e {
                    let f_name = entry.file_name().to_string_lossy();
                    if f_name.starts_with("29160AdrianRoss.Gyroflow") {
                        if let Ok(buffer) = std::fs::read(format!("{packages_path}{f_name}/SystemAppData/Helium/User.dat")) {
                            if let Ok(hive) = nt_hive::Hive::new(buffer.as_ref()) {
                                if let Ok(root) = hive.root_key_node() {
                                    if let Some(Ok(key_node)) = root.subpath("Software\\Gyroflow\\Gyroflow") {
                                        if let Some(Ok(key_value)) = key_node.value(key) {
                                            if let Ok(sz) = key_value.string_data() {
                                                return Some(sz);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        let mut hkey = HKEY::default();
        let key_path = "Software\\Gyroflow\\Gyroflow\0".encode_utf16().collect::<Vec<_>>();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR::from_raw(key_path.as_ptr()), 0, KEY_READ, &mut hkey).is_ok() {
            let key = format!("{}\0", key).encode_utf16().collect::<Vec<_>>();
            let key = PCWSTR::from_raw(key.as_ptr());
            let mut size: u32 = 0;
            let mut typ = REG_VALUE_TYPE::default();
            if RegQueryValueExW(hkey, key, None, None, None, Some(&mut size)).is_ok() {
                if size > 0 {
                    let mut buf: Vec<u8> = vec![0u8; size as usize];
                    if RegQueryValueExW(hkey, key, None, Some(&mut typ), Some(buf.as_mut_ptr()), Some(&mut size)).is_ok() {
                        if typ == REG_SZ {
                            let u8buf = &buf[..size as usize - 1];
                            let u16buf = std::slice::from_raw_parts(u8buf.as_ptr() as *const u16, u8buf.len() / 2);
                            if let Ok(v) = String::from_utf16(u16buf) {
                                let _ = RegCloseKey(hkey);
                                return Some(v.to_owned());
                            }
                        }
                    }
                }
            }
            let _ = RegCloseKey(hkey);
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
