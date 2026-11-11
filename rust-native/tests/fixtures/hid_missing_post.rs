//! Standalone negative variant: genuinely no CGEventPost fixture export.
const HOST: bool = false;
const HID_HOST: Option<hid_sdk::Host> = None;
mod caps_sdk;
mod hid_sdk;
#[no_mangle]
extern "C" fn ruycast_caps_fixture_set(connection: u32, selector: i32, value: bool) -> i32 {
    caps_sdk::set(connection, selector, value)
}
