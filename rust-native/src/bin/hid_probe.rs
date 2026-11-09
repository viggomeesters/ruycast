//! Test-only owned synthetic callbacks. Always loads the complete fixture table.
use objc2::MainThreadMarker;
use ruycast_native::{
    hid_service::{Gate, Service},
    hotkeys::Plan,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::{
    cell::{Cell, RefCell},
    ffi::{c_void, CStr, CString},
    io::Read,
    rc::{Rc, Weak},
};
static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
struct Allocator;
unsafe impl std::alloc::GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
unsafe fn symbol(name: &CStr) -> Result<*mut c_void, String> {
    let value = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if value.is_null() {
        Err("HID test controller unavailable.".into())
    } else {
        Ok(value)
    }
}
struct Nested {
    service: RefCell<Weak<Service>>,
    mode: RefCell<String>,
    gate: Rc<Cell<Gate>>,
    panic_gate: Rc<Cell<bool>>,
}
unsafe extern "C" fn nested(pointer: *mut c_void) {
    let value = unsafe { &*pointer.cast::<Nested>() };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let service = value.service.borrow().upgrade();
        let mode = value.mode.borrow().clone();
        if let Some(service) = service {
            match mode.as_str() {
                "panic-gate" => value.panic_gate.set(true),
                "stop" => {
                    service.stop();
                }
                "off" => service.update(2, Plan::default()),
                "sleep" => service.sleeping(true),
                "record" => {
                    let mut gate = value.gate.get();
                    gate.epoch += 1;
                    gate.recording = true;
                    value.gate.set(gate);
                }
                _ => {}
            }
        }
    }));
}
fn plan(value: &Value) -> Plan {
    let mut plan = Plan {
        hyper: value["hyper"].as_bool().unwrap_or(false),
        ..Default::default()
    };
    if let Some(map) = value["sided"].as_object() {
        for (key, id) in map {
            plan.sided.insert(key.clone(), id.as_str().unwrap().into());
        }
    }
    if let Some(map) = value["modifiers"].as_object() {
        for (key, id) in map {
            plan.modifier_taps
                .insert(key.parse().unwrap(), id.as_str().unwrap().into());
        }
    }
    plan
}
fn run() -> Result<Value, String> {
    let main = MainThreadMarker::new().ok_or("Main required.")?;
    let gate = Rc::new(Cell::new(Gate {
        running: true,
        recording: false,
        epoch: 0,
    }));
    let panic_gate = Rc::new(Cell::new(false));
    let gate_calls = Rc::new(Cell::new(0u64));
    let stop_at = Rc::new(Cell::new(None));
    let gate_owner: Rc<RefCell<Weak<Service>>> = Rc::new(RefCell::new(Weak::new()));
    let calls = gate_calls.clone();
    let stop = stop_at.clone();
    let owner = gate_owner.clone();
    let check = panic_gate.clone();
    let sample = gate.clone();
    let authority: Rc<dyn Fn() -> Gate> = Rc::new(move || {
        assert!(!check.get(), "synthetic lifecycle gate failure");
        calls.set(calls.get() + 1);
        if stop.get() == Some(calls.get()) {
            stop.set(None);
            let service = owner.borrow().upgrade();
            if let Some(service) = service {
                service.stop();
            }
        }
        sample.get()
    });
    let actions = Rc::new(RefCell::new(Vec::new()));
    let output = actions.clone();
    let accepts = Rc::new(Cell::new(true));
    let admission = accepts.clone();
    let sink = Rc::new(move |press: ruycast_native::hotkeys::Press| {
        if !admission.get() {
            return false;
        }
        output.borrow_mut().push(json!({"generation":press.generation,"canonical":press.canonical,"action":press.action}));
        true
    });
    let mut service = Some(Rc::new(Service::new(
        main,
        true,
        authority.clone(),
        sink.clone(),
    )?));
    *gate_owner.borrow_mut() = Rc::downgrade(service.as_ref().unwrap());
    let mut text = String::new();
    std::io::stdin()
        .take(1024 * 1024)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    let data: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    unsafe {
        let configure: unsafe extern "C" fn(bool, u32) =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_configure")?);
        let state: unsafe extern "C" fn() -> *const libc::c_char =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_state")?);
        let failure: unsafe extern "C" fn(*const libc::c_char) =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_failure")?);
        let caps_failures: unsafe extern "C" fn(u64) =
            std::mem::transmute(symbol(c"ruycast_caps_fixture_failures")?);
        let environment: unsafe extern "C" fn(u32) =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_environment")?);
        let input: unsafe extern "C" fn(u32, u32, u32, bool, f64) =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_input")?);
        let remove: unsafe extern "C" fn(u32) =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_remove")?);
        let event: unsafe extern "C" fn(u32, u16, u64, i64, bool) -> u64 =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_event")?);
        let event_flags: unsafe extern "C" fn() -> u64 =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_event_flags")?);
        let quiet: unsafe extern "C" fn(bool) =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_quiet")?);
        let hook: unsafe extern "C" fn(
            *const libc::c_char,
            Option<unsafe extern "C" fn(*mut c_void)>,
            *mut c_void,
        ) = std::mem::transmute(symbol(c"ruycast_hid_fixture_hook")?);
        configure(
            data["original"].as_bool().unwrap_or(true),
            data["env"].as_u64().unwrap_or(0) as u32,
        );
        let mut nested_owner = Box::new(Nested {
            service: RefCell::new(Rc::downgrade(service.as_ref().unwrap())),
            mode: RefCell::new(String::new()),
            gate: gate.clone(),
            panic_gate: panic_gate.clone(),
        });
        let mut rows = Vec::new();
        for operation in data["operations"]
            .as_array()
            .ok_or("Operations required.")?
        {
            let mut result = json!(null);
            match operation["op"].as_str().ok_or("Operation required.")? {
                "plan" => service.as_ref().ok_or("Dropped.")?.update(
                    operation["generation"].as_u64().unwrap_or(1),
                    plan(operation),
                ),
                "refresh" => {
                    result = json!(format!(
                        "{:?}",
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| service
                            .as_ref()
                            .unwrap()
                            .refresh()))
                        .map_err(|_| "panic")
                    ));
                }
                "stop" => result = json!(service.as_ref().unwrap().stop()),
                "sleep" => service
                    .as_ref()
                    .unwrap()
                    .sleeping(operation["value"].as_bool().unwrap()),
                "drop" => drop(service.take()),
                "new" => {
                    service = Some(Rc::new(Service::new(
                        main,
                        true,
                        authority.clone(),
                        sink.clone(),
                    )?));
                    *nested_owner.service.borrow_mut() = Rc::downgrade(service.as_ref().unwrap());
                    *gate_owner.borrow_mut() = Rc::downgrade(service.as_ref().unwrap());
                }
                "recover" => result = json!(Service::recover(main)),
                "failure" => failure(
                    CString::new(operation["name"].as_str().unwrap())
                        .unwrap()
                        .as_ptr(),
                ),
                "caps-failures" => caps_failures(operation["value"].as_u64().unwrap()),
                "env" => environment(operation["value"].as_u64().unwrap() as u32),
                "accept" => accepts.set(operation["value"].as_bool().unwrap()),
                "panic-gate" => panic_gate.set(operation["value"].as_bool().unwrap()),
                "gate-stop" => stop_at.set(Some(
                    gate_calls.get() + operation["after"].as_u64().unwrap(),
                )),
                "gate" => gate.set(Gate {
                    running: operation["running"].as_bool().unwrap_or(true),
                    recording: operation["recording"].as_bool().unwrap_or(false),
                    epoch: operation["epoch"].as_u64().unwrap(),
                }),
                "hook" => {
                    *nested_owner.mode.borrow_mut() = operation["mode"].as_str().unwrap().into();
                    hook(
                        CString::new(operation["name"].as_str().unwrap())
                            .unwrap()
                            .as_ptr(),
                        Some(nested),
                        (&mut *nested_owner as *mut Nested).cast(),
                    );
                }
                "input" => {
                    let device = operation["device"].as_u64().unwrap_or(0);
                    if device >= 16 {
                        return Err("Invalid fixture device.".into());
                    }
                    input(
                        device as u32,
                        operation["page"].as_u64().unwrap_or(7) as u32,
                        operation["usage"].as_u64().unwrap() as u32,
                        operation["down"].as_bool().unwrap(),
                        operation["time"].as_f64().unwrap_or(0.0),
                    );
                }
                "remove" => remove(
                    operation["device"]
                        .as_u64()
                        .filter(|d| *d < 16)
                        .ok_or("Invalid device.")? as u32,
                ),
                "event" => {
                    let consumed = event(
                        operation["type"].as_u64().unwrap() as u32,
                        operation["key"].as_u64().unwrap_or(0) as u16,
                        operation["flags"].as_u64().unwrap_or(0),
                        operation["tag"].as_i64().unwrap_or(0),
                        operation["repeat"].as_bool().unwrap_or(false),
                    ) != 0;
                    result = json!({"consumed":consumed,"flags":event_flags()});
                }
                "allocation-events" => {
                    let count = operation["count"]
                        .as_u64()
                        .filter(|n| *n <= 100_000)
                        .ok_or("Invalid event count.")?;
                    quiet(true);
                    ALLOCATIONS.store(0, Ordering::Relaxed);
                    COUNTING.store(true, Ordering::Relaxed);
                    for _ in 0..count {
                        event(22, 0, 0, 0, false);
                        event(10, 0, 0, 0, false);
                        event(11, 0, 0, 0, false);
                    }
                    COUNTING.store(false, Ordering::Relaxed);
                    quiet(false);
                    result = json!({"rustAllocations":ALLOCATIONS.load(Ordering::Relaxed),"callbacks":count*3});
                }
                _ => return Err("Unknown HID operation.".into()),
            }
            let state: Value =
                serde_json::from_str(CStr::from_ptr(state()).to_str().unwrap()).unwrap();
            rows.push(json!({"result":result,"state":state,"status":service.as_ref().map(|s|s.status()),"failed":service.as_ref().is_some_and(|s|s.failed()),"actions":actions.borrow().clone()}));
        }
        hook(c"".as_ptr(), None, std::ptr::null_mut());
        panic_gate.set(false);
        caps_failures(0);
        failure(c"".as_ptr());
        drop(service);
        Service::recover(main);
        let cleanup: Value =
            serde_json::from_str(CStr::from_ptr(state()).to_str().unwrap()).unwrap();
        Ok(json!({"rows":rows,"cleanup":cleanup}))
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
