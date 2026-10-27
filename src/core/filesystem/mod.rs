// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

#[cfg(target_os = "android")]
pub mod android;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub mod apple;

use std::fs::*;
use std::io::{ Read, Write };
use std::path::*;
use std::collections::HashSet;
use itertools::Itertools;
use parking_lot::RwLock;

pub type Result<T> = std::result::Result<T, FilesystemError>;

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

lazy_static::lazy_static! {
    static ref ALLOWED_FOLDERS: RwLock<HashSet<String>> = RwLock::new(HashSet::new());
}

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

mod sequence;
pub use sequence::*;
pub fn start_accessing_url(_url: &str, _is_folder: bool) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    apple::start_accessing_url(_url, _is_folder);
}
pub fn stop_accessing_url(_url: &str, _is_folder: bool) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    apple::stop_accessing_url(_url, _is_folder);
}

pub struct FileWrapper {
    pub size: usize,
    pub url: String,

    file: Option<std::fs::File>,

    #[cfg(target_os = "android")]
    pub android_handle: android::AndroidFileHandle
}
impl FileWrapper {
    pub fn get_file(&mut self) -> &mut File {
        self.file.as_mut().unwrap()
    }
}
impl Read for FileWrapper {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.as_mut().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::Other))?.read(buf)
    }
}
impl std::io::Seek for FileWrapper {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.file.as_mut().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::Other))?.seek(pos)
    }
}
impl Drop for FileWrapper {
    fn drop(&mut self) {
        log::debug!("FileWrapper::drop {}", self.url);
        #[cfg(target_os = "android")]
        if let Some(f) = self.file.take() {
            // Discard the std::fs::File to prevent from closing the file descriptor.
            // It will be closed in Drop for AndroidFileHandle.
            std::mem::forget(f);
        }
        stop_accessing_url(&self.url, false);
    }
}
pub struct FfmpegPathWrapper {
    pub org_url: String,
    pub path: String,
    #[cfg(target_os = "android")]
    _file: FileWrapper
}
impl FfmpegPathWrapper {
    pub fn new(url: &str, _write: bool) -> Result<Self> {
        log::debug!("FfmpegPathWrapper::new {url}, write: {_write}");
        #[cfg(target_os = "android")]
        {
            // On android we have to use raw file descriptor, because ffmpeg can't use the content:// urls
            let file = FileWrapper::open_android(&url, if _write { "wt" } else { "r" })?;
            Ok(Self {
                org_url: url.to_owned(),
                path: format!("fd:{}", file.android_handle.fd),
                _file: file,
            })
        }
        #[cfg(not(target_os = "android"))]
        {
            start_accessing_url(url, false);
            let mut path = url.to_owned();
            if path.starts_with("file://") {
                path = url_to_path(&path);
            }
            Ok(Self {
                org_url: url.to_owned(),
                path: path,
            })
        }
    }
}

#[cfg(not(target_os = "android"))]
impl Drop for FfmpegPathWrapper {
    fn drop(&mut self) {
        log::debug!("FfmpegPathWrapper::drop {}", self.org_url);
        stop_accessing_url(&self.org_url, false);
    }
}

