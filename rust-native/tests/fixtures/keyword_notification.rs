//! Synthetic NSRunningApplication subclass. No foreign application discovery/activation.
use objc2::{
    msg_send,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
    sel, ClassType,
};
use objc2_app_kit::NSRunningApplication;
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicUsize, Ordering},
        LazyLock,
    },
};
pub(super) static LIVE: AtomicUsize = AtomicUsize::new(0);
static CLASS: LazyLock<&'static AnyClass> = LazyLock::new(|| {
    let mut builder = ClassBuilder::new(
        c"RuycastKeywordFixtureApplication",
        NSRunningApplication::class(),
    )
    .expect("Unique owned fixture class");
    builder.add_ivar::<u64>(c"token");
    builder.add_ivar::<i32>(c"fixturePID");
    unsafe {
        builder.add_method(
            sel!(processIdentifier),
            pid as unsafe extern "C-unwind" fn(_, _) -> _,
        );
        builder.add_method(
            sel!(isTerminated),
            terminated as unsafe extern "C-unwind" fn(_, _) -> _,
        );
        builder.add_method(
            sel!(isEqual:),
            equal as unsafe extern "C-unwind" fn(_, _, _) -> _,
        );
        builder.add_method(sel!(hash), hash as unsafe extern "C-unwind" fn(_, _) -> _);
        builder.add_method(sel!(dealloc), dispose as unsafe extern "C-unwind" fn(_, _));
    }
    builder.register()
});
unsafe fn identity(value: *mut AnyObject) -> (u64, i32) {
    (
        *CLASS
            .instance_variable(c"token")
            .unwrap()
            .load_ptr::<u64>(&*value),
        *CLASS
            .instance_variable(c"fixturePID")
            .unwrap()
            .load_ptr::<i32>(&*value),
    )
}
unsafe extern "C-unwind" fn pid(value: *mut AnyObject, _: Sel) -> i32 {
    identity(value).1
}
unsafe extern "C-unwind" fn terminated(_: *mut AnyObject, _: Sel) -> Bool {
    Bool::NO
}
unsafe extern "C-unwind" fn hash(value: *mut AnyObject, _: Sel) -> usize {
    identity(value).0 as usize
}
unsafe extern "C-unwind" fn equal(value: *mut AnyObject, _: Sel, other: *mut AnyObject) -> Bool {
    if value == other {
        return Bool::YES;
    }
    let same_class: Bool = msg_send![other,isKindOfClass:*CLASS];
    if same_class.as_bool() {
        return Bool::new(identity(value) == identity(other));
    }
    if identity(value).0 != u64::MAX {
        return Bool::NO;
    }
    let current = NSRunningApplication::currentApplication();
    msg_send![&*current,isEqual:other]
}
#[allow(
    deprecated,
    reason = "Exact MRC fixture bypasses the private superclass initializer/destructor and owns only scalar ivars"
)]
unsafe extern "C-unwind" fn dispose(value: *mut AnyObject, _: Sel) {
    LIVE.fetch_sub(1, Ordering::Relaxed);
    objc2::ffi::object_dispose(value);
}
/// # Safety
/// The returned retained object must be released using Objective-C ownership.
#[no_mangle]
pub(super) unsafe extern "C" fn ruycast_keyword_fixture_notification_application(
    token: u64,
    pid: i32,
) -> *mut c_void {
    // Bypass NSRunningApplication's private initializer, as the original MRC fixture did.
    let value = objc2::ffi::class_createInstance(*CLASS, 0);
    assert!(!value.is_null());
    CLASS
        .instance_variable(c"token")
        .unwrap()
        .load_ptr::<u64>(&*value)
        .write(token);
    CLASS
        .instance_variable(c"fixturePID")
        .unwrap()
        .load_ptr::<i32>(&*value)
        .write(pid);
    LIVE.fetch_add(1, Ordering::Relaxed);
    value.cast()
}
/// # Safety
/// Target must be a live synthetic app handle; value a live Objective-C object or nil.
#[no_mangle]
pub(super) unsafe extern "C" fn ruycast_keyword_fixture_app_matches_notification(
    target: *mut c_void,
    value: *mut c_void,
) -> bool {
    let value = value.cast::<AnyObject>();
    let own: Bool = msg_send![value,isKindOfClass:*CLASS];
    if own.as_bool() {
        let (token, pid) = identity(value);
        return super::keyword_core::match_notification_identity(target, token, pid);
    }
    let current = NSRunningApplication::currentApplication();
    let same: Bool = msg_send![&*current,isEqual:value];
    same.as_bool()
        && super::keyword_core::match_notification_identity(target, u64::MAX, libc::getpid())
}
