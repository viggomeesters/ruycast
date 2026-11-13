//! Main-owned Auto Quit monitor. Hidden test hosts never attach real observers,
//! query user applications or send termination requests.
use objc2::{rc::Retained, runtime::ProtocolObject, MainThreadMarker};
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidWakeNotification,
    NSWorkspaceSessionDidBecomeActiveNotification, NSWorkspaceSessionDidResignActiveNotification,
    NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue};
use ruycast_native::{
    auto_quit::{native::Native, runtime::Runtime},
    profile::ProfilePaths,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ptr::NonNull,
    rc::Rc,
    sync::Arc,
};
type Wake = Arc<dyn Fn() + Send + Sync>;
struct State {
    closed: Cell<bool>,
    paused: Cell<u8>,
    epoch: Cell<u64>,
    expected: Cell<u64>,
    due: Cell<bool>,
    reset: Cell<bool>,
    applications: RefCell<HashSet<String>>,
    wake: RefCell<Option<Wake>>,
}
impl State {
    fn notify(&self) {
        if let Some(epoch) = self.epoch.get().checked_add(1) {
            self.epoch.set(epoch);
        } else {
            self.closed.set(true);
        }
        self.due.set(true);
        if let Some(wake) = self.wake.borrow().as_ref() {
            wake();
        }
    }
}
pub struct Monitor {
    main: MainThreadMarker,
    state: Rc<State>,
    runtime: Runtime,
    native: Native,
    center: Retained<NSNotificationCenter>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    timer: Option<crate::main_loop::Timer>,
}
impl Monitor {
    pub fn new(main: MainThreadMarker, paths: ProfilePaths) -> Self {
        let state = Rc::new(State {
            closed: Cell::new(false),
            paused: Cell::new(0),
            epoch: Cell::new(0),
            expected: Cell::new(0),
            due: Cell::new(false),
            reset: Cell::new(false),
            applications: RefCell::new(HashSet::new()),
            wake: RefCell::new(None),
        });
        let weak = Rc::downgrade(&state);
        let native = Native::new(
            main,
            paths,
            Rc::new(move || {
                weak.upgrade().is_some_and(|s| {
                    !s.closed.get() && s.paused.get() == 0 && s.epoch.get() == s.expected.get()
                })
            }),
        );
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let mut observers = vec![];
        let events = unsafe {
            [
                (NSWorkspaceWillSleepNotification, 1, true),
                (NSWorkspaceDidWakeNotification, 1, false),
                (NSWorkspaceSessionDidResignActiveNotification, 2, true),
                (NSWorkspaceSessionDidBecomeActiveNotification, 2, false),
                (NSWorkspaceDidActivateApplicationNotification, 0, false),
            ]
        };
        for (name, reason, paused) in events {
            let weak = Rc::downgrade(&state);
            let block = block2::RcBlock::new(move |notification: NonNull<NSNotification>| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                if state.closed.get() {
                    return;
                }
                if reason != 0 {
                    state.paused.set(if paused {
                        state.paused.get() | reason
                    } else {
                        state.paused.get() & !reason
                    });
                    // Both edges reset, even when sleep and wake arrive before
                    // the next tick. Session-lock time never expires a timer.
                    state.reset.set(true);
                } else {
                    let app = unsafe { notification.as_ref() }
                        .userInfo()
                        .and_then(|info| info.objectForKey(unsafe { NSWorkspaceApplicationKey }))
                        .and_then(|value| value.downcast::<NSRunningApplication>().ok())
                        .and_then(|app| app.bundleIdentifier());
                    if let Some(application) = app {
                        state
                            .applications
                            .borrow_mut()
                            .insert(application.to_string());
                    } else {
                        state.reset.set(true);
                    }
                }
                state.notify();
            });
            observers.push(unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(name),
                    None,
                    Some(&NSOperationQueue::mainQueue()),
                    &block,
                )
            });
        }
        Self {
            main,
            state,
            runtime: Runtime::default(),
            native,
            center,
            observers,
            timer: None,
        }
    }
    pub fn start(&mut self, wake: Wake) -> Result<(), String> {
        *self.state.wake.borrow_mut() = Some(wake);
        self.state.due.set(true);
        self.arm()
    }
    fn arm(&mut self) -> Result<(), String> {
        if self.state.closed.get() {
            return Ok(());
        }
        let weak = Rc::downgrade(&self.state);
        self.timer = Some(crate::main_loop::Timer::new_common(
            self.main,
            2.0,
            move || {
                if let Some(state) = weak.upgrade() {
                    state.due.set(true);
                    if let Some(wake) = state.wake.borrow().as_ref() {
                        wake();
                    }
                }
            },
        )?);
        Ok(())
    }
    pub fn pump(&mut self) -> Result<(), String> {
        if !self.state.due.replace(false) || self.state.closed.get() {
            return Ok(());
        }
        self.timer.take();
        if self.state.reset.replace(false) {
            self.runtime.reset();
        }
        for application in self.state.applications.borrow_mut().drain() {
            self.runtime.reset_application(&application);
        }
        self.state.expected.set(self.state.epoch.get());
        if self.state.paused.get() != 0 {
            self.runtime.reset();
        } else {
            // Missing/invalid/transitioning config fails closed and retires the
            // intervals. It is retried on the next bounded main-loop tick.
            let _ = self.runtime.tick(&mut self.native);
        }
        self.arm()
    }
    pub fn stop(&mut self) {
        self.state.closed.set(true);
        self.timer.take();
        self.state.wake.borrow_mut().take();
        self.runtime.reset();
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop();
        for observer in self.observers.drain(..) {
            unsafe {
                self.center.removeObserver(
                    std::convert::AsRef::<objc2::runtime::AnyObject>::as_ref(&*observer),
                );
            }
        }
    }
}
