//! Fail closed before native SDK calls. Only complete owned substitutes run.
use ruycast_native::{
    system_search::{self as os, Ticket},
    window_layout::Manager,
    window_placement,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
unsafe fn symbol<T: Copy>(name: &std::ffi::CStr) -> Result<T, String> {
    let raw = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if raw.is_null() {
        return Err("Complete owned SDK substitute is required.".into());
    }
    Ok(unsafe { std::mem::transmute_copy(&raw) })
}
fn run() -> Result<serde_json::Value, String> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").ok_or("Owned home required")?);
    if !ruycast_native::workspace::fixture_installed(&home) {
        return Err("Complete owned workspace substitute is required.".into());
    }
    let _main = objc2::MainThreadMarker::new().ok_or("Main SDK probe required")?;
    let ticket = || Ticket {
        deadline: Instant::now() + Duration::from_secs(3),
        live: Arc::new(|| true),
    };
    if std::env::args().nth(1).as_deref() == Some("workspace") {
        ruycast_native::system_actions::perform(
            ruycast_native::system_actions::Effect::ActivityMonitor,
            &ticket(),
        )?;
        ruycast_native::system_actions::perform(
            ruycast_native::system_actions::Effect::Configuration(home.join("owned-config.json")),
            &ticket(),
        )?;
        return Ok(
            serde_json::json!({"ok":true,"directNativeWorkspace":true,"realDesktopEffects":false}),
        );
    }
    let ready: unsafe extern "C" fn() -> u64 = unsafe { symbol(c"ruycast_system_fixture_ready")? };
    if unsafe { ready() } != 0x525553595354454d {
        return Err("Complete owned AX substitute is required.".into());
    }
    let mode: unsafe extern "C" fn(*const libc::c_char) =
        unsafe { symbol(c"ruycast_system_fixture_mode")? };
    let effects: unsafe extern "C" fn() -> i32 =
        unsafe { symbol(c"ruycast_system_fixture_effects")? };
    let platform = os::native::Native::new();
    let identity = os::native::identity(730001).ok_or("Owned identity missing")?;
    let screens = [
        rustcast_core::Frame {
            x: 0.,
            y: 0.,
            width: 1200.,
            height: 800.,
        },
        rustcast_core::Frame {
            x: 1200.,
            y: -100.,
            width: 800.,
            height: 600.,
        },
    ];
    let mut manager = Manager::default();
    let half = manager.move_window("left-half", identity, &screens, &platform, &ticket())?;
    if !window_placement::matches(
        half,
        rustcast_core::Frame {
            x: 0.,
            y: 0.,
            width: 600.,
            height: 800.,
        },
    ) || unsafe { effects() } != 3
    {
        return Err("Owned layout effect/readback mismatch".into());
    }
    let old = manager.move_window("restore", identity, &screens, &platform, &ticket())?;
    if !window_placement::matches(
        old,
        rustcast_core::Frame {
            x: 40.,
            y: 50.,
            width: 600.,
            height: 400.,
        },
    ) {
        return Err("Owned restore mismatch".into());
    }
    unsafe {
        mode(c"frame-cannot-complete".as_ptr());
    }
    manager.move_window("maximize", identity, &screens, &platform, &ticket())?;
    unsafe {
        mode(c"frame-constrained".as_ptr());
    }
    if manager
        .move_window("left-half", identity, &screens, &platform, &ticket())
        .is_ok()
    {
        return Err("Constrained geometry was accepted".into());
    }
    unsafe {
        mode(c"frame-denied".as_ptr());
    }
    if manager
        .move_window("maximize", identity, &screens, &platform, &ticket())
        .is_ok()
        || unsafe { effects() } != 0
    {
        return Err("Hard read failure caused a write".into());
    }
    unsafe {
        mode(c"restarted".as_ptr());
    }
    if manager
        .move_window("maximize", identity, &screens, &platform, &ticket())
        .is_ok()
        || unsafe { effects() } != 0
    {
        return Err("Reused PID caused a write".into());
    }
    Ok(
        serde_json::json!({"ok":true,"directNativeAXValues":true,"verifiedGeometry":true,"constrainedRejected":true,"cannotCompleteRequiresReadback":true,"reusedPIDRejected":true,"realDesktopEffects":false}),
    )
}
fn main() {
    let result = objc2::rc::autoreleasepool(|_| run());
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", serde_json::json!({"ok":false,"error":error}));
            std::process::exit(1);
        }
    }
}
