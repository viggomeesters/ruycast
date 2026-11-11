//! Standalone owned HID/Quartz provider; never invokes native listeners/posts.
use std::{
    ffi::{c_void, CStr},
    sync::Mutex,
};
type Ptr = *mut c_void;
type HidCallback = unsafe extern "C" fn(Ptr, i32, Ptr, Ptr);
type TapCallback = unsafe extern "C" fn(Ptr, u32, Ptr, Ptr) -> Ptr;
type Hook = unsafe extern "C" fn(Ptr);
pub(super) struct Host {
    pub ready: unsafe extern "C" fn() -> u64,
    pub emit: unsafe extern "C" fn(u32, u32) -> i32,
}
#[derive(Default)]
struct Obj {
    kind: u32,
    valid: bool,
    enabled: bool,
    scheduled: bool,
    input: Option<HidCallback>,
    remove: Option<HidCallback>,
    tap: Option<TapCallback>,
    context: usize,
    flags: u64,
    key: i64,
    repeat: i64,
    tag: i64,
    event_type: u32,
    down: bool,
}
struct State {
    manager: usize,
    tap: usize,
    allocated: u32,
    posts: u32,
    envflags: u32,
    manager_creates: u32,
    last_flags: u64,
    last_event_flags: u64,
    last_down: bool,
    failure: [libc::c_char; 100],
    hook_name: [libc::c_char; 100],
    hook: Option<Hook>,
    hook_context: usize,
    now: f64,
    quiet: bool,
    initialized: bool,
    buffer: [libc::c_char; 500],
}
static STATE: Mutex<State> = Mutex::new(State {
    manager: 0,
    tap: 0,
    allocated: 0,
    posts: 0,
    envflags: 0,
    manager_creates: 0,
    last_flags: 0,
    last_event_flags: 0,
    last_down: false,
    failure: [0; 100],
    hook_name: [0; 100],
    hook: None,
    hook_context: 0,
    now: 0.0,
    quiet: false,
    initialized: false,
    buffer: [0; 500],
});
static DEVICES: [i32; 16] = [0; 16];
fn manager() -> *mut Obj {
    STATE.lock().unwrap().manager as *mut Obj
}
fn tap() -> *mut Obj {
    STATE.lock().unwrap().tap as *mut Obj
}
unsafe fn hidcall(name: &CStr) -> bool {
    let quiet = STATE.lock().unwrap().quiet;
    if !quiet {
        let mut line = [0 as libc::c_char; 100];
        let n = libc::snprintf(
            line.as_mut_ptr(),
            line.len(),
            c"hid-fixture:%s\n".as_ptr(),
            name.as_ptr(),
        );
        libc::write(libc::STDERR_FILENO, line.as_ptr().cast(), n as usize);
    }
    let hook = {
        let mut state = STATE.lock().unwrap();
        if libc::strcmp(state.hook_name.as_ptr(), name.as_ptr()) == 0 {
            state.hook.take().map(|hook| (hook, state.hook_context))
        } else {
            None
        }
    };
    // Never retain a lock or object borrow while a hook can reenter the owner.
    if let Some((hook, context)) = hook {
        hook(context as Ptr);
    }
    if super::HID_HOST.is_some() {
        let home = libc::getenv(c"RUYCAST_KEYBOARD_FIXTURE_HOME".as_ptr());
        if !home.is_null() {
            let mut path = [0 as libc::c_char; 4096];
            libc::snprintf(
                path.as_mut_ptr(),
                path.len(),
                c"%s/hid-hold-%s".as_ptr(),
                home,
                name.as_ptr(),
            );
            if libc::access(path.as_ptr(), libc::F_OK) == 0 {
                let mut entered = [0 as libc::c_char; 4096];
                libc::snprintf(
                    entered.as_mut_ptr(),
                    entered.len(),
                    c"%s/hid-entered-%s".as_ptr(),
                    home,
                    name.as_ptr(),
                );
                let file = libc::fopen(entered.as_ptr(), c"w".as_ptr());
                assert!(!file.is_null());
                libc::fputs(c"entered".as_ptr(), file);
                libc::fclose(file);
                // No SDK lock or object borrow across the external hold.
                for _ in 0..5000 {
                    if libc::access(path.as_ptr(), libc::F_OK) != 0 {
                        break;
                    }
                    libc::usleep(1000);
                }
                assert_ne!(libc::access(path.as_ptr(), libc::F_OK), 0);
            }
            libc::snprintf(
                path.as_mut_ptr(),
                path.len(),
                c"%s/hid-fail-%s".as_ptr(),
                home,
                name.as_ptr(),
            );
            if libc::access(path.as_ptr(), libc::F_OK) == 0 {
                return true;
            }
        }
    }
    libc::strcmp(STATE.lock().unwrap().failure.as_ptr(), name.as_ptr()) == 0
}
fn object(kind: u32) -> Ptr {
    let value = Box::into_raw(Box::new(Obj {
        kind,
        valid: true,
        ..Obj::default()
    }));
    let mut state = STATE.lock().unwrap();
    state.allocated = state.allocated.wrapping_add(1);
    value.cast()
}
macro_rules! sdk {
    ($local:ident => $native:ident($($arg:ident: $ty:ty),*) -> $ret:ty $body:block) => {
        #[export_name = concat!("ruycast_hid_fixture_", stringify!($native))]
        unsafe extern "C" fn $local($($arg: $ty),*) -> $ret $body
    };
}
#[no_mangle]
extern "C" fn ruycast_hid_fixture_ready() -> u64 {
    if let Some(host) = &super::HID_HOST {
        if unsafe { (host.ready)() } == 0 {
            return 0;
        }
        let initialize = {
            let mut state = STATE.lock().unwrap();
            let initialize = !state.initialized;
            state.initialized = true;
            if initialize {
                state.quiet = true;
            }
            initialize
        };
        if initialize {
            super::caps_sdk::initialize_host();
        }
    }
    0x5255484944303031
}
#[no_mangle]
extern "C" fn ruycast_hid_fixture_quiet(value: bool) {
    STATE.lock().unwrap().quiet = value;
}
#[no_mangle]
extern "C" fn ruycast_hid_fixture_configure(original: bool, environment: u32) {
    assert_eq!(STATE.lock().unwrap().allocated, 0);
    super::caps_sdk::ruycast_caps_fixture_configure(original, 0);
    let mut state = STATE.lock().unwrap();
    state.posts = 0;
    state.last_flags = 0;
    state.last_down = false;
    state.envflags = environment;
    state.failure[0] = 0;
    state.hook = None;
    state.now = 0.0;
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_failure(name: *const libc::c_char) {
    let mut state = STATE.lock().unwrap();
    libc::snprintf(
        state.failure.as_mut_ptr(),
        state.failure.len(),
        c"%s".as_ptr(),
        name,
    );
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_hook(
    name: *const libc::c_char,
    hook: Option<Hook>,
    context: Ptr,
) {
    let mut state = STATE.lock().unwrap();
    libc::snprintf(
        state.hook_name.as_mut_ptr(),
        state.hook_name.len(),
        c"%s".as_ptr(),
        name,
    );
    state.hook = hook;
    state.hook_context = context as usize;
}
#[no_mangle]
extern "C" fn ruycast_hid_fixture_environment(value: u32) {
    STATE.lock().unwrap().envflags = value;
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_state() -> *const libc::c_char {
    let caps = super::caps_sdk::ruycast_caps_fixture_state();
    let mut state = STATE.lock().unwrap();
    let (allocated, creates, posts, flags, down, manager, tap) = (
        state.allocated,
        state.manager_creates,
        state.posts,
        state.last_flags,
        state.last_down,
        state.manager != 0,
        state.tap != 0,
    );
    libc::snprintf(state.buffer.as_mut_ptr(), 500,
        c"{\"allocated\":%u,\"managerCreates\":%u,\"posts\":%u,\"lastFlags\":%llu,\"lastDown\":%s,\"caps\":%u,\"manager\":%s,\"tap\":%s}".as_ptr(),
        allocated, creates, posts, flags, if down { c"true" } else { c"false" }.as_ptr(), caps,
        if manager { c"true" } else { c"false" }.as_ptr(), if tap { c"true" } else { c"false" }.as_ptr());
    state.buffer.as_ptr()
}
sdk!(manager_create => IOHIDManagerCreate(allocator: Ptr, options: u32) -> Ptr {
    assert!(allocator.is_null() && options == 0);
    if hidcall(c"manager_create") { return std::ptr::null_mut(); }
    { let mut state = STATE.lock().unwrap(); assert_eq!(state.manager, 0); state.manager_creates = state.manager_creates.wrapping_add(1); }
    let value = object(1); STATE.lock().unwrap().manager = value as usize; value
});
sdk!(manager_open => IOHIDManagerOpen(value: Ptr, options: u32) -> i32 {
    assert!(value == manager().cast() && options == 0); if hidcall(c"manager_open") { -101 } else { 0 }
});
sdk!(manager_close => IOHIDManagerClose(value: Ptr, options: u32) -> i32 {
    assert!(value == manager().cast() && options == 0); if hidcall(c"manager_close") { -102 } else { 0 }
});
sdk!(device_matching => IOHIDManagerSetDeviceMatching(value: Ptr, dictionary: Ptr) -> () {
    assert!(value == manager().cast() && (*dictionary.cast::<Obj>()).kind == 4); hidcall(c"device_matching");
});
sdk!(input_matching => IOHIDManagerSetInputValueMatching(value: Ptr, dictionary: Ptr) -> () {
    assert!(value == manager().cast() && (*dictionary.cast::<Obj>()).kind == 5); hidcall(c"input_matching");
});
sdk!(register_input => IOHIDManagerRegisterInputValueCallback(value: Ptr, callback: Option<HidCallback>, context: Ptr) -> () {
    assert_eq!(value, manager().cast()); (*manager()).input = callback; (*manager()).context = context as usize;
    hidcall(if callback.is_some() { c"register_input" } else { c"unregister_input" });
});
sdk!(register_remove => IOHIDManagerRegisterDeviceRemovalCallback(value: Ptr, callback: Option<HidCallback>, context: Ptr) -> () {
    assert_eq!(value, manager().cast()); (*manager()).remove = callback; (*manager()).context = context as usize;
    hidcall(if callback.is_some() { c"register_remove" } else { c"unregister_remove" });
});
sdk!(schedule => IOHIDManagerScheduleWithRunLoop(value: Ptr, runloop: Ptr, mode: Ptr) -> () {
    assert!(value == manager().cast() && runloop as usize == 40 && mode as usize == 41); (*manager()).scheduled = true; hidcall(c"schedule");
});
sdk!(unschedule => IOHIDManagerUnscheduleFromRunLoop(value: Ptr, runloop: Ptr, mode: Ptr) -> () {
    assert!(value == manager().cast() && runloop as usize == 40 && mode as usize == 41); (*manager()).scheduled = false; hidcall(c"unschedule");
});
#[repr(C)]
struct Input {
    page: u32,
    usage: u32,
    device: Ptr,
    down: bool,
}
sdk!(element => IOHIDValueGetElement(value: Ptr) -> Ptr { hidcall(c"element"); value });
sdk!(integer => IOHIDValueGetIntegerValue(value: Ptr) -> isize { hidcall(c"integer"); (*value.cast::<Input>()).down as isize });
sdk!(usage_page => IOHIDElementGetUsagePage(value: Ptr) -> u32 { hidcall(c"usage_page"); (*value.cast::<Input>()).page });
sdk!(usage => IOHIDElementGetUsage(value: Ptr) -> u32 { hidcall(c"usage"); (*value.cast::<Input>()).usage });
sdk!(device => IOHIDElementGetDevice(value: Ptr) -> Ptr { hidcall(c"device"); (*value.cast::<Input>()).device });
sdk!(tap_create => CGEventTapCreate(location: u32, place: u32, options: u32, mask: u64, callback: Option<TapCallback>, context: Ptr) -> Ptr {
    const MASK: u64 = (1 << 1) | (1 << 3) | (1 << 10) | (1 << 11) | (1 << 12) | (1 << 25) | (1 << 22);
    assert!(location == 1 && place == 0 && options == 0 && mask == MASK);
    if hidcall(c"tap_create") { return std::ptr::null_mut(); }
    assert!(tap().is_null());
    let value = object(2).cast::<Obj>(); (*value).tap = callback; (*value).context = context as usize;
    STATE.lock().unwrap().tap = value as usize; value.cast()
});
sdk!(tap_enable => CGEventTapEnable(value: Ptr, enable: bool) -> () {
    assert_eq!(value, tap().cast()); if !hidcall(if enable { c"tap_enable" } else { c"tap_disable" }) { (*tap()).enabled = enable; }
});
sdk!(tap_enabled => CGEventTapIsEnabled(value: Ptr) -> bool { assert_eq!(value, tap().cast()); hidcall(c"tap_enabled"); (*tap()).enabled });
sdk!(port_valid => CFMachPortIsValid(value: Ptr) -> u8 { assert_eq!(value, tap().cast()); hidcall(c"port_valid"); (*tap()).valid as u8 });
sdk!(invalidate => CFMachPortInvalidate(value: Ptr) -> () { assert_eq!(value, tap().cast()); if !hidcall(c"invalidate") { (*tap()).valid = false; } });
sdk!(source => CFMachPortCreateRunLoopSource(allocator: Ptr, value: Ptr, order: isize) -> Ptr {
    assert!(allocator.is_null() && value == tap().cast() && order == 0);
    if hidcall(c"source") { std::ptr::null_mut() } else { object(3) }
});
sdk!(runloop => CFRunLoopGetMain() -> Ptr { hidcall(c"runloop"); 40usize as Ptr });
sdk!(add_source => CFRunLoopAddSource(runloop: Ptr, source: Ptr, mode: Ptr) -> () {
    assert!(runloop as usize == 40 && (*source.cast::<Obj>()).kind == 3 && mode as usize == 41); hidcall(c"add_source");
});
sdk!(remove_source => CFRunLoopRemoveSource(runloop: Ptr, source: Ptr, mode: Ptr) -> () {
    assert!(runloop as usize == 40 && (*source.cast::<Obj>()).kind == 3 && mode as usize == 41); hidcall(c"remove_source");
});
sdk!(release => CFRelease(value: Ptr) -> () {
    assert!(!value.is_null() && STATE.lock().unwrap().allocated != 0); hidcall(c"release");
    let value = value.cast::<Obj>();
    { let mut state = STATE.lock().unwrap();
      if value as usize == state.tap { assert!(!(*value).valid); state.tap = 0; }
      if value as usize == state.manager { assert!(!(*value).scheduled && (*value).input.is_none() && (*value).remove.is_none()); state.manager = 0; }
      state.allocated = state.allocated.wrapping_sub(1); }
    drop(Box::from_raw(value));
});
sdk!(flags => CGEventGetFlags(value: Ptr) -> u64 { hidcall(c"flags"); (*value.cast::<Obj>()).flags });
sdk!(set_flags => CGEventSetFlags(value: Ptr, flags: u64) -> () { (*value.cast::<Obj>()).flags = flags; hidcall(c"set_flags"); });
sdk!(field => CGEventGetIntegerValueField(value: Ptr, field: u32) -> i64 {
    hidcall(c"field"); let value = value.cast::<Obj>();
    if field == 42 { (*value).tag } else if field == 9 { (*value).key } else { assert_eq!(field, 8); (*value).repeat }
});
sdk!(set_field => CGEventSetIntegerValueField(value: Ptr, field: u32, data: i64) -> () { assert_eq!(field, 42); (*value.cast::<Obj>()).tag = data; hidcall(c"set_field"); });
sdk!(set_type => CGEventSetType(value: Ptr, kind: u32) -> () { (*value.cast::<Obj>()).event_type = kind; hidcall(c"set_type"); });
sdk!(event_source => CGEventSourceCreate(kind: i32) -> Ptr { assert_eq!(kind, -1); if hidcall(c"event_source") { std::ptr::null_mut() } else { object(6) } });
sdk!(keyboard_event => CGEventCreateKeyboardEvent(source: Ptr, key: u16, down: bool) -> Ptr {
    assert!((*source.cast::<Obj>()).kind == 6 && key == 57);
    if hidcall(c"keyboard_event") { return std::ptr::null_mut(); }
    let value = object(7).cast::<Obj>(); (*value).key = key.into(); (*value).down = down; value.cast()
});
/// # Safety
/// Event must be a live owned synthetic event from this provider.
#[allow(
    dead_code,
    reason = "The missing-post variant intentionally omits its export wrapper"
)]
pub(super) unsafe fn post(location: u32, value: Ptr) {
    let value = value.cast::<Obj>();
    assert!(location == 1 && (*value).event_type == 12 && (*value).tag == 0x52554859504552);
    {
        let mut state = STATE.lock().unwrap();
        state.posts = state.posts.wrapping_add(1);
        state.last_flags = (*value).flags;
        state.last_down = (*value).down;
    }
    hidcall(c"post");
}
sdk!(secure => IsSecureEventInputEnabled() -> u8 { hidcall(c"secure"); (STATE.lock().unwrap().envflags & 1 != 0) as u8 });
sdk!(accessibility => AXIsProcessTrusted() -> u8 { hidcall(c"accessibility"); (STATE.lock().unwrap().envflags & 2 == 0) as u8 });
sdk!(monitoring => CGPreflightListenEventAccess() -> bool { hidcall(c"monitoring"); STATE.lock().unwrap().envflags & 4 == 0 });
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_dictionary(device: bool) -> Ptr {
    if hidcall(if device {
        c"device_dictionary"
    } else {
        c"input_dictionary"
    }) {
        std::ptr::null_mut()
    } else {
        object(if device { 4 } else { 5 })
    }
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_common() -> Ptr {
    hidcall(c"common");
    41usize as Ptr
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_uptime() -> f64 {
    hidcall(c"uptime");
    STATE.lock().unwrap().now
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_raycast() -> bool {
    hidcall(c"raycast");
    STATE.lock().unwrap().envflags & 8 != 0
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_input(
    device: u32,
    page: u32,
    usage: u32,
    down: bool,
    time: f64,
) {
    assert!(device < 16);
    STATE.lock().unwrap().now = time;
    let input = Input {
        page,
        usage,
        device: DEVICES.as_ptr().add(device as usize).cast_mut().cast(),
        down,
    };
    let manager = manager();
    if !manager.is_null() && (*manager).scheduled {
        if let Some(callback) = (*manager).input {
            callback(
                (*manager).context as Ptr,
                0,
                manager.cast(),
                (&input as *const Input).cast_mut().cast(),
            );
        }
    }
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_remove(device: u32) {
    assert!(device < 16);
    let manager = manager();
    if !manager.is_null() && (*manager).scheduled {
        if let Some(callback) = (*manager).remove {
            callback(
                (*manager).context as Ptr,
                0,
                manager.cast(),
                DEVICES.as_ptr().add(device as usize).cast_mut().cast(),
            );
        }
    }
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_event(
    kind: u32,
    key: u16,
    flags: u64,
    tag: i64,
    repeated: bool,
) -> u64 {
    let mut value = Obj {
        key: key.into(),
        flags,
        tag,
        repeat: i64::from(repeated),
        ..Obj::default()
    };
    let pointer = (&mut value as *mut Obj).cast::<c_void>();
    let mut result = pointer;
    let tap = tap();
    if !tap.is_null() && (*tap).enabled && (*tap).valid {
        result = (*tap).tap.unwrap()(tap.cast(), kind, pointer, (*tap).context as Ptr);
    }
    STATE.lock().unwrap().last_event_flags = value.flags;
    if let Some(host) = &super::HID_HOST {
        if !result.is_null() && kind == 10 && !repeated {
            let mut modifiers = 0;
            for (event_bit, carbon_bit) in [(20, 8), (18, 12), (19, 11), (17, 9)] {
                if value.flags & (1 << event_bit) != 0 {
                    modifiers |= 1 << carbon_bit;
                }
            }
            (host.emit)(key.into(), modifiers);
        }
    }
    u64::from(result.is_null())
}
#[no_mangle]
extern "C" fn ruycast_hid_fixture_event_flags() -> u64 {
    STATE.lock().unwrap().last_event_flags
}
