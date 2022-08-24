// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use std::fs::File;
use std::io::Result;

pub fn get_video_metadata(filepath: &str) -> Result<(usize, usize, f64, f64)> { // -> (width, height, fps, duration_s)
    let mut stream = File::open(&filepath)?;
    let filesize = stream.metadata().unwrap().len() as usize;
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

/*
pub fn rename_calib_videos() {
    use telemetry_parser::Input;
    use walkdir::WalkDir;
    use std::fs::File;
    WalkDir::new("E:/clips/GoPro/calibration/").into_iter().for_each(|e| {
        if let Ok(entry) = e {
            let f_name = entry.path().to_string_lossy().replace('\\', "/");
            if f_name.ends_with(".MP4") {
                let (w, h, fps) = util::get_video_metadata(&f_name).unwrap();
                let mut stream = File::open(&f_name).unwrap();
                let filesize = stream.metadata().unwrap().len() as usize;

                let input = Input::from_stream(&mut stream, filesize, &f_name).unwrap();

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
                        std::fs::rename(std::path::Path::new(&f_name), path);
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
