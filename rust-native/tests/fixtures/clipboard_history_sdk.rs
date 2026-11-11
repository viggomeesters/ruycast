//! Before-main general-board object substitution, shared with workspace/effects.
use super::workspace_sdk as workspace;
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel, ClassType,
};
use objc2_app_kit::NSPasteboard;
use objc2_foundation::{NSString, NSUUID};
use std::sync::atomic::{AtomicPtr, Ordering};
static FRAMEWORK_BOARD: AtomicPtr<NSPasteboard> = AtomicPtr::new(std::ptr::null_mut());
static HISTORY_BOARD: AtomicPtr<NSPasteboard> = AtomicPtr::new(std::ptr::null_mut());
// Each slot owns one +1 reference until the root's process-exit finalizer.
// Never hold a Rust lock/borrow across AppKit, which may reenter callbacks.
unsafe fn board(
    slot: &AtomicPtr<NSPasteboard>,
    create: impl FnOnce() -> Retained<NSPasteboard>,
) -> *mut NSPasteboard {
    let current = slot.load(Ordering::Acquire);
    if !current.is_null() {
        return current;
    }
    let candidate = Retained::into_raw(create());
    match slot.compare_exchange(
        std::ptr::null_mut(),
        candidate,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => candidate,
        Err(current) => {
            drop(Retained::from_raw(candidate));
            current
        }
    }
}
type General = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut NSPasteboard;
unsafe extern "C-unwind" fn framework_board(_: *mut AnyObject, _: Sel) -> *mut NSPasteboard {
    board(&FRAMEWORK_BOARD, NSPasteboard::pasteboardWithUniqueName)
}
#[no_mangle]
pub unsafe extern "C" fn ruycast_clipboard_history_fixture_board() -> *mut NSPasteboard {
    // Preserve the original home-presence check. Full workspace authority is
    // checked by the actual consumer; even refusal paths never expose General.
    if workspace::home().is_none() {
        return std::ptr::null_mut();
    }
    board(&HISTORY_BOARD, || {
        NSPasteboard::pasteboardWithName(&NSString::from_str(&format!(
            "ruycast-clipboard-history-fixture-{}",
            NSUUID::UUID().UUIDString()
        )))
    })
}
pub(super) unsafe fn install() {
    if workspace::home().is_none() {
        return;
    }
    let board = NSPasteboard::class();
    let _: *const AnyClass = msg_send![board, class];
    assert!(
        workspace::hook(
            board.metaclass(),
            sel!(generalPasteboard),
            std::mem::transmute::<General, Imp>(framework_board)
        ),
        "Owned general-board selector required"
    );
}
pub(super) unsafe fn cleanup() {
    // Invoked only when all owned consumers have stopped at process exit.
    for slot in [&HISTORY_BOARD, &FRAMEWORK_BOARD] {
        drop(Retained::from_raw(
            slot.swap(std::ptr::null_mut(), Ordering::AcqRel),
        ));
    }
}
