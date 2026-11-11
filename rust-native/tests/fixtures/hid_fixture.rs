//! Complete standalone HID provider, sharing the exact Rust Caps state.
const HOST: bool = false;
mod caps_sdk;
mod hid_sdk;
#[no_mangle]
extern "C" fn ruycast_caps_fixture_set(connection: u32, selector: i32, value: bool) -> i32 {
    caps_sdk::set(connection, selector, value)
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_CGEventPost(location: u32, event: *mut std::ffi::c_void) {
    hid_sdk::post(location, event);
}
