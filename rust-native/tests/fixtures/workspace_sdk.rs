//! Test-only NSWorkspace swizzles. Never call original open/app-resolution IMPs.
#![allow(
    dead_code,
    reason = "Shared workspace basis is reused by subsequent effects/history/keyboard artifacts"
)]
use block2::{Block, RcBlock};
use objc2::{
    msg_send,
    rc::{autoreleasepool, Retained},
    runtime::{AnyClass, AnyObject, Bool, Imp, Sel},
    sel, ClassType,
};
use objc2_app_kit::{NSPasteboard, NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSString, NSURL};
use serde_json::{json, Value};
use std::{
    ffi::c_void,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicPtr, AtomicU64, Ordering},
        Mutex, Once, OnceLock,
    },
    time::Duration,
};
type CompletionBlock = Block<dyn Fn(*mut NSRunningApplication, *mut NSError)>;
type Read = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut AnyObject;
type General = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject;
type Resolve = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut NSString) -> *mut NSURL;
type OpenDefault = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut NSURL) -> Bool;
type OpenApp = unsafe extern "C-unwind" fn(
    *mut AnyObject,
    Sel,
    *mut NSURL,
    *mut AnyObject,
    *mut CompletionBlock,
);
type OpenWith = unsafe extern "C-unwind" fn(
    *mut AnyObject,
    Sel,
    *mut NSArray<NSURL>,
    *mut NSURL,
    *mut AnyObject,
    *mut CompletionBlock,
);
struct Callback(RcBlock<dyn Fn(*mut NSRunningApplication, *mut NSError)>);
// NSWorkspace completion handlers are explicitly callable on worker queues.
// This wrapper only moves retained blocks; invocation remains serial per copy,
// exactly as the SDK/old fixture. Captured owner state must obey that SDK contract.
unsafe impl Send for Callback {}
static HOME: OnceLock<PathBuf> = OnceLock::new();
static INSTALLED: AtomicU64 = AtomicU64::new(0);
static INSTALL: Once = Once::new();
static HELD: Mutex<Vec<Callback>> = Mutex::new(Vec::new());
static TRACE: Mutex<()> = Mutex::new(());
static LAST_HIDE: Mutex<Option<String>> = Mutex::new(None);
static ORIGINAL_READ: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
extern "C" {
    fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
    fn dispatch_time(when: u64, delta: i64) -> u64;
    fn dispatch_after_f(
        when: u64,
        queue: *mut c_void,
        context: *mut c_void,
        callback: unsafe extern "C" fn(*mut c_void),
    );
}
pub(super) fn home() -> Option<&'static Path> {
    HOME.get().map(PathBuf::as_path)
}
pub(super) fn ready() -> u64 {
    INSTALLED.load(Ordering::Acquire)
}
pub(super) fn record(phase: &str, mut fields: Value) {
    let Some(home) = home() else {
        return;
    };
    let object = fields.as_object_mut().expect("Own trace fields");
    object.insert("phase".into(), json!(phase));
    object.insert(
        "mainThread".into(),
        json!(unsafe { libc::pthread_main_np() != 0 }),
    );
    let mut data = serde_json::to_vec(&fields).unwrap();
    data.push(b'\n');
    let _guard = TRACE.lock().unwrap();
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("workspace-trace.jsonl"))
    {
        let _ = file.write_all(&data);
    }
}
pub(super) fn require_hide() {
    if std::env::var_os("RUYCAST_NATIVE_TEST_REQUIRE_HIDE").is_none() {
        return;
    }
    let text = home()
        .and_then(|home| std::fs::read_to_string(home.join("p/native-hide.jsonl")).ok())
        .unwrap_or_default();
    let line = text
        .trim_matches(|c| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'))
        .rsplit('\n')
        .next()
        .unwrap_or("");
    let ack: Value = serde_json::from_str(line).unwrap_or(Value::Null);
    let trace = ack["trace"].as_str();
    let mut last = LAST_HIDE.lock().unwrap();
    let reused = last.as_deref().is_some_and(|old| Some(old) == trace);
    if ack["mainThread"] != true || ack["afterVisible"].as_bool().unwrap_or(false) || reused {
        drop(last);
        record("missing-exact-hide", json!({}));
        unsafe { libc::abort() }
    }
    *last = trace.map(str::to_owned);
}
fn entered(name: &str, text: &str) {
    if let Some(home) = home() {
        let _ = std::fs::write(home.join(name), text);
    }
}
fn wait_release() {
    while !home().unwrap().join("workspace-release").exists() {
        std::thread::sleep(Duration::from_millis(2));
    }
}
unsafe fn string(value: *mut NSString) -> String {
    if value.is_null() {
        String::new()
    } else {
        (*value).to_string()
    }
}
unsafe fn mode(url: *mut NSURL) -> String {
    if url.is_null() {
        return "success".into();
    }
    let value = if (*url).isFileURL() {
        (*url).lastPathComponent()
    } else {
        (*url).host()
    };
    let Some(value) = value else {
        return "success".into();
    };
    if value.to_string().starts_with("Fake-") {
        value
            .substringFromIndex(5)
            .stringByDeletingPathExtension()
            .to_string()
    } else {
        value.to_string()
    }
}
unsafe fn url_text(url: *mut NSURL) -> String {
    if url.is_null() {
        String::new()
    } else {
        (*url)
            .absoluteString()
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}
unsafe fn error(mode: &str) -> Retained<NSError> {
    let message = if mode == "large-error" {
        NSString::from_str("Fixture Ω😀 ").stringByPaddingToLength_withString_startingAtIndex(
            100000,
            &NSString::from_str("Ω😀"),
            0,
        )
    } else {
        NSString::from_str("Fixture native error.")
    };
    let key = NSString::from_str("NSLocalizedDescription");
    let fields = NSDictionary::<NSString, AnyObject>::from_slices(&[&*key], &[&*message]);
    NSError::errorWithDomain_code_userInfo(
        &NSString::from_str("dev.ruycast.fixture"),
        7,
        Some(&fields),
    )
}
unsafe fn complete(callback: &Callback, mode: &str) {
    record("callback", json!({"mode":mode}));
    if mode == "error" || mode == "large-error" {
        let error = error(mode);
        callback
            .0
            .call((std::ptr::null_mut(), Retained::as_ptr(&error).cast_mut()));
    } else if mode == "nil-result" {
        callback
            .0
            .call((std::ptr::null_mut(), std::ptr::null_mut()));
    } else {
        let app = NSRunningApplication::currentApplication();
        callback
            .0
            .call((Retained::as_ptr(&app).cast_mut(), std::ptr::null_mut()));
    }
}
struct Pending {
    callback: Callback,
    mode: String,
}
unsafe extern "C" fn delayed(context: *mut c_void) {
    let pending = Box::from_raw(context.cast::<Pending>());
    autoreleasepool(|_| complete(&pending.callback, &pending.mode));
}
unsafe fn opening(url: *mut NSURL, callback: *mut CompletionBlock, selector: &str) {
    require_hide();
    let mode = mode(url);
    record(selector, json!({"url":url_text(url),"mode":mode}));
    entered("open-started", "entered");
    let Some(callback) = RcBlock::copy(callback).map(Callback) else {
        return;
    };
    if mode == "hold" {
        HELD.lock().unwrap().push(callback);
        return;
    }
    if mode == "duplicate" {
        complete(&callback, "success");
        complete(&callback, "error");
        return;
    }
    if mode == "inline" {
        complete(&callback, "success");
        return;
    }
    let delay = if mode == "slow" { 200000000 } else { 1000000 };
    let pending = Box::new(Pending { callback, mode });
    dispatch_after_f(
        dispatch_time(0, delay),
        dispatch_get_global_queue(0x15, 0),
        Box::into_raw(pending).cast(),
        delayed,
    );
}
unsafe extern "C-unwind" fn resolve(
    _: *mut AnyObject,
    _: Sel,
    identifier: *mut NSString,
) -> *mut NSURL {
    let identifier = string(identifier);
    record("resolve", json!({"bundleID":identifier}));
    if !identifier.starts_with("dev.fixture.") || identifier == "dev.fixture.missing" {
        return std::ptr::null_mut();
    }
    let mode = &identifier["dev.fixture.".len()..];
    if mode.starts_with("hold-resolve") {
        entered("resolve-entered", "entered");
        wait_release();
    }
    if mode == "slow-resolve" {
        std::thread::sleep(Duration::from_millis(150));
    }
    if mode == "invalid-location" {
        return NSURL::URLWithString(&NSString::from_str("https://fixture.invalid/App.app"))
            .map(Retained::autorelease_return)
            .unwrap_or_default();
    }
    let path = home().unwrap().join(format!("Fake-{mode}.app"));
    Retained::autorelease_return(NSURL::fileURLWithPath(&NSString::from_str(
        path.to_str().unwrap(),
    )))
}
unsafe extern "C-unwind" fn open_default(_: *mut AnyObject, _: Sel, url: *mut NSURL) -> Bool {
    require_hide();
    record("open-default", json!({"url":url_text(url)}));
    entered("open-started", "entered");
    let mode = mode(url);
    if mode == "slow-sync" {
        std::thread::sleep(Duration::from_millis(150));
    }
    if mode == "hold-sync" {
        wait_release();
    }
    Bool::new(mode != "reject")
}
unsafe extern "C-unwind" fn open_app(
    _: *mut AnyObject,
    _: Sel,
    url: *mut NSURL,
    _: *mut AnyObject,
    callback: *mut CompletionBlock,
) {
    opening(url, callback, "open-app");
}
unsafe extern "C-unwind" fn open_with(
    _: *mut AnyObject,
    _: Sel,
    urls: *mut NSArray<NSURL>,
    application: *mut NSURL,
    _: *mut AnyObject,
    callback: *mut CompletionBlock,
) {
    let urls = if urls.is_null() {
        vec![]
    } else {
        (*urls)
            .iter()
            .map(|url| {
                url.absoluteString()
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            })
            .collect()
    };
    record("open-with-targets", json!({"urls":urls}));
    opening(application, callback, "open-with");
}
pub(super) unsafe fn release_callbacks() {
    autoreleasepool(|_| {
        let callbacks = std::mem::take(&mut *HELD.lock().unwrap());
        for callback in callbacks {
            complete(&callback, "success");
        }
    });
}
pub(super) unsafe fn hook(class: &AnyClass, selector: Sel, implementation: Imp) -> bool {
    if let Some(method) = class.instance_method(selector) {
        method.set_implementation(implementation);
        true
    } else {
        false
    }
}
unsafe extern "C-unwind" fn deny_general(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    record("unexpected-general-pasteboard", json!({}));
    libc::abort()
}
unsafe extern "C-unwind" fn clipboard_read(
    value: *mut AnyObject,
    selector: Sel,
    kind: *mut AnyObject,
) -> *mut AnyObject {
    let name: *mut NSString = msg_send![value, name];
    if string(name) == "NSGeneralPboard" {
        record("unexpected-general-read", json!({}));
        libc::abort();
    }
    record("clipboard-read", json!({}));
    if std::env::var("RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT").as_deref() == Ok("hold-main") {
        entered("clipboard-entered", "entered");
        wait_release();
    }
    let original = ORIGINAL_READ.load(Ordering::Acquire);
    assert!(!original.is_null());
    let read: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut AnyObject =
        std::mem::transmute(original);
    read(value, selector, kind)
}
pub(super) unsafe fn install() {
    INSTALL.call_once(|| {
        autoreleasepool(|_| {
            let Ok(home) = std::env::var("RUYCAST_WORKSPACE_FIXTURE_HOME") else {
                return;
            };
            let home = HOME.get_or_init(|| PathBuf::from(home));
            if std::fs::read(home.join(".ruycast-test-fixture"))
                .ok()
                .as_deref()
                != Some(b"own-disposable-fixture-v1".as_slice())
            {
                return;
            }
            let class = NSWorkspace::class();
            let _: *const AnyClass = msg_send![class, class];
            if let Some(guard) = std::env::var_os("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD") {
                let board = NSPasteboard::class();
                let _: *const AnyClass = msg_send![board, class];
                let read = board
                    .instance_method(sel!(stringForType:))
                    .expect("Own clipboard method");
                ORIGINAL_READ.store(
                    read.implementation() as *const () as *mut c_void,
                    Ordering::Release,
                );
                hook(
                    board,
                    sel!(stringForType:),
                    std::mem::transmute::<Read, Imp>(clipboard_read),
                );
                if guard != "trace" {
                    hook(
                        board.metaclass(),
                        sel!(generalPasteboard),
                        std::mem::transmute::<General, Imp>(deny_general),
                    );
                }
            }
            let mut good = hook(
                class,
                sel!(URLForApplicationWithBundleIdentifier:),
                std::mem::transmute::<Resolve, Imp>(resolve),
            );
            good = hook(
                class,
                sel!(openURL:),
                std::mem::transmute::<OpenDefault, Imp>(open_default),
            ) && good;
            good = hook(
                class,
                sel!(openApplicationAtURL:configuration:completionHandler:),
                std::mem::transmute::<OpenApp, Imp>(open_app),
            ) && good;
            good = hook(
                class,
                sel!(openURLs:withApplicationAtURL:configuration:completionHandler:),
                std::mem::transmute::<OpenWith, Imp>(open_with),
            ) && good;
            if good {
                INSTALLED.store(0x52555943, Ordering::Release);
                record("installed", json!({"selectors":4}));
            }
        })
    });
}
