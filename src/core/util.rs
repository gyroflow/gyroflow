

use std::fs::File;
use std::io::Result;
use std::io::ErrorKind;

pub fn get_video_metadata(filepath: &str) -> Result<(usize, usize, f64)> { // -> (width, height, fps)
    let mut stream = File::open(&filepath)?;
    let filesize = stream.metadata().unwrap().len() as usize;
    let mp = telemetry_parser::util::parse_mp4(&mut stream, filesize)?;
    if !mp.tracks.is_empty() {
        if let Some(ref tkhd) = mp.tracks[0].tkhd {
            let w = tkhd.width >> 16;
            let h = tkhd.height >> 16;
            let matrix = (
                tkhd.matrix.a >> 16,
                tkhd.matrix.b >> 16,
                tkhd.matrix.c >> 16,
                tkhd.matrix.d >> 16,
            );
            let _rotation = match matrix {
                (0, 1, -1, 0) => 90,   // rotate 90 degrees
                (-1, 0, 0, -1) => 180, // rotate 180 degrees
                (0, -1, 1, 0) => 270,  // rotate 270 degrees
                _ => 0,
            };
            let mut fps = 0.0;
            if let Some(ref stts) = mp.tracks[0].stts {
                if !stts.samples.is_empty() {
                    let samples = stts.samples[0].sample_count;
                    let timescale = mp.tracks[0].timescale.unwrap();
                    let duration = mp.tracks[0].duration.unwrap();
                    let duration_us = duration.0 as f64 * 1000_000.0 / timescale.0 as f64;
                    let us_per_frame = duration_us / samples as f64;
                    fps = 1000_000.0 / us_per_frame;
                }
            }
            return Ok((w as usize, h as usize, fps));
        }
    }
    Err(ErrorKind::Other.into())
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
