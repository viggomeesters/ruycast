//! Component proof only. No provider constructor, interpose, discovery or readiness.
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
fn main() {
    autoreleasepool(|_| unsafe {
        let state = State::new();
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
