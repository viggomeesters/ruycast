//! Isolated complete SDK keyword owner; no native fallback or real input.
use objc2::MainThreadMarker;
use ruycast_native::keyword_capture::{self, Gate, Prepared, Service};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    ffi::{c_void, CString},
    io::{self, Read},
    rc::Rc,
    sync::Arc,
};
struct Control {
    service: RefCell<Option<Rc<Service>>>,
    gate: Cell<Gate>,
    mode: Cell<u8>,
    panic_gate: Cell<bool>,
}
unsafe extern "C" fn reenter(context: *mut c_void) {
    let c = unsafe { &*context.cast::<Control>() };
    let mode = c.mode.replace(0);
    let service = c.service.borrow().clone();
    match mode {
        1 => {
            if let Some(s) = service {
                s.stop();
            }
        }
        2 => {
            let mut gate = c.gate.get();
            gate.running = false;
            c.gate.set(gate);
            drop(service);
            let old = c.service.borrow_mut().take();
            drop(old);
        }
        3 => {
            let mut gate = c.gate.get();
            gate.generation += 1;
            c.gate.set(gate);
            let mut config = rustcast_core::validate(&rustcast_core::default_config()).unwrap();
            config.snippet_expansion_enabled = false;
            if let Some(s) = service {
                s.update(gate.generation, Arc::new(Prepared::new(&config).unwrap()));
            }
        }
        4 => {
            let mut gate = c.gate.get();
            gate.blocked = true;
            gate.recording_epoch += 1;
            c.gate.set(gate);
        }
        5 => c.panic_gate.set(true),
        7 => {
            let _ =
                keyword_capture::fixture(&json!({"control":"board-external","value":"EXTERNAL"}));
        }
        8 => {
            let _ = keyword_capture::fixture(
                &json!({"control":"board-external","value":"HELLO","html":"EXTERNAL HTML"}),
            );
        }
        6 => {
            let _ = keyword_capture::fixture(&json!({"control":"event","text":"z"}));
        }
        _ => {}
    }
}
fn run() -> Result<Value, String> {
    let main = MainThreadMarker::new().ok_or("Own main required")?;
    let mut input = vec![];
    io::stdin()
        .take(16_777_217)
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    if input.len() > 16_777_216 {
        return Err("Own input too large".into());
    }
    let ops: Vec<Value> = serde_json::from_slice(&input).map_err(|e| e.to_string())?;
    let control = Rc::new(Control {
        service: RefCell::new(None),
        gate: Cell::new(Gate {
            running: true,
            generation: 1,
            ..Default::default()
        }),
        mode: Cell::new(0),
        panic_gate: Cell::new(false),
    });
    let gate_control = Rc::downgrade(&control);
    let service = Rc::new(Service::new(
        main,
        true,
        Rc::new(move || {
            let c = gate_control.upgrade().expect("Own gate retained");
            assert!(!c.panic_gate.replace(false), "Own gate fault");
            c.gate.get()
        }),
        Rc::new(|| {}),
    )?);
    *control.service.borrow_mut() = Some(service);
    let hook = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_keyword_fixture_hook".as_ptr()) };
    if hook.is_null() {
        return Err("Own SDK hook missing".into());
    }
    let hook: unsafe extern "C" fn(*const i8, unsafe extern "C" fn(*mut c_void), *mut c_void) =
        unsafe { std::mem::transmute(hook) };
    let mut results = vec![];
    for op in ops {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<Value, String> {
                match op["op"].as_str() {
                    Some("config") => {
                        let config = rustcast_core::validate(&op["value"])?;
                        let generation = op["generation"].as_u64().unwrap_or(1);
                        let mut gate = control.gate.get();
                        gate.generation = generation;
                        control.gate.set(gate);
                        let service = control
                            .service
                            .borrow()
                            .clone()
                            .ok_or("Own service dropped")?;
                        service.update(generation, Arc::new(Prepared::new(&config)?));
                        Ok(json!({}))
                    }
                    Some("refresh") => {
                        let service = control
                            .service
                            .borrow()
                            .clone()
                            .ok_or("Own service dropped")?;
                        service.refresh();
                        Ok(json!({"status":service.status()}))
                    }
                    Some("wait") => {
                        let ms = op["ms"].as_u64().unwrap_or(100).min(1000);
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                        Ok(json!({}))
                    }
                    Some("pump") => {
                        let service = control
                            .service
                            .borrow()
                            .clone()
                            .ok_or("Own service dropped")?;
                        service.pump();
                        Ok(json!({"pendingCursor":service.next_wake().is_some()}))
                    }
                    Some("stop") => {
                        let service = control
                            .service
                            .borrow()
                            .clone()
                            .ok_or("Own service dropped")?;
                        Ok(json!({"retired":service.stop()}))
                    }
                    Some("drop") => {
                        let old = control.service.borrow_mut().take();
                        drop(old);
                        Ok(json!({}))
                    }
                    Some("recover") => Ok(json!({"retired":Service::recover(main)})),
                    Some("gate") => {
                        let mut g = control.gate.get();
                        if let Some(v) = op["running"].as_bool() {
                            g.running = v;
                        }
                        if let Some(v) = op["blocked"].as_bool() {
                            g.blocked = v;
                        }
                        if let Some(v) = op["hyperHeld"].as_bool() {
                            g.hyper_held = v;
                        }
                        if let Some(v) = op["generation"].as_u64() {
                            g.generation = v;
                        }
                        if let Some(v) = op["epoch"].as_u64() {
                            g.recording_epoch = v;
                        }
                        if let Some(v) = op["activationEpoch"].as_u64() {
                            g.activation_epoch = v;
                        }
                        if let Some(v) = op["stageEpoch"].as_u64() {
                            g.stage_epoch = v;
                        }
                        if let Some(v) = op["hyperEpoch"].as_u64() {
                            g.hyper_epoch = v;
                        }
                        control.gate.set(g);
                        Ok(json!({}))
                    }
                    Some("hook") => {
                        control.mode.set(match op["action"].as_str() {
                            Some("stop") => 1,
                            Some("drop") => 2,
                            Some("disable") => 3,
                            Some("record") => 4,
                            Some("panic") => 5,
                            Some("reenter") => 6,
                            Some("external") => 7,
                            Some("external-html") => 8,
                            _ => return Err("Bad own hook action".into()),
                        });
                        let at = CString::new(op["at"].as_str().ok_or("Own phase missing")?)
                            .map_err(|_| "Own phase invalid")?;
                        unsafe {
                            hook(at.as_ptr(), reenter, Rc::as_ptr(&control).cast_mut().cast())
                        };
                        Ok(json!({}))
                    }
                    Some("status") => {
                        let service = control.service.borrow().clone();
                        Ok(
                            json!({"status":service.as_ref().map(|s|s.status()),"retired":service.as_ref().is_none_or(|s|s.retired()),"counts":service.as_ref().map(|s|s.counters()),"pending":Service::recovery_pending(main),"cursor":service.as_ref().is_some_and(|s|s.next_wake().is_some()),"ownChange":service.as_ref().and_then(|s|s.own_change()),"ownActive":service.as_ref().is_some_and(|s|s.own_active()),"clipboardPartials":service.as_ref().map(|s|s.clipboard_partials())}),
                        )
                    }
                    Some("sdk") => keyword_capture::fixture(&op),
                    _ => Err("Unknown own operation".into()),
                }
            },
        ));
        results.push(match result {
            Ok(r) => r?,
            Err(_) => json!({"panic":true}),
        });
    }
    let old = control.service.borrow_mut().take();
    drop(old);
    let retired = Service::recover(main);
    let final_state = keyword_capture::fixture(&json!({"control":"state"}))?;
    Ok(json!({"ok":true,"steps":results,"retired":retired,"final":final_state}))
}
fn main() {
    let value =
        objc2::rc::autoreleasepool(|_| run()).unwrap_or_else(|e| json!({"ok":false,"error":e}));
    println!("{value}");
}
