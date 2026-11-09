//! The live, original-target native request. No handles leave the main owner.
use super::*;
use crate::{
    snippet_choice_request::{Activation, ConfigTicket, Input, Phase, Policy, Stamp},
    snippet_choice_window::{Callbacks, Event, Window},
};
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSEvent, NSEventMask, NSRunningApplication};
use objc2_foundation::NSObjectProtocol;

struct Life {
    count: Rc<Cell<usize>>,
    owner: Weak<Callback>,
}
impl Drop for Life {
    fn drop(&mut self) {
        self.count.set(self.count.get().saturating_sub(1));
        if let Some(owner) = self.owner.upgrade() {
            (owner.notify)();
        }
    }
}
struct MonitorState {
    closed: Cell<bool>,
    request: Weak<Request>,
    _life: Rc<Life>,
}
struct Monitor {
    state: Rc<MonitorState>,
    native: Option<Retained<AnyObject>>,
}
impl Monitor {
    fn new(request: &Rc<Request>) -> Option<Self> {
        let state = Rc::new(MonitorState {
            closed: Cell::new(false),
            request: Rc::downgrade(request),
            _life: request.life.clone(),
        });
        if request.context.fixture() {
            #[cfg(feature = "test-tools")]
            unsafe {
                let guard = libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"ruycast_choice_fixture_monitor_guard".as_ptr(),
                );
                if guard.is_null()
                    || !std::mem::transmute::<Ptr, unsafe extern "C" fn() -> bool>(guard)()
                {
                    return None;
                }
            }
            #[cfg(not(feature = "test-tools"))]
            {
                return None;
            }
        }
        let callback = state.clone();
        let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSEvent>| {
            let state = callback.clone();
            if state.closed.get() {
                return;
            }
            if let Some(request) = state.request.upgrade() {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| request.cancel(false)))
                    .is_err()
                {
                    if let Some(owner) = request.owner.upgrade() {
                        owner.failed.set(true);
                        owner.interrupt();
                    }
                }
            }
        });
        // Global events belong to other apps, including overlapping windows;
        // screen bounds are never used as an ownership classifier.
        let native = {
            NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown,
                &block,
            )
        }?;
        Some(Self {
            state,
            native: Some(native),
        })
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.state.closed.set(true);
        if let Some(handle) = self.native.take() {
            unsafe {
                NSEvent::removeMonitor(&handle);
            }
        }
    }
}