fn url_to_pathbuf(mut url: &str) -> Result<PathBuf> {
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
        } else {
            log::error!("Unknown android url scheme: {url}");
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
        if url.ends_with("/") { // it's already a directory url
            return Ok(url.to_string());
        }

        #[cfg(target_os = "android")]
        if url.starts_with("content://") {
            if android::is_dir_url(url) { // it's already a directory url
                return Ok(url.to_string());
            }
            // Try to find the folder in the allowed list
            if let Some(doc_pos) = url.find("/document/") {
                let tree_part = if url.contains("/tree/") {
                    url[..doc_pos + 1].to_string()
                } else if let Some(pos2) = url.rfind("%2F") {
                    url[..pos2].replace("/document/", "/tree/")
                } else {
                    url.to_string()
                };
                let lock = ALLOWED_FOLDERS.read();
                for x in lock.iter() {
                    if tree_part.trim_end_matches('/') == x.trim_end_matches('/') {
                        return Ok(x.clone());
                    }
                }
            }

            log::warn!("Cannot get directory path on android, url: {url}, info: {:?}", android::get_url_info(url));
            return Ok(String::new());
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if is_sandboxed() {
            if let Some(pos) = url.rfind('/') {
                let folder = &url[0..pos + 1];
                if ALLOWED_FOLDERS.read().contains(&normalize_url(folder, true)) {
                    return Ok(folder.to_string());
                }
            }
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

        start_accessing_url(url, false);
        let exists = url_to_pathbuf(url).map(|x| x.exists());
        stop_accessing_url(url, false);
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
pub fn list_folder(folder_url: &str) -> Vec<(String, String)> {
    fn inner(folder_url: &str) -> Vec<(String, String)> {
        let mut ret = Vec::new();

        #[cfg(target_os = "android")]
        if folder_url.starts_with("content://") && android::is_dir_url(folder_url) {
            if let Ok(files) = android::list_files(folder_url) {
                for x in files {
                    if let Some(url) = x.url {
                        if let Some(filename) = x.filename {
                            ret.push((filename, url));
                        }
                    }
                }
                return ret;
            }
        }
        if let Ok(entries) = std::fs::read_dir(url_to_path(folder_url)) {
            for entry in entries {
                if let Ok(entry) = entry {
                    if entry.metadata().as_ref().map(|x| x.is_file() || x.is_dir()).unwrap_or_default() {
                        ret.push((entry.file_name().to_string_lossy().to_string(), path_to_url(entry.path().to_string_lossy().as_ref())));
                    }
                }
            }
        }
        ret
    }

    let ret = inner(folder_url);
    dbg_call!(folder_url -> ret);
    ret
}
pub fn get_mime(filename: &str) -> &'static str {
    if filename.is_empty() || !filename.contains('.') { return "application/octet-stream"; }
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
                return Ok(String::new());
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
    start_accessing_url(url, false);
    let buf = {
        let mut f = open_file(&url, false, false)?;
        let mut buf = Vec::with_capacity(f.size);
        f.get_file().read_to_end(&mut buf)?;
        buf
    };
    stop_accessing_url(url, false);
    Ok(buf)
}
pub fn write(url: &str, data: &[u8]) -> Result<()> {
    dbg_call!(url);
    start_accessing_url(url, false);
    {
        let mut f = open_file(&url, true, true)?;
        f.get_file().write(data)?;
    }
    stop_accessing_url(url, false);
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
    let x = open_file(url, false, false).is_ok(); x
}
pub fn can_create_file(folder: &str, filename: &str) -> bool {
    if folder.is_empty() || filename.is_empty() { return false; }
    fn inner(folder: &str, filename: &str) -> bool {
        if is_sandboxed() && folder.contains("://") {
            let lock = ALLOWED_FOLDERS.read();
            if !lock.contains(&normalize_url(folder, true)) {
                return false; // Access not allowed
            }
        }

        let already_exists = exists(&get_file_url(folder, filename, false));

        let url = get_file_url(folder, filename, true);
        if !url.is_empty() {
            if open_file(&url, true, false).is_ok() {
                if !already_exists {
                    let _ = remove_file(&url);
                }
                return true;
            }
        }
        false
    }
    let ret = inner(folder, filename);
    dbg_call!(folder filename -> ret);
    ret
}

pub fn open_file(url: &str, writing: bool, truncate: bool) -> Result<FileWrapper> {
    dbg_call!(url writing);
    start_accessing_url(url, false);

    #[cfg(target_os = "android")]
    {
        // "w" can truncate too (it does in ParcelFileDescriptor.parseMode), "rw" keeps the content like on the other platforms
        return FileWrapper::open_android(url, if writing && truncate { "wt" } else if writing { "rw" } else { "r" });
    }
    #[cfg(not(target_os = "android"))]
    {
        let path = url_to_path(url);
        let file = if (writing && truncate) || (writing && !exists(url)) { File::create(path)? } else if writing { OpenOptions::new().read(true).write(true).open(path)? } else { File::open(path)? };
        let size = file.metadata()?.len() as usize;
        Ok(FileWrapper { file: Some(file), size, url: url.to_owned() })
    }
}

/// What identifies a file whatever URL or path reaches it: another spelling of the path, a letter case variant on a
/// case-insensitive volume, a symbolic or a hard link, another document URL of it on Android. The device and inode numbers
/// on Unix (Android and Apple too: a `content://` file is read through its descriptor), the volume serial number and file ID
/// on Windows, and the canonical path on a filesystem that gives no IDs
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileId(FileIdRepr);
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum FileIdRepr {
    /// Where the ID comes from, the device or volume, and the inode or file ID
    Id(u8, u64, u128),
    Path(PathBuf),
}
impl FileWrapper {
    /// The identity of the open file
    pub fn id(&self) -> Result<FileId> {
        let file = self.file.as_ref().ok_or(FilesystemError::Unknown)?;
        Ok(FileId(match system_file_id(file)? {
            Some(id) => id,
            None => FileIdRepr::Path(std::fs::canonicalize(url_to_pathbuf(&self.url)?)?)
        }))
    }
}
#[cfg(unix)]
fn system_file_id(file: &File) -> Result<Option<FileIdRepr>> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    Ok(Some(FileIdRepr::Id(0, m.dev(), m.ino() as u128)))
}
/// The volume serial number and the 128-bit file ID of `FILE_ID_INFO` (ReFS's IDs don't fit 64 bits), or where a filesystem
/// doesn't give those (exFAT, FAT), the 64-bit file index of `BY_HANDLE_FILE_INFORMATION`. One volume always gives the same
/// kind, and the kind is part of the ID, so the two are never compared. A file ID of 0 isn't one: some network filesystems
/// give 0 for every file
#[cfg(windows)]
fn system_file_id(file: &File) -> Result<Option<FileIdRepr>> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{ Foundation::HANDLE, Storage::FileSystem::* };
    let handle = HANDLE(file.as_raw_handle());
    let mut info = FILE_ID_INFO::default();
    // SAFETY: the handle is the open file's, and `info` is a FILE_ID_INFO of the size given
    if unsafe { GetFileInformationByHandleEx(handle, FileIdInfo, &mut info as *mut FILE_ID_INFO as *mut _, std::mem::size_of::<FILE_ID_INFO>() as u32) }.is_ok() {
        let id = u128::from_le_bytes(info.FileId.Identifier);
        return Ok((id != 0).then_some(FileIdRepr::Id(1, info.VolumeSerialNumber, id)));
    }
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is the open file's, and `info` is a BY_HANDLE_FILE_INFORMATION
    unsafe { GetFileInformationByHandle(handle, &mut info) }.map_err(std::io::Error::from)?;
    let id = (info.nFileIndexHigh as u128) << 32 | info.nFileIndexLow as u128;
    Ok((id != 0).then_some(FileIdRepr::Id(2, info.dwVolumeSerialNumber as u64, id)))
}
#[cfg(not(any(unix, windows)))]
fn system_file_id(_file: &File) -> Result<Option<FileIdRepr>> {
    Ok(None)
}
/// The identity of the file at `url`, `None` when there's no file there
pub fn file_id(url: &str) -> Result<Option<FileId>> {
    if url.is_empty() || !exists(url) { return Ok(None); }
    Ok(Some(open_file(url, false, false)?.id()?))
}
/// The identity of the file or folder at `path`, through the symbolic links there, and how many names it has (hard links;
/// `None` where that can't be told). A file with one name is reached only by it, and by the symbolic links to it
pub fn path_id(path: &Path) -> Result<(FileId, Option<u64>)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path)?;
        Ok((FileId(FileIdRepr::Id(0, m.dev(), m.ino() as u128)), Some(m.nlink())))
    }
    #[cfg(windows)]
    {
        use std::os::windows::{ fs::OpenOptionsExt, io::AsRawHandle };
        use windows::Win32::{ Foundation::HANDLE, Storage::FileSystem::* };
        // Its attributes only, which no program's sharing keeps from, and so a folder too
        let file = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES.0).custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0).open(path)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the handle is the open file's, and `info` is a BY_HANDLE_FILE_INFORMATION
        let links = unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.ok().map(|_| info.nNumberOfLinks as u64);
        Ok((FileId(match system_file_id(&file)? {
            Some(id) => id,
            None => FileIdRepr::Path(std::fs::canonicalize(path)?)
        }), links))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok((FileId(FileIdRepr::Path(std::fs::canonicalize(path)?)), None))
    }
}
/// The file `path` reaches: the one at the end of the symbolic links there (as [`output_target`] finds it), or `path` itself
pub fn resolve_links(path: &Path) -> std::io::Result<PathBuf> {
    if is_link(path) { linked_file(path) } else { Ok(path.to_owned()) }
}
/// Whether the URLs `a` and `b` reach the same file, by its [`FileId`]: `false` when there's no file at either
pub fn is_same_file(a: &str, b: &str) -> Result<bool> {
    let ret = match (file_id(a)?, file_id(b)?) {
        (Some(a), Some(b)) => a == b,
        _ => false
    };
    dbg_call!(a b -> ret);
    Ok(ret)
}

