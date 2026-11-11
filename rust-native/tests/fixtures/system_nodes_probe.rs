//! Component proof only. No provider constructor, interpose, discovery or readiness.
mod system_apps;
mod system_effects;
mod system_nodes;
use objc2::{rc::autoreleasepool, runtime::AnyObject};
use objc2_foundation::{NSPoint, NSSize, NSString};
use serde_json::json;
use system_nodes::*;
fn key(value: &str) -> objc2::rc::Retained<NSString> {
    NSString::from_str(value)
}
unsafe fn attr(state: &State, index: usize, name: &str) -> CF {
    let mut out = std::ptr::null_mut();
    let key = key(name);
    assert_eq!(
        state.attr(
            state.node(index),
            (&*key as *const NSString as *mut NSString).cast(),
            &mut out
        ),
        0
    );
    assert!(!out.is_null());
    CF(out)
}
unsafe fn text(value: &CF) -> String {
    (&*value.0.cast::<NSString>()).to_string()
}
unsafe fn boolean(value: &CF) -> bool {
    assert_eq!(CFGetTypeID(value.0), CFBooleanGetTypeID());
    CFBooleanGetValue(value.0) != 0
}
unsafe fn frame(state: &State) -> (f64, f64, f64, f64) {
    let position = attr(state, 2, "AXPosition");
    let dimensions = attr(state, 2, "AXSize");
    let mut point = NSPoint::new(0., 0.);
    let mut size = NSSize::new(0., 0.);
    assert_ne!(
        AXValueGetValue(position.0, 1, (&mut point as *mut NSPoint).cast()),
        0
    );
    assert_ne!(
        AXValueGetValue(dimensions.0, 2, (&mut size as *mut NSSize).cast()),
        0
    );
    (point.x, point.y, size.width, size.height)
}
unsafe fn write_frame(state: &State, expected_error: i32) {
    let point = NSPoint::new(84., 93.);
    let size = NSSize::new(500., 400.);
    let point = CF(AXValueCreate(1, (&point as *const NSPoint).cast()));
    let size = CF(AXValueCreate(2, (&size as *const NSSize).cast()));
    for (name, value) in [("AXPosition", &point), ("AXSize", &size)] {
        let key = key(name);
        assert_eq!(
            system_effects::set(
                state,
                state.node(2),
                (&*key as *const NSString as *mut NSString).cast(),
                value.0
            ),
            expected_error
        );
    }
}
fn main() {
    autoreleasepool(|_| unsafe {
        let state = std::sync::Arc::new(State::new());
        let mode = std::env::args().nth(1).unwrap();
        assert!(
            libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_system_fixture_ready".as_ptr()).is_null()
        );
        match mode.as_str() {
            "normal" => {
                for a in 1..=8 {
                    assert_eq!(CFGetTypeID(state.node(a)), AXUIElementGetTypeID());
                    for b in 1..a {
                        assert_eq!(CFEqual(state.node(a), state.node(b)), 0);
                    }
                }
                let app = CF(state.application(730001));
                assert_ne!(CFEqual(app.0, state.node(1)), 0);
                let mut pid = 0;
                assert_eq!(state.pid(app.0, &mut pid), 0);
                assert_eq!(pid, 730001);
                assert_eq!(state.timeout(app.0, 0.12), 0);
                assert_eq!(state.trust(), 1);
                assert_eq!(text(&attr(&state, 1, "AXRole")), "AXApplication");
                assert_eq!(text(&attr(&state, 2, "AXTitle")), "Owned Window");
                assert_eq!(text(&attr(&state, 3, "AXTitle")), "Owned Minimized");
                assert!(boolean(&attr(&state, 7, "AXEnabled")));
                assert!(!boolean(&attr(&state, 8, "AXEnabled")));
                assert!(boolean(&attr(&state, 3, "AXMinimized")));
                assert!(!boolean(&attr(&state, 2, "AXMinimized")));
                let windows = attr(&state, 1, "AXWindows");
                assert_eq!(CFGetTypeID(windows.0), CFArrayGetTypeID());
                assert_eq!(CFArrayGetCount(windows.0), 2);
                assert_ne!(
                    CFEqual(CFArrayGetValueAtIndex(windows.0, 0), state.node(2)),
                    0
                );
                assert_ne!(
                    CFEqual(CFArrayGetValueAtIndex(windows.0, 1), state.node(3)),
                    0
                );
                let k = key("AXWindows");
                let k = (&*k as *const NSString as *mut NSString).cast();
                let mut count = -1;
                assert_eq!(state.count(app.0, k, &mut count), 0);
                assert_eq!(count, 2);
                let mut slice = std::ptr::null_mut();
                assert_eq!(state.values(app.0, k, 1, 5, &mut slice), 0);
                let slice = CF(slice);
                drop(windows);
                assert_eq!(CFArrayGetCount(slice.0), 1);
                assert_ne!(
                    CFEqual(CFArrayGetValueAtIndex(slice.0, 0), state.node(3)),
                    0
                );
                assert_ne!(CFEqual(attr(&state, 1, "AXMenuBar").0, state.node(4)), 0);
                assert_ne!(
                    CFEqual(attr(&state, 1, "AXFocusedWindow").0, state.node(2)),
                    0
                );
                for (index, children) in [(4, vec![5]), (5, vec![6]), (6, vec![7, 8]), (7, vec![])]
                {
                    let array = attr(&state, index, "AXChildren");
                    assert_eq!(CFArrayGetCount(array.0), children.len() as isize);
                    for (i, n) in children.into_iter().enumerate() {
                        assert_ne!(
                            CFEqual(CFArrayGetValueAtIndex(array.0, i as isize), state.node(n)),
                            0
                        );
                    }
                }
                assert_eq!(text(&attr(&state, 7, "AXMenuItemCmdChar")), "o");
                let number = attr(&state, 7, "AXMenuItemCmdModifiers");
                let zero: i32 = objc2::msg_send![number.0.cast::<AnyObject>(), intValue];
                assert_eq!(zero, 0);
                let position = attr(&state, 2, "AXPosition");
                let dimensions = attr(&state, 2, "AXSize");
                for (value, kind) in [(&position, 1), (&dimensions, 2)] {
                    assert_eq!(CFGetTypeID(value.0), AXValueGetTypeID());
                    assert_eq!(AXValueGetType(value.0), kind);
                }
                let mut point = NSPoint::new(0., 0.);
                let mut size = NSSize::new(0., 0.);
                assert_ne!(
                    AXValueGetValue(position.0, 1, (&mut point as *mut NSPoint).cast()),
                    0
                );
                assert_ne!(
                    AXValueGetValue(dimensions.0, 2, (&mut size as *mut NSSize).cast()),
                    0
                );
                assert_eq!(
                    (point.x, point.y, size.width, size.height),
                    (40., 50., 600., 400.)
                );
                assert_eq!(state.data.lock().unwrap().effects, 0);
            }
            "boundaries" => {
                let unknown = key("AXUnknown");
                let unknown = (&*unknown as *const NSString as *mut NSString).cast();
                let sentinel = std::ptr::NonNull::<u8>::dangling().as_ptr().cast();
                let mut out = sentinel;
                assert_eq!(state.attr(state.node(1), unknown, &mut out), UNSUPPORTED);
                assert!(out.is_null());
                let mut count = -777;
                assert_eq!(state.count(state.node(1), unknown, &mut count), UNSUPPORTED);
                assert_eq!(count, -777);
                out = sentinel;
                assert_eq!(
                    state.values(state.node(1), unknown, 0, 1, &mut out),
                    UNSUPPORTED
                );
                assert_eq!(out, sentinel);
                state.set_mode("denied");
                assert_eq!(state.trust(), 0);
                state.set_mode("renamed");
                assert_eq!(text(&attr(&state, 2, "AXTitle")), "Changed");
                state.set_mode("restored");
                assert!(!boolean(&attr(&state, 3, "AXMinimized")));
                state.set_mode("frame-denied");
                for name in ["AXPosition", "AXSize"] {
                    let key = key(name);
                    out = sentinel;
                    assert_eq!(
                        state.attr(
                            state.node(2),
                            (&*key as *const NSString as *mut NSString).cast(),
                            &mut out
                        ),
                        UNSUPPORTED
                    );
                    assert!(out.is_null());
                }
                assert_eq!(state.data.lock().unwrap().effects, 0);
            }
            "write-normal" => {
                write_frame(&state, 0);
                assert_eq!(frame(&state), (84., 93., 500., 400.));
                assert_eq!(state.data.lock().unwrap().effects, 2);
                let key = key("AXMinimized");
                let value = objc2_foundation::NSNumber::numberWithBool(false);
                assert_eq!(
                    system_effects::set(
                        &state,
                        state.node(3),
                        (&*key as *const NSString as *mut NSString).cast(),
                        (&*value as *const _ as *mut objc2_foundation::NSNumber).cast()
                    ),
                    0
                );
                assert!(!boolean(&attr(&state, 3, "AXMinimized")));
                assert_eq!(state.data.lock().unwrap().mode, "restored");
                for (index, name) in [(2, "AXRaise"), (7, "AXPress")] {
                    let key = NSString::from_str(name);
                    assert_eq!(
                        system_effects::action(
                            &state,
                            state.node(index),
                            (&*key as *const NSString as *mut NSString).cast()
                        ),
                        0
                    );
                }
                assert_eq!(state.data.lock().unwrap().effects, 5);
            }
            "write-faults" => {
                for (mode, error, width) in [
                    ("frame-constrained", 0, 100.),
                    (
                        "frame-cannot-complete",
                        system_effects::CANNOT_COMPLETE,
                        500.,
                    ),
                ] {
                    state.set_mode(mode);
                    write_frame(&state, error);
                    assert_eq!(frame(&state), (84., 93., width, 400.));
                    assert_eq!(state.data.lock().unwrap().effects, 2);
                }
                state.set_mode("frame-denied");
                let key = key("AXPosition");
                let mut out = std::ptr::NonNull::<u8>::dangling().as_ptr().cast();
                assert_eq!(
                    state.attr(
                        state.node(2),
                        (&*key as *const NSString as *mut NSString).cast(),
                        &mut out
                    ),
                    UNSUPPORTED
                );
                assert!(out.is_null());
                assert_eq!(state.data.lock().unwrap().effects, 0);
            }
            "app-normal" => {
                let apps = system_apps::Apps::new(state.clone());
                let shared = apps.shared();
                assert!(std::ptr::eq(&*shared, &*apps.shared()));
                let running = apps.running();
                assert_eq!(running.count(), 1);
                let bundle = apps.by_bundle(&NSString::from_str("com.apple.Safari"));
                assert_eq!(bundle.count(), 1);
                let front = apps.front();
                let by_pid = apps.by_pid(730001);
                assert!(std::ptr::eq(&*front, &*by_pid));
                assert!(std::ptr::eq(&*front, &*running.objectAtIndex(0)));
                assert!(std::ptr::eq(&*front, &*bundle.objectAtIndex(0)));
                let pid: i32 = objc2::msg_send![&*front, processIdentifier];
                let terminated: objc2::runtime::Bool = objc2::msg_send![&*front, isTerminated];
                let name: objc2::rc::Retained<NSString> = objc2::msg_send![&*front, localizedName];
                let bundle_id: objc2::rc::Retained<NSString> =
                    objc2::msg_send![&*front, bundleIdentifier];
                let url: objc2::rc::Retained<objc2_foundation::NSURL> =
                    objc2::msg_send![&*front, bundleURL];
                let policy: objc2_app_kit::NSApplicationActivationPolicy =
                    objc2::msg_send![&*front, activationPolicy];
                assert_eq!(pid, 730001);
                assert!(!terminated.as_bool());
                assert_eq!(name.to_string(), "Owned Native App");
                assert_eq!(bundle_id.to_string(), "com.apple.Safari");
                assert_eq!(url.path().unwrap().to_string(), "/Applications/Safari.app");
                assert_eq!(
                    policy,
                    objc2_app_kit::NSApplicationActivationPolicy::Regular
                );
                assert_eq!(std::mem::size_of::<libc::proc_bsdinfo>(), 136);
                let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
                assert_eq!(
                    system_apps::proc(
                        &state,
                        730001,
                        libc::PROC_PIDTBSDINFO,
                        0,
                        info.as_mut_ptr().cast(),
                        136
                    ),
                    136
                );
                let info = info.assume_init();
                assert_eq!(
                    (
                        info.pbi_pid,
                        info.pbi_uid,
                        info.pbi_start_tvsec,
                        info.pbi_start_tvusec
                    ),
                    (730001, libc::getuid(), 11, 22)
                );
                assert_eq!(state.data.lock().unwrap().reads, 3);
                assert_eq!(state.data.lock().unwrap().effects, 0);
                // A retained app owns its state independently of the factory.
                let own = std::sync::Arc::new(State::new());
                let weak = std::sync::Arc::downgrade(&own);
                let factory = system_apps::Apps::new(own.clone());
                let retained = factory.by_pid(730001);
                drop(factory);
                drop(own);
                assert!(weak.upgrade().is_some());
                drop(retained);
                assert!(weak.upgrade().is_none());
            }
            "app-modes" => {
                let apps = system_apps::Apps::new(state.clone());
                let mut old = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
                assert_eq!(
                    system_apps::proc(
                        &state,
                        730001,
                        libc::PROC_PIDTBSDINFO,
                        0,
                        old.as_mut_ptr().cast(),
                        136
                    ),
                    136
                );
                let old = old.assume_init();
                state.set_mode("restarted");
                let mut changed = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
                assert_eq!(
                    system_apps::proc(
                        &state,
                        730001,
                        libc::PROC_PIDTBSDINFO,
                        0,
                        changed.as_mut_ptr().cast(),
                        136
                    ),
                    136
                );
                let changed = changed.assume_init();
                assert_eq!(
                    (changed.pbi_start_tvsec, changed.pbi_start_tvusec),
                    (12, 22)
                );
                assert_ne!(old.pbi_start_tvsec, changed.pbi_start_tvsec);
                let mut untouched = [0x5au8; 136];
                for (pid, flavor, arg, size) in [
                    (730002, libc::PROC_PIDTBSDINFO, 0, 136),
                    (730001, 0, 0, 136),
                    (730001, libc::PROC_PIDTBSDINFO, 1, 136),
                    (730001, libc::PROC_PIDTBSDINFO, 0, 135),
                ] {
                    assert_eq!(
                        system_apps::proc(
                            &state,
                            pid,
                            flavor,
                            arg,
                            untouched.as_mut_ptr().cast(),
                            size
                        ),
                        0
                    );
                    assert!(untouched.iter().all(|byte| *byte == 0x5a));
                }
                state.set_mode("mutate-activate");
                let activated: objc2::runtime::Bool = objc2::msg_send![&*apps.app, activateWithOptions:objc2_app_kit::NSApplicationActivationOptions::empty()];
                assert!(activated.as_bool());
                assert_eq!(state.data.lock().unwrap().effects, 1);
                assert_eq!(text(&attr(&state, 2, "AXTitle")), "Changed");
                assert_eq!(state.data.lock().unwrap().mode, "renamed");
            }
            _ => panic!("Unknown owned node probe"),
        }
        let (reads, effects) = {
            let data = state.data.lock().unwrap();
            (data.reads, data.effects)
        };
        println!(
            "{}",
            json!({"ok":true,"mode":mode,"reads":reads,"effects":effects,"completeReady":false})
        );
    });
}
