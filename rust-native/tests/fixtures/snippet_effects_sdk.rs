//! Synthetic snippet effects above the single shared owned workspace context.
use super::workspace_sdk as workspace;
use objc2::{
    class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Imp, Sel},
    sel, ClassType,
};
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardTypeHTML, NSPasteboardTypeString,
    NSRunningApplication, NSWorkspace,
};
use objc2_foundation::{NSNumber, NSObject, NSString};
use serde_json::{json, Value};
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicPtr, AtomicU64, Ordering},
        LazyLock, Mutex, Once,
    },
    time::Duration,
};
type Set = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut NSString, *mut NSString) -> Bool;
type Write = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool;
type Clear = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> isize;
type Front = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject;
type Lookup = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, i32) -> *mut AnyObject;
static INSTALLED: AtomicU64 = AtomicU64::new(0);
static INSTALL: Once = Once::new();
static ORIGINAL_SET: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIGINAL_WRITE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIGINAL_CLEAR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
// A retained ObjC pointer, copied/replaced under the mutex; no Rust borrow is
// held across native calls. Product effects/control are main-thread owned.
static LAST_BOARD: Mutex<usize> = Mutex::new(0);
pub(super) fn flag(name: &str) -> bool {
    workspace::home().is_some_and(|p| p.join(name).exists())
}
fn hold(phase: &str) {
    if !flag(&format!("hold-{phase}")) {
        return;
    }
    let home = workspace::home().unwrap();
    let _ = std::fs::write(home.join(format!("{phase}-entered")), "entered");
    while !flag("snippet-release") {
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn record(phase: &str, mut fields: Value) {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) },
        0
    );
    fields["atNs"] = json!(now.tv_sec as u64 * 1_000_000_000 + now.tv_nsec as u64);
    workspace::record(phase, fields);
}
unsafe fn atomic_text(name: &str, value: &str) {
    let path = NSString::from_str(workspace::home().unwrap().join(name).to_str().unwrap());
    let value = NSString::from_str(value);
    let _: Bool = msg_send![&*value, writeToFile:&*path, atomically:Bool::YES, encoding:4usize, error:std::ptr::null_mut::<*mut AnyObject>()];
}
fn front_pid() -> i32 {
    workspace::home()
        .and_then(|p| std::fs::read_to_string(p.join("frontmost-pid")).ok())
        .map(|s| unsafe { msg_send![&*NSString::from_str(&s), intValue] })
        .unwrap_or(4260)
}
fn set_front_pid(pid: i32) {
    unsafe {
        atomic_text("frontmost-pid", &pid.to_string());
    }
}
static APP_CLASS: LazyLock<&'static AnyClass> = LazyLock::new(|| {
    let mut builder = ClassBuilder::new(c"SnippetFixtureApp", NSObject::class()).unwrap();
    builder.add_ivar::<i32>(c"fixturePID");
    builder.add_ivar::<usize>(c"fixtureGeneration");
    unsafe {
        builder.add_method(
            sel!(processIdentifier),
            app_pid as unsafe extern "C-unwind" fn(_, _) -> _,
        );
        builder.add_method(
            sel!(isTerminated),
            terminated as unsafe extern "C-unwind" fn(_, _) -> _,
        );
        builder.add_method(
            sel!(isEqual:),
            equal as unsafe extern "C-unwind" fn(_, _, _) -> _,
        );
        builder.add_method(
            sel!(bundleIdentifier),
            bundle as unsafe extern "C-unwind" fn(_, _) -> _,
        );
        builder.add_method(
            sel!(localizedName),
            name as unsafe extern "C-unwind" fn(_, _) -> _,
        );
        builder.add_method(
            sel!(activateWithOptions:),
            activate as unsafe extern "C-unwind" fn(_, _, _) -> _,
        );
    }
    builder.register()
});
unsafe fn identity(app: *mut AnyObject) -> (i32, usize) {
    (
        *APP_CLASS
            .instance_variable(c"fixturePID")
            .unwrap()
            .load_ptr::<i32>(&*app),
        *APP_CLASS
            .instance_variable(c"fixtureGeneration")
            .unwrap()
            .load_ptr::<usize>(&*app),
    )
}
unsafe extern "C-unwind" fn app_pid(app: *mut AnyObject, _: Sel) -> i32 {
    identity(app).0
}
unsafe extern "C-unwind" fn terminated(_: *mut AnyObject, _: Sel) -> Bool {
    Bool::new(flag("target-terminated"))
}
unsafe extern "C-unwind" fn equal(app: *mut AnyObject, _: Sel, other: *mut AnyObject) -> Bool {
    let same: Bool = msg_send![other, isKindOfClass:*APP_CLASS];
    Bool::new(same.as_bool() && identity(app) == identity(other))
}
unsafe extern "C-unwind" fn bundle(_: *mut AnyObject, _: Sel) -> *mut NSString {
    Retained::autorelease_return(NSString::from_str("dev.fixture.snippet"))
}
unsafe extern "C-unwind" fn name(_: *mut AnyObject, _: Sel) -> *mut NSString {
    Retained::autorelease_return(NSString::from_str("Snippet fixture target"))
}
unsafe extern "C-unwind" fn activate(app: *mut AnyObject, _: Sel, _: usize) -> Bool {
    workspace::require_hide();
    record("snippet-activate", json!({"pid":identity(app).0}));
    if flag("activation-noop") {
        return Bool::NO;
    }
    set_front_pid(identity(app).0);
    Bool::YES
}
unsafe fn app(pid: i32) -> *mut AnyObject {
    let value: Retained<AnyObject> = msg_send![*APP_CLASS, new];
    *APP_CLASS
        .instance_variable(c"fixturePID")
        .unwrap()
        .load_ptr::<i32>(&value) = pid;
    *APP_CLASS
        .instance_variable(c"fixtureGeneration")
        .unwrap()
        .load_ptr::<usize>(&value) = usize::from(flag("target-new-generation"));
    Retained::autorelease_return(value)
}
unsafe extern "C-unwind" fn front(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    app(front_pid())
}
unsafe extern "C-unwind" fn lookup(_: *mut AnyObject, _: Sel, pid: i32) -> *mut AnyObject {
    record("snippet-lookup", json!({"pid":pid}));
    if flag("missing-target") {
        return std::ptr::null_mut();
    }
    set_front_pid(libc::getpid());
    app(pid)
}
unsafe extern "C-unwind" fn item_set(
    value: *mut AnyObject,
    selector: Sel,
    text: *mut NSString,
    kind: *mut NSString,
) -> Bool {
    hold("prepare");
    if (flag("fail-string") && (*kind).isEqualToString(NSPasteboardTypeString))
        || (flag("fail-html") && (*kind).isEqualToString(NSPasteboardTypeHTML))
    {
        record("snippet-item-failed", json!({"type":(*kind).to_string()}));
        return Bool::NO;
    }
    std::mem::transmute::<*mut c_void, Set>(ORIGINAL_SET.load(Ordering::Acquire))(
        value, selector, text, kind,
    )
}
unsafe fn check_board(value: &AnyObject) -> &NSPasteboard {
    let board = &*(value as *const AnyObject).cast::<NSPasteboard>();
    if board.name().to_string() == "NSGeneralPboard" {
        libc::abort();
    }
    board
}
unsafe extern "C-unwind" fn clear_board(value: *mut AnyObject, selector: Sel) -> isize {
    check_board(&*value);
    record("snippet-clear", json!({}));
    std::mem::transmute::<*mut c_void, Clear>(ORIGINAL_CLEAR.load(Ordering::Acquire))(
        value, selector,
    )
}
unsafe extern "C-unwind" fn write_board(
    value: *mut AnyObject,
    selector: Sel,
    objects: *mut AnyObject,
) -> Bool {
    let board = check_board(&*value);
    hold("write");
    let retained = Retained::retain(value).unwrap();
    let old = std::mem::replace(
        &mut *LAST_BOARD.lock().unwrap(),
        Retained::into_raw(retained) as usize,
    );
    drop(Retained::<AnyObject>::from_raw(old as *mut AnyObject));
    if flag("fail-write") {
        record("snippet-write-failed", json!({}));
        return Bool::NO;
    }
    let ok = std::mem::transmute::<*mut c_void, Write>(ORIGINAL_WRITE.load(Ordering::Acquire))(
        value, selector, objects,
    );
    record(
        "snippet-write",
        json!({"ok":ok.as_bool(),"text":board.stringForType(NSPasteboardTypeString).map(|s|s.to_string()).unwrap_or_default(),"html":board.stringForType(NSPasteboardTypeHTML).map(|s|s.to_string()).unwrap_or_default(),"changeCount":board.changeCount(),"items":board.pasteboardItems().map(|v|v.len()).unwrap_or(0)}),
    );
    ok
}
fn require_ready() {
    if ready() == 0 {
        unsafe {
            libc::abort();
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_snippet_fixture_trusted() -> u8 {
    require_ready();
    record("snippet-trust", json!({}));
    u8::from(!flag("deny-ax"))
}
unsafe fn set_number(event: *mut AnyObject, key: &str, number: &NSNumber) {
    let _: () = msg_send![event, setObject:number, forKey:&*NSString::from_str(key)];
}
unsafe fn number(event: *mut AnyObject, key: &str) -> Retained<NSNumber> {
    msg_send![event, objectForKey:&*NSString::from_str(key)]
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_snippet_fixture_create(
    _: *const c_void,
    key: u16,
    down: bool,
) -> *mut c_void {
    require_ready();
    if flag("fail-event") {
        return std::ptr::null_mut();
    }
    let event: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];
    for (name, value) in [
        ("key", NSNumber::new_u16(key)),
        ("down", NSNumber::new_bool(down)),
        ("flags", NSNumber::new_u64(0)),
        ("tag", NSNumber::new_i64(0)),
    ] {
        set_number(Retained::as_ptr(&event).cast_mut(), name, &value);
    }
    Retained::into_raw(event).cast()
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_snippet_fixture_flags(event: *mut c_void, flags: u64) {
    set_number(event.cast(), "flags", &NSNumber::new_u64(flags));
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_snippet_fixture_integer(
    event: *mut c_void,
    field: u32,
    value: i64,
) {
    if field != 42 {
        libc::abort();
    }
    set_number(event.cast(), "tag", &NSNumber::new_i64(value));
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_snippet_fixture_post(tap: u32, event: *mut c_void) {
    require_ready();
    if tap != 0 {
        libc::abort();
    }
    let event = event.cast();
    record(
        "snippet-post",
        json!({"key":number(event,"key").as_u16(),"down":number(event,"down").as_bool(),"flags":number(event,"flags").as_u64(),"tag":number(event,"tag").as_i64(),"pid":front_pid()}),
    );
}
pub(super) fn ready() -> u64 {
    INSTALLED.load(Ordering::Acquire)
}
#[no_mangle]
pub extern "C" fn ruycast_snippet_fixture_ready() -> u64 {
    ready()
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_snippet_fixture_control(operation: u32) {
    require_ready();
    if libc::pthread_main_np() == 0 {
        libc::abort();
    }
    match operation {
        1 => {
            let value = {
                let last = LAST_BOARD.lock().unwrap();
                Retained::<AnyObject>::retain(*last as *mut AnyObject)
            };
            if let Some(value) = value {
                let board = check_board(&value);
                board.clearContents();
                board.setString_forType(
                    &NSString::from_str("External fixture clipboard"),
                    NSPasteboardTypeString,
                );
            }
            record("snippet-external-copy", json!({}));
        }
        2 => {
            set_front_pid(4261);
            record("snippet-external-focus", json!({}));
        }
        3 => {
            atomic_text("target-new-generation", "new");
            record("snippet-external-relaunch", json!({}));
        }
        _ => libc::abort(),
    }
}
pub(super) unsafe fn cleanup() {
    let old = std::mem::take(&mut *LAST_BOARD.lock().unwrap());
    drop(Retained::<AnyObject>::from_raw(old as *mut AnyObject));
}
pub(super) unsafe fn install() {
    if workspace::ready() == 0 || std::env::var_os("RUYCAST_NATIVE_TEST_SNIPPET").is_none() {
        return;
    }
    INSTALL.call_once(|| {
        // Initialize the real classes before replacing methods, without querying
        // any foreign app or accessing the general pasteboard.
        let _ = NSRunningApplication::currentApplication();
        let _ = NSPasteboardItem::new();
        let mut good = workspace::hook(
            NSWorkspace::class(),
            sel!(frontmostApplication),
            std::mem::transmute::<Front, Imp>(front),
        );
        good = workspace::hook(
            NSRunningApplication::class().metaclass(),
            sel!(runningApplicationWithProcessIdentifier:),
            std::mem::transmute::<Lookup, Imp>(lookup),
        ) && good;
        let set = NSPasteboardItem::class().instance_method(sel!(setString:forType:));
        let write = NSPasteboard::class().instance_method(sel!(writeObjects:));
        let clear = NSPasteboard::class().instance_method(sel!(clearContents));
        if let (true, Some(set), Some(write), Some(clear)) = (good, set, write, clear) {
            ORIGINAL_SET.store(
                set.set_implementation(std::mem::transmute::<Set, Imp>(item_set)) as *mut c_void,
                Ordering::Release,
            );
            ORIGINAL_WRITE.store(
                write.set_implementation(std::mem::transmute::<Write, Imp>(write_board))
                    as *mut c_void,
                Ordering::Release,
            );
            ORIGINAL_CLEAR.store(
                clear.set_implementation(std::mem::transmute::<Clear, Imp>(clear_board))
                    as *mut c_void,
                Ordering::Release,
            );
            INSTALLED.store(0x5255504153544531, Ordering::Release);
            record("snippet-installed", json!({}));
        }
    });
}
