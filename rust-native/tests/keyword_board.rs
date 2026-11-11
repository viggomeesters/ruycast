//! Actual partial Rust board component. No native/general pasteboard involved.
#![cfg(feature = "test-tools")]
#[path = "fixtures/keyword_board.rs"]
mod board;
#[path = "fixtures/keyword_core.rs"]
mod keyword_core;
use std::{ffi::c_void, sync::Mutex};
static TEST_LOCK: Mutex<()> = Mutex::new(());
unsafe fn item(text: &[u8], html: Option<&[u8]>) -> *mut c_void {
    let bytes = html.unwrap_or(&[]);
    board::item_create(
        text.as_ptr(),
        text.len(),
        bytes.as_ptr(),
        bytes.len(),
        html.is_some(),
    )
}
#[test]
fn byte_html_unicode_and_large_owned_values_survive_item_release() {
    let _guard = TEST_LOCK.lock().unwrap();
    unsafe {
        keyword_core::failure(c"".as_ptr());
        board::board_clear();
        let text = "🦀é\0".repeat(20_000);
        let html = b"<b>owned</b>";
        let value = item(text.as_bytes(), Some(html));
        assert!(!value.is_null());
        assert!(board::board_write(value));
        assert!(board::board_matches(value));
        assert!(board::board_ours(value));
        board::item_release(value);
        let clip = board::clipboard();
        let len = keyword_core::string_len(clip);
        assert_eq!(len as usize, text.encode_utf16().count());
        let mut units = vec![0u16; len as usize];
        keyword_core::string_units(
            clip,
            keyword_core::Range {
                location: 0,
                length: len,
            },
            units.as_mut_ptr(),
        );
        assert_eq!(units, text.encode_utf16().collect::<Vec<_>>());
        keyword_core::cf_release(clip);
        {
            let state = board::STATE.lock().unwrap();
            assert_eq!(state.text, text.as_bytes());
            assert_eq!(state.html, html);
        }
        assert_eq!(keyword_core::STATE.lock().unwrap().allocated, 0);
    }
}
#[test]
fn partial_write_and_external_extra_types_preserve_ownership_boundary() {
    let _guard = TEST_LOCK.lock().unwrap();
    unsafe {
        keyword_core::failure(c"".as_ptr());
        board::board_clear();
        let value = item(b"owned", Some(b"<b>owned</b>"));
        keyword_core::failure(c"board_partial".as_ptr());
        assert!(!board::board_write(value));
        keyword_core::failure(c"".as_ptr());
        assert!(!board::board_matches(value));
        assert!(board::board_ours(value));
        assert_eq!(
            board::STATE.lock().unwrap().html.len(),
            0,
            "Partial write reports zero HTML bytes, as the original provider does"
        );
        board::board_external(b"owned".as_ptr(), 5, std::ptr::null(), 0, false, true);
        assert!(!board::board_ours(value));
        board::board_external(b"changed".as_ptr(), 7, std::ptr::null(), 0, false, false);
        assert!(!board::board_ours(value));
        board::item_release(value);
        keyword_core::failure(c"item_html".as_ptr());
        assert!(item(b"owned", Some(b"html")).is_null());
        keyword_core::failure(c"".as_ptr());
        assert_eq!(keyword_core::STATE.lock().unwrap().allocated, 0);
    }
}
unsafe extern "C" fn external(_: *mut c_void) {
    board::board_external(b"external".as_ptr(), 8, std::ptr::null(), 0, false, false);
}
#[test]
fn fault_and_reentry_keep_snapshot_before_callback_without_locked_state() {
    let _guard = TEST_LOCK.lock().unwrap();
    unsafe {
        keyword_core::failure(c"".as_ptr());
        board::board_clear();
        let value = item(b"owned", None);
        assert!(board::board_write(value));
        let before = board::board_change();
        keyword_core::hook(
            c"board_change".as_ptr(),
            Some(external),
            std::ptr::null_mut(),
        );
        assert_eq!(board::board_change(), before);
        assert_eq!(board::board_change(), before + 1);
        assert!(board::board_write(value));
        keyword_core::hook(
            c"board_matches".as_ptr(),
            Some(external),
            std::ptr::null_mut(),
        );
        assert!(
            board::board_matches(value),
            "Return snapshot evaluated before reentry"
        );
        assert!(!board::board_matches(value));
        keyword_core::failure(c"board_clear".as_ptr());
        let count = board::board_change();
        assert_eq!(board::board_clear(), count);
        keyword_core::failure(c"board_write".as_ptr());
        assert!(!board::board_write(value));
        assert_eq!(board::board_change(), count);
        keyword_core::failure(c"".as_ptr());
        board::item_release(value);
        assert_eq!(keyword_core::STATE.lock().unwrap().allocated, 0);
    }
}
