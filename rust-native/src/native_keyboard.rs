//! Main-only Carbon/HID host. Callback userdata never borrows this owner.
use objc2::MainThreadMarker;
use ruycast_native::{
    carbon_hotkeys::Carbon,
    connected::Connection,
    control::HostEvent,
    hid_service::{self, Gate},
    hotkeys::{Plan, Press, Registry, RoutedPress},
    keyboard_service::{self, Client, HookHealth},
};
use std::{
    collections::VecDeque,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc,
    },
};
fn snapshot(client: &Client, ui: &Connection, press: Press, hook: bool) -> Option<RoutedPress> {
    if !client.running() || client.paused() || client.recording() || ui.stop_requested() {
        return None;
    }
    Some(RoutedPress {
        hook_epoch: hook.then(|| client.hook_epoch()),
        press,
        received: std::time::Instant::now(),
        recording_epoch: client.recording_epoch(),
        ui_epoch: ui.latest_epoch(),
        ui_intent: ui.routing_revision(),
        ui_dismissal: ui.latest_dismissal(),
    })
}
fn enqueue_press(
    client: &Client,
    events: &SyncSender<HostEvent>,
    ui: &Connection,
    press: Press,
) -> bool {
    let Some(event) = snapshot(client, ui, press, false) else {
        return false;
    };
    if events.try_send(HostEvent::Hotkey(event)).is_err() {
        client.note_dropped();
        false
    } else {
        true
    }
}
// Same ingress capacity as the existing owner queue. The credit survives a
// popped/in-flight head and stays owned until forwarding or explicit retirement.
const CAPACITY: usize = 32;
struct Credit(Arc<AtomicUsize>);
impl Drop for Credit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Queued {
    event: RoutedPress,
    _credit: Credit,
}
struct Ingress {
    sender: mpsc::SyncSender<Queued>,
    credits: Arc<AtomicUsize>,
}
impl Ingress {
    fn new() -> (Self, mpsc::Receiver<Queued>) {
        let (sender, inbox) = mpsc::sync_channel(CAPACITY);
        (
            Self {
                sender,
                credits: Arc::new(AtomicUsize::new(0)),
            },
            inbox,
        )
    }
    fn submit(&self, event: RoutedPress) -> bool {
        if self
            .credits
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < CAPACITY).then_some(n + 1)
            })
            .is_err()
        {
            return false;
        }
        self.sender
            .try_send(Queued {
                event,
                _credit: Credit(self.credits.clone()),
            })
            .is_ok()
    }
}
enum Forward {
    Sent,
    Full(Queued),
    Disconnected,
}
#[derive(Default)]
struct FeedbackQueue(std::cell::RefCell<Option<HostEvent>>);
impl FeedbackQueue {
    fn submit(
        &self,
        failure: ruycast_native::keyword_capture::Failure,
        epoch: u64,
        routing: u64,
        dismissal: u64,
    ) {
        self.0.replace(Some(HostEvent::SnippetFailure {
            epoch,
            routing,
            dismissal,
            failure,
        }));
    }
    fn pending(&self) -> bool {
        self.0.borrow().is_some()
    }
    fn drain(&self, events: &SyncSender<HostEvent>, ui: &Connection) {
        let pending = self.0.borrow_mut().take();
        let Some(event) = pending else {
            return;
        };
        let live = matches!(&event, HostEvent::SnippetFailure {epoch,routing,dismissal,..}
            if !ui.stop_requested() && *epoch==ui.latest_epoch() && *routing==ui.routing_revision() && *dismissal==ui.latest_dismissal());
        if live {
            if let Err(TrySendError::Full(event)) = events.try_send(event) {
                self.0.replace(Some(event));
            }
        }
    }
}
fn forward(events: &SyncSender<HostEvent>, mut head: Queued) -> Forward {
    match events.try_send(HostEvent::Hotkey(head.event)) {
        Ok(()) => Forward::Sent,
        Err(TrySendError::Full(HostEvent::Hotkey(event))) => {
            head.event = event;
            Forward::Full(head)
        }
        Err(_) => Forward::Disconnected,
    }
}
pub(crate) struct Service {
    feedback: Rc<FeedbackQueue>,
    #[cfg(feature = "test-tools")]
    fixture: bool,
    server: keyboard_service::Server,
    registry: Registry<Carbon>,
    client: Client,
    failed: Arc<AtomicBool>,
    hid: Rc<hid_service::Service>,
    keywords: Rc<ruycast_native::keyword_capture::Service>,
    keyword_timer: Option<crate::main_loop::Timer>,
    keyword_deadline: Option<std::time::Instant>,
    environment: crate::keyboard_environment::Environment,
    inbox: mpsc::Receiver<Queued>,
    pending: VecDeque<Queued>,
    events: SyncSender<HostEvent>,
    ui: Arc<Connection>,
    generation: u64,
    hyper: bool,
    modifiers: bool,
    sided: bool,
    main: MainThreadMarker,
}
impl Service {
    pub fn new(
        main: MainThreadMarker,
        fixture: bool,
        events: SyncSender<HostEvent>,
        ui: Arc<Connection>,
        own_writes: Rc<ruycast_native::clipboard_ownership::OwnWrites>,
    ) -> Result<Self, String> {
        let (client, server) = keyboard_service::channel();
        let sink_client = client.clone();
        let carbon_events = events.clone();
        let carbon_ui = ui.clone();
        let carbon = Carbon::new(
            main,
            fixture,
            Arc::new(move |press| {
                enqueue_press(&sink_client, &carbon_events, &carbon_ui, press);
            }),
        )?;
        client.attach_callback_gate(carbon.callback_gate());
        let failed = carbon.failure_flag();
        let (ingress, inbox) = Ingress::new();
        let sink_client = client.clone();
        let sink_ui = ui.clone();
        let sink = Rc::new(move |press| {
            let Some(event) = snapshot(&sink_client, &sink_ui, press, true) else {
                return false;
            };
            if !ingress.submit(event) {
                sink_client.note_dropped();
                false
            } else {
                sink_client.wake_main();
                true
            }
        });
        let gate_client = client.clone();
        let gate = Rc::new(move || Gate {
            running: gate_client.running() && !gate_client.paused(),
            recording: gate_client.recording(),
            epoch: gate_client.recording_epoch(),
        });
        let hid = Rc::new(hid_service::Service::new(main, fixture, gate, sink)?);
        let activation_owner = Rc::new(std::cell::RefCell::new(std::rc::Weak::<
            ruycast_native::keyword_capture::Service,
        >::new()));
        let activation_slot = activation_owner.clone();
        let environment = crate::keyboard_environment::Environment::new(
            main,
            fixture,
            client.clone(),
            Rc::new(move |event| {
                let owner = activation_slot.borrow().upgrade();
                if let Some(owner) = owner {
                    owner.workspace_activated(event);
                }
            }),
        );
        let state = environment.state();
        let interrupted = client.clone();
        hid.set_interrupt_notifier(Rc::new(move || {
            interrupted.invalidate_hook_delivery();
            state.request_refresh();
        }));
        let keyword_client = client.clone();
        let keyword_ui = ui.clone();
        let keyword_hid = hid.clone();
        let keyword_gate = Rc::new(move || ruycast_native::keyword_capture::Gate {
            running: keyword_client.running()
                && !keyword_client.paused()
                && !keyword_ui.stop_requested(),
            blocked: keyword_client.recording() || keyword_ui.palette_visible(),
            hyper_held: keyword_hid.hyper_held(),
            hyper_epoch: keyword_hid.hyper_epoch(),
            activation_epoch: keyword_client.external_activation_epoch(),
            stage_epoch: keyword_client.keyword_stage_epoch(),
            generation: keyword_client.keyword_generation(),
            recording_epoch: keyword_client.recording_epoch(),
            ui_epoch: keyword_ui.latest_epoch(),
            ui_intent: keyword_ui.routing_revision(),
            ui_dismissal: keyword_ui.latest_dismissal(),
        });
        let keyword_client = client.clone();
        let keyword_environment = environment.state();
        let keywords = Rc::new(ruycast_native::keyword_capture::Service::with_ownership(
            main,
            fixture,
            keyword_gate,
            Rc::new(move || {
                keyword_client.publish_keyword_status(
                    keyword_client.keyword_generation(),
                    "Recovering".into(),
                );
                keyword_environment.request_refresh();
                keyword_client.wake_main();
            }),
            own_writes,
        )?);
        *activation_owner.borrow_mut() = Rc::downgrade(&keywords);
        let reporting_client = client.clone();
        let reporting_ui = ui.clone();
        let feedback = Rc::new(FeedbackQueue::default());
        let reporting_feedback = feedback.clone();
        let reporting_wake = client.clone();
        keywords.set_reporting(
            Rc::new(move || reporting_client.analytics()),
            Rc::new(move |failure, gate| {
                if !reporting_ui.stop_requested() {
                    reporting_feedback.submit(
                        failure,
                        gate.ui_epoch,
                        gate.ui_intent,
                        gate.ui_dismissal,
                    );
                    reporting_wake.wake_main();
                }
            }),
        );
        #[cfg(feature = "test-tools")]
        if fixture {
            ruycast_native::keyword_capture::fixture_bind(&keywords);
        }
        Ok(Self {
            feedback,
            #[cfg(feature = "test-tools")]
            fixture,
            server,
            registry: Registry::new(carbon),
            client,
            failed,
            hid,
            keywords,
            keyword_timer: None,
            keyword_deadline: None,
            environment,
            inbox,
            pending: VecDeque::with_capacity(CAPACITY),
            events,
            ui,
            generation: 0,
            hyper: false,
            modifiers: false,
            sided: false,
            main,
        })
    }
    pub fn client(&self) -> Client {
        self.client.clone()
    }
    fn publish(&self) {
        let status = self.hid.status();
        let recovering = !self.hid.retired() && status != "On"
            || hid_service::Service::recovery_pending(self.main);
        self.client.publish_hook_health(HookHealth {
            generation: self.generation,
            status: status.clone(),
            hyper: if self.hyper {
                status.clone()
            } else {
                "Off".into()
            },
            modifier_taps: if self.modifiers {
                status.clone()
            } else {
                "Off".into()
            },
            sided: if self.sided { status } else { "Off".into() },
            recovering,
        });
    }
    fn live(&self, event: &RoutedPress) -> bool {
        self.client.running()
            && !self.client.paused()
            && !self.client.recording()
            && !self.ui.stop_requested()
            && event.recording_epoch == self.client.recording_epoch()
            && event.hook_epoch == Some(self.client.hook_epoch())
            && event.press.generation != 0
            && event.press.generation == self.client.accepted_generation()
            && event.ui_epoch == self.ui.latest_epoch()
            && event.ui_intent == self.ui.routing_revision()
            && event.ui_dismissal == self.ui.latest_dismissal()
            && event.received.elapsed() < std::time::Duration::from_secs(5)
    }
    fn drain(&mut self) {
        while let Ok(event) = self.inbox.try_recv() {
            self.pending.push_back(event);
        }
        while let Some(head) = self.pending.pop_front() {
            // No queue/state borrow survives the SDK secure-input recheck.
            if !self.live(&head.event) || !self.hid.delivery_allowed() || !self.live(&head.event) {
                self.client.note_dropped();
                continue;
            }
            match forward(&self.events, head) {
                Forward::Sent => self.client.note_hook_forwarded(),
                Forward::Full(head) => {
                    self.pending.push_front(head);
                    break;
                }
                Forward::Disconnected => self.client.note_dropped(),
            }
        }
    }
    pub fn pump(&mut self) {
        if self.failed() && self.client.running() {
            self.client.stop();
        }
        if !self.client.running() {
            self.stop();
            return;
        }
        self.server.pump(&mut self.registry);
        let generation = self.registry.generation();
        let changed =
            generation != self.generation && generation == self.client.accepted_generation();
        if changed {
            let plan = self.registry.current();
            self.hyper = plan.hyper;
            self.modifiers = !plan.modifier_taps.is_empty();
            self.sided = !plan.sided.is_empty();
            self.hid.update(
                generation,
                Plan {
                    carbon: Default::default(),
                    sided: plan.sided.clone(),
                    modifier_taps: plan.modifier_taps.clone(),
                    hyper: plan.hyper,
                },
            );
            self.generation = generation;
        }
        let mut keyword_changed = false;
        if let Some((generation, prepared)) = self.client.keyword_prepared() {
            if generation != self.keywords.generation() {
                self.keywords.update(generation, prepared);
                keyword_changed = true;
            }
        }
        let refresh = self.environment.state().take_refresh();
        if changed || refresh || keyword_changed {
            self.publish(); // Publish the new generation before a possibly held SDK setup.
            self.hid.sleeping(self.client.paused());
            self.hid.refresh();
            self.keywords.refresh();
            self.client
                .publish_keyword_status(self.keywords.generation(), self.keywords.status());
            self.publish();
        }
        self.keywords.pump();
        self.feedback.drain(&self.events, &self.ui);
        let deadline = self.keywords.next_wake();
        if deadline != self.keyword_deadline
            || self.keyword_timer.as_ref().is_some_and(|t| !t.valid())
        {
            self.keyword_timer.take();
            self.keyword_deadline = deadline;
            if let Some(deadline) = deadline {
                let client = self.client.clone();
                match crate::main_loop::Timer::new_common(
                    self.main,
                    deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .as_secs_f64(),
                    move || client.wake_main(),
                ) {
                    Ok(t) => self.keyword_timer = Some(t),
                    Err(_) => {
                        self.failed.store(true, Ordering::Release);
                        self.client.stop();
                    }
                }
            }
        }
        self.drain();
        if self.environment.state().refresh_pending() {
            self.publish();
        }
        let wanted = self.hyper
            || self.modifiers
            || self.sided
            || !self.keywords.retired()
            || self.client.keyword_status() != "Off"
            || ruycast_native::keyword_capture::Service::recovery_pending(self.main)
            || !self.hid.retired()
            || hid_service::Service::recovery_pending(self.main);
        if self.environment.schedule(wanted).is_err()
            || self
                .environment
                .retry(!self.pending.is_empty() || self.feedback.pending())
                .is_err()
        {
            self.failed.store(true, Ordering::Release);
            self.client.stop();
        }
    }
    /// Keep main/timers/profile ownership alive until native resources retire.
    pub fn stop(&mut self) -> bool {
        if self.client.running() {
            self.client.stop();
        }
        self.keyword_timer.take();
        self.keyword_deadline = None;
        self.pending.clear();
        self.feedback.0.borrow_mut().take();
        while self.inbox.try_recv().is_ok() {}
        self.server.pump(&mut self.registry);
        let keyword_own = self.keywords.stop();
        let keyword_global = ruycast_native::keyword_capture::Service::recover(self.main);
        self.client
            .publish_keyword_status(self.keywords.generation(), self.keywords.status());
        let own = self.hid.stop();
        let global = hid_service::Service::recover(self.main);
        let retired = own
            && global
            && keyword_own
            && keyword_global
            && self.registry.suspension_clean()
            && !self.client.pending();
        #[cfg(feature = "test-tools")]
        if retired && self.fixture {
            let proof = fixture_hid(&serde_json::json!({"control":"state"}));
            if let (Ok(mut proof), Some(home)) =
                (proof, std::env::var_os("RUYCAST_KEYBOARD_FIXTURE_HOME"))
            {
                if let Ok(keyword) = ruycast_native::keyword_capture::fixture(
                    &serde_json::json!({"control":"state"}),
                ) {
                    let keyword = serde_json::json!({"pid":std::process::id(),"state":keyword,"ownChange":self.keywords.own_change(),"ownActive":self.keywords.own_active()});
                    if std::fs::write(
                        std::path::PathBuf::from(&home).join("keyword-retired.json"),
                        keyword.to_string(),
                    )
                    .is_err()
                    {
                        self.failed.store(true, Ordering::Release);
                    }
                } else {
                    self.failed.store(true, Ordering::Release);
                }
                proof["pid"] = serde_json::json!(std::process::id());
                if std::fs::write(
                    std::path::PathBuf::from(home).join("keyboard-retired.json"),
                    proof.to_string(),
                )
                .is_err()
                {
                    self.failed.store(true, Ordering::Release);
                }
            }
        }
        self.publish();
        if self.environment.schedule(!retired).is_err() {
            self.failed.store(true, Ordering::Release);
        }
        let _ = self.environment.retry(false);
        retired
    }
    pub fn running(&self) -> bool {
        self.client.running()
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire) || self.hid.failed()
    }
}
#[cfg(feature = "test-tools")]
pub fn fixture_hid(request: &serde_json::Value) -> Result<serde_json::Value, String> {
    use std::ffi::{c_void, CStr, CString};
    unsafe {
        macro_rules! method {
            ($name:literal,$ty:ty) => {{
                let pointer = libc::dlsym(libc::RTLD_DEFAULT, concat!($name, "\0").as_ptr().cast());
                if pointer.is_null() {
                    return Err("Owned complete HID controller unavailable.".into());
                }
                std::mem::transmute::<*mut c_void, $ty>(pointer)
            }};
        }
        let keyboard = method!(
            "ruycast_keyboard_fixture_ready",
            unsafe extern "C" fn() -> u64
        );
        let hid = method!("ruycast_hid_fixture_ready", unsafe extern "C" fn() -> u64);
        if keyboard() != 0x52554b4559533031 || hid() != 0x5255484944303031 {
            return Err("Owned complete HID controller unavailable.".into());
        }
        let mut result = serde_json::Value::Null;
        match request["control"].as_str().unwrap_or("state") {
            "input" => {
                let device = request["device"].as_u64().unwrap_or(0);
                if device >= 16 {
                    return Err("Invalid fixture device.".into());
                }
                method!(
                    "ruycast_hid_fixture_input",
                    unsafe extern "C" fn(u32, u32, u32, bool, f64)
                )(
                    device as u32,
                    request["page"].as_u64().unwrap_or(7) as u32,
                    request["usage"]
                        .as_u64()
                        .ok_or("Usage required.")?
                        .try_into()
                        .map_err(|_| "Invalid usage.")?,
                    request["down"].as_bool().ok_or("Direction required.")?,
                    request["time"].as_f64().unwrap_or(0.0),
                );
            }
            "remove" => method!("ruycast_hid_fixture_remove", unsafe extern "C" fn(u32))(
                request["device"]
                    .as_u64()
                    .filter(|id| *id < 16)
                    .ok_or("Invalid fixture device.")? as u32,
            ),
            "event" => {
                let consumed = method!(
                    "ruycast_hid_fixture_event",
                    unsafe extern "C" fn(u32, u16, u64, i64, bool) -> u64
                )(
                    request["type"]
                        .as_u64()
                        .unwrap_or(10)
                        .try_into()
                        .map_err(|_| "Invalid event type.")?,
                    request["key"]
                        .as_u64()
                        .unwrap_or(0)
                        .try_into()
                        .map_err(|_| "Invalid key.")?,
                    request["flags"].as_u64().unwrap_or(0),
                    request["tag"].as_i64().unwrap_or(0),
                    request["repeat"].as_bool().unwrap_or(false),
                ) != 0;
                let flags = method!(
                    "ruycast_hid_fixture_event_flags",
                    unsafe extern "C" fn() -> u64
                )();
                result = serde_json::json!({"consumed":consumed,"flags":flags});
            }
            "env" => {
                method!("ruycast_hid_fixture_environment", unsafe extern "C" fn(u32))(
                    request["value"]
                        .as_u64()
                        .ok_or("Environment required.")?
                        .try_into()
                        .map_err(|_| "Invalid environment.")?,
                );
                crate::keyboard_environment::fixture_post("launch")?;
            }
            "failure" => {
                let name = CString::new(request["name"].as_str().ok_or("Failure name required.")?)
                    .map_err(|_| "Invalid failure name.")?;
                method!(
                    "ruycast_hid_fixture_failure",
                    unsafe extern "C" fn(*const libc::c_char)
                )(name.as_ptr());
                crate::keyboard_environment::fixture_post("launch")?;
            }
            "notification" => crate::keyboard_environment::fixture_post(
                request["name"].as_str().ok_or("Notification required.")?,
            )?,
            "activation-burst" => {
                let values = request["values"]
                    .as_array()
                    .filter(|v| v.len() <= 64)
                    .ok_or("Own activation burst required")?;
                for value in values {
                    crate::keyboard_environment::fixture_post_application(
                        value.as_str().ok_or("Own activation kind required")?,
                    )?;
                }
            }
            "state" => {}
            _ => return Err("Unknown HID fixture control.".into()),
        }
        let data = method!(
            "ruycast_hid_fixture_state",
            unsafe extern "C" fn() -> *const libc::c_char
        )();
        let state: serde_json::Value = serde_json::from_str(
            CStr::from_ptr(data)
                .to_str()
                .map_err(|_| "Invalid fixture state.")?,
        )
        .map_err(|_| "Invalid fixture state.")?;
        Ok(serde_json::json!({"state":state,"event":result}))
    }
}
#[cfg(feature = "test-tools")]
pub fn fixture_emit(code: u32, modifiers: u32, id: Option<u32>) -> Result<i32, String> {
    unsafe {
        let stamp = libc::dlsym(
            libc::RTLD_DEFAULT,
            c"ruycast_keyboard_fixture_ready".as_ptr(),
        );
        if stamp.is_null()
            || std::mem::transmute::<*mut std::ffi::c_void, unsafe extern "C" fn() -> u64>(stamp)()
                != 0x52554b4559533031
        {
            return Err("Owned keyboard SDK boundary unavailable.".into());
        }
        if let Some(id) = id {
            let f = libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ruycast_keyboard_fixture_emit_id".as_ptr(),
            );
            if f.is_null() {
                return Err("Own keyboard emit method unavailable.".into());
            }
            Ok(std::mem::transmute::<
                *mut std::ffi::c_void,
                unsafe extern "C" fn(u32) -> i32,
            >(f)(id))
        } else {
            let f = libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ruycast_keyboard_fixture_emit_binding".as_ptr(),
            );
            if f.is_null() {
                return Err("Own keyboard emit method unavailable.".into());
            }
            Ok(std::mem::transmute::<
                *mut std::ffi::c_void,
                unsafe extern "C" fn(u32, u32) -> i32,
            >(f)(code, modifiers))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event() -> RoutedPress {
        let (client, _server) = keyboard_service::channel();
        let (tx, _rx) = mpsc::sync_channel(1);
        let ui = Connection::new(tx, true);
        snapshot(
            &client,
            &ui,
            Press {
                generation: 7,
                canonical: "lcmd+a".into(),
                action: "saved.id".into(),
            },
            true,
        )
        .unwrap()
    }
    #[test]
    fn ingress_credit_includes_popped_head_and_full_owner_preserves_original_event() {
        let (ingress, inbox) = Ingress::new();
        for _ in 0..CAPACITY {
            assert!(ingress.submit(event()));
        }
        assert!(!ingress.submit(event()));
        let head = inbox.try_recv().unwrap();
        let received = head.event.received;
        let clocks = (
            head.event.hook_epoch,
            head.event.recording_epoch,
            head.event.ui_epoch,
            head.event.ui_intent,
            head.event.ui_dismissal,
        );
        // Popping a head must not free a slot while it waits for the owner.
        assert!(!ingress.submit(event()));
        let (tx, rx) = mpsc::sync_channel(1);
        tx.try_send(HostEvent::UiWake).unwrap();
        let Forward::Full(head) = forward(&tx, head) else {
            panic!("lost full head")
        };
        assert_eq!(head.event.received, received);
        assert_eq!(head.event.press.action, "saved.id");
        assert_eq!(ingress.credits.load(Ordering::Acquire), CAPACITY);
        assert!(matches!(rx.try_recv(), Ok(HostEvent::UiWake)));
        assert!(matches!(forward(&tx, head), Forward::Sent));
        let HostEvent::Hotkey(actual) = rx.try_recv().unwrap() else {
            panic!("wrong owner event")
        };
        assert_eq!(actual.received, received);
        assert_eq!(
            (
                actual.hook_epoch,
                actual.recording_epoch,
                actual.ui_epoch,
                actual.ui_intent,
                actual.ui_dismissal
            ),
            clocks
        );
        assert_eq!(actual.press.generation, 7);
        assert_eq!(actual.press.canonical, "lcmd+a");
        assert!(ingress.submit(event()));
        drop(inbox);
        assert_eq!(ingress.credits.load(Ordering::Acquire), 0);
        assert!(!ingress.submit(event()));
        assert_eq!(ingress.credits.load(Ordering::Acquire), 0);
    }
    #[test]
    fn failed_snippet_feedback_survives_full_queue_but_cannot_repin_new_ui_intent() {
        let (tx, rx) = mpsc::sync_channel(1);
        let ui = Connection::new(tx.clone(), true);
        let queue = FeedbackQueue::default();
        let clocks = (
            ui.latest_epoch(),
            ui.routing_revision(),
            ui.latest_dismissal(),
        );
        tx.try_send(HostEvent::UiWake).unwrap();
        queue.submit(
            ruycast_native::keyword_capture::Failure::Partial,
            clocks.0,
            clocks.1,
            clocks.2,
        );
        queue.drain(&tx, &ui);
        assert!(queue.pending());
        assert!(matches!(rx.try_recv(), Ok(HostEvent::UiWake)));
        queue.drain(&tx, &ui);
        assert!(!queue.pending());
        let HostEvent::SnippetFailure {
            epoch,
            routing,
            dismissal,
            ..
        } = rx.try_recv().unwrap()
        else {
            panic!("lost original failure")
        };
        assert_eq!((epoch, routing, dismissal), clocks);
        tx.try_send(HostEvent::UiWake).unwrap();
        queue.submit(
            ruycast_native::keyword_capture::Failure::Partial,
            clocks.0,
            clocks.1,
            clocks.2,
        );
        queue.drain(&tx, &ui);
        assert!(queue.pending());
        ui.query("new user intent");
        queue.drain(&tx, &ui);
        assert!(!queue.pending());
        assert!(matches!(rx.try_recv(), Ok(HostEvent::UiWake)));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn callback_drops_on_full_queue_and_never_waits_for_owner() {
        let (client, _server) = keyboard_service::channel();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let ui = Connection::new(tx.clone(), true);
        tx.try_send(HostEvent::UiWake).unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            enqueue_press(
                &client,
                &tx,
                &ui,
                ruycast_native::hotkeys::Press {
                    generation: 1,
                    canonical: "cmd+a".into(),
                    action: "own.id".into(),
                },
            );
            done_tx.send(client.dropped()).unwrap();
        });
        assert_eq!(
            done_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap(),
            1
        );
        assert!(matches!(rx.try_recv(), Ok(HostEvent::UiWake)));
        assert!(rx.try_recv().is_err());
        worker.join().unwrap();
    }
}
