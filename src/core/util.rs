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
    let mut file = crate::filesystem::open_file(&base, &url, false, false)?;
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

pub fn init_telemetry_parser() {
    use telemetry_parser::filesystem as tp_fs;
    fn telemetry_parser_open_file<'a>(base: &'a tp_fs::FilesystemBase, path: &str) -> std::io::Result<tp_fs::FileWrapper<'a>> {
        match crate::filesystem::open_file(&base, path, false, false) {
            Ok(file) => {
                let size = file.size;
                return Ok(tp_fs::FileWrapper { file: Box::new(file), size });
            }
            Err(e) => {
                log::error!("Failed to open file: {e:?}");
                return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
            }
        }
    }

    static TP_INITED: std::sync::Once = std::sync::Once::new();
    TP_INITED.call_once(|| {
        unsafe {
            tp_fs::set_filesystem_functions(tp_fs::FilesystemFunctions {
                get_filename: crate::filesystem::get_filename,
                get_folder:   crate::filesystem::get_folder,
                list_folder:  crate::filesystem::list_folder,
                open_file:    telemetry_parser_open_file
            });
        }
    });
}

pub fn map_coord<T>(x: T, in_min: T, in_max: T, out_min: T, out_max: T) -> T
where T: std::ops::Sub<Output = T> + std::ops::Mul<Output = T> + std::ops::Div<Output = T> + std::ops::Add<Output = T> + Copy {
    return (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min;
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