/// How an output replaces a file that's already where it goes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overwrite {
    /// The complete output is renamed onto it, which replaces it at once (desktop)
    Rename,
    /// It's written into in place: an image sequence's frames, and outputs on Android and iOS
    InPlace,
}

/// What keeps an output from being written where it goes, found by [`output_target`] before anything is written, or by
/// [`publish`]. The first field is the output, as it's shown
#[derive(thiserror::Error, Debug)]
pub enum OutputError {
    #[error("The output {0} is a folder. Choose another name for the output.")]
    IsAFolder(String),
    #[error("The output {0} isn't a file name. Choose another name for the output.")]
    NotAFileName(String),
    #[error("The output {0} can't be created in {1}: {2}")]
    NoFolder(String, String, std::io::Error),
    #[error("The output {0} is open in another program, which keeps it from being overwritten. Close it there, or choose another name or folder for the output.")]
    Busy(String),
    #[error("The output {0} can't be written to: {1}")]
    CantWrite(String, std::io::Error),
    #[error("The output {0} can't be replaced: {1}")]
    CantReplace(String, std::io::Error),
    #[error("Failed to check the output {0}: {1}")]
    Check(String, std::io::Error),
    #[error("The complete output couldn't be given the name {0}: {1}")]
    Publish(String, std::io::Error),
    #[error("Failed to create the output {0}: {1}")]
    Create(String, std::io::Error),
    #[error("Failed to write the output {0}: {1}")]
    Write(String, std::io::Error),
    /// Why the complete output couldn't get its name, and the file it's kept in
    #[error("{}", kept_message(.0, .1))]
    Kept(Box<OutputError>, PathBuf),
}
fn kept_message(e: &OutputError, kept: &Path) -> String {
    let e = e.to_string();
    format!("{e}{} The complete output is kept as {}.", if e.ends_with('.') { "" } else { "." }, kept.display())
}

