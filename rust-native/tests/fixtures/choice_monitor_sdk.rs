//! Process-local synthetic global monitors, with the original native local observers.
use block2::{Block, RcBlock};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Imp, Sel},
    sel, ClassType,
};
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSObject, NSPoint, NSRunLoop, NSString};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        LazyLock,
    },
    time::{Duration, Instant},
};
type Handler = Block<dyn Fn(*mut NSEvent)>;
type OwnedHandler = RcBlock<dyn Fn(*mut NSEvent)>;
type LocalHandler = Block<dyn Fn(*mut NSEvent) -> *mut NSEvent>;
type Add =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, NSEventMask, *mut Handler) -> *mut AnyObject;
type LocalAdd = unsafe extern "C-unwind" fn(
    *mut AnyObject,
    Sel,
    NSEventMask,
    *mut LocalHandler,
) -> *mut AnyObject;
type Remove = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
struct Monitor {
    token: Retained<AnyObject>,
    handler: RefCell<Option<OwnedHandler>>,
}
struct State {
    monitors: Vec<Rc<Monitor>>,
    locals: Vec<Retained<AnyObject>>,
    native_local: LocalAdd,
    native_remove: Remove,
    created: u64,
    removed: u64,
    delivered: u64,
}
struct MainState(RefCell<Option<State>>);
// Every access goes through state(), which checks main before exposing the cell.
// No reference or borrow is held over Objective-C callbacks or nested runloops.
unsafe impl Sync for MainState {}
static STATE: MainState = MainState(RefCell::new(None));
static INSTALLED: AtomicBool = AtomicBool::new(false);
fn state() -> &'static RefCell<Option<State>> {
    assert!(unsafe { libc::pthread_main_np() != 0 });
    &STATE.0
}
static MONITOR_CLASS: LazyLock<&'static AnyClass> = LazyLock::new(|| {
    ClassBuilder::new(c"RuycastChoiceMonitor", NSObject::class())
        .unwrap()
        .register()
});
unsafe extern "C-unwind" fn own_local(
    object: *mut AnyObject,
    selector: Sel,
    mask: NSEventMask,
    handler: *mut LocalHandler,
) -> *mut AnyObject {
    let native = state().borrow().as_ref().unwrap().native_local;
    let token = native(object, selector, mask, handler);
    if let Some(retained) = Retained::retain(token) {
        state().borrow_mut().as_mut().unwrap().locals.push(retained);
    }
    token
}
unsafe extern "C-unwind" fn own_monitor(
    _: *mut AnyObject,
    _: Sel,
    mask: NSEventMask,
    handler: *mut Handler,
) -> *mut AnyObject {
    assert!(
        INSTALLED.load(Ordering::Acquire)
            && super::choice_board_sdk::ruycast_choice_fixture_guard()
    );
    assert!(
        !handler.is_null() && mask == (NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown)
    );
    let token: Retained<AnyObject> = msg_send![*MONITOR_CLASS, new];
    let result = &*token as *const AnyObject as *mut AnyObject;
    let monitor = Rc::new(Monitor {
        token,
        handler: RefCell::new(Some(RcBlock::copy(handler).unwrap())),
    });
    let mut slot = state().borrow_mut();
    let own = slot.as_mut().unwrap();
    own.monitors.push(monitor);
    own.created += 1;
    result
}
unsafe fn hold_removal() {
    let Some(root) = std::env::var_os("RUYCAST_KEYWORD_FIXTURE_HOME").map(std::path::PathBuf::from)
    else {
        return;
    };
    if !root.join("hold-choice-remove").exists() {
        return;
    }
    assert_eq!(
        std::fs::read(root.join(".ruycast-keyboard-fixture")).unwrap(),
        b"own-keyboard-fixture-v1"
    );
    let stamp = NSString::from_str(root.join("choice-remove-entered").to_str().unwrap());
    let _: Bool = msg_send![&*NSString::from_str("entered"), writeToFile:&*stamp, atomically:Bool::YES, encoding:4usize, error:std::ptr::null_mut::<*mut AnyObject>()];
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.join("choice-remove-release").exists() && Instant::now() < deadline {
        let _: Bool = msg_send![&*NSRunLoop::currentRunLoop(), runMode:NSDefaultRunLoopMode, beforeDate:&*NSDate::dateWithTimeIntervalSinceNow(0.002)];
        std::thread::sleep(Duration::from_millis(1));
    }
}
unsafe extern "C-unwind" fn own_remove(
    object: *mut AnyObject,
    selector: Sel,
    token: *mut AnyObject,
) {
    if token.is_null() {
        return;
    }
    let (local, native, monitor) = {
        let slot = state().borrow();
        let own = slot.as_ref().unwrap();
        (
            own.locals.iter().any(|value| std::ptr::eq(&**value, token)),
            own.native_remove,
            own.monitors
                .iter()
                .find(|value| std::ptr::eq(&*value.token, token))
                .cloned(),
        )
    };
    if local {
        native(object, selector, token);
        let removed = {
            let mut slot = state().borrow_mut();
            let own = slot.as_mut().unwrap();
            own.locals
                .iter()
                .position(|value| std::ptr::eq(&**value, token))
                .map(|index| own.locals.remove(index))
        };
        drop(removed);
        return;
    }
    // Preserve AppKit's observed private, process-local observer retirement.
    if (*token).class().name() == c"_NSLocalEventObserver" {
        native(object, selector, token);
        return;
    }
    let is_monitor: Bool = msg_send![token, isKindOfClass:*MONITOR_CLASS];
    assert!(is_monitor.as_bool());
    let monitor = monitor.expect("Only an active owned global monitor may be removed");
    hold_removal();
    let handler = monitor.handler.borrow_mut().take();
    let removed = {
        let mut slot = state().borrow_mut();
        let own = slot.as_mut().unwrap();
        let removed = own
            .monitors
            .iter()
            .position(|value| std::ptr::eq(&*value.token, token))
            .map(|index| own.monitors.remove(index));
        own.removed += 1;
        removed
    };
    drop(handler);
    drop(removed);
}
#[no_mangle]
extern "C" fn ruycast_choice_fixture_monitor_guard() -> bool {
    INSTALLED.load(Ordering::Acquire) && super::choice_board_sdk::ruycast_choice_fixture_guard()
}
#[no_mangle]
unsafe extern "C" fn ruycast_choice_fixture_click() -> u64 {
    assert!(ruycast_choice_fixture_monitor_guard());
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(NSEventType::LeftMouseDown, NSPoint::new(0.,0.), NSEventModifierFlags::empty(), 0.,0,None,0,1,0.).unwrap();
    let monitors = state().borrow().as_ref().unwrap().monitors.clone();
    let mut delivered = 0;
    for monitor in monitors {
        let handler = monitor.handler.borrow().clone();
        if let Some(handler) = handler {
            handler.call((&*event as *const NSEvent as *mut NSEvent,));
            delivered += 1;
            state().borrow_mut().as_mut().unwrap().delivered += 1;
        }
    }
    delivered
}
#[no_mangle]
unsafe extern "C" fn ruycast_choice_fixture_schedule_click() {
    assert!(ruycast_choice_fixture_monitor_guard());
    let handler = RcBlock::new(|_: *mut AnyObject| {
        ruycast_choice_fixture_click();
    });
    let _: *mut AnyObject = msg_send![AnyClass::get(c"NSTimer").unwrap(), scheduledTimerWithTimeInterval:0.01f64, repeats:Bool::NO, block:&*handler];
}
#[no_mangle]
unsafe extern "C" fn ruycast_choice_fixture_monitor_counts(
    created: *mut u64,
    removed: *mut u64,
    delivered: *mut u64,
    active: *mut u64,
) {
    assert!(!created.is_null() && !removed.is_null() && !delivered.is_null() && !active.is_null());
    let slot = state().borrow();
    let own = slot.as_ref().unwrap();
    *created = own.created;
    *removed = own.removed;
    *delivered = own.delivered;
    *active = own.monitors.len() as u64;
}
pub(super) unsafe fn install() {
    assert!(super::choice_board_sdk::ruycast_choice_fixture_guard());
    let class = NSEvent::class();
    let _: *const AnyClass = msg_send![class, class];
    let add = class
        .class_method(sel!(addGlobalMonitorForEventsMatchingMask:handler:))
        .unwrap();
    let remove = class.class_method(sel!(removeMonitor:)).unwrap();
    let local = class
        .class_method(sel!(addLocalMonitorForEventsMatchingMask:handler:))
        .unwrap();
    assert!(state().borrow().is_none());
    *state().borrow_mut() = Some(State {
        monitors: vec![],
        locals: vec![],
        native_local: std::mem::transmute::<Imp, LocalAdd>(local.implementation()),
        native_remove: std::mem::transmute::<Imp, Remove>(remove.implementation()),
        created: 0,
        removed: 0,
        delivered: 0,
    });
    local.set_implementation(std::mem::transmute::<LocalAdd, Imp>(own_local));
    add.set_implementation(std::mem::transmute::<Add, Imp>(own_monitor));
    remove.set_implementation(std::mem::transmute::<Remove, Imp>(own_remove));
    INSTALLED.store(true, Ordering::Release);
}
pub(super) unsafe fn cleanup() {
    assert!(
        state().borrow().as_ref().unwrap().monitors.is_empty(),
        "Every owned global monitor must retire before SDK cleanup"
    );
    INSTALLED.store(false, Ordering::Release);
    let locals = std::mem::take(&mut state().borrow_mut().as_mut().unwrap().locals);
    drop(locals);
}
