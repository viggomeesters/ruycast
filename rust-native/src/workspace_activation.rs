//! Main-owned, per-notification activation delivery. The retained typed app is
//! the notification's original instance, never a later foreground/PID lookup.
//! The sink runs before the owner's coalesced refresh. No app data is logged.
use objc2::{rc::Retained, runtime::ProtocolObject, MainThreadMarker};
use objc2_app_kit::{
    NSRunningApplication, NSWorkspaceApplicationKey, NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue};
use std::{cell::Cell, panic::AssertUnwindSafe, ptr::NonNull, rc::Rc};

/// Native notification payloads stay on main, despite the SDK app being Send.
/// ```compile_fail
/// fn send<T: Send>() {}
/// send::<ruycast_native::workspace_activation::Event>();
/// ```
/// ```compile_fail
/// fn sync<T: Sync>() {}
/// sync::<ruycast_native::workspace_activation::Event>();
/// ```
pub struct Event {
    /// None means that the sequence has exhausted; it can never authorize work.
    pub sequence: Option<u64>,
    /// Missing or malformed userInfo stays Unknown, rather than sampling front.
    application: Option<Retained<NSRunningApplication>>,
    _main: MainThreadMarker,
}
impl Event {
    pub fn application(&self) -> Option<&NSRunningApplication> {
        self.application.as_deref()
    }
}
pub type Sink = Rc<dyn Fn(Event)>;
struct State {
    closed: Cell<bool>,
    ingress: Rc<dyn Fn() -> Option<u64>>,
    sink: Sink,
    failed: Rc<dyn Fn()>,
    _main: MainThreadMarker,
}
pub struct Watch {
    state: Rc<State>,
    center: Retained<NSNotificationCenter>,
    observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}
impl Watch {
    pub fn new(
        main: MainThreadMarker,
        center: Retained<NSNotificationCenter>,
        ingress: Rc<dyn Fn() -> Option<u64>>,
        sink: Sink,
        failed: Rc<dyn Fn()>,
    ) -> Self {
        let state = Rc::new(State {
            closed: Cell::new(false),
            ingress,
            sink,
            failed,
            _main: main,
        });
        let weak = Rc::downgrade(&state);
        let block = block2::RcBlock::new(move |notification: NonNull<NSNotification>| {
            // Hold the receiver across native reads and sink calls, including a
            // sink which destroys the Watch and removes this very observer.
            let Some(state) = weak.upgrade() else { return };
            if state.closed.get() {
                return;
            }
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                // Revoke old scalar leases before any possibly reentrant SDK
                // call. Do not reload this sequence after a nested notification.
                let sequence = (state.ingress)();
                if state.closed.get() {
                    return;
                }
                let application = unsafe { notification.as_ref() }
                    .userInfo()
                    .and_then(|info| info.objectForKey(unsafe { NSWorkspaceApplicationKey }))
                    .and_then(|value| value.downcast::<NSRunningApplication>().ok());
                if !state.closed.get() {
                    (state.sink)(Event {
                        sequence,
                        application,
                        _main: state._main,
                    });
                }
            }));
            if outcome.is_err() && !state.closed.replace(true) {
                // No panic is allowed through the Objective-C block boundary.
                let _ = std::panic::catch_unwind(AssertUnwindSafe(|| (state.failed)()));
            }
        });
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidActivateApplicationNotification),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
        };
        Self {
            state,
            center,
            observer,
        }
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.state.closed.set(true);
        unsafe {
            self.center
                .removeObserver(std::convert::AsRef::<objc2::runtime::AnyObject>::as_ref(
                    &*self.observer,
                ));
        }
    }
}

