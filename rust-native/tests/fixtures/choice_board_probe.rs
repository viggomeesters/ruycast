//! Actual framework lookups, gated before any board creation in the missing case.
use objc2::{msg_send, rc::autoreleasepool, ClassType};
use objc2_app_kit::{NSPasteboard, NSPasteboardNameGeneral, NSPasteboardTypeString};
use objc2_foundation::NSString;
use serde_json::json;
fn main() {
    assert!(unsafe { libc::pthread_main_np() != 0 });
    let guard =
        unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_choice_fixture_guard".as_ptr()) };
    if guard.is_null() {
        println!("{}", json!({"ready":false,"boardCreated":false}));
        return;
    }
    let guard: unsafe extern "C" fn() -> bool = unsafe { std::mem::transmute(guard) };
    assert!(unsafe { guard() });
    for symbol in [
        c"ruycast_choice_fixture_monitor_guard",
        c"ruycast_choice_fixture_click",
        c"ruycast_choice_fixture_schedule_click",
        c"ruycast_choice_fixture_monitor_counts",
    ] {
        assert!(
            unsafe { libc::dlsym(libc::RTLD_DEFAULT, symbol.as_ptr()) }.is_null(),
            "Incomplete board basis must not expose monitor readiness"
        );
    }
    autoreleasepool(|_| unsafe {
        let first = NSPasteboard::generalPasteboard();
        let name = first.name();
        assert!(!name.isEqualToString(NSPasteboardNameGeneral));
        assert!(first.pasteboardItems().is_none_or(|items| items.is_empty()));
        assert!(first.stringForType(NSPasteboardTypeString).is_none());
        assert!(first.setString_forType(
            &NSString::from_str("OWN CHOICE BOARD"),
            NSPasteboardTypeString
        ));
        for _ in 0..3 {
            let second = NSPasteboard::generalPasteboard();
            let framework_lookup: *const NSPasteboard =
                msg_send![NSPasteboard::class(), generalPasteboard];
            assert!(std::ptr::eq(&*first, &*second));
            assert!(std::ptr::eq(&*first, framework_lookup));
            assert_eq!(
                second
                    .stringForType(NSPasteboardTypeString)
                    .unwrap()
                    .to_string(),
                "OWN CHOICE BOARD"
            );
        }
        drop(first);
        let cleanup = libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_choice_board_cleanup".as_ptr());
        assert!(!cleanup.is_null());
        let cleanup: unsafe extern "C" fn() = std::mem::transmute(cleanup);
        cleanup();
        assert!(!guard());
        cleanup();
        assert!(!guard());
        println!(
            "{}",
            json!({"readyBeforeCleanup":true,"samePrivateObject":true,"initiallyEmpty":true,"privateContent":true,"readyAfterCleanup":false,"monitorAPIsAbsent":true,"applicationCreated":false,"desktopInputSent":false})
        );
    });
}
