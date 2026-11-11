//! Main-thread integration of actual native owners through the loaded host SDK.
use objc2::MainThreadMarker;
use ruycast_native::{
    carbon_hotkeys::Carbon,
    hid_service::{Gate, Service},
    hotkeys::{Binding, Plan, Registry},
};
use std::{
    ffi::{c_void, CStr},
    rc::Rc,
    sync::{Arc, Mutex},
};
unsafe fn symbol(name: &CStr) -> *mut c_void {
    let pointer = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr());
    assert!(!pointer.is_null(), "Missing own controller {name:?}");
    pointer
}
fn main() {
    let main = MainThreadMarker::new().expect("Own main thread");
    let mode = std::env::args().nth(1).expect("Own probe mode");
    let gate = Rc::new(|| Gate {
        running: true,
        recording: false,
        epoch: 0,
    });
    let service = Service::new(main, true, gate, Rc::new(|_| true));
    if mode == "missing-post" || mode == "missing-marker" {
        assert!(service.is_err());
        println!("refused-before-HID-entry");
        return;
    }
    let service = service.expect("Complete host HID table");
    let presses = Arc::new(Mutex::new(Vec::new()));
    let sink = presses.clone();
    let carbon = Carbon::new(
        main,
        true,
        Arc::new(move |press| sink.lock().unwrap().push(press)),
    );
    if mode == "missing-parameter" {
        assert!(carbon.is_err());
        println!("refused-before-Carbon-entry");
        return;
    }
    let mut registry = Registry::new(carbon.expect("Complete host Carbon table"));
    let mut plan = Plan {
        hyper: true,
        ..Plan::default()
    };
    plan.carbon.insert(
        "cmd+a".into(),
        Binding {
            action: "owned.action".into(),
            code: 0,
            modifiers: 1 << 8,
        },
    );
    registry.stage(plan.clone()).unwrap();
    registry.commit();
    unsafe {
        let caps: unsafe extern "C" fn() -> u32 =
            std::mem::transmute(symbol(c"ruycast_caps_fixture_state"));
        let ready: unsafe extern "C" fn() -> u64 =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_ready"));
        let event: unsafe extern "C" fn(u32, u16, u64, i64, bool) -> u64 =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_event"));
        let input: unsafe extern "C" fn(u32, u32, u32, bool, f64) =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_input"));
        let state: unsafe extern "C" fn() -> *const libc::c_char =
            std::mem::transmute(symbol(c"ruycast_hid_fixture_state"));
        assert_eq!(caps(), 1, "Host readiness initializes original latch");
        service.update(1, plan);
        service.refresh();
        if mode == "failure" {
            assert_ne!(service.status(), "On");
            assert_eq!(caps(), 1, "Setup fault restores shared original latch");
        } else {
            assert_eq!(service.status(), "On");
            assert_eq!(caps() & 1, 0);
            assert_ne!(ready(), 0);
            assert_eq!(
                caps() & 1,
                0,
                "Repeated ready must not reset acquired latch"
            );
            assert_eq!(event(10, 0, 1 << 20, 0, false), 0);
            assert_eq!(presses.lock().unwrap().len(), 1);
            {
                let rows = presses.lock().unwrap();
                assert_eq!(rows[0].canonical, "cmd+a");
                assert_eq!(rows[0].action, "owned.action");
            }
            event(10, 0, 1 << 20, 0, true);
            event(11, 0, 1 << 20, 0, false);
            assert_eq!(
                presses.lock().unwrap().len(),
                1,
                "Repeat/key-up never forward"
            );
            input(0, 7, 57, true, 0.0);
            input(0, 7, 57, false, 0.0);
        }
        assert!(service.stop());
        drop(service);
        drop(registry);
        assert_eq!(
            caps(),
            u32::from(mode == "failure"),
            "Failure restores original latch; committed Hyper retires off; resources released"
        );
        let state: serde_json::Value =
            serde_json::from_slice(CStr::from_ptr(state()).to_bytes()).unwrap();
        assert_eq!(state["allocated"], 0);
        assert_eq!(state["lastFlags"], 0);
    }
    println!("{mode}:owners-and-shared-state-ok");
}
