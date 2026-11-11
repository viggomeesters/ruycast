//! One complete host image: each SDK state exists exactly once.
const HOST: bool = true;
const KEYBOARD_READY_GATE: Option<fn() -> bool> = Some(workspace_ready);
const HID_HOST: Option<hid_sdk::Host> = Some(hid_sdk::Host {
    ready: keyboard_sdk::ruycast_keyboard_fixture_ready,
    emit: keyboard_sdk::ruycast_keyboard_fixture_emit_binding,
});
mod caps_sdk;
mod hid_sdk;
mod keyboard_sdk;
mod keyword_ax;
mod keyword_board;
mod keyword_core;
mod keyword_events;
mod keyword_notification;
mod workspace_sdk;
fn workspace_ready() -> bool {
    workspace_sdk::ready() == 0x52555943
}
#[no_mangle]
extern "C" fn ruycast_workspace_fixture_ready() -> u64 {
    workspace_sdk::ready()
}
#[no_mangle]
unsafe extern "C" fn ruycast_workspace_fixture_release() {
    workspace_sdk::release_callbacks();
}
#[no_mangle]
extern "C" fn ruycast_caps_fixture_set(connection: u32, selector: i32, value: bool) -> i32 {
    caps_sdk::set(connection, selector, value)
}
#[no_mangle]
unsafe extern "C" fn ruycast_hid_fixture_CGEventPost(location: u32, event: *mut std::ffi::c_void) {
    hid_sdk::post(location, event);
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
#[no_mangle]
unsafe extern "C" fn ruycast_keyword_fixture_ready() -> u64 {
    if workspace_ready() && keyboard_sdk::ruycast_keyboard_fixture_ready() == 0x52554b4559533031 {
        0x52554b5744303031
    } else {
        0
    }
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyword_fixture_AXValueGetTypeID() -> usize {
    keyword_ax::value_type()
}
#[no_mangle]
unsafe extern "C" fn ruycast_keyboard_host_fixture_install() {
    workspace_sdk::install();
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL: unsafe extern "C" fn() = ruycast_keyboard_host_fixture_install;