/// The file that an output at `url` gets written to (a path, so not Android's documents, which have no links either),
/// checked before anything is written, so that writing it can't fail for what's already there. Nothing is created or
/// changed:
/// - a symbolic link there is followed, as creating the file follows it: the file at the end of its links is the one
///   replaced, or the one created where they point, and the links stay. A loop of links is refused, and so is a link to what
///   isn't a file name, or on Windows to a name that a program's path names otherwise than the link does;
/// - a new file needs a name (no separator or `.` at the end, on Windows no `:`) and a folder that exists;
/// - an existing file is replaced only where it can be written to, and with [`Overwrite::Rename`] only where the rename can
///   replace it: on Windows a program that has it open without letting it be deleted keeps it from that (FFmpeg opens
///   files so, and with it a player)
pub fn output_target(url: &str, overwrite: Overwrite) -> std::result::Result<PathBuf, OutputError> {
    start_accessing_url(url, false);
    let ret = url_to_pathbuf(url)
        .map_err(|e| OutputError::Check(display_url(url), std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string())))
        .and_then(|path| path_target(&path, overwrite, &|| display_url(url)));
    stop_accessing_url(url, false);
    dbg_call!(url -> ret);
    ret
}

/// [`output_target`] of a `path`, shown as `display()`
pub fn path_target(path: &Path, overwrite: Overwrite, display: &dyn Fn() -> String) -> std::result::Result<PathBuf, OutputError> {
    use std::io::ErrorKind;
    if not_a_file_name(path) {
        return Err(OutputError::NotAFileName(display()));
    }
    // Through the symbolic links there, which the system follows as it does to create the file, where it lets them be
    // followed: it reports a loop itself
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => Err(OutputError::IsAFolder(display())),
        Ok(_) => {
            // The file replaced: the one at the end of the symbolic links there, which keep pointing to it
            let target = if is_link(path) { linked_file(path).map_err(|e| OutputError::Check(display(), e))? } else { path.to_owned() };
            file_target(&target, overwrite, display)
        },
        Err(e) if e.kind() == ErrorKind::NotFound => {
            // Nothing there, or a symbolic link to where nothing is yet: the file is created where it points
            let target = if is_link(path) { links_end(path).map_err(|e| OutputError::Check(display(), e))? } else { path.to_owned() };
            let shown = || if target == path { display() } else { format!("{}, a symbolic link to {},", display(), target.display()) };
            if not_a_file_name(&target) {
                return Err(OutputError::NotAFileName(shown()));
            }
            let dir = target.parent().filter(|x| !x.as_os_str().is_empty()).unwrap_or(Path::new("."));
            match std::fs::metadata(dir) {
                Ok(m) if m.is_dir() => Ok(target),
                Ok(_) => Err(OutputError::NoFolder(shown(), dir.display().to_string(), std::io::Error::from(ErrorKind::NotADirectory))),
                Err(e) => Err(OutputError::NoFolder(shown(), dir.display().to_string(), e))
            }
        },
        Err(e) => Err(OutputError::Check(display(), e))
    }
}

