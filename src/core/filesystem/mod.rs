// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

#[cfg(target_os = "android")]
pub mod android;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub mod apple;

use std::fs::*;
use std::io::{ Read, Write };
use std::path::*;

pub type Result<T> = std::result::Result<T, FilesystemError>;

#[cfg(target_os = "android")]
pub type EngineBase = jni::JavaVM;
#[cfg(not(target_os = "android"))]
pub type EngineBase = ();

#[cfg(target_os = "android")]
pub fn get_engine_base() -> EngineBase { android::get_jvm() }
#[cfg(not(target_os = "android"))]
pub fn get_engine_base() -> EngineBase { () }

// Filesystem assumptions:
// 1. All file access should be based on URLs. On Desktop it will be file:///, but on mobile it can be arbitrary
// 2. URL can be arbitrary and doesn't have to contain any names
// 3. We can have access to a folder via URL, but we can't assume it's a path
// 4. We can create new files in a folder and get an URL to that file, and that URL can be arbitrary
// 5. We can have access to file data only, without any info about it's location, folder, parent or other files in the same folder

// Testing:
// - Dragging video file
// - Dragging .gyroflow file
// - Dragging .gyroflow preset
// - Opening video file
// - Opening .gyroflow file
// - Opening .gyroflow preset
// - Dragging lens profile
// - Opening lens profile
// - Dragging gyro data
// - Opening gyro data
// - Changing output path
// - Selecting output folder
// - Rendering to existing file
// - Adding file to queue
// - Adding multiple files to queue
// - Exporting lens profile
// - Dragging preset to render queue
// - Rendering with project file
// - Saving project file
// - Opening video with project file next to it
// - Merging mp4
// - Detecting mp4 sequence
// - Rendering r3d with conversion
// - Using CLI

#[derive(thiserror::Error, Debug)]
pub enum FilesystemError {
    #[error("Invalid url {0:?}")]              InvalidUrl((String, url::ParseError)),
    #[error("Not a file url {0}")]             NotAFile(String),
    #[error("Not a folder url {0}")]           NotAFolder(String),
    #[error("Path doesn't have a parent {0}")] NoParent(String),
    #[error("Invalid path {0}")]               InvalidPath(String),
    #[error("Invalid file descriptor {0}")]    InvalidFD(i32),
    #[error("Unknown error")]                  Unknown,
    #[error("IO error: {0:?}")]                IOError(#[from] std::io::Error),
    #[error("String error: {0:?}")]            Utf8Error(#[from] std::str::Utf8Error),
    #[cfg(target_os = "android")]
    #[error("Java exception: {0:?}")]
    JavaException(#[from] jni::errors::Error)
}
#[macro_export]
macro_rules! function_name {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str { std::any::type_name::<T>() }
        type_name_of(f).rsplit("::").find(|&part| part != "f" && part != "{{closure}}").expect("Short function name")
    }};
}
#[macro_export]
macro_rules! dbg_call {
    ($($param:ident )* $(-> $ret:expr)*) => {{
        let mut dbg_str = format!("{}", function_name!());
        $(
            dbg_str.push_str(&format!(" | {}: {}", stringify!($param), $param));
        )*
        $(
            dbg_str.push_str(" -> ");
            dbg_str.push_str(&format!("{:?}", $ret));
        )*
        log::debug!("{}", dbg_str);
    }};
}
macro_rules! result {
    ($result:expr $(, $param:ident)*) => {{
        let ret = $result;
        dbg_call!($($param )* -> ret);
        match ret {
            Ok(x) => x,
            Err(e) => { log::error!("{e:?}"); Default::default() }
        }
    }};
}
pub fn start_accessing_url(_url: &str) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    apple::start_accessing_url(_url);
}
pub fn stop_accessing_url(_url: &str) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    apple::stop_accessing_url(_url);
}