/// The private center posts notifications synchronously on main. The only
/// typed app is this probe process itself; no foreign app discovery/activation.
#[cfg(feature = "test-tools")]
pub fn fixture(case: &serde_json::Value) -> Result<serde_json::Value, String> {
    use objc2::{runtime::AnyObject, ClassType};
    use objc2_foundation::{NSDictionary, NSString};
    use serde_json::json;
    use std::cell::RefCell;
    let main = MainThreadMarker::new().ok_or("Own main fixture required")?;
    let center = NSNotificationCenter::new();
    let watch = Rc::new(RefCell::new(None::<Watch>));
    let events = Rc::new(RefCell::new(vec![]));
    let failed = Rc::new(Cell::new(0));
    let sequence = Rc::new(Cell::new(case["sequence"].as_u64().unwrap_or(0)));
    let ingress_once = Rc::new(Cell::new(false));
    let sink_once = Rc::new(Cell::new(false));
    let ingress_mode = case["ingress"].as_str().unwrap_or("").to_owned();
    let sink_mode = case["sink"].as_str().unwrap_or("").to_owned();
    let ingress_watch = Rc::downgrade(&watch);
    let sink_watch = Rc::downgrade(&watch);
    let ingress_center = center.clone();
    let sink_center = center.clone();
    let result_events = events.clone();
    let result_failed = failed.clone();
    let result_sequence = sequence.clone();
    let own = NSRunningApplication::currentApplication();
    let expected = own.clone();
    let value = Watch::new(
        main,
        center.clone(),
        Rc::new(move || {
            let next = sequence.get().checked_add(1);
            if let Some(next) = next {
                sequence.set(next);
            }
            if !ingress_once.replace(true) {
                match ingress_mode.as_str() {
                    "nested" => unsafe {
                        ingress_center.postNotificationName_object(
                            NSWorkspaceDidActivateApplicationNotification,
                            None,
                        )
                    },
                    "drop" => {
                        let old = ingress_watch
                            .upgrade()
                            .and_then(|slot| slot.borrow_mut().take());
                        drop(old);
                    }
                    "panic" => panic!("Own ingress panic"),
                    _ => {}
                }
            }
            next
        }),
        Rc::new(move |event| {
            // A native equality call is outside all RefCell borrows. No PID used.
            let typed = event.application().is_some();
            let own = event
                .application()
                .is_some_and(|app| app.isEqual(Some(&*expected)));
            events
                .borrow_mut()
                .push(json!({"sequence":event.sequence,"typed":typed,"ownInstance":own}));
            if !sink_once.replace(true) {
                match sink_mode.as_str() {
                    "nested" => unsafe {
                        sink_center.postNotificationName_object(
                            NSWorkspaceDidActivateApplicationNotification,
                            None,
                        )
                    },
                    "drop" => {
                        let old = sink_watch
                            .upgrade()
                            .and_then(|slot| slot.borrow_mut().take());
                        drop(old);
                    }
                    "panic" => panic!("Own sink panic"),
                    _ => {}
                }
            }
        }),
        Rc::new(move || failed.set(failed.get() + 1)),
    );
    *watch.borrow_mut() = Some(value);
    let values = case["events"].as_array().ok_or("Own events required")?;
    for value in values {
        let malformed = NSString::from_str("OWN-MALFORMED-NEVER-LOG");
        let object: Option<&AnyObject> = match value.as_str() {
            Some("typed") => Some(own.as_super().as_ref()),
            Some("malformed") => Some(malformed.as_ref()),
            Some("missing") => None,
            _ => return Err("Unknown own notification".into()),
        };
        let info = object.map(|object| {
            NSDictionary::from_slices(&[unsafe { NSWorkspaceApplicationKey }], &[object])
        });
        unsafe {
            center.postNotificationName_object_userInfo(
                NSWorkspaceDidActivateApplicationNotification,
                None,
                info.as_deref().map(|info| info.cast_unchecked()),
            )
        };
    }
    let before_close = result_events.borrow().len();
    let old = watch.borrow_mut().take();
    drop(old);
    unsafe {
        center.postNotificationName_object(NSWorkspaceDidActivateApplicationNotification, None)
    };
    let result = json!({"events":*result_events.borrow(),"failed":result_failed.get(),"sequence":result_sequence.get(),"beforeClose":before_close,"afterClose":result_events.borrow().len()});
    Ok(result)
}