/// [`path_target`] of the file `path`, where it's known to be there and to be no symbolic link nor folder (as a listing of
/// its folder tells): it's replaced only where it can be written to, and with [`Overwrite::Rename`] only where the rename can
/// replace it
pub fn file_target(path: &Path, overwrite: Overwrite, display: &dyn Fn() -> String) -> std::result::Result<PathBuf, OutputError> {
    if let Err(e) = OpenOptions::new().write(true).open(path) {
        return Err(if is_busy(&e) { OutputError::Busy(display()) } else { OutputError::CantWrite(display(), e) });
    }
    #[cfg(windows)]
    if overwrite == Overwrite::Rename && let Err(e) = can_be_replaced(path) {
        return Err(if is_busy(&e) { OutputError::Busy(display()) } else { OutputError::CantReplace(display(), e) });
    }
    let _ = overwrite;
    Ok(path.to_owned())
}

/// A complete output's file while it's written: a new file in the folder of the file that gets the output's name (`target`, as
/// [`output_target`] found it for the output's `url`), which gets that name once it's complete and on disk
/// ([`TempOutput::publish`]). So an output that fails never leaves a partial file under that name, nor replaces the file that
/// was there. It's removed when dropped before that, and kept, which the error says, when only the rename fails.
///
/// It's locked for as long as it's open ([`try_lock_temp`]): one that's there unlocked was left by a process that ended without
/// removing it (a crash), and the next output to that name reuses it, rather than leaving another next to it each time. A file
/// kept by a failed rename is reused so too
pub struct TempOutput {
    /// Holds the lock
    file: File,
    path: PathBuf,
    target: PathBuf,
    url: String,
    done: bool,
}
impl TempOutput {
    /// Named by `name(target's file name, n)` for the attempt `n`: a file that's there is reused only where it was left behind,
    /// as above, and the next name is tried where it's in use
    pub fn create(url: &str, target: PathBuf, name: impl Fn(&std::ffi::OsStr, u32) -> std::ffi::OsString) -> std::result::Result<Self, OutputError> {
        let dir = target.parent().filter(|x| !x.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_owned();
        let error = |e: std::io::Error| OutputError::Create(display_url(url), std::io::Error::new(e.kind(), format!("creating a temporary file in {}: {e}", dir.display())));
        let file_name = target.file_name().unwrap_or_default();
        for n in 0..1000u32 {
            let path = dir.join(name(file_name, n));
            let file = match OpenOptions::new().read(true).write(true).create_new(true).open(&path) {
                Ok(file) => match try_lock_temp(&file) {
                    // Taken as left behind by another process between creating and locking it: it's that process's now
                    Ok(false) => continue,
                    // Where files can't be locked, no other process can tell this one is left behind either
                    Ok(true) | Err(_) => file
                },
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => match reuse_temp(&path) {
                    Some(file) => file,
                    None => continue
                },
                // A name taken otherwise, as by a folder on Windows
                Err(e) => if std::fs::symlink_metadata(&path).is_ok() { continue; } else { return Err(error(e)); }
            };
            dbg_call!(url -> path);
            return Ok(Self { file, path, target, url: url.to_owned(), done: false });
        }
        Err(error(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "every temporary name is in use")))
    }
    pub fn path(&self) -> &Path { &self.path }
    pub fn file(&mut self) -> &mut File { &mut self.file }
    /// Once it's on disk, it gets the output's name, which replaces the file there at once. A program that opened that file
    /// since is told apart. Kept where only that rename fails
    pub fn publish(mut self) -> std::result::Result<(), OutputError> {
        self.file.sync_all().map_err(|e| OutputError::Write(display_url(&self.url), e))?;
        self.done = true;
        publish(&self.path, &self.target, &self.url).map_err(|e| OutputError::Kept(Box::new(e), self.path.clone()))
    }
}
impl Drop for TempOutput {
    fn drop(&mut self) {
        // Before the lock goes with the file, so that no other process takes it as left behind meanwhile
        if !self.done && let Err(e) = std::fs::remove_file(&self.path) {
            log::error!("Failed to remove the partial output {}: {e:?}", self.path.display());
        }
    }
}

