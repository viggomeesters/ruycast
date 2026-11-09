//! Six fixed, zero-payload Instruments intervals. No initialization on import.
//! ABI evidence: .go/evidence/full-rust-native/parallel/integration/
//! resume-startup-sdk/performance-signposts/ABI.md. Not yet runtime-qualified.
use crate::performance_trace::Stage;
use std::{ffi::{c_char, c_void}, ptr::NonNull, sync::OnceLock};

#[repr(C)]
struct OsLog { _opaque: [u8; 0] }
#[derive(Clone, Copy)]
struct LogHandle(NonNull<OsLog>);
// SAFETY: os/log.h describes the process-wide immortal handle collection;
// signpost.h supports process-wide matching across normal threads. We only
// pass the opaque handle back to those C APIs, never dereference or release it.
unsafe impl Send for LogHandle {}
unsafe impl Sync for LogHandle {}
static LOG: OnceLock<Option<LogHandle>> = OnceLock::new();

// The SDK's OS_LOG_STRING requires the actual literal bytes in this section,
// not a Rust slice/fat pointer. No caller can provide a format or stage name.
macro_rules! log_literal {
    ($name:ident, $bytes:literal) => {
        #[used]
        #[unsafe(link_section = "__TEXT,__oslogstring,cstring_literals")]
        static $name: [u8; $bytes.len()] = *$bytes;
    };
}
log_literal!(EMPTY_FORMAT, b"\0");
log_literal!(SEARCH_MODEL, b"Search model\0");
log_literal!(INPUT_MODEL, b"Input model\0");
log_literal!(INPUT_RECONCILE, b"Input to field reconciliation\0");
log_literal!(OPEN_READY, b"Palette ready\0");
log_literal!(HOST_DRAW, b"Host draw callback\0");
log_literal!(HOTKEY_HANDLER, b"Hotkey handler\0");

fn name(stage: Stage) -> &'static [u8] {
    match stage {
        Stage::SearchModel => &SEARCH_MODEL,
        Stage::InputModel => &INPUT_MODEL,
        Stage::InputReconcile => &INPUT_RECONCILE,
        Stage::OpenReady => &OPEN_READY,
        Stage::HostDraw => &HOST_DRAW,
        Stage::HotkeyHandler => &HOTKEY_HANDLER,
    }
}

// os_log's no-argument encoding is exactly summary=0, argument-count=0.
// The SDK macro aligns this buffer to 16; its padding is never transmitted.
#[repr(C, align(16))]
struct EmptyFormat([u8; 2]);
#[repr(u8)]
#[derive(Clone, Copy)]
enum Boundary { Begin = 1, End = 2 }
const fn valid_id(id: u64) -> bool { id != 0 && id != u64::MAX }

/// Initialize once before the Trace's first measured timestamp. No retry or
/// per-span allocation; the fixed log handle lives for the process lifetime.
/// Call only from ordinary application code, never a signal/post-fork hook.
pub(crate) fn prepare() -> bool {
    LOG.get_or_init(|| {
        // SAFETY: two fixed NUL-terminated literals, API available since10.12.
        NonNull::new(unsafe { os_log_create(c"nl.viggomeesters.ruycast".as_ptr(), c"Interaction".as_ptr()) }).map(LogHandle)
    }).is_some()
}

/// A unique interval; dropping or explicitly ending it uses the same OS ID,
/// fixed stage name and log handle exactly once. No user payload is retained.
#[must_use]
pub(crate) struct Interval { log: LogHandle, id: u64, stage: Stage }
impl Drop for Interval {
    fn drop(&mut self) { let _ = emit(self.log, self.id, self.stage, Boundary::End); }
}

pub(crate) fn begin(stage: Stage) -> Option<Interval> {
    // Initialization is deliberately explicit and outside the timing interval.
    let log = LOG.get().copied().flatten()?;
    // SAFETY: only the process-lifetime handle returned by os_log_create.
    let id = unsafe { os_signpost_id_generate(log.0.as_ptr()) };
    if !emit(log, id, stage, Boundary::Begin) { return None; }
    Some(Interval { log, id, stage })
}
pub(crate) fn end(interval: Interval) { drop(interval); }

#[inline(never)]
fn emit(log: LogHandle, id: u64, stage: Stage, boundary: Boundary) -> bool {
    // Match the SDK macro's exact ID and enablement checks for begin AND end.
    if !valid_id(id) || !unsafe { os_signpost_enabled(log.0.as_ptr()) } { return false; }
    let mut buffer = EmptyFormat([0, 0]);
    // SAFETY: SDK exact fixed-arity ABI. DSO is the linker's current image
    // header, literals remain in __oslogstring, and only the two initialized
    // encoding bytes are passed. API availability is10.14; app minimum is14.
    unsafe {
        _os_signpost_emit_with_name_impl(
            (&raw const __dso_handle).cast_mut().cast(), log.0.as_ptr(),
            boundary as u8, id, name(stage).as_ptr().cast(),
            EMPTY_FORMAT.as_ptr().cast(), buffer.0.as_mut_ptr(), 2,
        );
    }
    // The SDK marks this call OS_NOT_TAIL_CALLED. Keep real work after it;
    // optimized-artifact BL/CALL/return-address verification is still required.
    std::hint::black_box(buffer.0);
    true
}

#[link(name = "System")]
unsafe extern "C" {
    static __dso_handle: u8;
    fn os_log_create(subsystem: *const c_char, category: *const c_char) -> *mut OsLog;
    fn os_signpost_id_generate(log: *mut OsLog) -> u64;
    fn os_signpost_enabled(log: *mut OsLog) -> bool;
    fn _os_signpost_emit_with_name_impl(dso: *mut c_void, log: *mut OsLog, boundary: u8, id: u64, name: *const c_char, format: *const c_char, buffer: *mut u8, size: u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_argument_buffer_and_reserved_ids_match_sdk_contract() {
        assert_eq!(std::mem::align_of::<EmptyFormat>(), 16);
        assert_eq!(EmptyFormat([0,0]).0, [0,0]);
        assert_eq!(EmptyFormat([0,0]).0.len(), 2);
        assert!(!valid_id(0)); assert!(!valid_id(u64::MAX)); assert!(valid_id(1));
    }
    #[test]
    fn stages_have_only_the_six_original_fixed_names() {
        for (stage, expected) in [
            (Stage::SearchModel, b"Search model\0".as_slice()),
            (Stage::InputModel, b"Input model\0".as_slice()),
            (Stage::InputReconcile, b"Input to field reconciliation\0".as_slice()),
            (Stage::OpenReady, b"Palette ready\0".as_slice()),
            (Stage::HostDraw, b"Host draw callback\0".as_slice()),
            (Stage::HotkeyHandler, b"Hotkey handler\0".as_slice()),
        ] { assert_eq!(name(stage), expected); }
    }
}
