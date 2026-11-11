//! Actual AppKit monitor calls under the loaded synthetic provider, no event posting.
use block2::RcBlock;
use objc2::{msg_send, rc::autoreleasepool, runtime::AnyObject, ClassType, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventMask};
use objc2_foundation::NSObject;
use serde_json::json;
use std::{
    cell::Cell,
    path::PathBuf,
    ptr::NonNull,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
unsafe fn symbol<T: Copy>(name: &std::ffi::CStr) -> T {
    let pointer = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr());
    assert!(!pointer.is_null());
    std::mem::transmute_copy(&pointer)
}
unsafe fn counts() -> [u64; 4] {
    let read: unsafe extern "C" fn(*mut u64, *mut u64, *mut u64, *mut u64) =
        symbol(c"ruycast_choice_fixture_monitor_counts");
    let mut values = [0; 4];
    let [a, b, c, d] = &mut values;
    read(a, b, c, d);
    values
}
struct Life(Arc<AtomicU64>);
impl Drop for Life {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
fn main() {
    autoreleasepool(|_| unsafe { run() });
}
unsafe fn run() {
    let main = MainThreadMarker::new().unwrap();
    let ready: unsafe extern "C" fn() -> bool = symbol(c"ruycast_choice_fixture_monitor_guard");
    assert!(ready());
    let app = NSApplication::sharedApplication(main);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let mode = std::env::args().nth(1).unwrap();
    eprintln!("owned-choice-boundary:{mode}");
    let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown;
    let click: unsafe extern "C" fn() -> u64 = symbol(c"ruycast_choice_fixture_click");
    match mode.as_str() {
        "basic" => {
            let lives = Arc::new(AtomicU64::new(1));
            let life = Life(lives.clone());
            let calls = Rc::new(Cell::new(0));
            let called = calls.clone();
            let block = RcBlock::new(move |_: NonNull<NSEvent>| {
                let _ = &life;
                called.set(called.get() + 1);
            });
            let token =
                NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &block).unwrap();
            drop(block);
            assert_eq!(lives.load(Ordering::SeqCst), 1);
            assert_eq!(counts(), [1, 0, 0, 1]);
            assert_eq!(click(), 1);
            assert_eq!(calls.get(), 1);
            assert_eq!(counts(), [1, 0, 1, 1]);
            NSEvent::removeMonitor(&token);
            assert_eq!(lives.load(Ordering::SeqCst), 0);
            assert_eq!(counts(), [1, 1, 1, 0]);
            assert_eq!(click(), 0);
        }
        "local" => {
            let block = RcBlock::new(|event: NonNull<NSEvent>| event.as_ptr());
            let local =
                NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block).unwrap();
            assert_eq!(local.class().name(), c"_NSLocalEventObserver");
            NSEvent::removeMonitor(&local);
            assert_eq!(counts(), [0, 0, 0, 0]);
            // The original accepts nil AppKit startup removals without a hook.
            let _: () = msg_send![NSEvent::class(),removeMonitor:std::ptr::null_mut::<AnyObject>()];
        }
        "snapshot" => {
            let other = Rc::new(Cell::new(std::ptr::null_mut::<AnyObject>()));
            let selected = other.clone();
            let once = Rc::new(Cell::new(false));
            let first = RcBlock::new(move |_: NonNull<NSEvent>| {
                if !once.replace(true) {
                    NSEvent::removeMonitor(&*selected.get());
                }
            });
            let first =
                NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &first).unwrap();
            let calls = Rc::new(Cell::new(0));
            let called = calls.clone();
            let second = RcBlock::new(move |_: NonNull<NSEvent>| called.set(called.get() + 1));
            let second =
                NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &second).unwrap();
            other.set(&*second as *const AnyObject as *mut AnyObject);
            assert_eq!(click(), 1);
            assert_eq!(calls.get(), 0);
            assert_eq!(counts(), [2, 1, 1, 1]);
            NSEvent::removeMonitor(&first);
            assert_eq!(counts(), [2, 2, 1, 0]);
        }
        "held" => {
            let home = PathBuf::from(std::env::var_os("RUYCAST_KEYWORD_FIXTURE_HOME").unwrap());
            std::fs::write(
                home.join(".ruycast-keyboard-fixture"),
                b"own-keyboard-fixture-v1",
            )
            .unwrap();
            std::fs::write(home.join("hold-choice-remove"), b"").unwrap();
            let path = home.clone();
            let block = RcBlock::new(move |_: NonNull<NSEvent>| {
                std::fs::write(path.join("click-seen"), b"nested").unwrap();
            });
            let token =
                NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &block).unwrap();
            drop(block);
            let path = home.clone();
            let release = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(3);
                while !(path.join("choice-remove-entered").is_file()
                    && path.join("click-seen").is_file())
                    && Instant::now() < deadline
                {
                    std::thread::sleep(Duration::from_millis(1));
                }
                let nested = path.join("click-seen").is_file();
                std::fs::write(path.join("choice-remove-release"), b"").unwrap();
                nested
            });
            let schedule: unsafe extern "C" fn() = symbol(c"ruycast_choice_fixture_schedule_click");
            schedule();
            NSEvent::removeMonitor(&token);
            assert!(release.join().unwrap());
            assert_eq!(counts(), [1, 1, 1, 0]);
        }
        "bad-mask" => {
            let block = RcBlock::new(|_: NonNull<NSEvent>| {});
            let _ = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                NSEventMask::empty(),
                &block,
            );
            libc::_exit(0); // Unexpected acceptance must make the negative test fail.
        }
        "null-handler" => {
            let _: *mut AnyObject = msg_send![NSEvent::class(),addGlobalMonitorForEventsMatchingMask:mask,handler:std::ptr::null_mut::<std::ffi::c_void>()];
            libc::_exit(0);
        }
        "unknown-token" => {
            let token = NSObject::new();
            NSEvent::removeMonitor(&token);
            libc::_exit(0);
        }
        "live-exit" => {
            let block = RcBlock::new(|_: NonNull<NSEvent>| {});
            let _ = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &block).unwrap();
        }
        _ => panic!("Unknown owned probe mode"),
    }
    assert!(!app.isActive());
    assert_eq!(app.windows().count(), 0);
    println!(
        "{}",
        json!({"ok":true,"mode":mode,"counts":counts(),"ownApplicationActive":app.isActive(),"ownWindowCount":app.windows().count()})
    );
}