/// The temporary output `path` that a process left behind, opened and emptied for reuse: one that no process holds locked
/// ([`try_lock_temp`]). `None` for one in use, one that's no regular file, and where files can't be locked, as then that can't be
/// told
fn reuse_temp(path: &Path) -> Option<File> {
    if !std::fs::symlink_metadata(path).is_ok_and(|x| x.is_file()) { return None; }
    let file = OpenOptions::new().read(true).write(true).open(path).ok()?;
    if !try_lock_temp(&file).ok()? { return None; }
    // Still the file by that name: its process may have given it the output's name, and let it go, since it was opened
    let id = |x: Option<FileIdRepr>| x.filter(|x| matches!(x, FileIdRepr::Id(..)));
    let same = id(system_file_id(&file).ok()?).is_some_and(|x| path_id(path).is_ok_and(|(y, _)| y.0 == x));
    if !same { return None; }
    file.set_len(0).ok()?;
    log::info!("Reusing the temporary output {}, left by a process that ended while writing it", path.display());
    Some(file)
}

/// Locks `file` as a temporary output that's being written, for as long as it's open, or the process runs: `Ok(false)` where
/// another holds the lock. On Unix it's `flock`'s, which only other locks heed. A lock on Windows keeps every other handle from
/// the bytes it covers, as the one FFmpeg writes through, so it covers a byte past any data: at 2^62
#[cfg(windows)]
fn try_lock_temp(file: &File) -> std::io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{ Foundation::{ HANDLE, ERROR_LOCK_VIOLATION }, Storage::FileSystem::*, System::IO::OVERLAPPED };
    let mut overlapped = OVERLAPPED::default();
    overlapped.Anonymous.Anonymous.OffsetHigh = 1 << 30;
    // SAFETY: the handle is the open file's, which std opens for synchronous I/O, so the call ends before `overlapped` does
    match unsafe { LockFileEx(HANDLE(file.as_raw_handle()), LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY, None, 1, 0, &mut overlapped) } {
        Ok(()) => Ok(true),
        Err(e) => {
            let e = std::io::Error::from(e);
            if e.raw_os_error() == Some(ERROR_LOCK_VIOLATION.0 as i32) { Ok(false) } else { Err(e) }
        }
    }
}
#[cfg(not(windows))]
fn try_lock_temp(file: &File) -> std::io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(e)) => Err(e)
    }
}

/// Gives the complete output `temp` (on disk) the name of `target`, which [`output_target`] found for `url`: a rename, which
/// replaces what's there at once. A program that opened the file there since is told apart
fn publish(temp: &Path, target: &Path, url: &str) -> std::result::Result<(), OutputError> {
    start_accessing_url(url, false);
    let ret = std::fs::rename(temp, target).map_err(|e| {
        #[cfg(windows)]
        if can_be_replaced(target).is_err_and(|e| is_busy(&e)) {
            return OutputError::Busy(display_url(url));
        }
        OutputError::Publish(display_url(url), e)
    });
    stop_accessing_url(url, false);
    dbg_call!(url -> ret);
    ret
}

/// Whether `path` is a symbolic link (on Windows also a junction)
fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// Whether `path` names no file: it ends with a separator, its last part is `.` or `..`, or on Windows its name holds a `:`.
/// Refused alike on every system: on Unix creating the file and `rename` both refuse those, Windows refuses a separator at the
/// end too, and there a `:` names a stream of a file, which a rename can't give the output
fn not_a_file_name(path: &Path) -> bool {
    let text = path.as_os_str().as_encoded_bytes();
    let separator = |x: &u8| std::path::is_separator(*x as char);
    // `file_name` skips a `.` at the end
    let dot = text.strip_suffix(b".").is_some_and(|x| x.last().is_none_or(separator));
    text.last().is_some_and(separator) || dot || path.file_name().is_none_or(|x| cfg!(windows) && x.as_encoded_bytes().contains(&b':'))
}

