// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use std::io::{ Result, Read, Seek };

#[unsafe(no_mangle)]
pub static NvOptimusEnablement: i32 = 1;
#[unsafe(no_mangle)]
pub static AmdPowerXpressRequestHighPerformance: i32 = 1;

/// The video's size, frame rate, duration and rotation, as gyroflow's player decodes the file (`load_video_file`, the
/// plugins' footage): telemetry-parser's, and a Blackmagic RAW clip timed as Blackmagic's SDK presents it
/// (`braw_clip_timing`)
pub fn get_video_metadata<T: Read + Seek>(stream: &mut T, filesize: usize, url: &str) -> std::result::Result<telemetry_parser::util::VideoMetadata, crate::GyroflowCoreError> {
    let filename = crate::filesystem::get_filename(url);
    let extensions = ["mp4", "mov", "braw", "insv", "360", "mxf"];
    if !extensions.into_iter().any(|ext| filename.to_ascii_lowercase().ends_with(ext)) {
        return Err(crate::GyroflowCoreError::UnsupportedFormat(filename));
    }
    let mut md = telemetry_parser::util::get_video_metadata(stream, filesize)?;
    if filename.to_ascii_lowercase().ends_with(".braw") {
        braw_clip_timing(stream, filesize as u64, &mut md);
    }
    Ok(md)
}

/// A Blackmagic RAW clip's frame rate and duration as Blackmagic's SDK presents the clip, which decodes it in gyroflow's
/// player and render queue (MDK's BRAW reader) and in the plugins' hosts (DaVinci Resolve): its frames at the clip's
/// frame rate, the one they're presented at (the most common frame duration), a frame that lasts longer held for its
/// whole frames. An off-speed recording ends with a frame that lasts until the recording's end (2370 frames of 1/24 s,
/// then one of 197.6 s, of an 8 fps recording in a 24 fps clip), which the SDK presents as that frame held (7113 frames
/// at 24 fps), where the frames' average rate (`VideoMetadata::fps`, 8 fps) puts frame `k` at `k / 8` s, not at the
/// `k / 24` s it's presented at. A clip whose frames all last as long, or whose frames aren't evenly spaced from its
/// start, is left as telemetry-parser has it
fn braw_clip_timing<T: Read + Seek>(stream: &mut T, filesize: u64, md: &mut telemetry_parser::util::VideoMetadata) {
    let Some(v) = telemetry_parser::segment_table::video_frame_times(stream, filesize) else { return };
    let Some(&last) = v.starts.last() else { return };
    let even = v.starts.iter().enumerate().all(|(k, t)| *t == k as i64 * v.nominal);
    if v.timescale == 0 || v.nominal <= 0 || !even || v.is_constant() { return; }
    // The frames before the last one, and the last one held for its whole frames, at least itself
    let frames = (v.starts.len() - 1) as f64 + ((v.end - last) / v.nominal).max(1) as f64;
    md.fps = v.timescale as f64 / v.nominal as f64;
    md.duration_s = frames * v.nominal as f64 / v.timescale as f64;
}

