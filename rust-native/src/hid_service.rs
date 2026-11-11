//! Main-owned HID/Quartz resources, with separately retained callback state.
//! The caller supplies the already accepted master plan/generation and a
//! nonblocking event sink. This component does not create configuration authority.
use crate::{
    caps_io, caps_lock,
    hid_io::{Api, HidCallback, Ptr},
    hotkeys::{Plan, Press},
    keyboard_state::{self, CommandFilter, Environment, Event, Physical},
};
use objc2::MainThreadMarker;
use std::{
    cell::{Cell, RefCell},
    ptr::null_mut,
    rc::Rc,
};

#[derive(Clone, Copy, Debug)]
pub struct Gate {
    pub running: bool,
    pub recording: bool,
    pub epoch: u64,
}
pub(crate) struct Callback {
    api: Api,
    caps: caps_lock::Control<caps_io::Native>,
    live: Cell<bool>,
    interrupted: Cell<bool>,
    failed: Cell<bool>,
    generation: Cell<u64>,
    plan: RefCell<Rc<Plan>>,
    physical: RefCell<Physical>,
    taps: RefCell<keyboard_state::TapSequence>,
    filter: RefCell<CommandFilter>,
    epoch: Cell<u64>,
    emitted_hyper: Cell<bool>,
    balance: Cell<Option<u64>>,
    emitting: Cell<bool>,
    revision: Cell<u64>,
    hyper_epoch: Cell<u64>,
    gate: Rc<dyn Fn() -> Gate>,
    sink: Rc<dyn Fn(Press) -> bool>,
    interrupt_notifier: RefCell<Option<Rc<dyn Fn()>>>,
    #[cfg(feature = "test-tools")]
    source_ticket: Cell<Option<crate::owned_host_entry::NativeInputSourceTicket>>,
    #[cfg(feature = "test-tools")]
    source_uncertain: Cell<bool>,
}
impl Callback {
    #[cfg(feature = "test-tools")]
    fn publish_source_ticket(owner: Rc<Self>) -> Result<(), String> {
        let (owner, ticket) = crate::owned_host_entry::NativeInputSourceTicket::for_hid(owner);
        match ticket {
            Ok(ticket) => { owner.source_ticket.set(ticket); Ok(()) }
            Err(error) => { owner.source_uncertain.set(true); Err(error) }
        }
    }
    #[cfg(feature = "test-tools")]
    fn revoke_source_ticket(&self) -> bool {
        if self.source_uncertain.get() { return false; }
        let Some(mut ticket) = self.source_ticket.take() else { return true; };
        if ticket.revoke().is_ok() { true } else { self.source_ticket.set(Some(ticket)); false }
    }
    fn gate(&self) -> Gate {
        let gate = (self.gate)();
        if self.epoch.replace(gate.epoch) != gate.epoch {
            self.physical.borrow_mut().cancel_tap();
            self.taps.borrow_mut().reset();
            self.filter.borrow_mut().reset();
        }
        gate
    }
    fn interrupt(&self) {
        self.live.set(false);
        let changed = !self.interrupted.replace(true);
        if let Some(next) = self.revision.get().checked_add(1) {
            self.revision.set(next);
        } else {
            self.failed.set(true);
        }
        if changed {
            let notify = self.interrupt_notifier.borrow().clone();
            if let Some(notify) = notify {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| notify())).is_err() {
                    self.failed.set(true);
                }
            }
        }
    }
    fn valid(&self, generation: u64, epoch: u64) -> bool {
        let gate = self.gate();
        generation != 0
            && self.live.get()
            && gate.running
            && !gate.recording
            && gate.epoch == epoch
            && self.generation.get() == generation
    }
    fn bump_hyper(&self) {
        if let Some(next) = self.hyper_epoch.get().checked_add(1) {
            self.hyper_epoch.set(next);
        } else {
            self.failed.set(true);
            self.interrupt();
        }
    }
    fn emission_valid(&self, revision: u64) -> bool {
        let gate = (self.gate)();
        let hyper = self.plan.borrow().hyper;
        self.live.get() && self.revision.get() == revision && hyper && gate.running
    }
    fn emit(&self, held: bool, flags: u64) -> bool {
        self.emit_mode(held, flags, false)
    }
    fn emit_mode(&self, held: bool, flags: u64, balancing: bool) -> bool {
        if self.emitting.replace(true) {
            return false;
        }
        struct Emission<'a>(&'a Cell<bool>);
        impl Drop for Emission<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let emission = Emission(&self.emitting);
        let revision = self.revision.get();
        let valid = || {
            if balancing {
                self.balance.get() == Some(flags)
            } else {
                self.emission_valid(revision)
            }
        };
        struct Owned {
            value: Ptr,
            api: Api,
        }
        impl Drop for Owned {
            fn drop(&mut self) {
                unsafe { (self.api.release)(self.value) }
            }
        }
        let result = unsafe {
            let source = (self.api.event_source)(-1);
            if source.is_null() {
                false
            } else {
                let source = Owned {
                    value: source,
                    api: self.api,
                };
                let event = (self.api.keyboard_event)(source.value, 57, held);
                if event.is_null() {
                    false
                } else {
                    let event = Owned {
                        value: event,
                        api: self.api,
                    };
                    (self.api.set_type)(event.value, 12);
                    (self.api.set_flags)(event.value, flags);
                    (self.api.set_field)(event.value, 42, keyboard_state::GENERATED_HYPER);
                    if valid() {
                        // Publish possible output before a reentrant post.
                        if !balancing {
                            self.emitted_hyper.set(held);
                        }
                        (self.api.post)(1, event.value);
                        true
                    } else {
                        false
                    }
                }
            }
        };
        drop(emission);
        if balancing {
            if result && self.balance.get() == Some(flags) {
                self.balance.set(None);
            }
        } else if let Some(pending) = self.balance.get() {
            self.emit_mode(false, pending, true);
        }
        result && (balancing || self.emission_valid(revision))
    }
    fn input(&self, value: Ptr) {
        let gate = self.gate();
        if !self.live.get() || !gate.running {
            return;
        }
        let generation = self.generation.get();
        let (usage, device, down, time) = unsafe {
            let element = (self.api.element)(value);
            if element.is_null() || (self.api.usage_page)(element) != 7 {
                return;
            }
            let usage = (self.api.usage)(element);
            if !(4..=0xe7).contains(&usage) {
                return;
            }
            let device = (self.api.device)(element);
            if device.is_null() {
                return;
            }
            (
                usage,
                device as usize as u64,
                (self.api.integer)(value) != 0,
                (self.api.uptime)(),
            )
        };
        if !self.live.get() || generation != self.generation.get() {
            return;
        }
        if unsafe { (self.api.secure)() != 0 } {
            self.interrupt();
            return;
        }
        let plan = self.plan.borrow().clone();
        if usage == 0x39 {
            self.taps.borrow_mut().reset();
        }
        let (action, changed, held, modifiers) = {
            let mut state = self.physical.borrow_mut();
            let was = state.caps_held();
            let action = state.input(device, usage, down, time);
            (
                action,
                was != state.caps_held(),
                state.caps_held(),
                state.modifiers(),
            )
        };
        if plan.hyper && changed {
            self.bump_hyper();
            if self.caps.clear().is_err() || !self.live.get() {
                self.interrupt();
                return;
            }
            if !self.emit(
                held,
                modifiers | if held { keyboard_state::HYPER } else { 0 },
            ) {
                self.interrupt();
                return;
            }
        }
        if let Some(usage) = action {
            if let Some(action) = plan.modifier_taps.get(&usage) {
                if self.valid(generation, gate.epoch) {
                    let canonical = if usage == 0xe0 {
                        "ctrl+left-control"
                    } else {
                        "alt+left-option"
                    };
                    (self.sink)(Press {
                        generation,
                        canonical: canonical.into(),
                        action: action.clone(),
                    });
                }
            }
        }
    }
    fn removed(&self, device: Ptr) {
        self.gate();
        if !self.live.get() {
            return;
        }
        let (changed, held, modifiers) = {
            let mut state = self.physical.borrow_mut();
            let was = state.caps_held();
            state.remove(device as usize as u64);
            (
                was != state.caps_held(),
                state.caps_held(),
                state.modifiers(),
            )
        };
        let hyper = self.plan.borrow().hyper;
        if hyper && changed {
            self.bump_hyper();
        }
        if hyper
            && changed
            && (self.caps.clear().is_err()
                || !self.live.get()
                || !self.emit(
                    held,
                    modifiers | if held { keyboard_state::HYPER } else { 0 },
                ))
        {
            self.interrupt();
        }
    }

    fn filter(&self, kind: u32, event: Ptr) -> Ptr {
        if kind == u32::MAX || kind == u32::MAX - 1 {
            self.interrupt();
            return event;
        }
        let gate = self.gate();
        if !self.live.get() || !gate.running {
            return event;
        }
        let generation = self.generation.get();
        let (tag, key, repeated, flags) = unsafe {
            (
                (self.api.field)(event, 42),
                (self.api.field)(event, 9),
                (self.api.field)(event, 8) != 0,
                (self.api.flags)(event),
            )
        };
        if keyboard_state::generated(tag) {
            return event;
        }
        if [1, 3, 10, 22, 25].contains(&kind) {
            self.physical.borrow_mut().cancel_tap();
        }
        if unsafe { (self.api.secure)() != 0 } {
            self.interrupt();
            return event;
        }
        if !self.live.get() {
            return event;
        }
        let plan = self.plan.borrow().clone();
        let original_flags = flags;
        let revision = self.revision.get();
        let flags = if plan.hyper {
            if kind == 12 && key == 57 {
                self.taps.borrow_mut().reset();
                if self.caps.clear().is_err() {
                    self.interrupt();
                    return event;
                }
                return if self.live.get() { null_mut() } else { event };
            }
            let held = self.physical.borrow().caps_held();
            let flags = keyboard_state::rewritten_flags(flags, held, tag);
            unsafe { (self.api.set_flags)(event, flags) };
            flags
        } else {
            flags
        };
        let allowed = self.valid(generation, gate.epoch);
        if self.revision.get() != revision || !self.live.get() {
            if plan.hyper {
                unsafe { (self.api.set_flags)(event, original_flags) }
            }
            return event;
        }
        if !allowed {
            return event;
        }
        let Ok(key) = u16::try_from(key) else {
            return event;
        };
        if plan.sided.contains_key("fn") || plan.sided.keys().any(|key| key.starts_with("double:")) {
            let modifiers = flags & (keyboard_state::HYPER | keyboard_state::FUNCTION);
            let tap = if kind == 12 {
                keyboard_state::modifier_key(key).and_then(|(specific, generic, mask, side)| {
                    let family = match generic {
                        "ctrl" => 0x2001,
                        "alt" => keyboard_state::LEFT_OPTION | keyboard_state::RIGHT_OPTION,
                        "cmd" => crate::recorder::LEFT_COMMAND | crate::recorder::RIGHT_COMMAND,
                        "shift" => 0x6,
                        _ => keyboard_state::FUNCTION,
                    };
                    let down = flags & side != 0 || (flags & family == 0 && modifiers == mask);
                    (modifiers & !mask == 0).then_some((specific, generic, down))
                })
            } else if matches!(kind, 10 | 11) {
                crate::recorder::key_name(key)
                    .filter(|name| modifiers == 0 || (modifiers == keyboard_state::FUNCTION
                        && name.strip_prefix('f').is_some_and(|number| number.parse::<u8>().is_ok())))
                    .map(|name| (name, name, kind == 10))
            } else {
                None
            };
            let mut taps = self.taps.borrow_mut();
            let admitted = tap.and_then(|(specific, generic, down)| {
                let specific_key = format!("double:{specific}");
                let generic_key = format!("double:{generic}");
                let double = plan.sided.get_key_value(&specific_key).or_else(|| plan.sided.get_key_value(&generic_key));
                let single = (specific == "fn").then(|| plan.sided.get_key_value("fn")).flatten();
                let complete = taps.observe(key, down, repeated, std::time::Instant::now())?;
                if complete { double.or(single) } else { single }
            });
            if tap.is_none() {
                taps.reset();
            }
            drop(taps);
            if let Some((canonical, action)) = admitted {
                (self.sink)(Press { generation, canonical: canonical.clone(), action: action.clone() });
            }
        }
        let event_kind = match kind {
            10 => Event::Down,
            11 => Event::Up,
            _ => Event::Other,
        };
        let decision = self
            .filter
            .borrow()
            .preview(event_kind, key, flags, repeated);
        if let Some(canonical) = decision.action.as_ref() {
            let Some(action) = plan.sided.get(canonical) else {
                return event;
            };
            // A full queue must not swallow a key whose action was not admitted.
            if !(self.sink)(Press {
                generation,
                canonical: canonical.clone(),
                action: action.clone(),
            }) || !self.valid(generation, gate.epoch)
            {
                if plan.hyper && (self.revision.get() != revision || !self.live.get()) {
                    unsafe { (self.api.set_flags)(event, original_flags) };
                }
                return event;
            }
        }
        self.filter
            .borrow_mut()
            .commit(event_kind, key, decision.consumed);
        if decision.consumed {
            null_mut()
        } else {
            event
        }
    }
}
unsafe fn retained(context: Ptr) -> Option<Rc<Callback>> {
    if context.is_null() || MainThreadMarker::new().is_none() {
        return None;
    }
    let pointer = context.cast::<Callback>();
    unsafe {
        Rc::increment_strong_count(pointer);
        Some(Rc::from_raw(pointer))
    }
}
/// Actual typed input/removal/filter entries, not a source or lifetime grant.
#[cfg(feature = "test-tools")]
pub const fn callback_entrypoints() -> (HidCallback, HidCallback, crate::hid_io::TapCallback) {
    (input, removed, filter)
}
unsafe extern "C" fn input(context: Ptr, result: i32, _sender: Ptr, value: Ptr) {
    let Some(callback) = (unsafe { retained(context) }) else {
        return;
    };
    if result != 0 || value.is_null() {
        return;
    }
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback.input(value))).is_err() {
        callback.failed.set(true);
        callback.interrupt();
    }
}
unsafe extern "C" fn removed(context: Ptr, _result: i32, _sender: Ptr, device: Ptr) {
    let Some(callback) = (unsafe { retained(context) }) else {
        return;
    };
    if device.is_null() {
        return;
    }
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback.removed(device))).is_err()
    {
        callback.failed.set(true);
        callback.interrupt();
    }
}
unsafe extern "C" fn filter(_proxy: Ptr, kind: u32, event: Ptr, context: Ptr) -> Ptr {
    let Some(callback) = (unsafe { retained(context) }) else {
        return event;
    };
    if kind == u32::MAX || kind == u32::MAX - 1 {
        callback.interrupt();
        return event;
    }
    if event.is_null() {
        return event;
    }
    let original = unsafe { (callback.api.flags)(event) };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        callback.filter(kind, event)
    })) {
        Ok(value) => value,
        Err(_) => {
            callback.failed.set(true);
            callback.interrupt();
            unsafe { (callback.api.set_flags)(event, original) };
            event
        }
    }
}
#[derive(Default)]
struct Resources {
    tap: Ptr,
    source: Ptr,
    manager: Ptr,
    runloop: Ptr,
    mode: Ptr,
}
impl Resources {
    fn empty(&self) -> bool {
        self.tap.is_null() && self.source.is_null() && self.manager.is_null()
    }
}
pub struct Service {
    callback: Rc<Callback>,
    resources: RefCell<Resources>,
    busy: Cell<bool>,
    sleeping: Cell<bool>,
    status: RefCell<String>,
    main: MainThreadMarker,
}
// If a Rust gate/sink panics during a lifecycle call, native resources and
// callback userdata must remain owned. Recovery happens on a subsequent pump.
struct ResourceGuard<'a> {
    owner: &'a Service,
    value: Option<Resources>,
}
impl std::ops::Deref for ResourceGuard<'_> {
    type Target = Resources;
    fn deref(&self) -> &Resources {
        self.value.as_ref().unwrap()
    }
}
impl std::ops::DerefMut for ResourceGuard<'_> {
    fn deref_mut(&mut self) -> &mut Resources {
        self.value.as_mut().unwrap()
    }
}
impl ResourceGuard<'_> {
    fn commit(mut self) {
        *self.owner.resources.borrow_mut() = self.value.take().unwrap();
        self.owner.busy.set(false);
    }
}
impl Drop for ResourceGuard<'_> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.owner.callback.interrupt();
            *self.owner.resources.borrow_mut() = value;
            self.owner.busy.set(false);
            *self.owner.status.borrow_mut() = "Keyboard setup interrupted; retrying".into();
        }
    }
}
impl Service {
    pub fn new(
        main: MainThreadMarker,
        fixture: bool,
        gate: Rc<dyn Fn() -> Gate>,
        sink: Rc<dyn Fn(Press) -> bool>,
    ) -> Result<Self, String> {
        let api = Api::load(main, fixture)?;
        let caps = caps_lock::Control::new(caps_io::Native::load(main, fixture)?);
        Ok(Self {
            callback: Rc::new(Callback {
                api,
                caps,
                live: Cell::new(false),
                interrupted: Cell::new(false),
                failed: Cell::new(false),
                generation: Cell::new(0),
                plan: RefCell::new(Rc::new(Plan::default())),
                physical: RefCell::new(Physical::default()),
                taps: RefCell::new(keyboard_state::TapSequence::default()),
                filter: RefCell::new(CommandFilter::default()),
                epoch: Cell::new(0),
                emitted_hyper: Cell::new(false),
                balance: Cell::new(None),
                emitting: Cell::new(false),
                revision: Cell::new(0),
                hyper_epoch: Cell::new(0),
                gate,
                sink,
                interrupt_notifier: RefCell::new(None),
                #[cfg(feature = "test-tools")]
                source_ticket: Cell::new(None),
                #[cfg(feature = "test-tools")]
                source_uncertain: Cell::new(false),
            }),
            resources: RefCell::new(Resources::default()),
            busy: Cell::new(false),
            sleeping: Cell::new(false),
            status: RefCell::new("Off".into()),
            main,
        })
    }
    pub fn status(&self) -> String {
        let status = self.status.borrow().clone();
        if self.callback.failed.get() {
            "Keyboard callback failed".into()
        } else if self.callback.interrupted.get() && status == "On" {
            "Keyboard filter interrupted; retrying".into()
        } else {
            status
        }
    }
    pub fn hyper_epoch(&self) -> u64 {
        self.callback.hyper_epoch.get()
    }
    pub fn hyper_held(&self) -> bool {
        self.callback.plan.borrow().hyper && self.callback.physical.borrow().caps_held()
    }
    pub fn failed(&self) -> bool {
        self.callback.failed.get()
    }
    pub fn set_interrupt_notifier(&self, notify: Rc<dyn Fn()>) {
        *self.callback.interrupt_notifier.borrow_mut() = Some(notify);
    }
    /// Main dispatch rechecks secure input after the original tap callback,
    /// matching the old asynchronous sided-action guard before delivery.
    pub fn delivery_allowed(&self) -> bool {
        if unsafe { (self.callback.api.secure)() != 0 } {
            self.callback.interrupt();
            return false;
        }
        let gate = self.callback.gate();
        self.callback.live.get() && gate.running && !gate.recording
    }
    pub fn retired(&self) -> bool {
        !self.busy.get()
            && !self.callback.emitting.get()
            && self.resources.borrow().empty()
            && self.callback.balance.get().is_none()
            && !self.callback.emitted_hyper.get()
            && self.callback.caps.retired()
    }
    pub fn recovery_pending(main: MainThreadMarker) -> bool {
        RECOVERING.with(Cell::get)
            || RETIRED.with(|q| !q.borrow().is_empty())
            || caps_io::Native::recovery_pending(main)
    }
    pub fn sleeping(&self, value: bool) {
        self.sleeping.set(value);
        if value && self.stop() {
            *self.status.borrow_mut() = "Paused".into();
        }
    }
    pub fn update(&self, generation: u64, plan: Plan) {
        let changed = {
            let old = self.callback.plan.borrow();
            old.hyper != plan.hyper
                || old.sided != plan.sided
                || old.modifier_taps != plan.modifier_taps
        };
        if changed {
            self.stop();
        }
        self.callback.generation.set(generation);
        if changed {
            let bindings = plan
                .sided
                .iter()
                .filter_map(|(key, _)| keyboard_state::Binding::parse(key, key.clone()).ok())
                .collect();
            self.callback.filter.borrow_mut().set_bindings(bindings);
            *self.callback.plan.borrow_mut() = Rc::new(plan);
        }
    }
    fn retire(callback: &Callback, resources: &mut Resources) -> bool {
        let balance = &callback.balance;
        let api = callback.api;
        if callback.live.replace(false) {
            callback.interrupt();
        }
        let modifiers = callback.physical.borrow().modifiers();
        if callback.emitted_hyper.replace(false) {
            balance.set(Some(modifiers));
        }
        callback.physical.borrow_mut().reset();
        callback.taps.borrow_mut().reset();
        callback.filter.borrow_mut().reset();
        if let Some(flags) = balance.get() {
            if callback.emit_mode(false, flags, true) {
                balance.set(None);
            }
        }
        unsafe {
            if !resources.manager.is_null() {
                (api.unschedule)(resources.manager, resources.runloop, resources.mode);
                (api.register_input)(resources.manager, None, null_mut());
                (api.register_remove)(resources.manager, None, null_mut());
                if (api.manager_close)(resources.manager, 0) == 0 {
                    (api.release)(resources.manager);
                    resources.manager = null_mut();
                }
            }
            if !resources.source.is_null() {
                (api.remove_source)(resources.runloop, resources.source, resources.mode);
                (api.release)(resources.source);
                resources.source = null_mut();
            }
            if !resources.tap.is_null() {
                (api.tap_enable)(resources.tap, false);
                (api.invalidate)(resources.tap);
                if (api.port_valid)(resources.tap) == 0 {
                    (api.release)(resources.tap);
                    resources.tap = null_mut();
                }
            }
        }
        let caps = callback.caps.retire().is_ok();
        #[cfg(feature = "test-tools")]
        let source = callback.revoke_source_ticket();
        #[cfg(not(feature = "test-tools"))]
        let source = true;
        resources.empty() && caps && balance.get().is_none() && source
    }
    pub fn stop(&self) -> bool {
        self.callback.interrupt();
        if self.busy.replace(true) {
            return false;
        }
        let mut resources = ResourceGuard {
            owner: self,
            value: Some(std::mem::take(&mut *self.resources.borrow_mut())),
        };
        let result = Self::retire(&self.callback, &mut resources);
        resources.commit();
        *self.status.borrow_mut() = if result {
            "Off"
        } else {
            "Keyboard cleanup interrupted; retrying"
        }
        .into();
        result
    }
    pub fn refresh(&self) {
        if self.busy.replace(true) {
            return;
        }
        let callback = &self.callback;
        let api = callback.api;
        let mut resources = ResourceGuard {
            owner: self,
            value: Some(std::mem::take(&mut *self.resources.borrow_mut())),
        };
        let result = (|| {
            if !Self::recover(self.main) {
                return Err("Keyboard recovery pending".into());
            }
            let plan = callback.plan.borrow().clone();
            let gate = callback.gate();
            // Preserve the production short-circuit precedence: Off/sleep
            // never performs application or permission discovery.
            let mut environment = Environment {
                sleeping: self.sleeping.get() || !gate.running,
                raycast_running: false,
                secure_input: false,
                accessibility: true,
                input_monitoring: true,
            };
            if (plan.hyper || !plan.modifier_taps.is_empty() || !plan.sided.is_empty())
                && !environment.sleeping
            {
                unsafe {
                    environment.raycast_running = (api.raycast)();
                    if !environment.raycast_running {
                        environment.secure_input = (api.secure)() != 0;
                        if !environment.secure_input {
                            environment.accessibility = (api.accessibility)() != 0;
                            if environment.accessibility {
                                environment.input_monitoring = (api.monitoring)();
                            }
                        }
                    }
                }
            }
            if let Some(reason) = environment.blocked(
                plan.hyper,
                !plan.modifier_taps.is_empty(),
                !plan.sided.is_empty(),
            ) {
                if !Self::retire(callback, &mut resources) {
                    return Err("Keyboard cleanup interrupted; retrying".into());
                }
                return Ok(reason.into());
            }
            if callback.failed.get() {
                return Err("Keyboard callback failed".into());
            }
            if callback.live.get()
                && !callback.interrupted.get()
                && !resources.tap.is_null()
                && unsafe { (api.tap_enabled)(resources.tap) }
            {
                if plan.hyper {
                    callback
                        .caps
                        .reconcile()
                        .map_err(|_| "Caps Lock control unavailable".to_string())?;
                    if !callback.live.get() || callback.interrupted.get() {
                        return Err("Keyboard refresh interrupted; retrying".into());
                    }
                }
                return Ok("On".into());
            }
            if !Self::retire(callback, &mut resources) {
                return Err("Keyboard cleanup interrupted; retrying".into());
            }
            callback.interrupted.set(false);
            macro_rules! ready {
                () => {
                    if callback.interrupted.get() || !(callback.gate)().running {
                        return Err("Keyboard setup interrupted; retrying".into());
                    }
                };
            }
            if plan.hyper {
                callback
                    .caps
                    .activate()
                    .map_err(|_| "Caps Lock control unavailable".to_string())?;
                ready!();
            }
            #[cfg(feature = "test-tools")]
            Callback::publish_source_ticket(callback.clone())?;
            #[cfg(feature = "test-tools")]
            ready!();
            unsafe {
                resources.runloop = (api.runloop)();
                resources.mode = (api.common)();
                if resources.runloop.is_null() || resources.mode.is_null() {
                    return Err("Keyboard run loop unavailable".into());
                }
                let context = Rc::as_ptr(callback).cast_mut().cast();
                let mask = [1, 3, 10, 11, 12, 22, 25]
                    .iter()
                    .fold(0, |mask, kind| mask | (1u64 << kind));
                resources.tap = (api.tap_create)(1, 0, 0, mask, filter, context);
                if resources.tap.is_null() {
                    return Err("Keyboard filter unavailable; check Accessibility".into());
                }
                ready!();
                resources.source = (api.source)(null_mut(), resources.tap, 0);
                if resources.source.is_null() {
                    return Err("Keyboard run loop source unavailable".into());
                }
                (api.add_source)(resources.runloop, resources.source, resources.mode);
                ready!();
                resources.manager = (api.manager_create)(null_mut(), 0);
                if resources.manager.is_null() {
                    return Err("Keyboard input manager unavailable".into());
                }
                ready!();
                for (device, method) in [(true, api.device_matching), (false, api.input_matching)] {
                    let dictionary = (api.dictionary)(device);
                    if dictionary.is_null() {
                        return Err("Keyboard matching unavailable".into());
                    }
                    method(resources.manager, dictionary);
                    (api.release)(dictionary);
                    ready!();
                }
                (api.register_input)(resources.manager, Some(input as HidCallback), context);
                (api.register_remove)(resources.manager, Some(removed as HidCallback), context);
                (api.schedule)(resources.manager, resources.runloop, resources.mode);
                ready!();
                if (api.manager_open)(resources.manager, 0) != 0 {
                    return Err("Keyboard input unavailable; check Input Monitoring".into());
                }
                ready!();
                (api.tap_enable)(resources.tap, true);
                ready!();
                if !(api.tap_enabled)(resources.tap) {
                    return Err("Keyboard filter could not be enabled".into());
                }
                ready!();
            }
            if plan.hyper {
                callback
                    .caps
                    .commit_off()
                    .map_err(|_| "Caps Lock control unavailable".to_string())?;
            }
            callback.live.set(true);
            Ok("On".into())
        })();
        let status = match result {
            Ok(status) => status,
            Err(error) => {
                if Self::retire(callback, &mut resources) {
                    error
                } else {
                    "Keyboard cleanup interrupted; retrying".into()
                }
            }
        };
        *self.status.borrow_mut() = status;
        resources.commit();
    }
    /// Main refresh/shutdown must keep this scheduled even when all features
    /// are Off. Retired native resources continue to own their callback context.
    pub fn recover(main: MainThreadMarker) -> bool {
        if RECOVERING.with(|busy| busy.replace(true)) {
            return false;
        }
        let retired = RETIRED.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
        for (callback, mut resources) in retired {
            if !Self::retire(&callback, &mut resources) {
                RETIRED.with(|queue| queue.borrow_mut().push((callback, resources)));
            }
        }
        let caps = caps_io::Native::retry_recovery(main).is_ok();
        RECOVERING.with(|busy| busy.set(false));
        caps && RETIRED.with(|queue| queue.borrow().is_empty())
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if !self.stop() {
            // An unretired callback context must outlive native resources.
            // The main recovery queue owns both, rather than freeing userdata.
            RETIRED.with(|queue| {
                queue.borrow_mut().push((
                    self.callback.clone(),
                    std::mem::take(self.resources.get_mut()),
                ))
            });
        }
    }
}
type Retired = (Rc<Callback>, Resources);
thread_local! {
    static RETIRED:RefCell<Vec<Retired>>=const{RefCell::new(Vec::new())};
    static RECOVERING:Cell<bool>=const{Cell::new(false)};
}
