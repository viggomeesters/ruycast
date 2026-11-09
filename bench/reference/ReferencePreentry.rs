//! Test-oracle admission only; ready magic does NOT prove SDK identity/coverage.
//! Linked as one no_std object into the unchanged legacy executable before main.
#![no_std]
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
#[cfg_attr(feature = "full-reference-host", export_name = "ruycast_reference_initializer_v1")]
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
    // Reference's dedicated SDK role has no initializer. Its pinned export
    // must first perform sealed admission and real module assembly; requiring
    // ready before that handoff would be circular. Native's SDK role is separate.
    #[cfg(feature = "full-reference-host")]
    if !unsafe { full_reference::handoff() } {
        unsafe { _exit(89) }
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

// This extra transport is linked ONLY by the existing full_host producer
// recipe. Partial ABI libraries and admission-only tests retain their old path.
// Direct C relocations bind the actual main-image thunks; no Swift symbol is
// discovered at runtime. The receiving SDK must qualify its private main/table/
// code pins BEFORE reading even this header. Neither table nor ready magic is
// source authority, a physical image lease or a fourteen-scope admission.
#[cfg(feature = "full-reference-host")]
mod full_reference {
    use super::*;
    type Guard = extern "C" fn(u32) -> i32;
    type Reply = extern "C" fn(*mut c_void, *const c_void, isize);
    type ProcessProvider = extern "C" fn(*mut c_void) -> *mut c_void;
    #[repr(C)]
    struct Functions {
        install_guard: unsafe extern "C" fn(Option<Guard>),
        borrow_delegate: unsafe extern "C" fn() -> *mut c_void,
        create_with_process_provider: unsafe extern "C" fn(*const c_char, i32, i32, Option<ProcessProvider>, *mut c_void) -> *mut c_void,
        cancel_system_search: unsafe extern "C" fn(*mut c_void),
        drop_box: unsafe extern "C" fn(*mut c_void),
        query: unsafe extern "C" fn(*mut c_void, *const c_void, isize) -> i32,
        mode: unsafe extern "C" fn(*mut c_void, *const c_void, isize) -> i32,
        select: unsafe extern "C" fn(*mut c_void, isize),
        actions: unsafe extern "C" fn(*mut c_void, i32),
        set_commands: unsafe extern "C" fn(*mut c_void, *const c_void, isize) -> i32,
        snapshot: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        result_count: unsafe extern "C" fn(*mut c_void) -> isize,
        visible_count: unsafe extern "C" fn(*mut c_void) -> isize,
        selected_id_count: unsafe extern "C" fn(*mut c_void) -> isize,
        filtered_count: unsafe extern "C" fn(*mut c_void) -> isize,
        apply_system_results: unsafe extern "C" fn(*mut c_void, *const c_void, isize, *const c_void, isize, i32) -> i32,
        process_sort: unsafe extern "C" fn(*mut c_void) -> i32,
        set_process_sort: unsafe extern "C" fn(*mut c_void, i32) -> i32,
        process_snapshot: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        config_data: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        encode_clips: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        record_timing: unsafe extern "C" fn(*mut c_void, *const c_void, isize, f64) -> i32,
        record_recency: unsafe extern "C" fn(*mut c_void, *const c_void, isize, f64) -> i32,
        call: unsafe extern "C" fn(*mut c_void, *const c_void, isize, Reply, *mut c_void) -> i32,
        content_view: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    }
    #[repr(C)]
    struct MainFunctionsV1 {
        magic: u64,
        abi: u32,
        count: u32,
        functions: Functions,
    }
    const _: () = assert!(core::mem::size_of::<MainFunctionsV1>() == 216);
    const _: () = assert!(core::mem::align_of::<MainFunctionsV1>() == 8);
    unsafe extern "C" {
        fn rcf1_install_guard(guard: Option<Guard>);
        fn rcf1_borrow_delegate() -> *mut c_void;
        fn rcf1_create_with_process_provider(root: *const c_char, headless: i32, native_view: i32, provider: Option<ProcessProvider>, context: *mut c_void) -> *mut c_void;
        fn rcf1_cancel_system_search(object: *mut c_void);
        fn rcf1_drop(object: *mut c_void);
        fn rcf1_query(object: *mut c_void, bytes: *const c_void, count: isize) -> i32;
        fn rcf1_mode(object: *mut c_void, bytes: *const c_void, count: isize) -> i32;
        fn rcf1_select(object: *mut c_void, index: isize);
        fn rcf1_actions(object: *mut c_void, opened: i32);
        fn rcf1_set_commands(object: *mut c_void, bytes: *const c_void, count: isize) -> i32;
        fn rcf1_snapshot(object: *mut c_void) -> *mut c_void;
        fn rcf1_result_count(object: *mut c_void) -> isize;
        fn rcf1_visible_count(object: *mut c_void) -> isize;
        fn rcf1_selected_id_count(object: *mut c_void) -> isize;
        fn rcf1_filtered_count(object: *mut c_void) -> isize;
        fn rcf1_apply_system_results(object: *mut c_void, commands: *const c_void, commands_count: isize, resources: *const c_void, resources_count: isize, preserve: i32) -> i32;
        fn rcf1_process_sort(object: *mut c_void) -> i32;
        fn rcf1_set_process_sort(object: *mut c_void, value: i32) -> i32;
        fn rcf1_process_snapshot(object: *mut c_void) -> *mut c_void;
        fn rcf1_config_data(object: *mut c_void) -> *mut c_void;
        fn rcf1_encode_clips(object: *mut c_void) -> *mut c_void;
        fn rcf1_record_timing(object: *mut c_void, bytes: *const c_void, count: isize, milliseconds: f64) -> i32;
        fn rcf1_record_recency(object: *mut c_void, bytes: *const c_void, count: isize, epoch: f64) -> i32;
        fn rcf1_call(object: *mut c_void, bytes: *const c_void, count: isize, reply: Reply, context: *mut c_void) -> i32;
        fn rcf1_content_view(object: *mut c_void) -> *mut c_void;
    }
    #[used]
    #[export_name = "RUYCAST_REFERENCE_MAIN_FUNCTIONS_V1"]
    #[link_section = "__DATA_CONST,__ruycast_ref"]
    static MAIN: MainFunctionsV1 = MainFunctionsV1 {
        magic: 0xF100_0026_4D41_494E,
        abi: 1,
        count: 25,
        functions: Functions {
            install_guard: rcf1_install_guard,
            borrow_delegate: rcf1_borrow_delegate,
            create_with_process_provider: rcf1_create_with_process_provider,
            cancel_system_search: rcf1_cancel_system_search,
            drop_box: rcf1_drop,
            query: rcf1_query,
            mode: rcf1_mode,
            select: rcf1_select,
            actions: rcf1_actions,
            set_commands: rcf1_set_commands,
            snapshot: rcf1_snapshot,
            result_count: rcf1_result_count,
            visible_count: rcf1_visible_count,
            selected_id_count: rcf1_selected_id_count,
            filtered_count: rcf1_filtered_count,
            apply_system_results: rcf1_apply_system_results,
            process_sort: rcf1_process_sort,
            set_process_sort: rcf1_set_process_sort,
            process_snapshot: rcf1_process_snapshot,
            config_data: rcf1_config_data,
            encode_clips: rcf1_encode_clips,
            record_timing: rcf1_record_timing,
            record_recency: rcf1_record_recency,
            call: rcf1_call,
            content_view: rcf1_content_view,
        },
    };
    pub unsafe fn handoff() -> bool {
        let symbol = unsafe {
            dlsym((-2_isize) as *mut c_void, c"ruycast_f1_reference_preentry_v1".as_ptr())
        };
        if symbol.is_null() {
            return false;
        }
        let enter: unsafe extern "C" fn(*const MainFunctionsV1) -> i32 =
            unsafe { core::mem::transmute(symbol) };
        // Only the already source-qualified SDK can validate this pointer;
        // success stores its actual code/image custody before original main.
        unsafe { enter(&MAIN) == 1 }
    }
}
