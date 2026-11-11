//! Complete standalone synthetic SDK assembled solely from the Rust components.
mod keyword_ax;
mod keyword_board;
mod keyword_core;
mod keyword_events;
mod keyword_notification;
#[no_mangle]
extern "C" fn ruycast_keyword_fixture_ready() -> u64 {
    0x52554b5744303031
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyword_fixture_AXValueGetTypeID() -> usize {
    keyword_ax::value_type()
}
