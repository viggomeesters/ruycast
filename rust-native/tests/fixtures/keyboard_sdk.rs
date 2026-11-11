//! Owned Carbon substitute. No global hotkey target/handler/registration calls.
use std::{
    ffi::{c_void, CStr},
    sync::Mutex,
};
type Handler = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;
const KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const HOTKEY: u32 = u32::from_be_bytes(*b"hkid");
const DIRECT: u32 = u32::from_be_bytes(*b"----");
const NOT_HANDLED: i32 = -9874;
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct HotkeyId {
    pub signature: u32,
    pub id: u32,
}
#[repr(C)]
pub(super) struct EventType {
    class: u32,
    kind: u32,
}
#[repr(C)]
pub(super) struct FixtureEvent {
    event_class: u32,
    kind: u32,
    signature: u32,
    id: u32,
    status: i32,
    parameter_type: u32,
    size: usize,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Key {
    code: u32,
    modifiers: u32,
    id: u32,
}
struct State {
    callback: Option<Handler>,
    userdata: usize,
    keys: [Option<Box<Key>>; 256],
}
static STATE: Mutex<State> = Mutex::new(State {
    callback: None,
    userdata: 0,
    keys: [const { None }; 256],
});
// Only addresses are observed; different scalar values ensure distinct tokens.
static TARGET_TOKEN: i32 = 1;
static HANDLER_TOKEN: i32 = 2;
fn target() -> *mut c_void {
    std::ptr::addr_of!(TARGET_TOKEN).cast_mut().cast()
}
fn handler_token() -> *mut c_void {
    std::ptr::addr_of!(HANDLER_TOKEN).cast_mut().cast()
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: u8) -> i32;
}
unsafe fn path(name: &CStr) -> [libc::c_char; 4096] {
    let mut path = [0; 4096];
    libc::snprintf(
        path.as_mut_ptr(),
        path.len(),
        c"%s/%s".as_ptr(),
        libc::getenv(c"RUYCAST_KEYBOARD_FIXTURE_HOME".as_ptr()),
        name.as_ptr(),
    );
    path
}
unsafe fn flag(name: &CStr) -> bool {
    !libc::getenv(c"RUYCAST_KEYBOARD_FIXTURE_HOME".as_ptr()).is_null()
        && libc::access(path(name).as_ptr(), libc::F_OK) == 0
}
unsafe fn record(phase: &CStr, a: libc::c_long, b: libc::c_long, c: libc::c_long) {
    if libc::getenv(c"RUYCAST_KEYBOARD_FIXTURE_HOME".as_ptr()).is_null()
        || libc::pthread_main_np() == 0
    {
        libc::abort();
    }
    let file = libc::fopen(path(c"keyboard-trace.jsonl").as_ptr(), c"a".as_ptr());
    if file.is_null() {
        libc::abort();
    }
    libc::fprintf(
        file,
        c"{\"phase\":\"%s\",\"a\":%ld,\"b\":%ld,\"c\":%ld,\"mainThread\":true}\n".as_ptr(),
        phase.as_ptr(),
        a,
        b,
        c,
    );
    libc::fclose(file);
}
unsafe fn fail_code() -> i32 {
    let file = libc::fopen(path(c"fail-code").as_ptr(), c"r".as_ptr());
    let mut code = -1;
    if !file.is_null() {
        if libc::fscanf(file, c"%d".as_ptr(), &mut code) != 1 {
            code = -1;
        }
        libc::fclose(file);
    }
    code
}
unsafe fn hold(entered: &CStr, release: &CStr, pump: bool, failure: i32) -> i32 {
    let entered = path(entered);
    let release = path(release);
    let file = libc::fopen(entered.as_ptr(), c"w".as_ptr());
    if file.is_null() {
        libc::abort();
    }
    libc::fputs(c"entered".as_ptr(), file);
    libc::fclose(file);
    for _ in 0..5000 {
        if libc::access(release.as_ptr(), libc::F_OK) == 0 {
            return 0;
        }
        // No SDK lock is held during the nested runloop or callback delivery.
        if pump || flag(c"pump-register") {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.001, 1);
        } else {
            libc::usleep(1000);
        }
    }
    if libc::access(release.as_ptr(), libc::F_OK) == 0 {
        0
    } else {
        failure
    }
}
unsafe fn emit(event: *const FixtureEvent) -> i32 {
    let (callback, data) = {
        let state = STATE.lock().unwrap();
        (state.callback, state.userdata)
    };
    callback.map_or(NOT_HANDLED, |callback| {
        callback(
            std::ptr::null_mut(),
            event.cast_mut().cast(),
            data as *mut c_void,
        )
    })
}
unsafe fn reenter(id: u32, phase: &CStr) {
    let event = FixtureEvent {
        event_class: KEYBOARD,
        kind: 5,
        signature: 0x52554341,
        id,
        status: 0,
        parameter_type: HOTKEY,
        size: 8,
    };
    let result = emit(&event);
    record(phase, id.into(), result.into(), 0);
}
/// # Safety
/// The fixture environment must remain stable while the SDK is in use.
#[no_mangle]
pub(super) unsafe extern "C" fn ruycast_keyboard_fixture_ready() -> u64 {
    if let Some(gate) = super::KEYBOARD_READY_GATE {
        if !gate() {
            return 0;
        }
    }
    if libc::getenv(c"RUYCAST_KEYBOARD_FIXTURE_HOME".as_ptr()).is_null() {
        return 0;
    }
    let file = libc::fopen(path(c".ruycast-keyboard-fixture").as_ptr(), c"r".as_ptr());
    if file.is_null() {
        return 0;
    }
    let mut text = [0 as libc::c_char; 64];
    libc::fread(text.as_mut_ptr().cast(), 1, 63, file);
    libc::fclose(file);
    if libc::strcmp(text.as_ptr(), c"own-keyboard-fixture-v1".as_ptr()) == 0 {
        0x52554b4559533031
    } else {
        0
    }
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_fixture_target() -> *mut c_void {
    record(c"target", 0, 0, 0);
    target()
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_fixture_install(
    target_value: *mut c_void,
    handler: Option<Handler>,
    count: u32,
    types: *const EventType,
    data: *mut c_void,
    output: *mut *mut c_void,
) -> i32 {
    assert!(
        target_value == target()
            && count == 1
            && (*types).class == KEYBOARD
            && (*types).kind == 5
            && handler.is_some()
            && !data.is_null()
    );
    record(
        c"install",
        count.into(),
        (*types).class.into(),
        (*types).kind.into(),
    );
    if flag(c"fail-install") {
        return -50;
    }
    {
        let mut state = STATE.lock().unwrap();
        state.callback = handler;
        state.userdata = data as usize;
    }
    *output = if flag(c"null-install") {
        std::ptr::null_mut()
    } else {
        handler_token()
    };
    0
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_fixture_remove(handler: *mut c_void) -> i32 {
    assert_eq!(handler, handler_token());
    record(c"remove", 0, 0, 0);
    if flag(c"fail-remove") {
        return -50;
    }
    let mut state = STATE.lock().unwrap();
    state.callback = None;
    state.userdata = 0;
    0
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_fixture_class(event: *mut c_void) -> u32 {
    (*event.cast::<FixtureEvent>()).event_class
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_fixture_kind(event: *mut c_void) -> u32 {
    (*event.cast::<FixtureEvent>()).kind
}
/// # Safety
/// Event and output pointers must be live, aligned and large enough for their ABI types.
#[allow(
    dead_code,
    reason = "The missing-parameter artifact deliberately omits its export wrapper"
)]
pub(super) unsafe fn parameter(
    event: *mut c_void,
    name: u32,
    kind: u32,
    actual: *mut u32,
    capacity: usize,
    size: *mut usize,
    output: *mut c_void,
) -> i32 {
    assert!(
        name == DIRECT
            && kind == HOTKEY
            && capacity == 8
            && !actual.is_null()
            && !size.is_null()
            && !output.is_null()
    );
    let event = event.cast::<FixtureEvent>();
    record(
        c"parameter",
        (*event).id.into(),
        capacity as libc::c_long,
        (*event).size as libc::c_long,
    );
    *actual = (*event).parameter_type;
    *size = (*event).size;
    let id = HotkeyId {
        signature: (*event).signature,
        id: (*event).id,
    };
    std::ptr::copy_nonoverlapping(
        (&id as *const HotkeyId).cast::<u8>(),
        output.cast::<u8>(),
        8,
    );
    (*event).status
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_fixture_register(
    code: u32,
    modifiers: u32,
    id: HotkeyId,
    target_value: *mut c_void,
    options: u32,
    output: *mut *mut c_void,
) -> i32 {
    assert!(
        target_value == target() && options == 0 && id.signature == 0x52554341 && !output.is_null()
    );
    record(c"register", code.into(), modifiers.into(), id.id.into());
    if flag(c"hold-register") {
        let result = hold(
            c"keyboard-register-entered",
            c"keyboard-register-release",
            false,
            -9878,
        );
        if result != 0 {
            return result;
        }
    }
    if code as i32 == fail_code() {
        return -9878;
    }
    let handle = {
        let mut state = STATE.lock().unwrap();
        if state
            .keys
            .iter()
            .flatten()
            .any(|key| key.code == code && key.modifiers == modifiers)
        {
            return -9878;
        }
        let Some(slot) = state.keys.iter_mut().find(|slot| slot.is_none()) else {
            return -108;
        };
        let mut key = Box::new(Key {
            code,
            modifiers,
            id: id.id,
        });
        let handle = (&mut *key as *mut Key).cast::<c_void>();
        *slot = Some(key);
        handle
    };
    *output = handle;
    if flag(c"emit-on-register") {
        reenter(id.id, c"register-callback");
    }
    if flag(c"partial-register") {
        -50
    } else {
        0
    }
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_fixture_unregister(handle: *mut c_void) -> i32 {
    if flag(c"emit-latest-on-unregister") {
        let latest = STATE
            .lock()
            .unwrap()
            .keys
            .iter()
            .flatten()
            .map(|key| key.id)
            .max()
            .unwrap_or(0);
        if latest != 0 {
            reenter(latest, c"unregister-latest-callback");
        }
    }
    if flag(c"hold-unregister") {
        let result = hold(
            c"keyboard-unregister-entered",
            c"keyboard-unregister-release",
            true,
            -50,
        );
        if result != 0 {
            return result;
        }
    }
    let found = {
        let state = STATE.lock().unwrap();
        state.keys.iter().enumerate().find_map(|(index, key)| {
            key.as_ref()
                .filter(|key| (&***key as *const Key).cast_mut().cast::<c_void>() == handle)
                .map(|key| (index, **key))
        })
    };
    let Some((index, key)) = found else {
        libc::abort();
    };
    record(
        c"unregister",
        key.code.into(),
        key.modifiers.into(),
        key.id.into(),
    );
    if flag(c"emit-on-unregister") {
        reenter(key.id, c"unregister-callback");
    }
    if flag(c"fail-unregister") {
        return -50;
    }
    // Match the original slot cleanup, even when a nested callback changed it.
    STATE.lock().unwrap().keys[index] = None;
    0
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_fixture_emit(event: *const FixtureEvent) -> i32 {
    emit(event)
}
/// # Safety
/// Must run on the fixture main thread with stable callback userdata.
#[no_mangle]
pub(super) unsafe extern "C" fn ruycast_keyboard_fixture_emit_binding(
    code: u32,
    modifiers: u32,
) -> i32 {
    let id = STATE
        .lock()
        .unwrap()
        .keys
        .iter()
        .flatten()
        .find(|key| key.code == code && key.modifiers == modifiers)
        .map(|key| key.id);
    let Some(id) = id else {
        record(c"emit-missing", code.into(), modifiers.into(), 0);
        return NOT_HANDLED;
    };
    let event = FixtureEvent {
        event_class: KEYBOARD,
        kind: 5,
        signature: 0x52554341,
        id,
        status: 0,
        parameter_type: HOTKEY,
        size: 8,
    };
    let result = emit(&event);
    record(c"emit-binding", id.into(), result.into(), 0);
    result
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_fixture_emit_id(id: u32) -> i32 {
    let event = FixtureEvent {
        event_class: KEYBOARD,
        kind: 5,
        signature: 0x52554341,
        id,
        status: 0,
        parameter_type: HOTKEY,
        size: 8,
    };
    let result = emit(&event);
    record(c"emit-id", id.into(), result.into(), 0);
    result
}
#[export_name = "ruycast_keyboard_fixture_cleanup"]
unsafe extern "C" fn cleanup() {
    for slot in &mut STATE.lock().unwrap().keys {
        *slot = None;
    }
}
#[used]
#[link_section = "__DATA,__mod_term_func"]
static CLEANUP: unsafe extern "C" fn() = cleanup;
const _: [(); 8] = [(); std::mem::size_of::<HotkeyId>()];
