//! Complete own SDK required; no native API fallback or real application use.
use objc2::MainThreadMarker;
use ruycast_native::keyword_target::{Anchor, Context, Continuity, Target};
use serde_json::{json, Value};
use std::{
    cell::Cell,
    ffi::{c_void, CStr, CString},
    io::{self, Read},
    rc::Rc,
};
fn symbol<T: Copy>(name: &CStr) -> Result<T, String> {
    let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if p.is_null() {
        return Err("Own keyword controller missing".into());
    }
    assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
    Ok(unsafe { std::mem::transmute_copy(&p) })
}
unsafe extern "C" fn revoke(context: *mut c_void) {
    unsafe { &*context.cast::<Cell<bool>>() }.set(false);
}
fn run() -> Result<Value, String> {
    let main = MainThreadMarker::new().ok_or("Main required")?;
    let mut bytes = vec![];
    io::stdin()
        .take(16_777_217)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16_777_216 {
        return Err("Own target input too large".into());
    }
    let ops: Vec<Value> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let context = Context::new(main, true)?;
    let env: unsafe extern "C" fn(u32, i32, u64, u64) =
        symbol(c"ruycast_keyword_fixture_environment")?;
    let change: unsafe extern "C" fn(*const i8, u32, i32, u64, u64) =
        symbol(c"ruycast_keyword_fixture_change")?;
    let text: unsafe extern "C" fn(*const u16, usize, isize, isize) =
        symbol(c"ruycast_keyword_fixture_text")?;
    let failure: unsafe extern "C" fn(*const i8) = symbol(c"ruycast_keyword_fixture_failure")?;
    let hook: unsafe extern "C" fn(*const i8, unsafe extern "C" fn(*mut c_void), *mut c_void) =
        symbol(c"ruycast_keyword_fixture_hook")?;
    let state: unsafe extern "C" fn() -> *const i8 = symbol(c"ruycast_keyword_fixture_state")?;
    let gate = Rc::new(Cell::new(true));
    let allowed = || gate.get();
    let mut target: Option<Target> = None;
    let mut anchor: Option<Anchor> = None;
    let mut results = vec![];
    for op in ops {
        let result = match op["op"].as_str() {
            Some("bundle-units") => {
                let units = op["units"]
                    .as_array()
                    .ok_or("Own units required")?
                    .iter()
                    .map(|v| {
                        v.as_u64()
                            .filter(|n| *n <= u16::MAX as u64)
                            .map(|n| n as u16)
                            .ok_or("Own UTF16 unit required")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let set: unsafe extern "C" fn(*const u16, usize) =
                    symbol(c"ruycast_keyword_fixture_set_bundle")?;
                unsafe {
                    set(units.as_ptr(), units.len());
                }
                json!({})
            }
            Some("activation") => ruycast_native::workspace_activation::fixture(&op["case"])?,
            Some("env") => {
                unsafe {
                    env(
                        op["flags"].as_u64().unwrap_or(0) as u32,
                        op["pid"].as_i64().unwrap_or(42) as i32,
                        op["app"].as_u64().unwrap_or(1),
                        op["element"].as_u64().unwrap_or(1),
                    )
                };
                json!({})
            }
            Some("change") => {
                let name =
                    CString::new(op["at"].as_str().unwrap_or("")).map_err(|_| "Bad phase")?;
                unsafe {
                    change(
                        name.as_ptr(),
                        op["flags"].as_u64().unwrap_or(0) as u32,
                        op["pid"].as_i64().unwrap_or(42) as i32,
                        op["app"].as_u64().unwrap_or(1),
                        op["element"].as_u64().unwrap_or(1),
                    )
                };
                json!({})
            }
            Some("text") => {
                let units: Vec<u16> = if let Some(u) = op["units"].as_array() {
                    u.iter()
                        .map(|n| n.as_u64().expect("Own unit") as u16)
                        .collect()
                } else {
                    op["value"].as_str().unwrap_or("").encode_utf16().collect()
                };
                unsafe {
                    text(
                        units.as_ptr(),
                        units.len(),
                        op["at"].as_i64().unwrap_or(units.len() as i64) as isize,
                        op["length"].as_i64().unwrap_or(0) as isize,
                    )
                };
                json!({})
            }
            Some("failure") => {
                let name =
                    CString::new(op["name"].as_str().unwrap_or("")).map_err(|_| "Bad name")?;
                unsafe { failure(name.as_ptr()) };
                json!({})
            }
            Some("revoke") => {
                let name =
                    CString::new(op["at"].as_str().unwrap_or("front")).map_err(|_| "Bad phase")?;
                unsafe { hook(name.as_ptr(), revoke, Rc::as_ptr(&gate).cast_mut().cast()) };
                json!({})
            }
            Some("gate") => {
                gate.set(op["value"].as_bool().unwrap_or(true));
                json!({})
            }
            Some("front") => match context.frontmost(&allowed) {
                Ok(value) => {
                    target = value;
                    json!({"ok":true,"target":target.as_ref().map(|t|json!({"pid":t.pid(),"bundle":t.bundle()}))})
                }
                Err(e) => json!({"error":format!("{e:?}")}),
            },
            Some("safe") => match context.safe_field(&allowed) {
                Ok(value) => json!({"safe":value}),
                Err(e) => json!({"error":format!("{e:?}")}),
            },
            Some("mixed-backend") => {
                // Construct function tables only; mixed authorities must refuse
                // before calling either the native or fixture SDK.
                let native = Context::new(main, false)?;
                let target = target.as_ref().ok_or("Target missing")?;
                let replacement = ruycast_native::keyword_matcher::Replacement::new(
                    "abc",
                    "c",
                    "OWN".into(),
                    None,
                );
                let keyboard = Cell::new(false);
                let clipboard = Cell::new(false);
                let own = Rc::new(ruycast_native::clipboard_ownership::OwnWrites::default());
                let plain = ruycast_native::keyword_effects::apply_plain(
                    native,
                    target,
                    &replacement,
                    &allowed,
                    &keyboard,
                );
                let rich = ruycast_native::keyword_effects::apply_rich(
                    native,
                    target,
                    &replacement,
                    "html",
                    &allowed,
                    ruycast_native::keyword_effects::RichEffects {
                        keyboard_possible: &keyboard,
                        clipboard_possible: &clipboard,
                        own: &own,
                    },
                );
                let anchor = native.anchor(target, "abc", &allowed);
                json!({"plain":plain.consume_trigger(),"rich":rich.outcome.consume_trigger(),"cursor":rich.cursor.is_some(),"keyboardPossible":keyboard.get(),"clipboardPossible":clipboard.get(),"anchorRejected":matches!(anchor,Err(ruycast_native::keyword_target::Error::InvalidData))})
            }
            Some("current") => match target.as_ref().ok_or("Target missing")?.current(&allowed) {
                Ok(value) => json!({"current":value}),
                Err(e) => json!({"error":format!("{e:?}")}),
            },
            Some("activate") => match target.as_ref().ok_or("Target missing")?.activate(&allowed) {
                Ok(value) => json!({"activated":value}),
                Err(e) => json!({"error":format!("{e:?}")}),
            },
            Some("anchor") => match context.anchor(
                target.as_ref().ok_or("Target missing")?,
                op["keyword"].as_str().unwrap_or("abc"),
                &allowed,
            ) {
                Ok(value) => {
                    anchor = Some(value);
                    json!({"ok":true})
                }
                Err(e) => json!({"error":format!("{e:?}")}),
            },
            Some("verify") => match anchor.as_ref().ok_or("Anchor missing")?.verify(&allowed) {
                Ok(value) => {
                    json!({"continuity":match value{Continuity::Pinned=>"pinned",Continuity::Changed=>"changed",Continuity::EventGuardsRequired=>"event-guards-required"}})
                }
                Err(e) => json!({"error":format!("{e:?}")}),
            },
            Some("clear") => {
                anchor = None;
                target = None;
                json!({})
            }
            Some("state") => {
                let raw = unsafe { CStr::from_ptr(state()) };
                serde_json::from_slice(raw.to_bytes()).map_err(|e| e.to_string())?
            }
            _ => return Err("Unknown own keyword operation".into()),
        };
        results.push(result);
    }
    drop(anchor);
    drop(target);
    let final_state: Value = serde_json::from_slice(unsafe { CStr::from_ptr(state()) }.to_bytes())
        .map_err(|e| e.to_string())?;
    Ok(json!({"ok":true,"steps":results,"final":final_state}))
}
fn main() {
    let result = objc2::rc::autoreleasepool(|_| run())
        .unwrap_or_else(|error| json!({"ok":false,"error":error}));
    println!("{result}");
}
