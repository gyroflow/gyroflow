mod braw;

use std::io::*;
use std::io;
use gyroflow_core::flate2::read::GzDecoder;

pub fn requires_install(path: &str) -> bool {
    if path.to_lowercase().ends_with(".braw") { return !braw::BrawSdk::is_installed(); }
    false
}

pub fn install<F: Fn((f64, &'static str, String)) + Send + Sync + Clone + 'static>(path: &str, cb: F) {
    let (url, sdk_name) = if path.to_lowercase().ends_with(".braw") {
        (braw::BrawSdk::get_download_url(), "Blackmagic RAW SDK")
    } else {
        (None, "")
    };

    if let Some(url) = url {
        crate::core::run_threaded(move || {
            let result = (|| -> Result<()> {
                log::info!("Downloading from {}", url);
                let reader = ureq::get(url).call().map_err(|e| Error::new(ErrorKind::Other, e.to_string()))?;
                let size = reader.header("content-length").and_then(|x| x.parse::<usize>().ok()).unwrap_or(1).max(1);
                let mut reader = ProgressReader::new(reader.into_reader(), |read| {
                    cb(((read as f64 / size as f64) * 0.5, sdk_name, "".into()));
                });
                let mut buf = Vec::with_capacity(4*1024*1024);
                io::copy(&mut reader, &mut buf)?;

                let mut out_dir = std::env::current_exe()?.parent().ok_or_else(|| Error::new(ErrorKind::Other, "Cannot get exe parent"))?.to_path_buf();
                if cfg!(target_os = "macos") {
                    out_dir.push("../Frameworks/");
                }
                let size = buf.len().max(1) as f64;
                let br = ProgressReader::new(Cursor::new(buf), |read| {
                    cb((0.5 + (read as f64 / size) * 0.5, sdk_name, "".into()));
                });
                let gz = GzDecoder::new(br);
                let mut archive = tar::Archive::new(gz);
                archive.unpack(out_dir)?;
                Ok(())
            })();
            if let Err(e) = result {
                cb((1.0, sdk_name, e.to_string()));
            } else {
                cb((1.0, sdk_name, String::new()));
            }
        });
    } else {
        cb((1.0, sdk_name, "SDK is not available.".to_string()));
    }
}

pub struct ProgressReader<R: Read, C: FnMut(usize)> {
    reader: R,
    callback: C,
    total_read: usize
}
impl<R: Read, C: FnMut(usize)> ProgressReader<R, C> {
    pub fn new(reader: R, callback: C) -> Self {
        Self { reader, callback, total_read: 0 }
    }
}
impl<R: Read, C: FnMut(usize)> Read for ProgressReader<R, C> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.reader.read(buf)?;
        self.total_read += read;
        (self.callback)(self.total_read);
        Ok(read)
    }
}
