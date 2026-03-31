// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use objc2_core_foundation::*;
use std::io::{ Read, Write };
use parking_lot::Mutex;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use super::normalize_url;
use crate::{ function_name, dbg_call };

const UTF8_ENCODING: CFStringEncoding = CFStringBuiltInEncodings::EncodingUTF8.0;

lazy_static::lazy_static! {
    static ref OPENED_URLS: Mutex<HashMap<String, i64>> = Mutex::new(HashMap::new());
    static ref CLOSE_TIMEOUT: AtomicUsize = AtomicUsize::new(0);
}

pub fn start_accessing_url(url: &str, is_folder: bool) {
    if !url.contains("://") { return; }
    let url = normalize_url(url, is_folder);
    dbg_call!(url);
    let mut map = OPENED_URLS.lock();
    match map.entry(url.clone()) {
        Entry::Occupied(o) => { *o.into_mut() += 1; }
        Entry::Vacant(v) => {
            log::info!("start_accessing_url: {url} - OPEN");
            start_accessing_security_scoped_resource(&url);
            v.insert(1);
        }
    }
}
pub fn stop_accessing_url(url: &str, is_folder: bool) {
    if !url.contains("://") { return; }
    let url = normalize_url(url, is_folder);
    dbg_call!(url);
    let mut map = OPENED_URLS.lock();
    match map.entry(url.clone()) {
        Entry::Occupied(mut o) => {
            *o.get_mut() -= 1;
            let v = *o.get();
            log::debug!("stop_accessing_url: {url} - count: {v}");
            if v == 0 {
                spawn_close_thread();
            }
            if v < 0 {
                panic!("Cannot happen!")
            }
        }
        Entry::Vacant(_) => { panic!("Stop accessing url without starting! {url}"); }
    }
}

