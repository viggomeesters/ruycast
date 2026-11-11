//! Shared private-board substitution only. Monitor APIs are a separate component.
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel, ClassType,
};
use objc2_app_kit::NSPasteboard;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

type General = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut NSPasteboard;
static INSTALLED: AtomicBool = AtomicBool::new(false);
static BOARD: AtomicPtr<NSPasteboard> = AtomicPtr::new(std::ptr::null_mut());

unsafe extern "C-unwind" fn own_general(_: *mut AnyObject, _: Sel) -> *mut NSPasteboard {
    let board = BOARD.load(Ordering::Acquire);
    // Never invoke the original implementation, including during partial setup.
    if !INSTALLED.load(Ordering::Acquire) || board.is_null() {
        libc::abort();
    }
    board
}
#[no_mangle]
pub extern "C" fn ruycast_choice_fixture_guard() -> bool {
    INSTALLED.load(Ordering::Acquire) && !BOARD.load(Ordering::Acquire).is_null()
}
pub(super) unsafe fn install() {
    assert!(libc::pthread_main_np() != 0);
    assert!(!INSTALLED.load(Ordering::Acquire) && BOARD.load(Ordering::Acquire).is_null());
    let class = NSPasteboard::class();
    let _: *const AnyClass = msg_send![class, class];
    let general = class
        .class_method(sel!(generalPasteboard))
        .expect("Original general-board selector");
    general.set_implementation(std::mem::transmute::<General, Imp>(own_general));
    let board = NSPasteboard::pasteboardWithUniqueName();
    board.clearContents();
    BOARD.store(Retained::into_raw(board), Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
}
pub(super) unsafe fn cleanup() {
    assert!(libc::pthread_main_np() != 0);
    INSTALLED.store(false, Ordering::Release);
    if let Some(board) = Retained::from_raw(BOARD.swap(std::ptr::null_mut(), Ordering::AcqRel)) {
        board.clearContents();
        let _: () = msg_send![&*board, releaseGlobally];
    }
}
