//! Feature-only, complete SDK fixture; no production/global registration path.
use objc2::MainThreadMarker;
use ruycast_native::{
    carbon_hotkeys::Carbon,
    hotkeys::{Plan, Registry},
};
use serde_json::{json, Value};
use std::{
    ffi::c_void,
    io::{self, Read},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
#[repr(C)]
struct FixtureEvent {
    class: u32,
    kind: u32,
    signature: u32,
    id: u32,
    status: i32,
    parameter_type: u32,
    size: usize,
}
fn run() -> Result<Value, String> {
    let main = MainThreadMarker::new().ok_or("Main fixture thread required")?;
    let mut bytes = vec![];
    io::stdin()
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("Fixture packet exceeds limit".into());
    }
    let input: Vec<Value> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if input.len() > 128 {
        return Err("Too many fixture operations".into());
    }
    let presses = Arc::new(Mutex::new(vec![]));
    let sink = presses.clone();
    let panic = Arc::new(AtomicBool::new(false));
    let should_panic = panic.clone();
    let platform = Carbon::new(
        main,
        true,
        Arc::new(move |press| {
            assert!(!should_panic.load(Ordering::Acquire), "Own callback panic");
            sink.lock().unwrap().push(press);
        }),
    );
    let mut gate = None;
    let (mut registry, initialization) = match platform {
        Ok(platform) => {
            gate = Some(platform.callback_gate());
            (Some(Registry::new(platform)), json!({"ok":true}))
        }
        Err(error) => (None, json!({"ok":false,"error":error})),
    };
    let mut results = vec![];
    for op in input {
        let result = match op["action"].as_str() {
            Some("stage") => {
                let cfg = rustcast_core::validate(&op["config"])?;
                let plan = Plan::from_config(&cfg)?;
                match registry
                    .as_mut()
                    .ok_or("Own registry unavailable")?
                    .stage(plan)
                {
                    Ok(generation) => json!({"ok":true,"generation":generation}),
                    Err(error) => json!({"ok":false,"error":error}),
                }
            }
            Some("commit") => {
                registry
                    .as_mut()
                    .ok_or("Own registry unavailable")?
                    .commit();
                json!({"ok":true})
            }
            Some("rollback") => {
                registry
                    .as_mut()
                    .ok_or("Own registry unavailable")?
                    .rollback();
                json!({"ok":true})
            }
            Some("suspend") => {
                registry
                    .as_mut()
                    .ok_or("Own registry unavailable")?
                    .suspend();
                json!({"ok":true})
            }
            Some("resume") => match registry
                .as_mut()
                .ok_or("Own registry unavailable")?
                .resume()
            {
                Ok(()) => json!({"ok":true}),
                Err(error) => json!({"ok":false,"error":error}),
            },
            Some("drop") => {
                registry.take();
                json!({"ok":true})
            }
            Some("gate") => {
                if let Some(gate) = &gate {
                    gate.store(op["enabled"] == true, Ordering::Release);
                }
                json!({"ok":true})
            }
            Some("panic") => {
                panic.store(true, Ordering::Release);
                json!({"ok":true})
            }
            Some("emit") => {
                let pointer = unsafe {
                    libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"ruycast_keyboard_fixture_emit".as_ptr(),
                    )
                };
                if pointer.is_null() {
                    return Err("Own callback fixture missing".into());
                }
                let emit: unsafe extern "C" fn(*const FixtureEvent) -> i32 =
                    unsafe { std::mem::transmute::<*mut c_void, _>(pointer) };
                let field =
                    |key: &str, default: u32| op[key].as_u64().map(|v| v as u32).unwrap_or(default);
                let event = FixtureEvent {
                    class: field("class", u32::from_be_bytes(*b"keyb")),
                    kind: field("kind", 5),
                    signature: field("signature", 0x52554341),
                    id: field("id", 1),
                    status: op["status"].as_i64().unwrap_or(0) as i32,
                    parameter_type: field("parameterType", u32::from_be_bytes(*b"hkid")),
                    size: op["size"].as_u64().unwrap_or(8) as usize,
                };
                json!({"status":unsafe{emit(&event)}})
            }
            _ => return Err("Unknown own keyboard operation".into()),
        };
        results.push(result);
    }
    drop(registry);
    let presses = presses
        .lock()
        .unwrap()
        .iter()
        .map(|p| json!({"generation":p.generation,"key":p.canonical,"action":p.action}))
        .collect::<Vec<_>>();
    Ok(json!({"initialization":initialization,"steps":results,"presses":presses}))
}
fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", json!({"ok":false,"error":error}));
            std::process::exit(1);
        }
    }
}