/// Where the symbolic links from `path` on lead: each followed to what it points to, a relative one from its folder, up to
/// the first path that isn't a link, which needn't exist. A relative one is joined as it is, so the system resolves the path
/// that results as it would have resolved the link: a `..` in it physically on Unix, by the letters of the path on Windows.
/// Windows takes those letters from the path the link was reached by, in which a directory symbolic link has been replaced
/// by its target but a junction hasn't: a `..` that would go up to another folder for one than for the other isn't followed.
/// Each must point to a file name
fn links_end(path: &Path) -> std::io::Result<PathBuf> {
    let folder = |p: &Path| p.parent().filter(|x| !x.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_owned();
    let mut end = path.to_owned();
    // As many links as Windows follows in a path, more than Linux (40) and macOS (32), so a chain the system followed is never
    // cut short; and then what the last one leads to
    for _ in 0..=63 {
        if !is_link(&end) {
            return Ok(end);
        }
        let to = std::fs::read_link(&end)?;
        if not_a_file_name(&to) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("the symbolic link {} points to {}, which isn't a file name", end.display(), to.display())));
        }
        // Windows follows a link by the letters of `to`, while a program's path loses the dots and spaces at the end of its
        // parts (`GetFullPathNameW`) and reads a `:` as a stream: through `final.mp4.` creating the file makes `final.mp4.`,
        // which the rename would publish as `final.mp4`, leaving the link pointing to nothing
        let renamed = |x: &[u8]| x.ends_with(b".") || x.ends_with(b" ") || x.contains(&b':');
        if cfg!(windows) && to.components().any(|x| matches!(x, Component::Normal(x) if renamed(x.as_encoded_bytes()))) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("the symbolic link {} points to {}, a name that Windows follows as it's written but programs can't use: a part of it ends with a dot or a space, or holds a ':'", end.display(), to.display())));
        }
        let next = end.parent().unwrap_or(Path::new("")).join(&to);
        let relative = matches!(to.components().next(), Some(Component::Normal(_) | Component::CurDir | Component::ParentDir));
        if cfg!(windows) && relative && to.components().any(|x| x == Component::ParentDir) {
            // The folder it goes into, and the one `..` going up from where the link's folder really is leads to
            let refused = |why: String| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("the symbolic link {} points to {}, up from its folder, {why}", end.display(), to.display()));
            match (std::fs::canonicalize(folder(&next)), std::fs::canonicalize(folder(&end)).map(|dir| std::fs::canonicalize(folder(&dir.join(&to))))) {
                (Ok(a), Ok(Ok(b))) if a == b => {},
                // Neither is there: it leads into a folder that doesn't exist, which the check then says
                (Err(a), Ok(Err(b))) if a.kind() == std::io::ErrorKind::NotFound && b.kind() == std::io::ErrorKind::NotFound => {},
                (Err(e), Ok(Err(_))) | (_, Err(e)) => return Err(refused(format!("and where that folder really is can't be told: {e}"))),
                _ => return Err(refused("which is reached through another link: which folder Windows takes it to can't be told".to_owned()))
            }
        }
        end = next;
    }
    Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "it leads through more than 63 symbolic links, or a loop of them"))
}

/// The file the symbolic link `path` leads to: its canonical path, or where its volume has none (on Windows one without a
/// DOS name), the end of its links when that is the same file as the one opened through the link
fn linked_file(path: &Path) -> std::io::Result<PathBuf> {
    let e = match std::fs::canonicalize(path) {
        Ok(target) => return Ok(target),
        Err(e) => e
    };
    let end = links_end(path)?;
    let id = |p: &Path| File::open(p).ok().and_then(|f| system_file_id(&f).ok().flatten());
    match (id(path), id(&end)) {
        (Some(a), Some(b)) if a == b => Ok(end),
        _ => Err(std::io::Error::new(e.kind(), format!("finding the file its symbolic links lead to: {e}")))
    }
}

/// Whether `e` is a sharing violation: a program has the file open without sharing what was asked for
fn is_busy(e: &std::io::Error) -> bool {
    #[cfg(windows)]
    { e.raw_os_error() == Some(windows::Win32::Foundation::ERROR_SHARING_VIOLATION.0 as i32) }
    #[cfg(not(windows))]
    { let _ = e; false }
}

/// Whether the file `path` can be replaced by a rename: opening it to delete it fails as the rename would, where a program
/// has it open without letting it be deleted (a sharing violation: the C runtime's `open`, and so FFmpeg, opens files so) or
/// it can't be deleted. A read-only file can be opened so, and it's refused for its writing
#[cfg(windows)]
fn can_be_replaced(path: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::*;
    OpenOptions::new().access_mode(DELETE.0).share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0).open(path).map(drop)
}

