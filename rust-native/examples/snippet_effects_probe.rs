//! Loaded component probe: actual prepared clipboard items and synthetic apps/events.
use objc2::{msg_send, rc::autoreleasepool, runtime::Bool, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSPasteboard,
    NSRunningApplication, NSWorkspace,
};
use ruycast_native::{snippet_clipboard, snippet_output::Payload};
use serde_json::{json, Value};
use std::ffi::{c_void, CStr};
unsafe fn symbol<T: Copy>(name: &CStr) -> Option<T> {
    let p = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr());
    (!p.is_null()).then(|| std::mem::transmute_copy(&p))
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const c_void);
}
unsafe fn run() -> Value {
    let ready = symbol::<unsafe extern "C" fn() -> u64>(c"ruycast_snippet_fixture_ready");
    if ready.is_none_or(|f| f() != 0x5255504153544531) {
        return json!({"ready":false});
    }
    let mt = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mt);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let trusted =
        symbol::<unsafe extern "C" fn() -> u8>(c"ruycast_snippet_fixture_trusted").unwrap()();
    if trusted == 0 {
        return json!({"ready":true,"trusted":false});
    }
    let initial = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .unwrap();
    let target = NSRunningApplication::runningApplicationWithProcessIdentifier(4260).unwrap();
    let activated = target.activateWithOptions(NSApplicationActivationOptions::empty());
    let payload = Payload {
        text: "A😀end".into(),
        html: Some("<div>A😀end</div>".into()),
        cursor: None,
    };
    // Preserve the actual staging boundary: preparation may fail before a
    // board is acquired or cleared.
    let prepared = snippet_clipboard::PreparedItem::new(mt, &payload);
    let board = prepared
        .as_ref()
        .map(|_| NSPasteboard::pasteboardWithUniqueName());
    let written = match (prepared, &board) {
        (Some(item), Some(board)) => matches!(
            item.publish(board, || true),
            snippet_clipboard::Publish::Attempted(true)
        ),
        _ => false,
    };
    let create = symbol::<unsafe extern "C" fn(*const c_void, u16, bool) -> *mut c_void>(
        c"ruycast_snippet_fixture_create",
    )
    .unwrap();
    let flags =
        symbol::<unsafe extern "C" fn(*mut c_void, u64)>(c"ruycast_snippet_fixture_flags").unwrap();
    let integer =
        symbol::<unsafe extern "C" fn(*mut c_void, u32, i64)>(c"ruycast_snippet_fixture_integer")
            .unwrap();
    let post =
        symbol::<unsafe extern "C" fn(u32, *mut c_void)>(c"ruycast_snippet_fixture_post").unwrap();
    let control = symbol::<unsafe extern "C" fn(u32)>(c"ruycast_snippet_fixture_control").unwrap();
    let mut created = 0;
    if written && activated {
        for down in [true, false] {
            let event = create(std::ptr::null(), 9, down);
            if event.is_null() {
                break;
            }
            flags(event, 1 << 20);
            integer(event, 42, 0x52554859504552);
            post(0, event);
            CFRelease(event);
            created += 1;
        }
    }
    let same: Bool = msg_send![&*target,isEqual:&*initial];
    control(3);
    let replacement = NSRunningApplication::runningApplicationWithProcessIdentifier(4260).unwrap();
    let equal_after_relaunch: Bool = msg_send![&*replacement,isEqual:&*target];
    control(2);
    let external = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .unwrap();
    // As in the original live-owner assertions, sample operation effects before
    // teardown releases the named board (which may itself clear its contents).
    let home =
        std::path::PathBuf::from(std::env::var_os("RUYCAST_WORKSPACE_FIXTURE_HOME").unwrap());
    let trace: Vec<Value> = std::fs::read_to_string(home.join("workspace-trace.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let result = json!({"ready":true,"trusted":true,"initialPID":initial.processIdentifier(),"activated":activated,"written":written,"created":created,"sameIdentity":same.as_bool(),"sameAfterRelaunch":equal_after_relaunch.as_bool(),"externalPID":external.processIdentifier(),"active":app.isActive(),"windows":app.windows().len(),"operationTrace":trace});
    if let Some(board) = board {
        let _: () = msg_send![&*board, releaseGlobally];
    }
    result
}
fn main() {
    let value = autoreleasepool(|_| unsafe { run() });
    println!("{value}");
}
