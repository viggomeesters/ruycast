//! Shared fake Caps SDK. Only scalar handles; never invokes IOKit.
use std::{
    ffi::{c_void, CStr},
    sync::Mutex,
};
type Hook = unsafe extern "C" fn(*mut c_void);
struct State {
    failures: u64,
    calls: u32,
    latch: bool,
    service: bool,
    connection: bool,
    hook_at: u32,
    hook: Option<Hook>,
    context: usize,
}
static STATE: Mutex<State> = Mutex::new(State {
    failures: 0,
    calls: 0,
    latch: false,
    service: false,
    connection: false,
    hook_at: 0,
    hook: None,
    context: 0,
});

fn call(name: &CStr) -> i32 {
    if !super::HOST {
        let mut line = [0 as libc::c_char; 64];
        unsafe {
            let n = libc::snprintf(
                line.as_mut_ptr(),
                line.len(),
                c"caps-fixture:%s\n".as_ptr(),
                name.as_ptr(),
            );
            libc::write(libc::STDERR_FILENO, line.as_ptr().cast(), n as usize);
        }
    }
    {
        let mut state = STATE.lock().unwrap();
        state.calls = state.calls.wrapping_add(1);
    }
    if super::HOST {
        unsafe {
            let home = libc::getenv(c"RUYCAST_KEYBOARD_FIXTURE_HOME".as_ptr());
            if !home.is_null() {
                let phase = if name == c"set:true" {
                    c"restore"
                } else {
                    name
                };
                let mut path = [0 as libc::c_char; 4096];
                libc::snprintf(
                    path.as_mut_ptr(),
                    path.len(),
                    c"%s/caps-fail-%s".as_ptr(),
                    home,
                    phase.as_ptr(),
                );
                if libc::access(path.as_ptr(), libc::F_OK) == 0 {
                    return -100;
                }
            }
        }
    }
    let hook = {
        let mut state = STATE.lock().unwrap();
        if state.calls == state.hook_at {
            state.hook.take().map(|hook| (hook, state.context))
        } else {
            None
        }
    };
    // Never hold the lock across callbacks: nested acquisition is tested.
    if let Some((hook, context)) = hook {
        unsafe {
            hook(context as *mut c_void);
        }
    }
    // Callbacks may change the failure mask or enter more SDK calls.
    let state = STATE.lock().unwrap();
    if state.calls < 64 && state.failures & (1u64 << state.calls) != 0 {
        -(state.calls as i32)
    } else {
        0
    }
}

/// # Safety
/// Callback and context must remain valid until the hook fires or is replaced.
#[no_mangle]
pub(super) unsafe extern "C" fn ruycast_caps_fixture_hook(
    at: u32,
    callback: Option<Hook>,
    context: *mut c_void,
) {
    let mut state = STATE.lock().unwrap();
    state.hook_at = at;
    state.hook = callback;
    state.context = context as usize;
}
#[no_mangle]
extern "C" fn ruycast_caps_fixture_ready() -> u64 {
    0x5255434150533031
}
#[no_mangle]
pub(super) extern "C" fn ruycast_caps_fixture_configure(original: bool, mask: u64) {
    let mut state = STATE.lock().unwrap();
    assert!(!state.connection && !state.service);
    state.calls = 0;
    state.latch = original;
    state.failures = mask;
}
#[no_mangle]
extern "C" fn ruycast_caps_fixture_failures(mask: u64) {
    STATE.lock().unwrap().failures = mask;
}
#[no_mangle]
pub(super) extern "C" fn ruycast_caps_fixture_state() -> u32 {
    let state = STATE.lock().unwrap();
    u32::from(state.latch) | (u32::from(state.connection) << 1) | (u32::from(state.service) << 2)
}
/// # Safety
/// Name must be a live NUL-terminated SDK string.
#[no_mangle]
unsafe extern "C" fn ruycast_caps_fixture_matching(name: *const libc::c_char) -> *mut c_void {
    assert_eq!(CStr::from_ptr(name), c"IOHIDSystem");
    if call(c"matching") != 0 {
        std::ptr::null_mut()
    } else {
        11usize as *mut c_void
    }
}
#[no_mangle]
extern "C" fn ruycast_caps_fixture_service(port: u32, dictionary: *mut c_void) -> u32 {
    assert!(port == 0 && dictionary as usize == 11);
    if call(c"service") != 0 {
        return 0;
    }
    let mut state = STATE.lock().unwrap();
    assert!(!state.service);
    state.service = true;
    12
}
#[no_mangle]
extern "C" fn ruycast_caps_fixture_task() -> u32 {
    call(c"task");
    13
}
/// # Safety
/// Output must point to one live initialized u32 for the duration of the call.
#[no_mangle]
unsafe extern "C" fn ruycast_caps_fixture_open(
    object: u32,
    task: u32,
    kind: u32,
    output: *mut u32,
) -> i32 {
    assert!(
        STATE.lock().unwrap().service
            && object == 12
            && task == 13
            && kind == 1
            && !output.is_null()
            && *output == 0
    );
    let result = call(c"open");
    if result != 0 {
        *output = 99;
        return result;
    }
    let mut state = STATE.lock().unwrap();
    assert!(!state.connection);
    state.connection = true;
    *output = 14;
    0
}
#[no_mangle]
extern "C" fn ruycast_caps_fixture_release(object: u32) -> i32 {
    {
        let mut state = STATE.lock().unwrap();
        assert!(state.service && object == 12);
        state.service = false;
    }
    call(c"release")
}
#[no_mangle]
extern "C" fn ruycast_caps_fixture_close(connection: u32) -> i32 {
    {
        let mut state = STATE.lock().unwrap();
        assert!(state.connection && connection == 14);
        state.connection = false;
    }
    call(c"close")
}
/// # Safety
/// Output must point to writable bool storage for the duration of the call.
#[no_mangle]
unsafe extern "C" fn ruycast_caps_fixture_get(
    connection: u32,
    selector: i32,
    output: *mut bool,
) -> i32 {
    assert!(
        STATE.lock().unwrap().connection && connection == 14 && selector == 1 && !output.is_null()
    );
    let result = call(c"get");
    if result == 0 {
        *output = STATE.lock().unwrap().latch;
    }
    result
}
// The intentionally incomplete artifact does not export a setter wrapper.
#[allow(
    dead_code,
    reason = "Shared implementation intentionally omitted from the missing-set variant"
)]
pub(super) fn set(connection: u32, selector: i32, value: bool) -> i32 {
    {
        let mut state = STATE.lock().unwrap();
        assert!(state.connection && connection == 14 && selector == 1);
        state.latch = value;
    }
    call(if value { c"set:true" } else { c"set:false" })
}
