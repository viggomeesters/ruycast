//! Complete owned System SDK. No original discovery, AX message, event or consent fallback.
mod system_apps;
mod system_effects;
mod system_events;
mod system_nodes;
use objc2::{
    msg_send,
    rc::{autoreleasepool, Retained},
    runtime::{AnyClass, AnyObject, Bool, Imp, Sel},
    sel, ClassType,
};
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSAppleEventDescriptor as D, NSAppleEventSendOptions, NSObject, NSString};
use std::{
    ffi::{c_void, CStr},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};
use system_nodes::{Ptr, State};
struct Context {
    state: Arc<State>,
    apps: system_apps::Apps,
}
// Only immutable synthetic NSObject instances escape this context. Their shared
// Rust state is Arc+Mutex; activation itself checks main. No UI objects are held.
unsafe impl Send for Context {}
unsafe impl Sync for Context {}
// Match the original process-lifetime globals, with one bounded allocation.
static CONTEXT: OnceLock<Context> = OnceLock::new();
static READY: AtomicU64 = AtomicU64::new(0);
fn context() -> &'static Context {
    CONTEXT.get().unwrap_or_else(|| unsafe { libc::abort() })
}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> Ptr;
    fn AXUIElementGetPid(node: Ptr, out: *mut i32) -> i32;
    fn AXUIElementSetMessagingTimeout(node: Ptr, seconds: f32) -> i32;
    fn AXUIElementCopyAttributeValue(node: Ptr, key: Ptr, out: *mut Ptr) -> i32;
    fn AXUIElementGetAttributeValueCount(node: Ptr, key: Ptr, out: *mut isize) -> i32;
    fn AXUIElementCopyAttributeValues(
        node: Ptr,
        key: Ptr,
        start: isize,
        max: isize,
        out: *mut Ptr,
    ) -> i32;
    fn AXUIElementSetAttributeValue(node: Ptr, key: Ptr, value: Ptr) -> i32;
    fn AXUIElementPerformAction(node: Ptr, key: Ptr) -> i32;
}
#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn AEDeterminePermissionToAutomateTarget(
        target: *const c_void,
        class: u32,
        id: u32,
        ask: u8,
    ) -> i32;
}
unsafe extern "C" fn trust() -> u8 {
    context().state.trust()
}
unsafe extern "C" fn app(pid: i32) -> Ptr {
    context().state.application(pid)
}
unsafe extern "C" fn pid(node: Ptr, out: *mut i32) -> i32 {
    context().state.pid(node, out)
}
unsafe extern "C" fn timeout(node: Ptr, seconds: f32) -> i32 {
    context().state.timeout(node, seconds)
}
unsafe extern "C" fn attr(node: Ptr, key: Ptr, out: *mut Ptr) -> i32 {
    context().state.attr(node, key, out)
}
unsafe extern "C" fn count(node: Ptr, key: Ptr, out: *mut isize) -> i32 {
    context().state.count(node, key, out)
}
unsafe extern "C" fn values(node: Ptr, key: Ptr, start: isize, max: isize, out: *mut Ptr) -> i32 {
    context().state.values(node, key, start, max, out)
}
unsafe extern "C" fn set(node: Ptr, key: Ptr, value: Ptr) -> i32 {
    system_effects::set(&context().state, node, key, value)
}
unsafe extern "C" fn action(node: Ptr, key: Ptr) -> i32 {
    system_effects::action(&context().state, node, key)
}
unsafe extern "C" fn consent(target: *const c_void, class: u32, id: u32, ask: u8) -> i32 {
    system_events::consent(&context().state, target, class, id, ask)
}
#[repr(C)]
struct Interpose {
    replacement: *const c_void,
    original: *const c_void,
}
// Immutable loader metadata, never dereferenced by Rust.
unsafe impl Sync for Interpose {}
macro_rules! interpose {
    ($name:ident,$replacement:ident,$original:ident) => {
        #[used]
        #[link_section = "__DATA,__interpose"]
        static $name: Interpose = Interpose {
            replacement: $replacement as *const c_void,
            original: $original as *const c_void,
        };
    };
}
interpose!(TRUST, trust, AXIsProcessTrusted);
interpose!(APP, app, AXUIElementCreateApplication);
interpose!(PID, pid, AXUIElementGetPid);
interpose!(TIMEOUT, timeout, AXUIElementSetMessagingTimeout);
interpose!(ATTR, attr, AXUIElementCopyAttributeValue);
interpose!(COUNT, count, AXUIElementGetAttributeValueCount);
interpose!(VALUES, values, AXUIElementCopyAttributeValues);
interpose!(SET, set, AXUIElementSetAttributeValue);
interpose!(ACTION, action, AXUIElementPerformAction);
interpose!(CONSENT, consent, AEDeterminePermissionToAutomateTarget);
type Getter = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject;
type Bundle = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut NSString) -> *mut AnyObject;
type Process = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, i32) -> *mut AnyObject;
type Event = unsafe extern "C-unwind" fn(
    *mut D,
    Sel,
    NSAppleEventSendOptions,
    f64,
    *mut *mut AnyObject,
) -> *mut D;
unsafe extern "C-unwind" fn shared(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    Retained::autorelease_return(context().apps.shared()).cast()
}
unsafe extern "C-unwind" fn running(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    Retained::autorelease_return(context().apps.running()).cast()
}
unsafe extern "C-unwind" fn front(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    Retained::autorelease_return(context().apps.front()).cast()
}
unsafe extern "C-unwind" fn bundle(
    _: *mut AnyObject,
    _: Sel,
    value: *mut NSString,
) -> *mut AnyObject {
    assert!(!value.is_null());
    Retained::autorelease_return(context().apps.by_bundle(&*value)).cast()
}
unsafe extern "C-unwind" fn process(_: *mut AnyObject, _: Sel, value: i32) -> *mut AnyObject {
    Retained::autorelease_return(context().apps.by_pid(value)).cast()
}
unsafe extern "C-unwind" fn event(
    value: *mut D,
    _: Sel,
    flags: NSAppleEventSendOptions,
    seconds: f64,
    _error: *mut *mut AnyObject,
) -> *mut D {
    assert!(!value.is_null());
    Retained::autorelease_return(system_events::event(
        &context().state,
        &*value,
        flags,
        seconds,
    ))
}
extern "C" {
    fn class_addMethod(
        class: *const AnyClass,
        selector: Sel,
        implementation: Imp,
        types: *const libc::c_char,
    ) -> Bool;
}
unsafe fn install_instance(class: &AnyClass, selector: Sel, implementation: Imp) {
    let method = class
        .instance_method(selector)
        .expect("Required owned SDK instance method");
    method.set_implementation(implementation);
    assert_eq!(method.implementation() as usize, implementation as usize);
}
unsafe fn install_class(class: &AnyClass, selector: Sel, implementation: Imp) {
    let method = class
        .class_method(selector)
        .expect("Required owned SDK class method");
    method.set_implementation(implementation);
    assert_eq!(method.implementation() as usize, implementation as usize);
}
unsafe fn object_getter(selector: Sel, implementation: Imp) {
    let class = NSObject::class();
    if class.instance_method(selector).is_none() {
        assert!(class_addMethod(class, selector, implementation, c"@@:".as_ptr()).as_bool());
    }
    install_instance(class, selector, implementation);
}
#[no_mangle]
extern "C" fn ruycast_system_fixture_ready() -> u64 {
    READY.load(Ordering::Acquire)
}
#[no_mangle]
unsafe extern "C" fn ruycast_system_fixture_mode(value: *const libc::c_char) {
    assert!(!value.is_null());
    context()
        .state
        .set_mode(&CStr::from_ptr(value).to_string_lossy());
}
#[no_mangle]
extern "C" fn ruycast_system_fixture_effects() -> i32 {
    context().state.data.lock().unwrap().effects
}
#[no_mangle]
extern "C" fn ruycast_system_fixture_reads() -> i32 {
    context().state.data.lock().unwrap().reads
}
#[no_mangle]
extern "C" fn ruycast_system_fixture_consents() -> i32 {
    context().state.data.lock().unwrap().consents
}
#[no_mangle]
unsafe extern "C" fn ruycast_system_fixture_proc(
    pid: i32,
    flavor: i32,
    arg: u64,
    out: Ptr,
    size: i32,
) -> i32 {
    system_apps::proc(&context().state, pid, flavor, arg, out, size)
}
unsafe extern "C" fn install() {
    let Some(home) = std::env::var_os("RUYCAST_SYSTEM_SDK_FIXTURE_HOME") else {
        return;
    };
    let home = std::path::PathBuf::from(home);
    assert!(
        home.to_str().is_some(),
        "Original fixture home requires UTF-8"
    );
    assert_eq!(
        std::fs::read(home.join(".ruycast-test-fixture")).unwrap(),
        b"own-disposable-fixture-v1"
    );
    autoreleasepool(|_| {
        let state = Arc::new(State::new());
        let apps = system_apps::Apps::new(state.clone());
        assert!(CONTEXT.set(Context { state, apps }).is_ok());
        // Complete lazy class initialization before replacing any methods.
        for class in [
            NSObject::class(),
            NSWorkspace::class(),
            NSRunningApplication::class(),
            D::class(),
        ] {
            let _: *const AnyClass = msg_send![class, class];
        }
        install_class(
            NSWorkspace::class(),
            sel!(sharedWorkspace),
            std::mem::transmute::<Getter, Imp>(shared),
        );
        object_getter(
            sel!(runningApplications),
            std::mem::transmute::<Getter, Imp>(running),
        );
        object_getter(
            sel!(frontmostApplication),
            std::mem::transmute::<Getter, Imp>(front),
        );
        install_instance(
            NSWorkspace::class(),
            sel!(runningApplications),
            std::mem::transmute::<Getter, Imp>(running),
        );
        install_instance(
            NSWorkspace::class(),
            sel!(frontmostApplication),
            std::mem::transmute::<Getter, Imp>(front),
        );
        install_class(
            NSRunningApplication::class(),
            sel!(runningApplicationsWithBundleIdentifier:),
            std::mem::transmute::<Bundle, Imp>(bundle),
        );
        install_class(
            NSRunningApplication::class(),
            sel!(runningApplicationWithProcessIdentifier:),
            std::mem::transmute::<Process, Imp>(process),
        );
        install_instance(
            D::class(),
            sel!(sendEventWithOptions:timeout:error:),
            std::mem::transmute::<Event, Imp>(event),
        );
        READY.store(0x525553595354454d, Ordering::Release);
    });
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL: unsafe extern "C" fn() = install;
