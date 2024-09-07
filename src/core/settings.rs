// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

use app_dirs2::{ AppDataType, AppInfo };
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::{ Arc, atomic::{ AtomicUsize, Ordering::SeqCst } };
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    let mut path = app_dirs2::get_app_dir(AppDataType::UserData, &AppInfo { name: "Gyroflow", author: "Gyroflow" }, "").unwrap();
    if path.file_name().unwrap() == path.parent().unwrap().file_name().unwrap() {
        path = path.parent().unwrap().to_path_buf();
    }

    #[cfg(target_os = "macos")]
    unsafe {
        use std::ffi::{CStr, OsString};
        use std::mem::MaybeUninit;
        use std::os::unix::ffi::OsStringExt;
        let init_size = match libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) {
            -1 => 1024,
            n => n as usize,
        };
        let mut buf = Vec::with_capacity(init_size);
        let mut pwd: MaybeUninit<libc::passwd> = MaybeUninit::uninit();
        let mut pwdp = std::ptr::null_mut();
        match libc::getpwuid_r(libc::geteuid(), pwd.as_mut_ptr(), buf.as_mut_ptr(), buf.capacity(), &mut pwdp) {
            0 if !pwdp.is_null() => {
                let pwd = pwd.assume_init();
                let bytes = CStr::from_ptr(pwd.pw_dir).to_bytes().to_vec();
                let pw_dir = OsString::from_vec(bytes);
                path = PathBuf::from(pw_dir);
                path.push("Library");
                path.push("Application Support");
                path.push("Gyroflow");
            }
            _ => { },
        }
    }
    let _ = std::fs::create_dir_all(&path);
    path
}

pub fn get_all() -> HashMap<String, serde_json::Value> {
    map().read().clone()
}

pub fn get(key: &str, default: serde_json::Value) -> serde_json::Value {
    map().read().get(key).unwrap_or(&default).clone()
}

pub fn set(key: &str, value: serde_json::Value) {
    map().write().insert(key.to_string(), value);
    spawn_store_thread();
}

pub fn contains(key: &str) -> bool {
    map().read().contains_key(key)
}

pub fn clear() {
    map().write().clear();
    store();
}

pub fn try_get(key: &str) -> Option<serde_json::Value> { map().read().get(key).map(Clone::clone) }
pub fn get_u64(key: &str, default: u64) -> u64 { map().read().get(key).and_then(|x| x.as_u64()).unwrap_or(default) }
pub fn get_f64(key: &str, default: f64) -> f64 { map().read().get(key).and_then(|x| x.as_f64()).unwrap_or(default) }
pub fn get_bool(key: &str, default: bool) -> bool { map().read().get(key).and_then(|x| x.as_bool()).unwrap_or(default) }
pub fn get_str(key: &str, default: &str) -> String { map().read().get(key).and_then(|x| x.as_str()).map(|x| x.to_owned()).unwrap_or_else(|| default.to_owned()) }

fn map() -> Arc<RwLock<HashMap<String, serde_json::Value>>> {
    static MAP: std::sync::OnceLock<Arc<RwLock<HashMap<String, serde_json::Value>>>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = HashMap::new();
        let file = data_dir().join("settings.json");
        log::info!("Settings file path: {}", file.display());

        if let Ok(v) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&std::fs::read_to_string(file).unwrap_or_default()) {
            map = v;
        }

        Arc::new(RwLock::new(map))
    }).clone()
}

fn timestamp() -> usize { std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap().as_secs() as usize }
fn spawn_store_thread() {
    static STORE_TIMEOUT: AtomicUsize = AtomicUsize::new(0);

    let is_thread_running = STORE_TIMEOUT.load(SeqCst) != 0;
    STORE_TIMEOUT.store(timestamp() + 1, SeqCst); // 1 second

    if is_thread_running { return; }
    std::thread::spawn(|| {
        while STORE_TIMEOUT.load(SeqCst) > timestamp() {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        store();
        STORE_TIMEOUT.store(0, SeqCst);
    });
}

fn store() {
    let file = data_dir().join("settings.json");
    let map = map().read().clone();
    let json = serde_json::to_string_pretty(&map).unwrap();
    if let Err(e) = std::fs::write(&file, json) {
        log::error!("Failed to write the settings file {file:?}: {e:?}");
    } else {
        log::info!("Settings saved to {file:?}");
    }
}
