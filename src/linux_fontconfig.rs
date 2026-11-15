// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Works around a Qt crash in font fallback on Linux.
//!
//! Qt before 6.11 (`QFontEngineMultiFontConfig::shouldLoadFontEngineForCharacter`) calls
//! `FcPatternGetCharSet` with an uninitialized `FcCharSet *`, ignores the result and passes the
//! pointer to `FcCharSetHasChar`. When the pattern of a fallback font has no charset (e.g. served
//! from a font cache written by a newer fontconfig), whatever was on the stack reaches fontconfig
//! and the app crashes with SIGSEGV while laying out text that needs a fallback font.
//! Fixed upstream in qtbase 80ddc8dee6; this file can go once the Linux build uses Qt 6.11+.
//!
//! The executable defines and exports `FcPatternGetCharSet` (see the `--dynamic-list` in build.rs).
//! The dynamic linker searches the executable first, so Qt's calls land here. We forward to
//! fontconfig and set the output to NULL when there is no charset; `FcCharSetHasChar(NULL, ..)`
//! returns false. Callers that check the result are unaffected.

use std::ffi::{ c_char, c_int, c_void };
use std::sync::atomic::{ AtomicPtr, Ordering };

type FcPatternGetCharSetFn = unsafe extern "C" fn(*const c_void, *const c_char, c_int, *mut *mut c_void) -> c_int;

const FC_RESULT_MATCH: c_int = 0;
const FC_RESULT_NO_MATCH: c_int = 1;

// glibc values
const RTLD_NEXT: *mut c_void = -1isize as *mut c_void;
const RTLD_LAZY: c_int = 1;
const RTLD_NOLOAD: c_int = 4;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

static REAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn find_real() -> *mut c_void {
    let name = c"FcPatternGetCharSet".as_ptr();
    // The next definition after this executable, i.e. fontconfig's.
    let mut ptr = unsafe { dlsym(RTLD_NEXT, name) };
    if ptr.is_null() {
        // A fontconfig that only came in with a library loaded through dlopen (RTLD_LOCAL) is not in the global scope.
        let lib = unsafe { dlopen(c"libfontconfig.so.1".as_ptr(), RTLD_LAZY | RTLD_NOLOAD) };
        if !lib.is_null() {
            ptr = unsafe { dlsym(lib, name) };
        }
    }
    ptr
}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FcPatternGetCharSet(pattern: *const c_void, object: *const c_char, id: c_int, charset: *mut *mut c_void) -> c_int {
    let mut real = REAL.load(Ordering::Acquire);
    if real.is_null() {
        real = find_real();
        REAL.store(real, Ordering::Release);
    }
    let result = if real.is_null() {
        FC_RESULT_NO_MATCH
    } else {
        let real = unsafe { std::mem::transmute::<*mut c_void, FcPatternGetCharSetFn>(real) };
        unsafe { real(pattern, object, id, charset) }
    };
    if result != FC_RESULT_MATCH && !charset.is_null() {
        unsafe { *charset = std::ptr::null_mut(); }
    }
    result
}
