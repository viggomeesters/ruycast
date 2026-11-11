//! Focused partial-component checks; no complete SDK/ready or native OS fallback.
#![cfg(feature = "test-tools")]
#[path = "fixtures/keyword_core.rs"]
mod core;
use std::{ffi::c_void, sync::Mutex};
static TEST_LOCK: Mutex<()> = Mutex::new(());
#[test]
fn immutable_app_identity_and_notification_matching_release_owned_objects() {
    let _guard = TEST_LOCK.lock().unwrap();
    unsafe {
        core::failure(c"".as_ptr());
        core::environment(0, 42, 1, 1);
        let a = core::front();
        core::environment(0, 42, 2, 3);
        let b = core::front();
        assert_eq!(core::app_pid(a), 42);
        assert!(
            !core::app_equal(a, b),
            "PID reuse never equals immutable app identity"
        );
        assert!(core::match_notification_identity(a, 1, 42));
        assert!(!core::match_notification_identity(a, 2, 42));
        core::environment(0, 43, 1, 1);
        let c = core::front();
        assert!(!core::app_equal(a, c), "Identity also binds PID");
        assert!(core::app_equal(a, a));
        for value in [a, b, c] {
            core::app_release(value);
        }
        let state = core::STATE.lock().unwrap();
        assert_eq!((state.allocated, state.apps), (0, 0));
    }
}
#[test]
fn utf16_ranges_preserve_non_bmp_nul_and_invalid_units_without_foreign_decoding() {
    let _guard = TEST_LOCK.lock().unwrap();
    unsafe {
        core::failure(c"".as_ptr());
        let units = [0x41, 0xd83d, 0xde80, 0, 0xd800, 0x42];
        let value = core::string(&units).cast();
        assert_eq!(core::type_id(value), core::string_type());
        assert_eq!(core::string_len(value), 6);
        let mut output = [0u16; 4];
        core::string_units(
            value,
            core::Range {
                location: 1,
                length: 4,
            },
            output.as_mut_ptr(),
        );
        assert_eq!(output, [0xd83d, 0xde80, 0, 0xd800]);
        (*value.cast::<core::Object>()).identity = u64::MAX;
        core::failure(c"value_len_huge".as_ptr());
        assert_eq!(core::string_len(value), isize::MAX / 2);
        core::failure(c"string_len".as_ptr());
        assert_eq!(core::string_len(value), -1);
        core::failure(c"".as_ptr());
        core::cf_release(value);
        let empty = core::string(&[]).cast();
        assert_eq!(core::string_len(empty), 0);
        core::cf_release(empty);
        assert_eq!(core::STATE.lock().unwrap().allocated, 0);
    }
}
unsafe extern "C" fn nested(context: *mut c_void) {
    let count = &mut *context.cast::<u32>();
    *count += 1;
    core::failure(c"front".as_ptr());
    core::environment(0, 55, 9, 10);
    // This nested call must not see a retained SDK lock or repeat the taken hook.
    assert!(core::front().is_null());
}
#[test]
fn fault_and_change_controls_reread_state_after_one_shot_nested_callback() {
    let _guard = TEST_LOCK.lock().unwrap();
    unsafe {
        core::failure(c"".as_ptr());
        core::environment(0, 42, 1, 1);
        core::change_after(c"front".as_ptr(), 2, 0, 43, 2, 3);
        let a = core::front();
        assert_eq!(core::app_pid(a), 42);
        let b = core::front();
        assert_eq!(core::app_pid(b), 43);
        assert!(core::match_notification_identity(b, 2, 43));
        let mut count = 0u32;
        core::hook(
            c"front".as_ptr(),
            Some(nested),
            (&mut count as *mut u32).cast(),
        );
        assert!(
            core::front().is_null(),
            "Nested failure selector applies to outer call"
        );
        assert_eq!(count, 1);
        core::failure(c"".as_ptr());
        let c = core::front();
        assert!(core::match_notification_identity(c, 9, 55));
        assert_eq!(count, 1);
        for value in [a, b, c] {
            core::app_release(value);
        }
        let state = core::STATE.lock().unwrap();
        assert_eq!((state.allocated, state.apps), (0, 0));
    }
}