fn timestamp() -> usize { std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap().as_secs() as usize }
// We need to defer closing the url for 10 seconds
// We have a lot of functions which start and stop url access and they happen asynchronously so we need to avoid opening and closing them too often
fn spawn_close_thread() {
    let is_thread_running = CLOSE_TIMEOUT.load(SeqCst) != 0;
    CLOSE_TIMEOUT.store(timestamp() + 10, SeqCst); // 10 seconds

    if is_thread_running { return; }
    std::thread::spawn(|| {
        while CLOSE_TIMEOUT.load(SeqCst) > timestamp() {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        let mut to_remove = Vec::new();
        let mut map = OPENED_URLS.lock();
        for (url, v) in map.iter() {
            if *v <= 0 {
                log::info!("stop_accessing_url: {url} - CLOSE");
                stop_accessing_security_scoped_resource(url);
                to_remove.push(url.clone());
            }
        }
        for x in to_remove {
            map.remove(&x);
        }
        CLOSE_TIMEOUT.store(0, SeqCst);
    });
}

fn url_from_bytes(url: &str) -> Option<CFRetained<CFURL>> {
    let bytes = url.as_bytes();
    unsafe { CFURL::with_bytes(None, bytes.as_ptr(), bytes.len() as isize, UTF8_ENCODING, None) }
}

pub fn start_accessing_security_scoped_resource(url: &str) -> bool {
    if let Some(url_ref) = url_from_bytes(url) {
        unsafe { url_ref.start_accessing_security_scoped_resource() }
    } else {
        false
    }
}

pub fn stop_accessing_security_scoped_resource(url: &str) -> bool {
    if let Some(url_ref) = url_from_bytes(url) {
        unsafe { url_ref.stop_accessing_security_scoped_resource() };
        true
    } else {
        false
    }
}

pub fn create_bookmark(url: &str, is_folder: bool, project_url: Option<&str>) -> String {
    let mut ret = String::new();
    if url.is_empty() { return ret; }
    start_accessing_url(url, is_folder);
    unsafe {
        let project_url_ref = if let Some(project_url) = project_url {
            if !super::exists(project_url) && !project_url.ends_with('/') { let _ = super::write(project_url, &[]); } // Create empty file if not exists
            start_accessing_url(project_url, false);
            url_from_bytes(project_url)
        } else {
            None
        };
        if let Some(url_ref) = url_from_bytes(url) {
            let opts = CFURLBookmarkCreationOptions(1usize << 11); // kCFURLBookmarkCreationWithSecurityScope
            let mut error = std::ptr::null_mut();
            let bookmark_data = CFURL::new_bookmark_data(None, Some(&*url_ref), opts, None, project_url_ref.as_ref().map(|x| &**x), &mut error);
            if error.is_null() {
                if let Some(bookmark_data) = bookmark_data {
                    let len = bookmark_data.length();
                    let ptr = bookmark_data.byte_ptr();
                    if len > 0 {
                        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
                        e.write_all(&std::slice::from_raw_parts(ptr, len as usize)).unwrap();
                        ret = String::from_utf8(base91::slice_encode(&e.finish().unwrap())).unwrap_or_default();
                    }
                }
            } else {
                log::error!("Failed to create bookmark: {}", get_error(error));
            }
        }
        if project_url_ref.is_some() {
            stop_accessing_url(project_url.unwrap(), false);
        }
    }
    stop_accessing_url(url, is_folder);
    dbg_call!(url -> ret);
    ret
}

pub fn resolve_bookmark(bookmark_data: &str, project_url: Option<&str>) -> (String, bool) {
    let mut ret = String::new();
    let mut is_stale = false;
    if bookmark_data.is_empty() { return (ret, is_stale); }
    let compressed = base91::slice_decode(bookmark_data.as_bytes());
    if !compressed.is_empty() {
        let mut e = flate2::read::ZlibDecoder::new(&compressed[..]);
        let mut decompressed = Vec::new();
        e.read_to_end(&mut decompressed).unwrap();
        unsafe {
            let project_url_ref = if let Some(project_url) = project_url {
                if !super::exists(project_url) && !project_url.ends_with('/') { let _ = super::write(project_url, &[]); } // Create empty file if not exists
                start_accessing_url(project_url, false);
                url_from_bytes(project_url)
            } else {
                None
            };
            if let Some(data_ref) = CFData::new(None, decompressed.as_ptr(), decompressed.len() as isize) {
                let mut error = std::ptr::null_mut();
                let opts = CFURLBookmarkResolutionOptions(1usize << 10); // kCFURLBookmarkResolutionWithSecurityScope
                let mut is_stale_cf: u8 = 0;
                let url = CFURL::new_by_resolving_bookmark_data(None, Some(&*data_ref), opts, project_url_ref.as_ref().map(|x| &**x), None, &mut is_stale_cf, &mut error);
                if error.is_null() {
                    if let Some(url) = url {
                        let len = url.bytes(std::ptr::null_mut(), 0);
                        let mut buf = vec![0u8; len as usize];
                        url.bytes(buf.as_mut_ptr(), len);
                        ret = String::from_utf8(buf).unwrap_or_default();
                    }
                } else {
                    log::error!("Failed to resolve bookmark: {}", get_error(error));
                }
                if is_stale_cf != 0 {
                    is_stale = true;
                }
            }
            if project_url_ref.is_some() {
                stop_accessing_url(project_url.unwrap(), false);
            }
        }
    }
    dbg_call!(bookmark_data -> ret);
    (ret, is_stale)
}

unsafe fn get_error(err: *mut CFError) -> String {
    if err.is_null() { return String::from("Unknown error"); }
    let err = unsafe { &*err };
    if let Some(cf_str) = err.description() {
        let c_string = cf_str.c_string_ptr(UTF8_ENCODING);
        if !c_string.is_null() {
            return unsafe { std::ffi::CStr::from_ptr(c_string).to_string_lossy().into() };
        }
        let range = CFRange { location: 0, length: cf_str.length() };
        let mut len: CFIndex = 0;
        unsafe { cf_str.bytes(range, UTF8_ENCODING, 0, false, std::ptr::null_mut(), 0, &mut len); }
        let mut buffer = vec![0u8; len as usize];
        unsafe { cf_str.bytes(range, UTF8_ENCODING, 0, false, buffer.as_mut_ptr(), buffer.len() as isize, std::ptr::null_mut()); }
        return unsafe { String::from_utf8_unchecked(buffer) };
    }
    String::from("Unknown error")
}
