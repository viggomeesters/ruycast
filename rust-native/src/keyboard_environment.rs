//! One main-owned notification center subscription set and coalesced timers.
//! Observer blocks touch only shared scalar state, never the registry/HID owner.
use objc2::{rc::Retained, runtime::ProtocolObject, MainThreadMarker};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidLaunchApplicationNotification,
    NSWorkspaceDidTerminateApplicationNotification, NSWorkspaceDidWakeNotification,
    NSWorkspaceSessionDidBecomeActiveNotification, NSWorkspaceSessionDidResignActiveNotification,
    NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue};
use ruycast_native::keyboard_service::Client;
use std::{
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
};

pub(crate) struct State {
    client: Client,
    pause_reasons: AtomicU8,
    dirty: AtomicBool,
    closed: AtomicBool,
}
impl State {
    pub fn request_refresh(&self) {
        if !self.closed.load(Ordering::Acquire) {
            self.dirty.store(true, Ordering::Release);
            self.client.wake_main();
        }
    }
    pub fn take_refresh(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }
    pub fn refresh_pending(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }
}
pub(crate) struct Environment {
    state: Arc<State>,
    center: Retained<NSNotificationCenter>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    timer: Option<crate::main_loop::Timer>,
    activation: Option<ruycast_native::workspace_activation::Watch>,
    retry: Option<crate::main_loop::Timer>,
    tick: Arc<AtomicBool>,
    main: MainThreadMarker,
}
impl Environment {
    pub fn new(
        main: MainThreadMarker,
        fixture: bool,
        client: Client,
        activated: ruycast_native::workspace_activation::Sink,
    ) -> Self {
        let state = Arc::new(State {
            client,
            pause_reasons: AtomicU8::new(0),
            dirty: AtomicBool::new(true),
            closed: AtomicBool::new(false),
        });
        let center = if fixture {
            NSNotificationCenter::new()
        } else {
            NSWorkspace::sharedWorkspace().notificationCenter()
        };
        let mut observers = Vec::new();
        let notifications = unsafe {
            [
                (NSWorkspaceWillSleepNotification, Some((1, true))),
                (
                    NSWorkspaceSessionDidResignActiveNotification,
                    Some((2, true)),
                ),
                (NSWorkspaceDidWakeNotification, Some((1, false))),
                (
                    NSWorkspaceSessionDidBecomeActiveNotification,
                    Some((2, false)),
                ),
                (NSWorkspaceDidLaunchApplicationNotification, None),
                (NSWorkspaceDidTerminateApplicationNotification, None),
            ]
        };
        for (name, paused) in notifications {
            let weak = Arc::downgrade(&state);
            let block = block2::RcBlock::new(move |_: NonNull<NSNotification>| {
                if let Some(state) = weak.upgrade() {
                    if state.closed.load(Ordering::Acquire) {
                        return;
                    }
                    if let Some((reason, paused)) = paused {
                        // These blocks are delivered serially on main. A wake
                        // releases sleep only; session-active releases its own
                        // reason. Duplicates and app notifications cannot resume
                        // hooks while the other reason is still held.
                        if paused {
                            state.pause_reasons.fetch_or(reason, Ordering::AcqRel);
                        } else {
                            state.pause_reasons.fetch_and(!reason, Ordering::AcqRel);
                        }
                        state
                            .client
                            .set_paused(state.pause_reasons.load(Ordering::Acquire) != 0);
                    }
                    state.request_refresh();
                }
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
        let ingress_state = state.clone();
        let sink_state = state.clone();
        let failed_state = state.clone();
        let activation = ruycast_native::workspace_activation::Watch::new(
            main,
            center.clone(),
            std::rc::Rc::new(move || ingress_state.client.note_external_activation()),
            std::rc::Rc::new(move |event| {
                activated(event);
                sink_state.request_refresh();
            }),
            std::rc::Rc::new(move || {
                failed_state.client.stop();
                failed_state.request_refresh();
            }),
        );
        #[cfg(feature = "test-tools")]
        if fixture {
            FIXTURE.with(|slot| *slot.borrow_mut() = Some(objc2::rc::Weak::new(&*center)));
        }
        Self {
            state,
            center,
            observers,
            timer: None,
            activation: Some(activation),
            retry: None,
            tick: Arc::new(AtomicBool::new(false)),
            main,
        }
    }
    pub fn state(&self) -> Arc<State> {
        self.state.clone()
    }
    pub fn schedule(&mut self, wanted: bool) -> Result<(), String> {
        if self.tick.swap(false, Ordering::AcqRel) {
            self.timer.take();
        }
        if !wanted {
            self.timer.take();
            return Ok(());
        }
        if self.timer.is_none() {
            let weak = Arc::downgrade(&self.state);
            let tick = self.tick.clone();
            self.timer = Some(crate::main_loop::Timer::new_common(
                self.main,
                2.0,
                move || {
                    tick.store(true, Ordering::Release);
                    if let Some(state) = weak.upgrade() {
                        state.request_refresh();
                    }
                },
            )?);
        }
        Ok(())
    }
    pub fn retry(&mut self, wanted: bool) -> Result<(), String> {
        if self.retry.as_ref().is_some_and(|timer| !timer.valid()) {
            self.retry.take();
        }
        if !wanted {
            self.retry.take();
            return Ok(());
        }
        if self.retry.is_none() {
            let weak = Arc::downgrade(&self.state);
            self.retry = Some(crate::main_loop::Timer::new_common(
                self.main,
                0.02,
                move || {
                    if let Some(state) = weak.upgrade() {
                        state.client.wake_main();
                    }
                },
            )?);
        }
        Ok(())
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        self.state.closed.store(true, Ordering::Release);
        self.activation.take();
        self.timer.take();
        self.retry.take();
        for observer in &self.observers {
            unsafe {
                self.center.removeObserver(
                    std::convert::AsRef::<objc2::runtime::AnyObject>::as_ref(&**observer),
                );
            }
        }
    }
}
#[cfg(feature = "test-tools")]
thread_local! {static FIXTURE:std::cell::RefCell<Option<objc2::rc::Weak<NSNotificationCenter>>>=const{std::cell::RefCell::new(None)};}
#[cfg(feature = "test-tools")]
pub fn fixture_post(name: &str) -> Result<(), String> {
    let center = FIXTURE
        .with(|slot| slot.borrow().as_ref().and_then(|value| value.load()))
        .ok_or("Owned keyboard notification center unavailable.")?;
    let name = unsafe {
        match name {
            "sleep" => NSWorkspaceWillSleepNotification,
            "resign" => NSWorkspaceSessionDidResignActiveNotification,
            "wake" => NSWorkspaceDidWakeNotification,
            "active" => NSWorkspaceSessionDidBecomeActiveNotification,
            "activate" => objc2_app_kit::NSWorkspaceDidActivateApplicationNotification,
            "launch" => NSWorkspaceDidLaunchApplicationNotification,
            "terminate" => NSWorkspaceDidTerminateApplicationNotification,
            _ => return Err("Unknown keyboard environment fixture event.".into()),
        }
    };
    unsafe { center.postNotificationName_object(name, None) };
    Ok(())
}

#[cfg(feature = "test-tools")]
pub fn fixture_post_application(kind: &str) -> Result<(), String> {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSString};
    let center = FIXTURE
        .with(|slot| slot.borrow().as_ref().and_then(|value| value.load()))
        .ok_or("Owned notification center required")?;
    let own = objc2_app_kit::NSRunningApplication::currentApplication();
    let malformed = NSString::from_str("OWN-MALFORMED-NOT-LOGGED");
    let fixture_app = match kind {
        "original" | "replacement" | "foreign" | "own" => {
            let factory = unsafe {
                libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"ruycast_keyword_fixture_notification_application".as_ptr(),
                )
            };
            if factory.is_null() {
                return Err("Complete own notification application adapter required".into());
            }
            let (token, pid) = match kind {
                "original" => (1, 42),
                "replacement" => (2, 42),
                "foreign" => (3, 43),
                _ => (u64::MAX, std::process::id() as i32),
            };
            let raw = unsafe {
                std::mem::transmute::<
                    *mut libc::c_void,
                    unsafe extern "C" fn(u64, i32) -> *mut objc2_app_kit::NSRunningApplication,
                >(factory)(token, pid)
            };
            Some(
                unsafe { Retained::from_raw(raw) }
                    .ok_or("Own typed application creation failed")?,
            )
        }
        _ => None,
    };
    let object: Option<&AnyObject> = match kind {
        "typed" => Some(std::convert::AsRef::<AnyObject>::as_ref(&*own)),
        "malformed" => Some(malformed.as_ref()),
        "missing" => None,
        "original" | "replacement" | "foreign" | "own" => fixture_app
            .as_deref()
            .map(std::convert::AsRef::<AnyObject>::as_ref),
        _ => return Err("Unknown own activation kind".into()),
    };
    let info = object.map(|object| {
        NSDictionary::from_slices(
            &[unsafe { objc2_app_kit::NSWorkspaceApplicationKey }],
            &[object],
        )
    });
    unsafe {
        center.postNotificationName_object_userInfo(
            objc2_app_kit::NSWorkspaceDidActivateApplicationNotification,
            None,
            info.as_deref().map(|info| info.cast_unchecked()),
        );
    }
    Ok(())
}
