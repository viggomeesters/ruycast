//! Deliberately incomplete choice fixture: private board only, no monitor APIs.
mod choice_board_sdk;
unsafe extern "C" fn install() {
    objc2::rc::autoreleasepool(|_| choice_board_sdk::install());
}
#[no_mangle]
/// # Safety
/// The standalone probe calls on main after all board operations have finished.
pub unsafe extern "C" fn ruycast_choice_board_cleanup() {
    objc2::rc::autoreleasepool(|_| choice_board_sdk::cleanup());
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL: unsafe extern "C" fn() = install;
#[used]
#[link_section = "__DATA,__mod_term_func"]
static CLEANUP: unsafe extern "C" fn() = ruycast_choice_board_cleanup;
