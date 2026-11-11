//! Complete three-layer SDK, one explicit before-main installation order.
mod clipboard_history_sdk;
mod snippet_effects_sdk;
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
    objc2::rc::autoreleasepool(|_| {
        workspace_sdk::install();
        snippet_effects_sdk::install();
        clipboard_history_sdk::install();
    });
}
unsafe extern "C" fn cleanup() {
    clipboard_history_sdk::cleanup();
    snippet_effects_sdk::cleanup();
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL: unsafe extern "C" fn() = install;
#[used]
#[link_section = "__DATA,__mod_term_func"]
static CLEANUP: unsafe extern "C" fn() = cleanup;