pub struct FileWrapper<'a> {
    pub size: usize,
    pub url: String,

    file: Option<std::fs::File>,

    #[cfg(target_os = "android")]
    pub android_handle: android::AndroidFileHandle<'a>,

    _lifetime: std::marker::PhantomData<&'a ()>,
}
impl<'a> FileWrapper<'a> {
    pub fn get_file(&mut self) -> &mut File {
        self.file.as_mut().unwrap()
    }
}
impl<'a> Drop for FileWrapper<'a> {
    fn drop(&mut self) {
        log::debug!("FileWrapper::drop {}", self.url);
        #[cfg(target_os = "android")]
        if let Some(f) = self.file.take() {
            // Discard the std::fs::File to prevent from closing the file descriptor.
            // It will be closed in Drop for AndroidFileHandle.
            std::mem::forget(f);
        }
        stop_accessing_url(&self.url);
    }
}
pub struct FfmpegPathWrapper<'a> {
    pub org_url: String,
    pub path: String,
    #[cfg(target_os = "android")]
    _file: FileWrapper<'a>,
    _lifetime: std::marker::PhantomData<&'a ()>,
}
impl<'a> FfmpegPathWrapper<'a> {
    pub fn new(_base: &'a EngineBase, url: &str, _write: bool) -> Result<Self> {
        log::debug!("FfmpegPathWrapper::new {url}, write: {_write}");
        #[cfg(target_os = "android")]
        {
            // On android we have to use raw file descriptor, because ffmpeg can't use the content:// urls
            let file = FileWrapper::open_android(_base, &url, if _write { "wt" } else { "r" })?;
            Ok(Self {
                org_url: url.to_owned(),
                path: format!("fd:{}", file.android_handle.fd),
                _file: file,
                _lifetime: Default::default()
            })
        }
        #[cfg(not(target_os = "android"))]
        {
            start_accessing_url(url);
            let mut path = url.to_owned();
            if path.starts_with("file://") {
                path = url_to_path(&path);
            }
            Ok(Self {
                org_url: url.to_owned(),
                path: path,
                _lifetime: Default::default()
            })
        }
    }
}

#[cfg(not(target_os = "android"))]
impl<'a> Drop for FfmpegPathWrapper<'a> {
    fn drop(&mut self) {
        log::debug!("FfmpegPathWrapper::drop {}", self.org_url);
        stop_accessing_url(&self.org_url);
    }
}

fn url_to_pathbuf(mut url: &str) -> Result<PathBuf> {
    if cfg!(target_os = "android") {
        return Ok(Path::new(&get_filename(url)).to_path_buf());
    }
    Ok(if url.contains("://") { // It's an url
        url::Url::parse(url).map_err(|e| FilesystemError::InvalidUrl((url.into(), e)))?.to_file_path().map_err(|_| FilesystemError::NotAFile(url.into()))?
    } else {
        if url.starts_with("//?/") { url = &url[4..]; } // Windows extended path
        Path::new(url).to_path_buf()
    })
}

pub fn get_filename(url: &str) -> String {
    fn inner(url: &str) -> Result<String> {
        if url.is_empty() { return Ok(String::new()) }
        if !url.contains("://") && !url.contains('/') && !url.contains('\\') { return Ok(url.to_owned()); }

        #[cfg(target_os = "android")]
        if url.starts_with("content://") {
            if android::is_dir_url(url) { return Ok(String::new()); } // no filename
            return Ok(android::get_url_info(url).map(|x| x.filename.unwrap_or_default()).unwrap_or_default());
        }

        let pathbuf = url_to_pathbuf(url)?;
        if pathbuf.is_dir() && !pathbuf.to_str().unwrap().ends_with(".RDC") {
            return Ok(String::new());
        }
        Ok(pathbuf.file_name().ok_or(FilesystemError::NotAFile(url.into()))?.to_string_lossy().to_string())
    }
    result!(inner(url), url)
}
pub fn get_folder(url: &str) -> String {
    fn inner(url: &str) -> Result<String> {
        if url.is_empty() { return Ok(String::new()) }

        #[cfg(target_os = "android")]
        if url.starts_with("content://") {
            if android::is_dir_url(url) { // it's already a directory url
                return Ok(url.to_string());
            }

            log::warn!("Cannot get directory path on android, url: {url}, info: {:?}", android::get_url_info(url));
            return Ok(String::new());
        }
        let pathbuf = url_to_pathbuf(url)?;
        if pathbuf.is_dir() {
            return Ok(path_to_url(&pathbuf.to_string_lossy()));
        }
        Ok(path_to_url(&pathbuf.parent().ok_or(FilesystemError::NoParent(url.into()))?.to_string_lossy()))
    }
    let mut ret = inner(url);
    if let Ok(ref mut ret) = ret {
        if !ret.is_empty() && !ret.ends_with('/') {
            ret.push('/');
        }
    }
    result!(ret, url)
}

