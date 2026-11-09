//! Requires the complete owned SDK substitute before any native discovery.
use ruycast_native::system_search::{self as os, Platform, Scope, Ticket};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn run() -> Result<serde_json::Value, String> {
    unsafe fn symbol<T: Copy>(name: &std::ffi::CStr) -> Result<T, String> {
        let p = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr());
        if p.is_null() {
            return Err("Owned SDK fixture missing; external discovery disabled.".into());
        }
        Ok(std::mem::transmute_copy(&p))
    }
    let ready: unsafe extern "C" fn() -> u64 = unsafe { symbol(c"ruycast_system_fixture_ready")? };
    if unsafe { ready() } != 0x525553595354454d {
        return Err("Owned SDK fixture not ready; external discovery disabled.".into());
    }
    let mode: unsafe extern "C" fn(*const std::ffi::c_char) =
        unsafe { symbol(c"ruycast_system_fixture_mode")? };
    let effects: unsafe extern "C" fn() -> i32 =
        unsafe { symbol(c"ruycast_system_fixture_effects")? };
    let reads: unsafe extern "C" fn() -> i32 = unsafe { symbol(c"ruycast_system_fixture_reads")? };
    let consents: unsafe extern "C" fn() -> i32 =
        unsafe { symbol(c"ruycast_system_fixture_consents")? };
    let ticket = || Ticket {
        deadline: Instant::now() + Duration::from_secs(8),
        live: Arc::new(|| true),
    };
    let platform = Arc::new(os::native::Native::new());
    let provider = os::providers::Provider::new(platform.clone(), false);
    let foreground = os::native::Foreground::default();
    foreground.remember(false);
    let original = foreground.snapshot().ok_or("Owned foreground missing")?;
    let activate: os::Activate = Arc::new(os::native::activate);
    let windows = provider.search(Scope::Windows, "", None, &ticket())?;
    if windows.commands.len() != 2 {
        return Err("Owned AX window discovery mismatch".into());
    }
    provider.execute(&windows.commands[1].id, None, &ticket(), &activate)?;
    if unsafe { effects() } != 3 {
        return Err("Owned minimized restore/raise mismatch".into());
    }
    unsafe {
        mode(c"renamed".as_ptr());
    }
    if provider
        .execute(&windows.commands[0].id, None, &ticket(), &activate)
        .is_ok()
        || unsafe { effects() } != 0
    {
        return Err("Changed window caused effect".into());
    }
    unsafe {
        mode(c"".as_ptr());
    }
    let menus = provider.search(Scope::Menus, "", Some(original.clone()), &ticket())?;
    if menus.commands.len() != 2 {
        return Err("Owned AX menu discovery mismatch".into());
    }
    provider.execute(
        &menus.commands[0].id,
        Some(original.identity),
        &ticket(),
        &activate,
    )?;
    if unsafe { effects() } != 2 {
        return Err("Owned menu activation/press mismatch".into());
    }
    unsafe {
        mode(c"".as_ptr());
    }
    let tabs = provider.search(Scope::Tabs, "native invalid", None, &ticket())?;
    if tabs.commands.len() != 1 {
        return Err("Owned real AE descriptor traversal mismatch".into());
    }
    provider.execute(&tabs.commands[0].id, None, &ticket(), &activate)?;
    if unsafe { effects() } != 3 {
        return Err("Owned Safari existing-tab write/raise/activate mismatch".into());
    }
    unsafe {
        mode(c"mutate-selection".as_ptr());
    }
    if provider
        .execute(&tabs.commands[0].id, None, &ticket(), &activate)
        .is_ok()
        || unsafe { effects() } != 1
    {
        return Err("Safari selection mismatch caused foreground effect".into());
    }
    provider.refresh();
    unsafe {
        mode(c"automation-denied".as_ptr());
    }
    if !provider
        .search(Scope::Tabs, "", None, &ticket())
        .unwrap_err()
        .contains("Automation")
    {
        return Err("Automation denial missing recovery".into());
    }
    if unsafe { consents() } != 0 {
        return Err("Passive lookup requested consent".into());
    }
    unsafe {
        mode(c"".as_ptr());
    }
    platform.safari_consent(&ticket())?;
    if unsafe { consents() } != 1 {
        return Err("Explicit consent bridge missing".into());
    }
    Ok(
        serde_json::json!({"ok":true,"ownedSDK":true,"directNativeAX":true,"directAppleEvents":true,"foregroundOwned":true,"noExternalQueries":true,"reads":unsafe{reads()},"consents":1,"realDesktopEffects":false,"immutableTabIdentity":false,"atomicSafariSelection":false}),
    )
}
fn main() {
    let r = objc2::rc::autoreleasepool(|_| run());
    match r {
        Ok(v) => println!("{v}"),
        Err(e) => {
            println!("{}", serde_json::json!({"ok":false,"error":e}));
            std::process::exit(1);
        }
    }
}
