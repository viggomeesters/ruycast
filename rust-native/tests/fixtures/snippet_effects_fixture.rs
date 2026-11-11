//! One constructor installs workspace before snippet effects in the same context.
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
    workspace_sdk::install();
    snippet_effects_sdk::install();
}
unsafe extern "C" fn cleanup() {
    snippet_effects_sdk::cleanup();
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL: unsafe extern "C" fn() = install;
#[used]
#[link_section = "__DATA,__mod_term_func"]
static CLEANUP: unsafe extern "C" fn() = cleanup;
