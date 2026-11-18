//! Test-oracle admission only; ready magic does NOT prove SDK identity/coverage.
//! Linked as one no_std object into the unchanged legacy executable before main.
#![no_std]

#[cfg(feature = "full-reference-host")]
compile_error!("full-reference-host preentry transport has been retired");
use core::ffi::{c_char, c_void};
unsafe extern "C" {
    fn getenv(name: *const c_char) -> *const c_char;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn _exit(status: i32) -> !;
}
const READY: u64 = 0xF100_0026_0000_3FFF;
unsafe fn absolute(name: *const c_char) -> bool {
    let value = unsafe { getenv(name) };
    !value.is_null() && unsafe { *value } == b'/' as c_char
}
unsafe fn hash(name: *const c_char) -> bool {
    let value = unsafe { getenv(name) };
    if value.is_null() {
        return false;
    }
    for i in 0..64 {
        let byte = unsafe { *value.add(i) } as u8;
        if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    (unsafe { *value.add(64) }) == 0
}
unsafe extern "C" fn preentry() {
    let admitted = unsafe {
        absolute(c"HOME".as_ptr())
            && absolute(c"RUYCAST_HOME".as_ptr())
            && absolute(c"RUSTCAST_HOME".as_ptr())
            && hash(c"RUYCAST_F1_AUTHORITY_SHA256".as_ptr())
            && hash(c"RUYCAST_F1_GUARD_SHA256".as_ptr())
    };
    if !admitted {
        unsafe { _exit(86) }
    }
    let symbol = unsafe {
        dlsym(
            (-2_isize) as *mut c_void,
            c"ruycast_f1_preentry_ready_v1".as_ptr(),
        )
    };
    if symbol.is_null() {
        unsafe { _exit(87) }
    }
    let ready: unsafe extern "C" fn() -> u64 = unsafe { core::mem::transmute(symbol) };
    if unsafe { ready() } != READY {
        unsafe { _exit(88) }
    }
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static PREENTRY: unsafe extern "C" fn() = preentry;
