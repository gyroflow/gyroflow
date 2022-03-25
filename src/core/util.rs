

use std::fs::File;
use std::io::Result;

pub fn get_video_metadata(filepath: &str) -> Result<(usize, usize, f64)> { // -> (width, height, fps)
    let mut stream = File::open(&filepath)?;
    let filesize = stream.metadata().unwrap().len() as usize;
    telemetry_parser::util::get_video_metadata(&mut stream, filesize)
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
