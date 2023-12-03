// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use std::ptr;
use core_foundation_sys::{ base::*, url::*, string::*, data::*, error::* };
use std::io::{ Read, Write };
use parking_lot::Mutex;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use super::normalize_url;
use crate::{ function_name, dbg_call };

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

pub fn start_accessing_security_scoped_resource(url: &str) -> bool {
    let mut ok = false;
    let url = url.as_bytes();
    unsafe {
        let url_ref = CFURLCreateWithBytes(ptr::null(), url.as_ptr(), url.len() as isize, kCFStringEncodingUTF8, ptr::null());
        if !url_ref.is_null() {
            ok = CFURLStartAccessingSecurityScopedResource(url_ref) == 1;
            CFRelease(url_ref as CFTypeRef);
        }
    }
    ok
}

pub fn stop_accessing_security_scoped_resource(url: &str) -> bool {
    let mut ok = false;
    let url = url.as_bytes();
    unsafe {
        let url_ref = CFURLCreateWithBytes(ptr::null(), url.as_ptr(), url.len() as isize, kCFStringEncodingUTF8, ptr::null());
        if !url_ref.is_null() {
            CFURLStopAccessingSecurityScopedResource(url_ref);
            ok = true;
            CFRelease(url_ref as CFTypeRef);
        }
    }
    ok
}

pub fn create_bookmark(url: &str, is_folder: bool, project_url: Option<&str>) -> String {
    let mut ret = String::new();
    if url.is_empty() { return ret; }
    start_accessing_url(url, is_folder);
    unsafe {
        let project_url_ref = if let Some(project_url) = project_url {
            if !super::exists(project_url) && !project_url.ends_with('/') { let _ = super::write(project_url, &[]); } // Create empty file if not exists
            start_accessing_url(project_url, false);
            let project_url = project_url.as_bytes();
            CFURLCreateWithBytes(ptr::null(), project_url.as_ptr(), project_url.len() as isize, kCFStringEncodingUTF8, ptr::null())
        } else {
            ptr::null()
        };
        let url = url.as_bytes();
        let url_ref = CFURLCreateWithBytes(ptr::null(), url.as_ptr(), url.len() as isize, kCFStringEncodingUTF8, ptr::null());
        if !url_ref.is_null() {
            let opts: CFURLBookmarkCreationOptions = 1usize << 11; // kCFURLBookmarkCreationWithSecurityScope
            let mut error = ptr::null_mut();
            let bookmark_data = CFURLCreateBookmarkData(kCFAllocatorDefault, url_ref, opts, ptr::null(), project_url_ref, &mut error);
            if error.is_null() {
                if !bookmark_data.is_null() {
                    let len = CFDataGetLength(bookmark_data);
                    let ptr = CFDataGetBytePtr(bookmark_data);
                    if len > 0 {
                        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
                        e.write_all(&std::slice::from_raw_parts(ptr, len as usize)).unwrap();
                        ret = String::from_utf8(base91::slice_encode(&e.finish().unwrap())).unwrap_or_default();
                    }
                    CFRelease(bookmark_data as CFTypeRef);
                }
            } else {
                log::error!("Failed to create bookmark: {}", get_error(error));
                CFRelease(error as CFTypeRef);
            }
            CFRelease(url_ref as CFTypeRef);
        }
        if !project_url_ref.is_null() {
            CFRelease(project_url_ref as CFTypeRef);
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
                let project_url = project_url.as_bytes();
                CFURLCreateWithBytes(ptr::null(), project_url.as_ptr(), project_url.len() as isize, kCFStringEncodingUTF8, ptr::null())
            } else {
                ptr::null()
            };
            let data_ref = CFDataCreate(kCFAllocatorDefault, decompressed.as_ptr(), decompressed.len() as isize);
            if !data_ref.is_null() {
                let mut error = ptr::null_mut();
                let opts: CFURLBookmarkResolutionOptions = 1usize << 10; // kCFURLBookmarkResolutionWithSecurityScope
                let is_stale_cf: Boolean = 0;
                let url = CFURLCreateByResolvingBookmarkData(kCFAllocatorDefault, data_ref, opts, project_url_ref, ptr::null(), is_stale_cf as *mut Boolean, &mut error);
                if error.is_null() {
                    if !url.is_null() {
                        let len = CFURLGetBytes(url, ptr::null_mut(), 0);
                        let mut buf = vec![0u8; len as usize];
                        CFURLGetBytes(url, buf.as_mut_ptr(), len);
                        ret = String::from_utf8(buf).unwrap_or_default();
                        CFRelease(url as CFTypeRef);
                    }
                } else {
                    log::error!("Failed to resolve bookmark: {}", get_error(error));
                    CFRelease(error as CFTypeRef);
                }
                if is_stale_cf != 0 {
                    is_stale = true;
                }
                CFRelease(data_ref as CFTypeRef);
            }
            if !project_url_ref.is_null() {
                CFRelease(project_url_ref as CFTypeRef);
                stop_accessing_url(project_url.unwrap(), false);
            }
        }
    }
    dbg_call!(bookmark_data -> ret);
    (ret, is_stale)
}

unsafe fn get_error(err: CFErrorRef) -> String {
    let cf_str = CFErrorCopyDescription(err);
    let c_string = CFStringGetCStringPtr(cf_str, kCFStringEncodingUTF8);
    let ret = if !c_string.is_null() {
        std::ffi::CStr::from_ptr(c_string).to_string_lossy().into()
    } else {
        let range = CFRange { location: 0, length: CFStringGetLength(cf_str) };
        let mut len: CFIndex = 0;
        CFStringGetBytes(cf_str, range, kCFStringEncodingUTF8, 0, false as Boolean, ptr::null_mut(), 0, &mut len);
        let mut buffer = vec![0u8; len as usize];
        CFStringGetBytes(cf_str, range, kCFStringEncodingUTF8, 0, false as Boolean, buffer.as_mut_ptr(), buffer.len() as isize, ptr::null_mut());
        String::from_utf8_unchecked(buffer)
    };
    CFRelease(cf_str as CFTypeRef);
    ret
}
