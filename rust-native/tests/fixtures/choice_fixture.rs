//! Complete choice fixture, one ordered main-thread constructor and cleanup.
mod choice_board_sdk;
mod choice_monitor_sdk;
static CLEANED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
unsafe extern "C" fn install() {
    objc2::rc::autoreleasepool(|_| {
        choice_board_sdk::install();
        choice_monitor_sdk::install();
    });
    // Mach-O termination sections alone are not run on this inserted-library
    // process-exit path. Preserve the original destructor invariant at exit.
    assert_eq!(libc::atexit(cleanup), 0);
}
extern "C" fn cleanup() {
    if CLEANED.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    unsafe {
        objc2::rc::autoreleasepool(|_| {
            choice_monitor_sdk::cleanup();
            choice_board_sdk::cleanup();
        });
    }
}
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL: unsafe extern "C" fn() = install;
#[used]
#[link_section = "__DATA,__mod_term_func"]
static CLEANUP: unsafe extern "C" fn() = cleanup;