pub(super) struct Request {
    owner: Weak<Callback>,
    main: MainThreadMarker,
    context: Context,
    prepared: Arc<crate::snippet_choice::Prepared>,
    target: Target,
    own: Retained<NSRunningApplication>,
    policy: RefCell<Policy>,
    original_gate: Gate,
    observation: u64,
    input: Cell<u64>,
    activation: Cell<u64>,
    stage: Cell<u64>,
    config: Cell<u64>,
    ticket: RefCell<Option<ConfigTicket>>,
    anchor: RefCell<Option<Rc<crate::keyword_target::Anchor>>>,
    window: RefCell<Option<Rc<Window>>>,
    monitor: RefCell<Option<Monitor>>,
    values: RefCell<Option<Vec<(String, String)>>>,
    activated_return: Cell<bool>,
    exclusion_key: String,
    life: Rc<Life>,
    theme: RefCell<Option<Option<String>>>,
}
impl Request {
    pub(super) fn new(
        owner: &Rc<Callback>,
        main: MainThreadMarker,
        prepared: Arc<crate::snippet_choice::Prepared>,
        target: Target,
        gate: Gate,
    ) -> Option<Rc<Self>> {
        if owner.choice_lives.get() != 0 {
            return None;
        }
        owner.choice_lives.set(1);
        let life = Rc::new(Life {
            count: owner.choice_lives.clone(),
            owner: Rc::downgrade(owner),
        });
        let exclusion_key = templates::canonical(target.bundle().unwrap_or(""));
        Some(Rc::new(Self {
            owner: Rc::downgrade(owner),
            main,
            context: owner.context,
            own: NSRunningApplication::currentApplication(),
            policy: RefCell::new(Policy::new(
                main,
                prepared.identity().clone(),
                Instant::now(),
                gate.generation,
                gate.activation_epoch,
                owner.input_epoch.get(),
            )),
            prepared,
            target,
            original_gate: gate,
            observation: owner.observation_epoch.get(),
            input: Cell::new(owner.input_epoch.get()),
            activation: Cell::new(gate.activation_epoch),
            stage: Cell::new(gate.stage_epoch),
            config: Cell::new(gate.generation),
            ticket: RefCell::new(None),
            anchor: RefCell::new(None),
            window: RefCell::new(None),
            monitor: RefCell::new(None),
            values: RefCell::new(None),
            activated_return: Cell::new(false),
            exclusion_key,
            life,
            theme: RefCell::new(None),
        }))
    }
    fn phase(&self) -> Phase {
        self.policy.borrow().phase()
    }
    fn stamp(&self) -> Stamp {
        self.policy.borrow().stamp()
    }
    fn matches(&self, stamp: &Stamp) -> bool {
        self.policy.borrow().matches(stamp) && self.live()
    }
    fn history(&self, activation: u64, input: u64) -> bool {
        let Some(owner) = self.owner.upgrade() else {
            return false;
        };
        let g = (owner.gate)();
        let original = self.original_gate;
        owner.live.get()
            && !owner.interrupted.get()
            && !owner.failed.get()
            && owner.observation_epoch.get() == self.observation
            && owner.input_epoch.get() == input
            && g.running
            && !g.blocked
            && !g.hyper_held
            && g.hyper_epoch == original.hyper_epoch
            && g.recording_epoch == original.recording_epoch
            && g.ui_epoch == original.ui_epoch
            && g.ui_intent == original.ui_intent
            && g.ui_dismissal == original.ui_dismissal
            && g.activation_epoch == activation
    }
    fn live(&self) -> bool {
        self.phase() != Phase::Finished && self.history(self.activation.get(), self.input.get())
    }
    fn window_ok(&self) -> bool {
        self.window.borrow().as_ref().is_none_or(|w| !w.failed())
    }
    fn ready(&self) -> bool {
        let Some(owner) = self.owner.upgrade() else {
            return false;
        };
        let gate = (owner.gate)();
        self.live()
            && self.window_ok()
            && self.config.get() != 0
            && gate.generation == self.config.get()
            && owner.generation.get() == gate.generation
            && gate.stage_epoch == self.stage.get()
    }
    fn notify(&self) {
        if let Some(owner) = self.owner.upgrade() {
            (owner.notify)();
        }
    }
    fn feedback(&self, failure: Failure) {
        if let Some(owner) = self.owner.upgrade() {
            owner.feedback(failure, self.original_gate);
        }
    }
    fn sync(&self) {
        if !self.live() || !self.window_ok() {
            self.cancel(false);
            return;
        }
        let Some(owner) = self.owner.upgrade() else {
            self.cancel(false);
            return;
        };
        let gate = (owner.gate)();
        if self.stage.get() != gate.stage_epoch
            || (self.config.get() != 0 && self.config.get() != gate.generation)
        {
            let ticket = self.policy.borrow_mut().config_pending();
            let old = self.ticket.replace(ticket);
            drop(old);
            self.config.set(0);
            self.stage.set(gate.stage_epoch);
        }
        if gate.generation != 0 && owner.generation.get() == gate.generation {
            let ticket = self.ticket.borrow().clone();
            if let Some(ticket) = ticket {
                let prepared = owner.prepared.borrow().clone();
                let compatible = prepared.as_ref().is_some_and(|p| {
                    p.enabled
                        && !p.excluded.contains(&self.exclusion_key)
                        && p.contains_choice(self.prepared.identity())
                });
                let accepted = self.policy.borrow_mut().confirm_config(
                    &ticket,
                    gate.generation,
                    compatible.then_some(self.prepared.identity().as_ref()),
                    compatible,
                );
                if accepted {
                    self.config.set(gate.generation);
                    self.ticket.borrow_mut().take();
                } else {
                    self.cancel(false);
                }
            }
        }
        let window = self.window.borrow().clone();
        if let Some(window) = window {
            self.apply_theme(&window);
            window.refresh_availability();
        }
    }
    fn apply_theme(&self, window: &Window) {
        if !matches!(self.phase(), Phase::Presenting | Phase::Visible) {
            return;
        }
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let prepared = owner.prepared.borrow().clone();
        let Some(prepared) = prepared else {
            return;
        };
        let unchanged = self.theme.borrow().as_ref() == Some(&prepared.theme);
        if unchanged {
            return;
        }
        window.set_theme(prepared.theme.as_deref());
        // A cosmetic SDK call may reenter a newer store/theme adoption. Cache
        // only the still-current value; otherwise repair on the next main wake.
        let current = owner
            .prepared
            .borrow()
            .as_ref()
            .is_some_and(|p| p.theme == prepared.theme);
        let old = self
            .theme
            .replace((self.live() && current).then(|| prepared.theme.clone()));
        drop(old);
        if self.live() && !current {
            self.notify();
        }
    }
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.policy.borrow().deadline()
    }
    pub(super) fn finished(&self) -> bool {
        self.phase() == Phase::Finished
    }
    pub(super) fn input(&self, sequence: u64, kind: u32, tag: i64) {
        if self.finished() {
            return;
        }
        let stamp = self.stamp();
        let window = self.window.borrow().clone();
        let own_key = kind == 10
            && tag != crate::keyboard_state::GENERATED_HYPER
            && window.as_ref().is_some_and(|w| w.is_key());
        if !self.policy.borrow().matches(&stamp) || !self.history(self.activation.get(), sequence) {
            self.cancel(false);
            return;
        }
        self.policy.borrow_mut().input(
            sequence,
            if own_key {
                Input::OwnFormKey
            } else if kind == 1 || kind == 3 {
                Input::MouseObserved
            } else {
                Input::Other
            },
        );
        self.input.set(sequence);
        if self.finished() {
            self.cancel(false);
        }
    }
    pub(super) fn activation(&self, event: &crate::workspace_activation::Event) {
        let Some(sequence) = event.sequence else {
            self.cancel(false);
            return;
        };
        let stamp = self.stamp();
        let guard =
            || self.policy.borrow().matches(&stamp) && self.history(sequence, self.input.get());
        let actor = if let Some(app) = event.application() {
            if !guard() {
                self.cancel(false);
                return;
            }
            let own = app.isEqual(Some(&*self.own));
            if !guard() {
                self.cancel(false);
                return;
            }
            if own {
                Activation::Own
            } else {
                match self.target.matches_notification(app, &guard) {
                    Ok(true) => Activation::Original,
                    Ok(false) => Activation::Foreign,
                    Err(_) => Activation::Unknown,
                }
            }
        } else {
            Activation::Unknown
        };
        if !guard() {
            self.cancel(false);
            return;
        }
        self.policy.borrow_mut().activated(sequence, actor);
        self.activation.set(sequence);
        if self.finished() {
            self.cancel(false);
        }
    }
    fn own_front(&self, guard: &impl Fn() -> bool) -> bool {
        self.context
            .frontmost(guard)
            .ok()
            .flatten()
            .is_some_and(|front| {
                front
                    .matches_notification(&self.own, guard)
                    .unwrap_or(false)
            })
    }
    pub(super) fn cancel(&self, restore: bool) {
        let restore = restore && self.phase() == Phase::Visible && self.live() && self.window_ok();
        self.policy.borrow_mut().cancel();
        self.config.set(0);
        let monitor = self.monitor.borrow_mut().take();
        drop(monitor);
        let window = self.window.borrow_mut().take();
        if let Some(window) = window {
            window.cancel(false);
            drop(window);
        }
        let anchor = self.anchor.borrow_mut().take();
        drop(anchor);
        let values = self.values.borrow_mut().take();
        drop(values);
        if restore {
            let guard = || {
                self.history(self.activation.get(), self.input.get())
                    && self.phase() == Phase::Finished
            };
            if self.own_front(&guard) {
                let _ = self.target.activate(&guard);
            }
        }
        self.notify();
    }
    fn event(&self, event: Event) {
        match event {
            Event::Cancel { restore } => self.cancel(restore),
            Event::Submit(values) => {
                if !self.ready() || self.phase() != Phase::Visible {
                    self.cancel(false);
                    return;
                }
                let returning = self
                    .policy
                    .borrow_mut()
                    .begin_return(Instant::now())
                    .is_some();
                if !returning {
                    self.cancel(false);
                    return;
                }
                let old = self.values.replace(Some(values));
                drop(old);
                self.notify();
            }
        }
    }
    fn present(self: &Rc<Self>) {
        if !self.ready() {
            return;
        }
        let stamp = self.stamp();
        let guard = || self.matches(&stamp) && self.ready();
        let anchor = self.anchor.borrow().clone();
        if anchor.as_ref().is_none_or(|a| {
            !matches!(
                a.verify(&guard),
                Ok(crate::keyword_target::Continuity::Pinned
                    | crate::keyword_target::Continuity::EventGuardsRequired)
            )
        }) {
            self.cancel(false);
            return;
        }
        let presenting = self.policy.borrow_mut().begin_present(&stamp).is_some();
        if !presenting {
            self.cancel(false);
            return;
        }
        // The fixture must supply the complete dedicated general-board
        // substitute before any AppKit control construction.
        if self.context.fixture() {
            #[cfg(feature = "test-tools")]
            unsafe {
                let p = libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_choice_fixture_guard".as_ptr());
                if p.is_null() || !std::mem::transmute::<Ptr, unsafe extern "C" fn() -> bool>(p)() {
                    self.cancel(false);
                    return;
                }
            }
            #[cfg(not(feature = "test-tools"))]
            {
                self.cancel(false);
                return;
            }
        }
        let Some(monitor) = Monitor::new(self) else {
            self.cancel(false);
            return;
        };
        if !self.ready() {
            drop(monitor);
            self.cancel(false);
            return;
        }
        let old = self.monitor.replace(Some(monitor));
        drop(old);
        let callback_life = self.life.clone();
        let weak = Rc::downgrade(self);
        let clip = weak.clone();
        let submit = weak.clone();
        let failed = weak.clone();
        let window = Rc::new(Window::new(
            self.main,
            self.prepared.clone(),
            !self.context.fixture(),
            Callbacks {
                submit_permit: Rc::new(move || {
                    let owner = submit.clone();
                    let stamp = owner.upgrade().map(|r| r.stamp());
                    Rc::new(move || {
                        owner.upgrade().zip(stamp.as_ref()).is_some_and(|(r, s)| {
                            r.phase() == Phase::Visible && r.matches(s) && r.ready()
                        })
                    })
                }),
                failure: Rc::new(move || {
                    if let Some(r) = failed.upgrade() {
                        r.cancel(false);
                    }
                }),
                clipboard: Rc::new(move || {
                    let r = clip.upgrade().ok_or("Cancelled choice")?;
                    let stamp = r.stamp();
                    let guard = || r.matches(&stamp) && r.ready() && r.phase() == Phase::Visible;
                    if !r.context.safe_field(&guard).unwrap_or(false) {
                        return Err("Unsafe choice input".into());
                    }
                    r.context
                        .clipboard(&guard)
                        .map_err(|_| "Cancelled choice".into())
                }),
                event: Rc::new(move |event| {
                    let _lease = callback_life.clone();
                    if let Some(r) = weak.upgrade() {
                        r.event(event);
                    }
                }),
            },
        ));
        if !self.ready() || self.phase() != Phase::Presenting {
            drop(window);
            self.cancel(false);
            return;
        }
        let old = self.window.replace(Some(window.clone()));
        drop(old);
        self.apply_theme(&window);
        let weak = Rc::downgrade(self);
        let permit: Rc<dyn Fn() -> bool> = Rc::new(move || {
            weak.upgrade().is_some_and(|r| {
                r.live()
                    && r.window_ok()
                    && match r.phase() {
                        Phase::Presenting => r.ready(),
                        Phase::Visible => true,
                        _ => false,
                    }
            })
        });
        let shown = if self.context.fixture() {
            #[cfg(feature = "test-tools")]
            {
                window.fixture_present(permit)
            }
            #[cfg(not(feature = "test-tools"))]
            {
                false
            }
        } else {
            window.show(permit)
        };
        let presented =
            shown && !window.failed() && self.live() && self.policy.borrow_mut().presented();
        if !presented {
            self.cancel(false);
        } else {
            window.refresh_availability();
        }
    }
    pub(super) fn pump(self: &Rc<Self>) {
        self.sync();
        if self.finished() {
            return;
        }
        match self.phase() {
            Phase::PendingTrigger => {
                if !self.ready() {
                    return;
                }
                let Some(stamp) = self.policy.borrow_mut().begin_capture(Instant::now()) else {
                    return;
                };
                let keyword = self.prepared.command().keyword.as_deref().unwrap_or("");
                let guard = || self.matches(&stamp) && self.ready();
                let anchor = match self.context.anchor(&self.target, keyword, &guard) {
                    Ok(a) => Rc::new(a),
                    Err(_) => {
                        self.cancel(false);
                        return;
                    }
                };
                let old = self.anchor.replace(Some(anchor));
                drop(old);
                if self.matches(&stamp) && self.ready() {
                    self.present();
                } else {
                    self.sync();
                }
            }
            Phase::Capturing => {
                if self.ready() && self.anchor.borrow().is_some() {
                    self.present();
                }
            }
            Phase::Returning => self.return_to_original(),
            Phase::Visible | Phase::Presenting | Phase::Applying | Phase::Finished => {}
        }
    }
    fn return_to_original(&self) {
        if !self.ready() {
            return;
        }
        if !self.activated_return.replace(true) {
            let guard = || self.ready() && self.phase() == Phase::Returning;
            if !self.target.activate(&guard).unwrap_or(false) {
                if guard() {
                    if let Some(owner) = self.owner.upgrade() {
                        owner.feedback(Failure::AppUnavailable, self.original_gate);
                    }
                }
                self.cancel(false);
                return;
            }
            let started = self.policy.borrow_mut().return_activated(Instant::now());
            if !started {
                self.cancel(false);
                return;
            }
        }
        if !self.policy.borrow().return_due(Instant::now()) {
            return;
        }
        let stamp = self.stamp();
        let guard = || self.matches(&stamp) && self.ready() && self.phase() == Phase::Returning;
        if !self.target.current(&guard).unwrap_or(false) {
            let own = self.own_front(&guard);
            let eligible = guard();
            if own && self.policy.borrow_mut().retry_own_app(Instant::now()) {
                return;
            }
            // The final policy retry cancels itself. Retain only the eligibility
            // checked before that pure transition, never a refreshed target.
            if eligible {
                self.feedback(Failure::TextChanged);
            }
            self.cancel(false);
            return;
        }
        let anchor = self.anchor.borrow().clone();
        let Some(anchor) = anchor else {
            self.cancel(false);
            return;
        };
        let unchanged = || {
            matches!(
                anchor.verify(&guard),
                Ok(crate::keyword_target::Continuity::Pinned
                    | crate::keyword_target::Continuity::EventGuardsRequired)
            )
        };
        if !unchanged() {
            if guard() {
                if let Some(owner) = self.owner.upgrade() {
                    owner.feedback(Failure::TextChanged, self.original_gate);
                }
            }
            self.cancel(false);
            return;
        }
        let clipboard = if self.prepared.needs_clipboard() {
            match self.context.clipboard(&guard) {
                Ok(v) => v,
                Err(_) => {
                    if guard() {
                        self.feedback(Failure::Copy);
                    }
                    self.cancel(false);
                    return;
                }
            }
        } else {
            None
        };
        let values = self.values.borrow().clone().unwrap_or_default();
        let rendered = self
            .prepared
            .render(&values, &templates::Context::current(clipboard.as_deref()));
        let Ok((text, cursor)) = rendered else {
            if guard() {
                self.feedback(Failure::TextChanged);
            }
            self.cancel(false);
            return;
        };
        if !guard() || !unchanged() {
            if guard() {
                self.feedback(Failure::TextChanged);
            }
            self.cancel(false);
            return;
        }
        let claim = self.policy.borrow_mut().claim_apply(&stamp, Instant::now());
        let Some(claim) = claim else {
            self.cancel(false);
            return;
        };
        let valid = || self.matches(&claim) && self.ready() && self.phase() == Phase::Applying;
        let Some(owner) = self.owner.upgrade() else {
            self.cancel(false);
            return;
        };
        let replacement = Replacement::new(
            self.prepared.command().keyword.as_deref().unwrap_or(""),
            "",
            text,
            cursor,
        );
        owner.effect_possible.set(false);
        owner.clipboard_possible.set(false);
        let analytics = owner.analytics_span(&self.prepared.command().id);
        let outcome = if let Some(mode @ ("teams" | "html")) =
            self.prepared.command().snippet_output.as_deref()
        {
            let result = keyword_effects::apply_rich(
                self.context,
                &self.target,
                &replacement,
                mode,
                &valid,
                keyword_effects::RichEffects {
                    keyboard_possible: &owner.effect_possible,
                    clipboard_possible: &owner.clipboard_possible,
                    own: &owner.own,
                },
            );
            if let Some(cursor) = result.cursor {
                let gate = (owner.gate)();
                let lease = owner.lease(owner.revision.get(), gate);
                if valid() && lease.live() {
                    let old = owner.cursor.replace(Some(PendingCursor {
                        due: Instant::now() + Duration::from_millis(80),
                        lease,
                        cursor,
                    }));
                    drop(old);
                }
            }
            if owner.clipboard_possible.get() && !result.outcome.complete {
                owner
                    .clipboard_partial
                    .set(owner.clipboard_partial.get().saturating_add(1));
            }
            result.outcome
        } else {
            keyword_effects::apply_plain(
                self.context,
                &self.target,
                &replacement,
                &valid,
                &owner.effect_possible,
            )
        };
        if let Some(span) = analytics {
            span.finish(outcome.complete, "action-failed");
        }
        if !outcome.complete {
            owner.feedback(
                if outcome.keyboard_possible || owner.clipboard_possible.get() {
                    Failure::Partial
                } else {
                    Failure::Copy
                },
                self.original_gate,
            );
        }
        if outcome.complete {
            owner.completed.set(owner.completed.get().saturating_add(1));
        } else if outcome.keyboard_possible {
            owner.partial.set(owner.partial.get().saturating_add(1));
        }
        self.cancel(false);
    }
    #[cfg(feature = "test-tools")]
    pub(super) fn fixture_control(
        &self,
        case: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if !self.context.fixture() {
            return Err("Own choice controller required".into());
        }
        match case["choiceAction"].as_str() {
            Some("click") => unsafe {
                let emit =
                    libc::dlsym(libc::RTLD_DEFAULT, c"ruycast_choice_fixture_click".as_ptr());
                if emit.is_null() {
                    return Err("Complete own global-monitor adapter required".into());
                }
                std::mem::transmute::<Ptr, unsafe extern "C" fn() -> u64>(emit)();
            },
            Some("cancel-request") => self.cancel(case["restore"].as_bool().unwrap_or(false)),
            Some("read-request") => {}
            _ => {
                let window = self.window.borrow().clone().ok_or("Own form not ready")?;
                return window.fixture_control(case);
            }
        }
        Ok(
            serde_json::json!({"phase":format!("{:?}",self.phase()),"anchor":self.anchor.borrow().is_some(),"window":self.window.borrow().is_some(),"configReady":self.ready()}),
        )
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        self.policy.get_mut().cancel();
        self.config.set(0);
        let monitor = self.monitor.get_mut().take();
        drop(monitor);
        let window = self.window.get_mut().take();
        drop(window);
    }
}
