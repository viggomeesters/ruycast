//! Real Objective-C retain/equality/matching on the partial Rust fixture component.
#![cfg(feature = "test-tools")]
#[path = "fixtures/keyword_core.rs"]
mod keyword_core;
#[path = "fixtures/keyword_notification.rs"]
mod notification;
use objc2::{
    msg_send,
    rc::{autoreleasepool, Retained},
    runtime::{AnyObject, Bool},
    ClassType,
};
use objc2_app_kit::NSRunningApplication;
use std::sync::atomic::Ordering;
unsafe fn app(token: u64, pid: i32) -> Retained<AnyObject> {
    Retained::from_raw(
        notification::ruycast_keyword_fixture_notification_application(token, pid).cast(),
    )
    .unwrap()
}
#[test]
fn retained_notification_identity_pid_hash_and_current_process_match() {
    autoreleasepool(|_| unsafe {
        keyword_core::environment(0, 42, 1, 1);
        keyword_core::failure(c"".as_ptr());
        let target = keyword_core::front();
        let a = app(1, 42);
        let equal = app(1, 42);
        let reused = app(2, 42);
        let wrong_pid = app(1, 43);
        let pointer = |v: &Retained<AnyObject>| Retained::as_ptr(v).cast_mut().cast();
        let pid: i32 = msg_send![&*a, processIdentifier];
        assert_eq!(pid, 42);
        let terminated: Bool = msg_send![&*a, isTerminated];
        assert!(!terminated.as_bool());
        let typed: Bool = msg_send![&*a,isKindOfClass:NSRunningApplication::class()];
        assert!(typed.as_bool());
        let hash: usize = msg_send![&*a, hash];
        assert_eq!(hash, 1);
        for (other, expected) in [
            (&a, true),
            (&equal, true),
            (&reused, false),
            (&wrong_pid, false),
        ] {
            let answer: Bool = msg_send![&*a,isEqual:&**other];
            assert_eq!(answer.as_bool(), expected);
            assert_eq!(
                notification::ruycast_keyword_fixture_app_matches_notification(
                    target,
                    pointer(other)
                ),
                expected
            );
        }
        keyword_core::failure(c"app_matches_notification".as_ptr());
        assert!(
            !notification::ruycast_keyword_fixture_app_matches_notification(target, pointer(&a))
        );
        keyword_core::failure(c"".as_ptr());
        let retained = a.clone();
        drop(a);
        assert_eq!(notification::LIVE.load(Ordering::Relaxed), 4);
        drop(retained);
        drop(equal);
        drop(reused);
        drop(wrong_pid);
        assert_eq!(
            notification::LIVE.load(Ordering::Relaxed),
            0,
            "Last release reaches own object_dispose override"
        );
        keyword_core::app_release(target);

        // The original special token compares only with this process, never another app.
        let pid = libc::getpid();
        keyword_core::environment(0, pid, u64::MAX, 1);
        let target = keyword_core::front();
        let own = app(u64::MAX, pid);
        let current = NSRunningApplication::currentApplication();
        let same: Bool = msg_send![&*own,isEqual:&*current];
        assert!(same.as_bool());
        assert!(
            notification::ruycast_keyword_fixture_app_matches_notification(
                target,
                Retained::as_ptr(&current).cast_mut().cast()
            )
        );
        assert!(
            !notification::ruycast_keyword_fixture_app_matches_notification(
                target,
                std::ptr::null_mut()
            )
        );
        drop(own);
        keyword_core::app_release(target);
        assert_eq!(notification::LIVE.load(Ordering::Relaxed), 0);
        assert_eq!(keyword_core::STATE.lock().unwrap().allocated, 0);
    });
}