pub fn exists(url: &str) -> bool {
    fn inner(url: &str) -> Result<bool> {
        if url.is_empty() { return Ok(false); }

        #[cfg(target_os = "android")]
        if url.starts_with("content://") {
            return android::get_url_info(url).map(|x| x.filename.is_some() && !x.filename.unwrap().is_empty());
        }

        start_accessing_url(url);
        let exists = url_to_pathbuf(url).map(|x| x.exists());
        stop_accessing_url(url);
        exists
    }
    result!(inner(url), url)
}
pub fn exists_in_folder(folder_url: &str, filename: &str) -> bool {
    fn inner(folder_url: &str, filename: &str) -> bool {
        if folder_url.is_empty() || filename.is_empty() { return false; }

        #[cfg(target_os = "android")]
        if folder_url.starts_with("content://") && android::is_dir_url(folder_url) {
            if let Ok(files) = android::list_files(folder_url) {
                let cmp = Some(filename.to_owned());
                for x in files {
                    if x.filename == cmp {
                        return true;
                    }
                }
                return false;
            }
        }
        exists(&get_file_url(folder_url, filename, false))
    }

    let ret = inner(folder_url, filename);
    dbg_call!(folder_url filename -> ret);
    ret
}
pub fn get_mime(filename: &str) -> &'static str {
    if filename.is_empty() || !filename.contains('.') { return ""; }
    let pos = filename.rfind('.').unwrap();
    let ext = filename[pos+1..].to_ascii_lowercase();
    match ext.as_str() {
        "gyroflow" => "application/octet-stream",
        "json"     => "application/json",
        "gcsv"     => "application/octet-stream",
        "mp4"      => "video/mp4",
        "mov"      => "video/quicktime",
        "png"      => "image/png",
        "exr"      => "image/x-exr",
        _          => "application/octet-stream"
    }
}
pub fn get_file_url(folder_url: &str, filename: &str, can_create: bool) -> String {
    fn inner(folder_url: &str, filename: &str, _can_create: bool) -> Result<String> {
        if folder_url.is_empty() { return Ok(String::new()); }

        #[cfg(target_os = "android")]
        if folder_url.starts_with("content://") && android::is_dir_url(folder_url) {
            if let Ok(files) = android::list_files(folder_url) {
                let cmp = Some(filename.to_owned());
                for x in files {
                    if x.filename == cmp {
                        if let Some(url) = x.url {
                            return Ok(url);
                        }
                    }
                }
                if _can_create {
                    match android::create_file(folder_url, filename, get_mime(filename)) {
                        Ok(new_url) => return Ok(new_url),
                        Err(e) => { log::error!("android::create_file failed: {e:?}"); }
                    }
                }
                return Ok(format!("{filename}"));
            }
        }

        let filename_escaped = urlencoding::encode(filename);
        if folder_url.ends_with('/') || folder_url.ends_with('\\') {
            Ok(format!("{folder_url}{filename_escaped}"))
        } else {
            Ok(format!("{folder_url}/{filename_escaped}"))
        }
    }
    result!(inner(folder_url, filename, can_create), folder_url, filename, can_create)
}
pub fn read(url: &str) -> Result<Vec<u8>> {
    dbg_call!(url);
    start_accessing_url(url);
    let buf = {
        let base = get_engine_base();
        let mut f = open_file(&base, &url, false)?;
        let mut buf = Vec::with_capacity(f.size);
        f.get_file().read_to_end(&mut buf)?;
        buf
    };
    stop_accessing_url(url);
    Ok(buf)
}
pub fn write(url: &str, data: &[u8]) -> Result<()> {
    dbg_call!(url);
    start_accessing_url(url);
    {
        let base = get_engine_base();
        let mut f = open_file(&base, &url, true)?;
        f.get_file().write(data)?;
    }
    stop_accessing_url(url);
    Ok(())
}
pub fn read_to_string(url: &str) -> Result<String> {
    dbg_call!(url);
    let data = read(url)?;
    Ok(std::str::from_utf8(&data)?.to_owned())
}
pub fn filename_with_extension(filename: &str, ext: &str) -> String {
    let new_filename = if let Some(pos) = filename.rfind('.') {
        format!("{}.{ext}", &filename[0..pos])
    } else {
        filename.to_string()
    };
    dbg_call!(filename ext -> new_filename);
    new_filename
}
pub fn filename_with_suffix(filename: &str, suffix: &str) -> String {
    let new_filename = if let Some(pos) = filename.rfind('.') {
        format!("{}{suffix}{}", &filename[0..pos], &filename[pos..])
    } else {
        filename.to_string()
    };
    dbg_call!(filename suffix -> new_filename);
    new_filename
}

