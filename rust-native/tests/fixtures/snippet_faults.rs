//! Per-process named-board SDK faults; the general board accessor aborts.
use objc2::{
    msg_send,
    rc::autoreleasepool,
    runtime::{AnyClass, AnyObject, Bool, Imp, Sel},
    sel,
};
use std::{
    ffi::c_void,
    sync::atomic::{AtomicPtr, Ordering},
};

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSPasteboardTypeString: *const AnyObject;
    static NSPasteboardTypeHTML: *const AnyObject;
    static NSPasteboardNameGeneral: *const AnyObject;
}
type Setter =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> Bool;
type Writer = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool;
type General = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject;
static FAULT: AtomicPtr<libc::c_char> = AtomicPtr::new(std::ptr::null_mut());
static ORIGINAL_SET: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIGINAL_WRITE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

unsafe fn fault(name: *const libc::c_char) -> bool {
    let value = FAULT.load(Ordering::Acquire);
    !value.is_null() && libc::strcmp(value, name) == 0
}
unsafe fn equal(left: *const AnyObject, right: *const AnyObject) -> bool {
    if left.is_null() {
        return false;
    }
    let answer: Bool = msg_send![left, isEqualToString: right];
    answer.as_bool()
}
unsafe extern "C-unwind" fn forbidden_general(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
    libc::abort()
}
unsafe extern "C-unwind" fn set_string(
    this: *mut AnyObject,
    selector: Sel,
    text: *mut AnyObject,
    kind: *mut AnyObject,
) -> Bool {
    if (fault(c"string".as_ptr()) && equal(kind, NSPasteboardTypeString))
        || (fault(c"html".as_ptr()) && equal(kind, NSPasteboardTypeHTML))
    {
        return Bool::NO;
    }
    let pointer = ORIGINAL_SET.load(Ordering::Acquire);
    if pointer.is_null() {
        libc::abort();
    }
    let original = std::mem::transmute::<*mut c_void, Setter>(pointer);
    original(this, selector, text, kind)
}
unsafe extern "C-unwind" fn write_objects(
    this: *mut AnyObject,
    selector: Sel,
    objects: *mut AnyObject,
) -> Bool {
    let name: *const AnyObject = msg_send![this, name];
    if equal(name, NSPasteboardNameGeneral) {
        libc::abort();
    }
    if fault(c"write".as_ptr()) {
        return Bool::NO;
    }
    let pointer = ORIGINAL_WRITE.load(Ordering::Acquire);
    if pointer.is_null() {
        libc::abort();
    }
    let original = std::mem::transmute::<*mut c_void, Writer>(pointer);
    original(this, selector, objects)
}
unsafe extern "C" fn install() {
    autoreleasepool(|_| {
        let guard = libc::getenv(c"RUYCAST_SNIPPET_FIXTURE".as_ptr());
        let fault = libc::getenv(c"RUYCAST_SNIPPET_FAULT".as_ptr());
        if guard.is_null()
            || libc::strcmp(guard, c"own-named-board".as_ptr()) != 0
            || fault.is_null()
        {
            libc::abort();
        }
        let Some(board) = AnyClass::get(c"NSPasteboard") else {
            libc::abort();
        };
        let Some(item) = AnyClass::get(c"NSPasteboardItem") else {
            libc::abort();
        };
        // Preserve the original class messages and their lazy +initialize calls.
        let _: *const AnyClass = msg_send![board, class];
        let _: *const AnyClass = msg_send![item, class];
        let Some(general) = board.class_method(sel!(generalPasteboard)) else {
            libc::abort();
        };
        let Some(set) = item.instance_method(sel!(setString:forType:)) else {
            libc::abort();
        };
        let Some(write) = board.instance_method(sel!(writeObjects:)) else {
            libc::abort();
        };
        FAULT.store(fault, Ordering::Release);
        // Publish the original IMPs before callbacks can become reachable.
        ORIGINAL_SET.store(set.implementation() as *mut c_void, Ordering::Release);
        ORIGINAL_WRITE.store(write.implementation() as *mut c_void, Ordering::Release);
        general.set_implementation(std::mem::transmute::<General, Imp>(forbidden_general));
        set.set_implementation(std::mem::transmute::<Setter, Imp>(set_string));
        write.set_implementation(std::mem::transmute::<Writer, Imp>(write_objects));
    });
}

// The existing private-board Rust probe checks the actual installed IMP before
// publishing only synthetic data to a newly created named board.
#[no_mangle]
pub extern "C" fn ruycast_owned_board_general_guard() -> bool {
    AnyClass::get(c"NSPasteboard")
        .and_then(|class| class.class_method(sel!(generalPasteboard)))
        .is_some_and(|method| {
            method.implementation() as *const c_void == forbidden_general as *const c_void
        })
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL: unsafe extern "C" fn() = install;
