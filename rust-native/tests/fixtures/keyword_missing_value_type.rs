//! Negative complete SDK variant genuinely omits AXValueGetTypeID.
mod keyword_ax;
mod keyword_board;
mod keyword_core;
mod keyword_events;
mod keyword_notification;
#[no_mangle]
extern "C" fn ruycast_keyword_fixture_ready() -> u64 {
    0x52554b5744303031
}
