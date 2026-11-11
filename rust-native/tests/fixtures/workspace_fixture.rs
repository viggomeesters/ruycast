//! Standalone workspace fixture; the shared module has no implicit constructor.
mod workspace_sdk;
#[no_mangle]
extern "C" fn ruycast_workspace_fixture_ready() -> u64 {
    workspace_sdk::ready()
}
#[no_mangle]
unsafe extern "C" fn ruycast_workspace_fixture_release() {
    workspace_sdk::release_callbacks();
}
unsafe extern "C" fn install() {
    workspace_sdk::install();
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL: unsafe extern "C" fn() = install;
