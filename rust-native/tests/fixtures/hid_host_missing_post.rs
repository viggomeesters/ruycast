//! Negative combined host SDK: genuinely no HID post symbol.
const HOST: bool = true;
const KEYBOARD_READY_GATE: Option<fn() -> bool> = None;
const HID_HOST: Option<hid_sdk::Host> = Some(hid_sdk::Host {
    ready: keyboard_sdk::ruycast_keyboard_fixture_ready,
    emit: keyboard_sdk::ruycast_keyboard_fixture_emit_binding,
});
mod caps_sdk;
mod hid_sdk;
mod keyboard_sdk;
#[no_mangle]
extern "C" fn ruycast_caps_fixture_set(connection: u32, selector: i32, value: bool) -> i32 {
    caps_sdk::set(connection, selector, value)
}
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
