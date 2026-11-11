//! Complete scalar-only Caps SDK provider.
const HOST: bool = false;
mod caps_sdk;
#[no_mangle]
extern "C" fn ruycast_caps_fixture_set(connection: u32, selector: i32, value: bool) -> i32 {
    caps_sdk::set(connection, selector, value)
}