pub fn remove_file(url: &str) -> Result<()> {
    dbg_call!(url);
    #[cfg(target_os = "android")]
    {
        android::remove_file(url).map(|_| ())
    }
    #[cfg(not(target_os = "android"))]
    {
        let path = url_to_path(url);
        Ok(std::fs::remove_file(path)?)
    }
}

pub fn can_open_file(url: &str) -> bool {
    dbg_call!(url);
    if !exists(url) { return false; }
    let base = get_engine_base();
    let x = open_file(&base, url, false).is_ok(); x
}
pub fn open_file<'a>(_base: &'a EngineBase, url: &str, writing: bool) -> Result<FileWrapper<'a>> {
    dbg_call!(url writing);
    start_accessing_url(url);

    #[cfg(target_os = "android")]
    {
        return FileWrapper::open_android(_base, url, if writing { "wt" } else { "r" });
    }
    #[cfg(not(target_os = "android"))]
    {
        let path = url_to_path(url);
        let file = if writing { File::create(path)? } else { File::open(path)? };
        let size = file.metadata()?.len() as usize;
        Ok(FileWrapper { file: Some(file), size, url: url.to_owned(), _lifetime: Default::default() })
    }
}

pub fn path_to_url(path: &str) -> String {
    fn inner(mut path: &str) -> Result<String> {
        if path.is_empty() { return Ok(String::new()) }
        if path.contains("://") { return Ok(path.to_owned()); } // Already an url
        if path.starts_with("//?/") { path = &path[4..]; } // Windows extended path
        let mut ret = url::Url::from_file_path(&path).map_err(|_| FilesystemError::NotAFile(path.into()))?.to_string();
        if (path.ends_with('/') || path.ends_with('\\')) && !ret.ends_with('/') {
            ret.push('/');
        }
        Ok(ret)
    }
    result!(inner(path), path)
}

pub fn url_to_path(url: &str) -> String {
    fn inner(url: &str) -> Result<String> {
        if url.is_empty() { return Ok(String::new()) }
        Ok(url_to_pathbuf(url)?.to_string_lossy().to_string())
    }
    result!(inner(url), url)
}
pub fn display_url(url: &str) -> String {
    dbg_call!(url);
    url_to_path(url)
}
pub fn display_folder_filename(folder: &str, filename: &str) -> String {
    fn inner(folder: &str, filename: &str) -> String {
        let url = get_file_url(folder, filename, false);
        if url.is_empty() && cfg!(target_os = "android") { return filename.to_owned(); }
        display_url(&url)
    }
    let ret = inner(folder, filename);
    dbg_call!(folder filename -> ret);
    ret
}