pub fn path_to_url(path: &str) -> String {
    fn inner(mut path: &str) -> Result<String> {
        if path.is_empty() { return Ok(String::new()) }
        if path.contains("://") { return Ok(path.to_owned()); } // Already an url
        if path.starts_with("//?/") { path = &path[4..]; } // Windows extended path
        let url_res = url::Url::from_file_path(&path).or_else(|_| {
            if path.len() >= 2 && path.as_bytes()[1] == b':' && path.as_bytes()[0].is_ascii_alphabetic() {
                url::Url::parse(&format!("file:///{}", path.replace('\\', "/"))).map_err(|_| ())
            } else {
                Err(())
            }
        });
        let mut ret = url_res.map_err(|_| FilesystemError::NotAFile(path.into()))?.to_string();
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
        if cfg!(target_os = "android") {
            return Ok(get_filename(url));
        }
        Ok(url_to_pathbuf(url)?.to_string_lossy().to_string())
    }
    result!(inner(url), url)
}
pub fn display_url(url: &str) -> String {
    dbg_call!(url);

    if cfg!(target_os = "android") && url.contains("/document/") && url.contains("%3A") {
        let parts = url.split("/document/").last().unwrap();
        if let Ok(decoded) = urlencoding::decode(parts) {
            if let Some(pos) = decoded.find(':') {
                return decoded[pos + 1..].trim_start_matches('/').to_string();
            }
        }
    }

    let path = url_to_path(url);
    if cfg!(any(target_os = "macos", target_os = "ios")) {
        if path.contains("/File Provider Storage/") {
            return path.split("/File Provider Storage/").last().unwrap().to_owned();
        } else if path.contains("/com~apple~CloudDocs/") {
            return format!("iCloud/{}", path.split("/com~apple~CloudDocs/").last().unwrap().to_owned());
        } else if path.contains("/Data/Application/") {
            return path.split("/Data/Application/").last().unwrap().split('/').skip(1).join("/").to_owned();
        } else if path.contains("/com.apple.filesystems.userfsd/") {
            return path.split("/com.apple.filesystems.userfsd/").last().unwrap().split('/').skip(1).join("/").to_owned();
        }
    }
    path
}
pub fn display_folder_filename(folder: &str, filename: &str) -> String {
    fn inner(folder: &str, filename: &str) -> String {
        if folder.is_empty() && !filename.is_empty() { return filename.to_string(); }

        let url = get_file_url(folder, filename, false);
        if cfg!(target_os = "android") {
            if url.contains("/document/") && url.contains("%3A") {
                return display_url(&url);
            } else if folder.contains("/tree/") {
                let parts = folder.split("/tree/").last().unwrap();
                if let Ok(decoded) = urlencoding::decode(parts) {
                    if let Some(pos) = decoded.find(':') {
                        return format!("{}/{filename}", &decoded[pos + 1..].trim_end_matches('/'));
                    }
                }
            }
            return filename.to_owned();
        }
        display_url(&url)
    }
    let ret = inner(folder, filename);
    dbg_call!(folder filename -> ret);
    ret
}

pub fn normalize_url(url: &str, is_folder: bool) -> String {
    if !url.contains("://") { return String::new(); }
    if let Ok(url) = url::Url::parse(url) {
        let mut url = url.to_string();
        if is_folder && !url.ends_with('/') {
            url.push('/');
        }
        url
    } else {
        String::new()
    }
}

pub fn folder_access_granted(folder_url: &str) {
    if folder_url.is_empty() || !folder_url.contains("://") { return; }

    let folder_url = normalize_url(folder_url, true);
    dbg_call!(folder_url);
    start_accessing_url(&folder_url, true); // This will not have an equivalent `stop_accessing_url` because we don't know when the access ends
    let already_allowed = ALLOWED_FOLDERS.read().contains(&folder_url);

    if !already_allowed {
        let mut lock = ALLOWED_FOLDERS.write();
        lock.insert(folder_url);
    }
}

pub fn restore_allowed_folders(list: &[String]) {
    for x in list {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            let (url, is_stale) = apple::resolve_bookmark(&x, None);
            if !url.is_empty() && url.contains("://") && !is_stale {
                folder_access_granted(&url);
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        {
            folder_access_granted(x);
        }
    }
}

pub fn get_allowed_folders() -> Vec<String> {
    let mut ret = Vec::new();
    let folders = ALLOWED_FOLDERS.read().clone();
    for x in folders.into_iter() {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            let bookmark = apple::create_bookmark(&x, true, None);
            if !bookmark.is_empty() {
                ret.push(bookmark);
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        {
            ret.push(x);
        }
    }
    ret
}

pub fn is_sandboxed() -> bool {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return true;
    }
    if cfg!(target_os = "macos") && std::env::var("APP_SANDBOX_CONTAINER_ID").is_ok() {
        return true;
    }
    false
}
