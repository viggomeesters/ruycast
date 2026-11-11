//! Actual framework text object plus private named boards; no real General read.
use objc2::{
    msg_send,
    rc::{autoreleasepool, Retained},
    MainThreadMarker,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSPasteboard, NSPasteboardTypeString, NSTextView,
};
use objc2_foundation::NSString;
use serde_json::{json, Value};
unsafe fn run() -> Value {
    let ready = libc::dlsym(
        libc::RTLD_DEFAULT,
        c"ruycast_workspace_fixture_ready".as_ptr(),
    );
    if ready.is_null()
        || std::mem::transmute::<*mut std::ffi::c_void, unsafe extern "C" fn() -> u64>(ready)()
            != 0x52555943
    {
        return json!({"ready":false});
    }
    let getter = libc::dlsym(
        libc::RTLD_DEFAULT,
        c"ruycast_clipboard_history_fixture_board".as_ptr(),
    );
    assert!(!getter.is_null());
    let get: unsafe extern "C" fn() -> *mut NSPasteboard = std::mem::transmute(getter);
    let mt = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mt);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    // NSTextView can ask AppKit for General internally. The before-main hook
    // must already return our private framework board, replacing the deny hook.
    let _editor = NSTextView::new(mt);
    let general = NSPasteboard::generalPasteboard();
    let history = Retained::retain(get()).expect("Owned history board");
    let general_again = NSPasteboard::generalPasteboard();
    let stable_general = Retained::as_ptr(&general) == Retained::as_ptr(&general_again);
    let stable_history = Retained::as_ptr(&history).cast_mut() == get();
    let distinct = Retained::as_ptr(&history) != Retained::as_ptr(&general);
    assert_ne!(general.name().to_string(), "NSGeneralPboard");
    assert_ne!(history.name().to_string(), "NSGeneralPboard");
    general.clearContents();
    history.clearContents();
    assert!(general.setString_forType(&NSString::from_str("Own framework"), NSPasteboardTypeString));
    assert!(history.setString_forType(&NSString::from_str("Own history Å"), NSPasteboardTypeString));
    let result = json!({"ready":true,"frameworkName":general.name().to_string(),"historyName":history.name().to_string(),"stableFramework":stable_general,"stableHistory":stable_history,"distinct":distinct,"frameworkText":general.stringForType(NSPasteboardTypeString).unwrap().to_string(),"historyText":history.stringForType(NSPasteboardTypeString).unwrap().to_string(),"active":app.isActive(),"windows":app.windows().len()});
    for board in [&general, &history] {
        board.clearContents();
        let _: () = msg_send![&**board, releaseGlobally];
    }
    result
}
fn main() {
    let result = autoreleasepool(|_| unsafe { run() });
    println!("{result}");
}
