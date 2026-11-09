//! Explicit test-only component process. Never loads the actual Caps SDK table.
use objc2::MainThreadMarker;
use ruycast_native::{caps_io::Native, caps_lock::Control};
use serde_json::{json, Value};
use std::{ffi::c_void, io::Read};
struct Nested {
    owner: Control<Native>,
    result: std::cell::RefCell<Option<String>>,
}
unsafe extern "C" fn nested(context: *mut c_void) {
    let nested = unsafe { &*context.cast::<Nested>() };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| nested.owner.activate()));
    *nested.result.borrow_mut() = Some(format!("{result:?}"));
}
unsafe fn symbol(name: &std::ffi::CStr) -> Result<*mut c_void, String> {
    let pointer = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if pointer.is_null() {
        Err("Caps test controller unavailable.".into())
    } else {
        Ok(pointer)
    }
}
fn run() -> Result<Value, String> {
    let main = MainThreadMarker::new().ok_or("Main thread required.")?;
    let io = Native::load(main, true)?; // validates every boundary before controller calls
    let mut input = String::new();
    std::io::stdin()
        .take(1024 * 1024)
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    let input: Value = serde_json::from_str(&input).map_err(|e| e.to_string())?;
    unsafe {
        let configure: unsafe extern "C" fn(bool, u64) =
            std::mem::transmute(symbol(c"ruycast_caps_fixture_configure")?);
        let failures: unsafe extern "C" fn(u64) =
            std::mem::transmute(symbol(c"ruycast_caps_fixture_failures")?);
        let state: unsafe extern "C" fn() -> u32 =
            std::mem::transmute(symbol(c"ruycast_caps_fixture_state")?);
        configure(
            input["original"].as_bool().unwrap_or(false),
            input["failures"].as_u64().unwrap_or(0),
        );
        let mut nested_owner = None;
        if let Some(at) = input["nestedAt"].as_u64() {
            let hook: unsafe extern "C" fn(u32, unsafe extern "C" fn(*mut c_void), *mut c_void) =
                std::mem::transmute(symbol(c"ruycast_caps_fixture_hook")?);
            let mut owner = Box::new(Nested {
                owner: Control::new(Native::load(main, true)?),
                result: std::cell::RefCell::new(None),
            });
            hook(
                at.try_into().map_err(|_| "Invalid nested boundary.")?,
                nested,
                (&mut *owner as *mut Nested).cast(),
            );
            nested_owner = Some(owner);
        }
        let mut control = Some(Control::new(io));
        let mut results = Vec::new();
        for operation in input["operations"]
            .as_array()
            .ok_or("Operations required.")?
        {
            let result = match operation.as_str() {
                Some("activate") => {
                    format!("{:?}", control.as_ref().ok_or("Owner dropped.")?.activate())
                }
                Some("clear") => format!("{:?}", control.as_ref().ok_or("Owner dropped.")?.clear()),
                Some("retire") => {
                    format!("{:?}", control.as_ref().ok_or("Owner dropped.")?.retire())
                }
                Some("drop") => {
                    drop(control.take());
                    "dropped".into()
                }
                Some("recover") => format!("{:?}", Native::retry_recovery(main)),
                Some("new") => {
                    control = Some(Control::new(Native::load(main, true)?));
                    "new".into()
                }
                _ if operation["failures"].is_u64() => {
                    failures(operation["failures"].as_u64().unwrap());
                    "configured".into()
                }
                _ => return Err("Unknown Caps test operation.".into()),
            };
            results.push(json!({"result":result, "state":state(), "health":control.as_ref().map(|v| format!("{:?}", v.health())), "recoveryPending":Native::recovery_pending(main)}));
        }
        drop(control);
        Ok(
            json!({"results":results,"finalState":state(),"recoveryPending":Native::recovery_pending(main),
            "nested":nested_owner.as_ref().and_then(|value| value.result.borrow().clone())}),
        )
    }
}
fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}
