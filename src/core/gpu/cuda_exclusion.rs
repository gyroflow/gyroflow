// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! The CRM decoder (MDK's `mdk-crm` plugin) develops frames with CUDA calls that no other CUDA call of the process may
//! overlap. The plugin exports its lock, `mdk_crm_cuda_exclusive`, and Gyroflow's own CUDA work runs under it: its CUDA
//! interop, and FFmpeg's decoding and encoding on CUDA (each codec call, and the creation and release of the CUDA
//! codecs and devices). The lock is taken whenever the plugin is loaded, also before the first CRM clip is opened, so
//! that work which began earlier can't overlap the decoder's first develops; it's reentrant and uncontended then.
//! Without the plugin (the video editor plugins, macOS, where the decoder develops on Metal) the work runs as it is.
//!
//! Keep the locked sections to the CUDA calls themselves: the decoder can't develop a frame, nor the player show one,
//! while the lock is held, so nothing that waits (a pause, a callback into the application) belongs inside.
//!
//! Not covered: CUDA work that MDK itself does for other media (its NVDEC decoder), which is outside Gyroflow's reach.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::OnceLock;

type ExclusiveFn = unsafe extern "C" fn(f: unsafe extern "C" fn(*mut c_void), user: *mut c_void);

static EXCLUSIVE: OnceLock<ExclusiveFn> = OnceLock::new();

/// The plugin's lock once the plugin is loaded (MDK loads it at startup, with its first player). Looked up on every
/// call until found (a lookup among the loaded modules, nothing is loaded), then kept: the plugin is never unloaded.
fn plugin_lock() -> Option<ExclusiveFn> {
    if let Some(f) = EXCLUSIVE.get() {
        return Some(*f);
    }
    let f = lookup()?;
    Some(*EXCLUSIVE.get_or_init(|| f))
}

#[cfg(target_os = "windows")]
fn lookup() -> Option<ExclusiveFn> {
    // Only finds a module already in the process; nothing is loaded or initialised.
    let lib = libloading::os::windows::Library::open_already_loaded("mdk-crm.dll").ok()?;
    resolve(lib.into())
}

#[cfg(target_os = "linux")]
fn lookup() -> Option<ExclusiveFn> {
    // The plugin has no soname, and MDK loads it by its path: find that path among the loaded objects.
    unsafe extern "C" fn find(info: *mut libc::dl_phdr_info, _size: libc::size_t, data: *mut c_void) -> libc::c_int {
        // SAFETY: the loader hands a valid entry; `data` is the `Option<CString>` below.
        let name = unsafe { (*info).dlpi_name };
        if name.is_null() {
            return 0;
        }
        let path = unsafe { std::ffi::CStr::from_ptr(name) };
        if path.to_bytes().ends_with(b"/libmdk-crm.so") {
            unsafe { *(data as *mut Option<std::ffi::CString>) = Some(path.to_owned()) };
            return 1;
        }
        0
    }
    let mut path: Option<std::ffi::CString> = None;
    // SAFETY: `find` only reads the entries and writes `path`.
    unsafe { libc::dl_iterate_phdr(Some(find), &mut path as *mut _ as *mut c_void) };
    let path = path?;
    let path = std::path::PathBuf::from(std::ffi::OsStr::new(path.to_str().ok()?));
    // SAFETY: RTLD_NOLOAD only takes another reference to the plugin that is already loaded.
    let lib = unsafe { libloading::os::unix::Library::open(Some(&path), libloading::os::unix::RTLD_NOW | libc::RTLD_NOLOAD) }.ok()?;
    resolve(lib.into())
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn lookup() -> Option<ExclusiveFn> {
    None // the CRM decoder develops on Metal here
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn resolve(lib: libloading::Library) -> Option<ExclusiveFn> {
    // SAFETY: the plugin's exported C function, with this signature.
    let f = unsafe { *lib.get::<ExclusiveFn>(b"mdk_crm_cuda_exclusive\0").ok()? };
    std::mem::forget(lib); // kept loaded, as the plugin is
    Some(f)
}

/// Run `f` under the CRM decoder's CUDA lock when its plugin is loaded, and as it is otherwise. Reentrant on the
/// calling thread. A panic in `f` comes back out of this function.
pub fn exclusive<R, F: FnOnce() -> R>(f: F) -> R {
    let Some(lock) = plugin_lock() else { return f() };
    struct Call<F, R> {
        f: Option<F>,
        result: Option<std::thread::Result<R>>,
    }
    unsafe extern "C" fn trampoline<F: FnOnce() -> R, R>(user: *mut c_void) {
        // SAFETY: `user` is the `Call` below, alive for the whole call.
        let call = unsafe { &mut *(user as *mut Call<F, R>) };
        if let Some(f) = call.f.take() {
            // No unwinding into the plugin.
            call.result = Some(catch_unwind(AssertUnwindSafe(f)));
        }
    }
    let mut call: Call<F, R> = Call { f: Some(f), result: None };
    // SAFETY: the plugin calls `trampoline` with `user` before returning.
    unsafe { lock(trampoline::<F, R>, &mut call as *mut Call<F, R> as *mut c_void) };
    match call.result {
        Some(Ok(r)) => r,
        Some(Err(p)) => resume_unwind(p),
        // The plugin did not run it (it always does): run it as it is.
        None => (call.f.take().expect("not run"))(),
    }
}