pub fn compress_to_base91<T>(value: &T) -> Option<String>
where T: serde::Serialize {
    use std::io::Write;

    let data = bincode::serde::encode_to_vec(value, bincode::config::legacy()).ok()?;
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
    /// The entry closest to `key` and less than `max_diff` away (one at `key` itself always), the later of two as close
    fn get_closest(&self, key: &i64, max_diff: i64) -> Option<&V>;
    /// `get_closest` among the entries of the keys `keys` that `pred` takes
    fn get_closest_in(&self, keys: std::ops::RangeInclusive<i64>, key: i64, max_diff: i64, pred: impl Fn(&V) -> bool) -> Option<&V>;
}
impl<V> MapClosest<V> for BTreeMap<i64, V> {
    fn get_closest(&self, key: &i64, max_diff: i64) -> Option<&V> {
        self.get_closest_in(i64::MIN..=i64::MAX, *key, max_diff, |_| true)
    }
    fn get_closest_in(&self, keys: std::ops::RangeInclusive<i64>, key: i64, max_diff: i64, pred: impl Fn(&V) -> bool) -> Option<&V> {
        // Only the entries within `max_diff` are looked through: the closest ones `pred` doesn't take may be followed by
        // many more it doesn't
        let max_diff = max_diff.max(0);
        let (lo, hi) = (key.saturating_sub(max_diff).max(*keys.start()), key.saturating_add(max_diff).min(*keys.end()));

        // A range whose start is past its end panics: past the last key nothing is after, before the first nothing before.
        // The two ranges overlap on an exact key hit; the tie-break below then picks that same entry
        let before = (lo <= key.min(hi)).then(|| self.range(lo..=key.min(hi)).rev().find(|(_, v)| pred(v))).flatten();
        let after  = (key.max(lo) <= hi).then(|| self.range(key.max(lo)..=hi).find(|(_, v)| pred(v))).flatten();

        // `abs_diff` returns an u64 and can't overflow, unlike `(key - other).abs()`
        let closest = match (before, after) {
            (Some(before), Some(after)) => if key.abs_diff(*after.0) <= key.abs_diff(*before.0) { after } else { before },
            (Some(before), None)        => before,
            (None,         Some(after)) => after,
            (None,         None)        => return None
        };

        // The ranges above are inclusive on both ends, so an entry exactly `max_diff` away still needs rejecting.
        // Anything farther than the closest entry is out of range as well, so one check is enough
        let distance = key.abs_diff(*closest.0);
        (distance == 0 || distance < max_diff as u64).then_some(closest.1)
    }
}
pub fn merge_json(a: &mut serde_json::Value, b: &serde_json::Value) {
    use serde_json::Value;
    match (a, b) {
        (Value::Object(a), &Value::Object(ref b)) => {
            for (k, v) in b {
                merge_json(a.entry(k).or_insert(Value::Null), v);
            }
        }
        (Value::Array(a), &Value::Array(ref b)) => {
            a.extend(b.clone());
        }
        (Value::Array(a), &Value::Object(ref b)) => {
            a.extend([Value::Object(b.clone())]);
        }
        (a, b) => {
            *a = b.clone();
        }
    }
}

pub fn init_telemetry_parser() {
    use telemetry_parser::filesystem as tp_fs;
    fn telemetry_parser_open_file(path: &str) -> std::io::Result<tp_fs::FileWrapper> {
        match crate::filesystem::open_file(path, false, false) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_closest_entry_is_less_than_max_diff_away_and_the_later_of_two() {
        let map: BTreeMap<i64, char> = [(0, 'a'), (100, 'b'), (200, 'c'), (1000, 'd')].into_iter().collect();
        assert_eq!(map.get_closest(&150, 60), Some(&'c'), "the later of two as close");
        assert_eq!(map.get_closest(&140, 60), Some(&'b'));
        assert_eq!(map.get_closest(&600, 400), None, "exactly max_diff away is out");
        assert_eq!(map.get_closest(&600, 401), Some(&'d'));
        assert_eq!(map.get_closest(&100, 0), Some(&'b'), "an entry at the key itself always");
        assert_eq!(map.get_closest(&-5, 10), Some(&'a'));
        assert_eq!(map.get_closest(&2000, i64::MAX), Some(&'d'));
    }

    #[test]
    fn a_closest_entry_in_keys_is_one_of_them_that_pred_takes() {
        let map: BTreeMap<i64, char> = [(0, 'a'), (100, 'b'), (200, 'c'), (1000, 'd')].into_iter().collect();
        assert_eq!(map.get_closest_in(i64::MIN..=i64::MAX, 110, 500, |v| *v != 'b'), Some(&'c'));
        assert_eq!(map.get_closest_in(i64::MIN..=150, 190, 500, |_| true), Some(&'b'), "past the last key: the last one");
        assert_eq!(map.get_closest_in(300..=900, 500, 500, |_| true), None);
        assert_eq!(map.get_closest_in(500..=100, 100, 500, |_| true), None, "no keys");
        assert_eq!(map.get_closest_in(150..=2000, 0, 300, |_| true), Some(&'c'), "before the first key: the first one");
    }
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
