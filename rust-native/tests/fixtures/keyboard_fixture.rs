//! Complete owned Carbon fixture.
const KEYBOARD_READY_GATE: Option<fn() -> bool> = None;
mod keyboard_sdk;
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_fixture_parameter(
    event: *mut std::ffi::c_void,
    name: u32,
    kind: u32,
    actual: *mut u32,
    capacity: usize,
    size: *mut usize,
    output: *mut std::ffi::c_void,
) -> i32 {
    keyboard_sdk::parameter(event, name, kind, actual, capacity, size, output)
}
