//! Main-owned keyword input, guarded plain/rich output and deferred original-target choices.
use crate::{
    keyword_effects,
    keyword_io::Ptr,
    keyword_matcher::{self, Catalog, Matcher, Replacement},
    keyword_target::{Context, Target},
    templates,
};
use objc2::MainThreadMarker;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ptr::null_mut,
    rc::{Rc, Weak},
    sync::Arc,
    time::{Duration, Instant},
};

mod choice;
pub struct Prepared {
    theme: Option<String>,
    catalog: Catalog,
    choices: std::collections::HashMap<String, Vec<Arc<crate::snippet_choice::Prepared>>>,
    clipboard: HashSet<String>,
    pub enabled: bool,
    excluded: HashSet<String>,
}
impl Prepared {
    /// Background configuration preparation; never parses templates in the tap.
    pub fn new(config: &rustcast_core::Config) -> Result<Self, String> {
        objc2::rc::autoreleasepool(|_| {
            let catalog = Catalog::new(&config.commands)?;
            let mut choices: std::collections::HashMap<
                String,
                Vec<Arc<crate::snippet_choice::Prepared>>,
            > = Default::default();
            for command in &config.commands {
                if command.enabled != Some(false)
                    && command.kind == "snippet"
                    && catalog
                        .plan(&command.id)
                        .is_some_and(|p| p.needs_interaction())
                {
                    choices
                        .entry(command.id.clone())
                        .or_default()
                        .push(Arc::new(crate::snippet_choice::Prepared::new(
                            command.clone(),
                        )?));
                }
            }
            Ok(Self {
                theme: config.theme.clone(),
                catalog,
                choices,
                clipboard: config
                    .commands
                    .iter()
                    .filter(|c| templates::needs_clipboard(&c.target))
                    .map(|c| c.id.clone())
                    .collect(),
                enabled: config.snippet_expansion_enabled,
                excluded: config
                    .clipboard
                    .excluded_apps
                    .iter()
                    .map(|v| templates::canonical(v))
                    .collect(),
            })
        })
    }
    /// Original prepared data, not a freshly rendered command in the tap.
    /// Duplicate raw IDs refuse admission rather than choosing an identity.
    pub fn choice(&self, id: &str) -> Option<&Arc<crate::snippet_choice::Prepared>> {
        self.choices
            .get(id)
            .filter(|v| v.len() == 1)
            .and_then(|v| v.first())
    }
    pub fn contains_choice(&self, original: &crate::command_identity::Identity) -> bool {
        self.enabled
            && self
                .choices
                .values()
                .flatten()
                .any(|p| p.identity().as_ref() == original)
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Gate {
    pub running: bool,
    pub blocked: bool,
    pub hyper_held: bool,
    pub hyper_epoch: u64,
    pub activation_epoch: u64,
    pub stage_epoch: u64,
    pub generation: u64,
    pub recording_epoch: u64,
    pub ui_epoch: u64,
    pub ui_intent: u64,
    pub ui_dismissal: u64,
}
struct Lease {
    owner: Weak<Callback>,
    revision: u64,
    gate: Gate,
    input: u64,
}
impl Lease {
    fn live(&self) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|c| c.input_epoch.get() == self.input && c.valid(self.revision, self.gate))
    }
}
struct PendingCursor {
    due: Instant,
    lease: Lease,
    cursor: keyword_effects::Cursor,
}
pub(crate) struct Callback {
    analytics: RefCell<Rc<dyn Fn() -> Option<crate::analytics::Client>>>,
    feedback: RefCell<Rc<dyn Fn(Failure, Gate)>>,
    main: MainThreadMarker,
    choice: RefCell<Option<Rc<choice::Request>>>,
    choice_lives: Rc<Cell<usize>>,
    weak: Weak<Callback>,
    input_epoch: Cell<u64>,
    cursor: RefCell<Option<PendingCursor>>,
    clipboard_possible: Cell<bool>,
    clipboard_partial: Cell<u64>,
    own: Rc<crate::clipboard_ownership::OwnWrites>,
    context: Context,
    live: Cell<bool>,
    observation_only: Cell<bool>,
    observation_epoch: Cell<u64>,
    failed: Cell<bool>,
    interrupted: Cell<bool>,
    revision: Cell<u64>,
    generation: Cell<u64>,
    prepared: RefCell<Option<Arc<Prepared>>>,
    matcher: RefCell<Matcher>,
    target: RefCell<Option<Target>>,
    last_gate: Cell<Option<Gate>>,
    processing: Cell<bool>,
    claimed: Cell<bool>,
    gate: Rc<dyn Fn() -> Gate>,
    notify: Rc<dyn Fn()>,
    effect_possible: Cell<bool>,
    completed: Cell<u64>,
    #[cfg(feature = "test-tools")]
    source_ticket: Cell<Option<crate::owned_host_entry::NativeInputSourceTicket>>,
    #[cfg(feature = "test-tools")]
    source_uncertain: Cell<bool>,
    partial: Cell<u64>,
    unsupported: Cell<u64>,
    choice_activation: RefCell<Option<crate::workspace_activation::Sink>>,
}
impl Callback {
    #[cfg(feature = "test-tools")]
    fn publish_source_ticket(owner: Rc<Self>) -> Result<(), &'static str> {
        let (owner, ticket) = crate::owned_host_entry::NativeInputSourceTicket::for_keyword(owner);
        match ticket {
            Ok(ticket) => { owner.source_ticket.set(ticket); Ok(()) }
            Err(_) => { owner.source_uncertain.set(true); Err("Keyword cleanup pending") }
        }
    }
    #[cfg(feature = "test-tools")]
    fn revoke_source_ticket(&self) -> bool {
        if self.source_uncertain.get() { return false; }
        let Some(mut ticket) = self.source_ticket.take() else { return true; };
        if ticket.revoke().is_ok() { true } else { self.source_ticket.set(Some(ticket)); false }
    }
    fn analytics_span(&self, id: &str) -> Option<crate::analytics::Span> {
        let get = self.analytics.borrow().clone();
        get().and_then(|client| crate::analytics::Span::new(client, id, "snippet"))
    }
    fn feedback(&self, failure: Failure, gate: Gate) {
        let callback = self.feedback.borrow().clone();
        callback(failure, gate);
    }
    fn cancel_choice(&self) {
        let request = self.choice.borrow_mut().take();
        if let Some(request) = request {
            request.cancel(false);
        }
    }
    fn suspend_effects(&self) -> Option<u64> {
        self.observation_only.set(true);
        let Some(next) = self.revision.get().checked_add(1) else {
            self.failed.set(true);
            self.live.set(false);
            return None;
        };
        self.revision.set(next);
        self.cancel_cursor();
        self.reset();
        // Return the revision established BEFORE any native resource release;
        // reentry must not silently become the caller's new expected revision.
        Some(next)
    }
    fn cancel_cursor(&self) {
        let old = self.cursor.borrow_mut().take();
        drop(old);
    }
    fn lease(&self, revision: u64, gate: Gate) -> Lease {
        Lease {
            owner: self.weak.clone(),
            revision,
            gate,
            input: self.input_epoch.get(),
        }
    }
    fn reset(&self) {
        let old = self.target.borrow_mut().take();
        self.matcher.borrow_mut().reset();
        self.last_gate.set(None);
        drop(old);
    }
    fn interrupt(&self) {
        self.note_observation_gap();
        self.live.set(false);
        self.cancel_choice();
        self.interrupted.set(true);
        match self.revision.get().checked_add(1) {
            Some(n) => self.revision.set(n),
            None => self.failed.set(true),
        };
        self.cancel_cursor();
        self.reset();
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.notify)())).is_err() {
            self.failed.set(true);
        }
    }
    fn valid(&self, revision: u64, gate: Gate) -> bool {
        self.live.get()
            && !self.observation_only.get()
            && !self.failed.get()
            && self.revision.get() == revision
            && self.generation.get() == gate.generation
            && gate.generation != 0
            && gate.running
            && !gate.blocked
            && !gate.hyper_held
            && (self.gate)() == gate
    }
    fn note_observation_gap(&self) {
        self.live.set(false);
        if let Some(next) = self.observation_epoch.get().checked_add(1) {
            self.observation_epoch.set(next);
        } else {
            self.failed.set(true);
        }
    }
    fn handle(&self, kind: u32, event: Ptr) -> bool {
        let api = self.context.api;
        if kind == u32::MAX || kind == u32::MAX - 1 {
            self.interrupt();
            return false;
        }
        if event.is_null() {
            return false;
        }
        let tag = unsafe { (api.field)(event, 42) };
        if tag == keyword_effects::GENERATED_TAG {
            return false;
        }
        if let Some(next) = self.input_epoch.get().checked_add(1) {
            self.input_epoch.set(next);
        } else {
            self.failed.set(true);
            self.interrupt();
            return false;
        }
        self.cancel_cursor();
        let request = self.choice.borrow().clone();
        if let Some(request) = request {
            request.input(self.input_epoch.get(), kind, tag);
            self.reset();
            (self.notify)();
            return false;
        }
        if tag == crate::keyboard_state::GENERATED_HYPER {
            self.reset();
            return false;
        }
        let gate = (self.gate)();
        let revision = self.revision.get();
        let valid = || self.valid(revision, gate);
        if !valid() || kind != 10 {
            self.reset();
            return false;
        }
        if self.last_gate.replace(Some(gate)) != Some(gate) {
            self.matcher.borrow_mut().reset();
        }
        let Some(prepared) = self.prepared.borrow().clone() else {
            return false;
        };
        if !prepared.enabled {
            self.reset();
            return false;
        }
        let flags = self
            .context
            .call(&valid, || unsafe { (api.flags)(event) })
            .ok();
        if flags.is_none_or(|f| f & ((1 << 18) | (1 << 19) | (1 << 20)) != 0)
            || self
                .context
                .call(&valid, || unsafe { (api.field)(event, 8) })
                .ok()
                != Some(0)
        {
            self.reset();
            return false;
        }
        if !self.context.safe_field(&valid).unwrap_or(false) {
            self.reset();
            return false;
        }
        let old = self.target.borrow().clone();
        let target = match old {
            Some(old) if old.current(&valid).unwrap_or(false) => Some(old),
            _ => {
                self.matcher.borrow_mut().reset();
                self.context.frontmost(&valid).ok().flatten()
            }
        };
        let Some(target) = target else {
            self.reset();
            return false;
        };
        let bundle = target.bundle().unwrap_or("");
        let excluded = if bundle.is_ascii() {
            prepared.excluded.contains(bundle)
        } else {
            prepared.excluded.contains(&templates::canonical(bundle))
        };
        if target.pid() == std::process::id() as i32 || excluded || !valid() {
            self.reset();
            return false;
        }
        let previous = self.target.replace(Some(target.clone()));
        drop(previous);
        let Some(code) = self
            .context
            .call(&valid, || unsafe { (api.field)(event, 9) })
            .ok()
        else {
            self.reset();
            return false;
        };
        if code == 51 {
            self.matcher.borrow_mut().backspace();
            return false;
        }
        if [123, 124, 125, 126, 115, 116, 117, 119, 121, 53].contains(&code) {
            self.reset();
            return false;
        }
        let mut units = [0u16; 16];
        let mut count = 0;
        if self
            .context
            .call(&valid, || unsafe {
                (api.unicode)(event, 16, &mut count, units.as_mut_ptr())
            })
            .is_err()
            || count == 0
            || count > 16
        {
            self.reset();
            return false;
        }
        let text = String::from_utf16_lossy(&units[..count]);
        // No matcher borrow survives normalization/native Foundation calls.
        let mut matcher = std::mem::take(&mut *self.matcher.borrow_mut());
        let command = matcher.type_text(&text, &prepared.catalog);
        if !valid() {
            self.reset();
            return false;
        }
        *self.matcher.borrow_mut() = matcher;
        let Some(command) = command else {
            return false;
        };
        let Some(plan) = prepared.catalog.plan(&command.id) else {
            return false;
        };
        if plan.needs_interaction() {
            let prepared_choice = prepared.choice(&command.id).cloned();
            if let Some((owner, prepared_choice)) = self.weak.upgrade().zip(prepared_choice) {
                let request =
                    choice::Request::new(&owner, self.main, prepared_choice, target, gate);
                if valid() {
                    let old = self.choice.replace(request);
                    drop(old);
                    self.reset();
                    (self.notify)();
                } else if let Some(request) = request {
                    request.cancel(false);
                }
                return false;
            }
            self.unsupported
                .set(self.unsupported.get().saturating_add(1));
            return false;
        }
        let clipboard = if prepared.clipboard.contains(&command.id) {
            match self.context.clipboard(&valid) {
                Ok(v) => v,
                Err(_) => return false,
            }
        } else {
            None
        };
        let Ok((rendered, cursor)) =
            plan.render(&[], &templates::Context::current(clipboard.as_deref()))
        else {
            return false;
        };
        let replacement = Replacement::new(
            command.keyword.as_deref().unwrap_or(""),
            &text,
            rendered,
            cursor,
        );
        let analytics = self.analytics_span(&command.id);
        let outcome = if let Some(mode @ ("teams" | "html")) = command.snippet_output.as_deref() {
            let result = keyword_effects::apply_rich(
                self.context,
                &target,
                &replacement,
                mode,
                &valid,
                keyword_effects::RichEffects {
                    keyboard_possible: &self.effect_possible,
                    clipboard_possible: &self.clipboard_possible,
                    own: &self.own,
                },
            );
            if let Some(cursor) = result.cursor {
                let lease = self.lease(revision, gate);
                if lease.live() {
                    let old = self.cursor.replace(Some(PendingCursor {
                        due: Instant::now() + Duration::from_millis(80),
                        lease,
                        cursor,
                    }));
                    drop(old);
                    (self.notify)();
                }
            }
            if self.clipboard_possible.get() && !result.outcome.complete {
                self.clipboard_partial
                    .set(self.clipboard_partial.get().saturating_add(1));
            }
            result.outcome
        } else {
            keyword_effects::apply_plain(
                self.context,
                &target,
                &replacement,
                &valid,
                &self.effect_possible,
            )
        };
        if let Some(span) = analytics {
            span.finish(outcome.complete, "action-failed");
        }
        if !outcome.complete {
            self.feedback(
                if outcome.keyboard_possible || self.clipboard_possible.get() {
                    Failure::Partial
                } else {
                    Failure::Copy
                },
                gate,
            );
        }
        if outcome.complete {
            self.completed.set(self.completed.get().saturating_add(1));
        } else if outcome.keyboard_possible {
            self.partial.set(self.partial.get().saturating_add(1));
        }
        outcome.consume_trigger()
    }
}
/// Actual typed filter entry for Input5's same-build compiler/source join.
#[cfg(feature = "test-tools")]
pub const fn callback_entrypoints() -> crate::keyword_io::TapCallback {
    filter
}
unsafe extern "C" fn filter(_proxy: Ptr, kind: u32, event: Ptr, context: Ptr) -> Ptr {
    if context.is_null() || MainThreadMarker::new().is_none() {
        return event;
    }
    let pointer = context.cast::<Callback>();
    let callback = unsafe {
        Rc::increment_strong_count(pointer);
        Rc::from_raw(pointer)
    };
    if callback.processing.replace(true) {
        if !event.is_null()
            && unsafe { (callback.context.api.field)(event, 42) } != keyword_effects::GENERATED_TAG
        {
            callback.interrupt();
        }
        return event;
    }
    struct Processing<'a>(&'a Cell<bool>);
    impl Drop for Processing<'_> {
        fn drop(&mut self) {
            self.0.set(false);
        }
    }
    let _processing = Processing(&callback.processing);
    callback.effect_possible.set(false);
    callback.clipboard_possible.set(false);
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        callback.handle(kind, event)
    })) {
        Ok(true) => null_mut(),
        Ok(false) => event,
        Err(_) => {
            if callback.clipboard_possible.get() {
                callback
                    .clipboard_partial
                    .set(callback.clipboard_partial.get().saturating_add(1));
            }
            if callback.effect_possible.get() {
                callback
                    .partial
                    .set(callback.partial.get().saturating_add(1));
            }
            callback.failed.set(true);
            callback.interrupt();
            if callback.effect_possible.get() {
                null_mut()
            } else {
                event
            }
        }
    }
}
#[derive(Default)]
struct Resources {
    tap: Ptr,
    source: Ptr,
    runloop: Ptr,
    mode: Ptr,
}
impl Resources {
    fn empty(&self) -> bool {
        self.tap.is_null() && self.source.is_null()
    }
}
pub struct Service {
    callback: Rc<Callback>,
    resources: RefCell<Resources>,
    busy: Cell<bool>,
    status: RefCell<String>,
    main: MainThreadMarker,
}
#[derive(Clone, Copy, Debug)]
pub enum Failure {
    AppUnavailable,
    TextChanged,
    Copy,
    Partial,
}
impl Failure {
    pub fn text(self) -> &'static str {
        match self {
            Self::AppUnavailable => "The app is unavailable. Type the keyword again.",
            Self::TextChanged => "The text field changed. Type the keyword again.",
            Self::Copy => "Could not copy snippet.",
            Self::Partial => "Snippet insertion may be incomplete. Check the text before retrying.",
        }
    }
}
struct Loan<'a> {
    owner: &'a Service,
    value: Option<Resources>,
}
impl std::ops::Deref for Loan<'_> {
    type Target = Resources;
    fn deref(&self) -> &Resources {
        self.value.as_ref().unwrap()
    }
}
impl std::ops::DerefMut for Loan<'_> {
    fn deref_mut(&mut self) -> &mut Resources {
        self.value.as_mut().unwrap()
    }
}
impl Loan<'_> {
    fn commit(mut self) {
        *self.owner.resources.borrow_mut() = self.value.take().unwrap();
        self.owner.busy.set(false);
    }
}
impl Drop for Loan<'_> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.owner.callback.interrupt();
            *self.owner.resources.borrow_mut() = value;
            self.owner.busy.set(false);
            *self.owner.status.borrow_mut() = "Keyword cleanup pending".into();
        }
    }
}
impl Service {
    pub fn new(
        main: MainThreadMarker,
        fixture: bool,
        gate: Rc<dyn Fn() -> Gate>,
        notify: Rc<dyn Fn()>,
    ) -> Result<Self, String> {
        Self::with_ownership(
            main,
            fixture,
            gate,
            notify,
            Rc::new(crate::clipboard_ownership::OwnWrites::default()),
        )
    }
    pub fn with_ownership(
        main: MainThreadMarker,
        fixture: bool,
        gate: Rc<dyn Fn() -> Gate>,
        notify: Rc<dyn Fn()>,
        own: Rc<crate::clipboard_ownership::OwnWrites>,
    ) -> Result<Self, String> {
        let context = Context::new(main, fixture)?;
        Ok(Self {
            callback: Rc::new_cyclic(|weak| Callback {
                analytics: RefCell::new(Rc::new(|| None)),
                feedback: RefCell::new(Rc::new(|_, _| {})),
                main,
                choice: RefCell::new(None),
                choice_lives: Rc::new(Cell::new(0)),
                weak: weak.clone(),
                input_epoch: Cell::new(0),
                cursor: RefCell::new(None),
                clipboard_possible: Cell::new(false),
                clipboard_partial: Cell::new(0),
                own,
                context,
                live: Cell::new(false),
                observation_only: Cell::new(true),
                observation_epoch: Cell::new(0),
                failed: Cell::new(false),
                interrupted: Cell::new(false),
                revision: Cell::new(0),
                generation: Cell::new(0),
                prepared: RefCell::new(None),
                matcher: RefCell::new(Matcher::default()),
                target: RefCell::new(None),
                last_gate: Cell::new(None),
                processing: Cell::new(false),
                claimed: Cell::new(false),
                gate,
                notify,
                effect_possible: Cell::new(false),
                completed: Cell::new(0),
                #[cfg(feature = "test-tools")]
                source_ticket: Cell::new(None),
                #[cfg(feature = "test-tools")]
                source_uncertain: Cell::new(false),
                partial: Cell::new(0),
                unsupported: Cell::new(0),
                choice_activation: RefCell::new(None),
            }),
            resources: RefCell::new(Resources::default()),
            busy: Cell::new(false),
            status: RefCell::new("Off".into()),
            main,
        })
    }
    pub fn set_reporting(
        &self,
        analytics: Rc<dyn Fn() -> Option<crate::analytics::Client>>,
        feedback: Rc<dyn Fn(Failure, Gate)>,
    ) {
        *self.callback.analytics.borrow_mut() = analytics;
        *self.callback.feedback.borrow_mut() = feedback;
    }
    pub fn update(&self, generation: u64, prepared: Arc<Prepared>) {
        let observe = self.callback.live.get()
            && !self.callback.interrupted.get()
            && !self.callback.failed.get()
            && generation != 0
            && prepared.enabled;
        self.callback.generation.set(generation);
        let old = self.callback.prepared.replace(Some(prepared));
        drop(old);
        if observe {
            // A new catalog revokes every old prefix/effect lease without an
            // input-observation gap. A live tap alone never refreshes a lease.
            if let Some(next) = self.callback.suspend_effects() {
                if self.callback.revision.get() == next
                    && self.callback.live.get()
                    && !self.callback.interrupted.get()
                {
                    self.callback.observation_only.set(false);
                }
            } else {
                self.callback.failed.set(true);
                self.callback.interrupt();
            }
        } else {
            self.callback.interrupt();
        }
    }
    pub fn next_wake(&self) -> Option<Instant> {
        let cursor = self.callback.cursor.borrow().as_ref().map(|v| v.due);
        let choice = self
            .callback
            .choice
            .borrow()
            .as_ref()
            .and_then(|v| v.deadline());
        cursor.into_iter().chain(choice).min()
    }
    /// The future native choice request owner attaches here. The host delivers
    /// every retained typed event synchronously, even while config is pending.
    pub fn set_choice_activation_handler(
        &self,
        handler: Option<crate::workspace_activation::Sink>,
    ) {
        let old = self.callback.choice_activation.replace(handler);
        drop(old);
    }
    pub fn workspace_activated(&self, event: crate::workspace_activation::Event) {
        let callback = self.callback.clone();
        // Revoke ordinary replacement leases before native destruction or a
        // choice owner's possibly reentrant callback. This is not a tap gap.
        match callback.revision.get().checked_add(1) {
            Some(next) => callback.revision.set(next),
            None => callback.failed.set(true),
        }
        let handler = callback.choice_activation.borrow().clone();
        let request = callback.choice.borrow().clone();
        if let Some(request) = request {
            request.activation(&event);
        }
        if let Some(handler) = handler {
            handler(event);
        }
        callback.cancel_cursor();
        callback.reset();
    }
    pub fn own_active(&self) -> bool {
        self.callback.own.active()
    }
    pub fn observation_epoch(&self) -> u64 {
        self.callback.observation_epoch.get()
    }
    pub fn observing(&self) -> bool {
        self.callback.live.get() && !self.callback.interrupted.get() && !self.callback.failed.get()
    }
    pub fn allows_capture(&self, count: isize) -> bool {
        self.callback.own.allows_capture(count)
    }
    pub fn own_change(&self) -> Option<isize> {
        self.callback.own.get()
    }
    pub fn clipboard_partials(&self) -> u64 {
        self.callback.clipboard_partial.get()
    }
    pub fn pump(&self) {
        if self.callback.processing.replace(true) {
            return;
        }
        struct Processing<'a>(&'a Cell<bool>);
        impl Drop for Processing<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _processing = Processing(&self.callback.processing);
        let request = self.callback.choice.borrow().clone();
        if let Some(request) = request {
            self.callback.effect_possible.set(false);
            self.callback.clipboard_possible.set(false);
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| request.pump())).is_err() {
                if self.callback.effect_possible.get() {
                    self.callback
                        .partial
                        .set(self.callback.partial.get().saturating_add(1));
                }
                if self.callback.clipboard_possible.get() {
                    self.callback
                        .clipboard_partial
                        .set(self.callback.clipboard_partial.get().saturating_add(1));
                }
                self.callback.failed.set(true);
                self.callback.interrupt();
            }
            if request.finished() {
                let same = self
                    .callback
                    .choice
                    .borrow()
                    .as_ref()
                    .is_some_and(|r| Rc::ptr_eq(r, &request));
                if same {
                    let old = self.callback.choice.borrow_mut().take();
                    drop(old);
                }
            }
        }
        let due = self.next_wake().is_some_and(|v| v <= Instant::now());
        if !due {
            return;
        }
        let pending = self.callback.cursor.borrow_mut().take();
        let Some(pending) = pending else {
            return;
        };
        if !pending.lease.live() {
            return;
        }
        let possible = Cell::new(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (outcome, next) = pending.cursor.batch(&|| pending.lease.live(), &possible);
            if let Some(cursor) = next {
                if pending.lease.live() {
                    let old = self.callback.cursor.replace(Some(PendingCursor {
                        due: Instant::now(),
                        lease: pending.lease,
                        cursor,
                    }));
                    drop(old);
                    (self.callback.notify)();
                }
            }
            let _ = outcome;
        }));
        if result.is_err() {
            if possible.get() {
                self.callback
                    .partial
                    .set(self.callback.partial.get().saturating_add(1));
            }
            self.callback.failed.set(true);
            self.callback.interrupt();
        }
    }
    pub fn generation(&self) -> u64 {
        self.callback.generation.get()
    }
    pub fn status(&self) -> String {
        if self.callback.failed.get() {
            "Keyword callback failed".into()
        } else if !self.callback.live.get() && self.status.borrow().as_str() == "Noninteractive" {
            "Keyword filter recovering".into()
        } else {
            self.status.borrow().clone()
        }
    }
    pub fn counters(&self) -> (u64, u64, u64) {
        (
            self.callback.completed.get(),
            self.callback.partial.get(),
            self.callback.unsupported.get(),
        )
    }
    pub fn retired(&self) -> bool {
        !self.busy.get()
            && !self.callback.processing.get()
            && self.resources.borrow().empty()
            && self.callback.choice_lives.get() == 0
    }
    fn release_slot(callback: &Callback, resources: &Resources) {
        if resources.empty()
            && !callback.processing.get()
            && callback.choice_lives.get() == 0
            && callback.claimed.replace(false)
        {
            OWNED.with(|v| v.set(false));
        }
    }
    fn retire(callback: &Callback, resources: &mut Resources) -> bool {
        if callback.live.get() || !resources.empty() {
            callback.note_observation_gap();
        }
        callback.live.set(false);
        callback.cancel_choice();
        let _ = callback.suspend_effects();
        let api = callback.context.api;
        unsafe {
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
        #[cfg(feature = "test-tools")]
        let source = callback.revoke_source_ticket();
        #[cfg(not(feature = "test-tools"))]
        let source = true;
        resources.empty() && !callback.processing.get() && callback.choice_lives.get() == 0 && source
    }
    pub fn stop(&self) -> bool {
        self.callback.interrupt();
        if self.busy.replace(true) {
            return false;
        }
        let mut loan = Loan {
            owner: self,
            value: Some(std::mem::take(&mut *self.resources.borrow_mut())),
        };
        let result = Self::retire(&self.callback, &mut loan);
        Self::release_slot(&self.callback, &loan);
        loan.commit();
        *self.status.borrow_mut() = if result {
            "Off"
        } else {
            "Keyword cleanup pending"
        }
        .into();
        result
    }
    pub fn refresh(&self) {
        if self.busy.replace(true) {
            return;
        }
        let mut loan = Loan {
            owner: self,
            value: Some(std::mem::take(&mut *self.resources.borrow_mut())),
        };
        let callback = &self.callback;
        let context = callback.context;
        let api = context.api;
        let prepared = callback.prepared.borrow().clone();
        let gate = (callback.gate)();
        let revision = Cell::new(callback.revision.get());
        let generation = callback.generation.get();
        callback.interrupted.set(false);
        let ready = || {
            !callback.interrupted.get()
                && callback.revision.get() == revision.get()
                && callback.generation.get() == generation
                && (callback.gate)() == gate
        };
        let result = (|| {
            if !Self::recover(self.main) {
                return Err("Keyword recovery pending");
            }
            if !ready() {
                return Err("Keyword setup interrupted");
            }
            let reason = if !gate.running {
                "Paused"
            } else if !prepared.as_ref().is_some_and(|p| p.enabled) || generation == 0 {
                "Off"
            } else if unsafe { (api.raycast)() } {
                "Quit Raycast to expand keywords"
            } else if unsafe { (api.trusted)() == 0 } {
                "Allow Accessibility to expand keywords"
            } else {
                ""
            };
            if !ready() {
                return Err("Keyword setup interrupted");
            }
            if !reason.is_empty() {
                callback.reset();
                if !Self::retire(callback, &mut loan) {
                    return Err("Keyword cleanup pending");
                }
                return Ok(reason);
            }
            if callback.failed.get() {
                return Err("Keyword callback failed");
            }
            let pending = gate.generation == 0 || gate.generation != generation;
            if pending && !callback.observation_only.get() {
                revision.set(
                    callback
                        .suspend_effects()
                        .ok_or("Keyword callback failed")?,
                );
                if !ready() {
                    return Err("Keyword setup interrupted");
                }
            }
            if callback.live.get()
                && !callback.interrupted.get()
                && !loan.tap.is_null()
                && unsafe { (api.tap_enabled)(loan.tap) }
            {
                if !ready() {
                    return Err("Keyword setup interrupted");
                }
                if pending {
                    return Ok("Configuration pending");
                }
                callback.observation_only.set(false);
                return Ok("Noninteractive");
            }
            // Predict our own retirement revision before native cleanup;
            // never adopt a revision minted by synchronous reentry.
            revision.set(
                callback
                    .revision
                    .get()
                    .checked_add(1)
                    .ok_or("Keyword callback failed")?,
            );
            if !Self::retire(callback, &mut loan) {
                return Err("Keyword cleanup pending");
            }
            if pending {
                return Ok("Configuration pending");
            }
            if !callback.claimed.get() {
                if OWNED.with(|v| v.replace(true)) {
                    return Err("Keyword filter already owned");
                }
                callback.claimed.set(true);
            }
            if !ready() {
                return Err("Keyword setup interrupted");
            }
            keyword_matcher::prepare_callback_thread();
            if !ready() {
                return Err("Keyword setup interrupted");
            }
            #[cfg(feature = "test-tools")]
            Callback::publish_source_ticket(callback.clone())?;
            #[cfg(feature = "test-tools")]
            if !ready() { return Err("Keyword setup interrupted"); }
            unsafe {
                loan.runloop = (api.runloop)();
                loan.mode = (api.common)();
                if loan.runloop.is_null() || loan.mode.is_null() {
                    return Err("Keyword run loop unavailable");
                }
                loan.tap = (api.tap_create)(
                    1,
                    0,
                    0,
                    (1 << 1) | (1 << 3) | (1 << 10),
                    filter,
                    Rc::as_ptr(callback).cast_mut().cast(),
                );
                if loan.tap.is_null() {
                    return Err("Allow Input Monitoring to expand keywords");
                }
                if !ready() {
                    return Err("Keyword setup interrupted");
                }
                loan.source = (api.source)(null_mut(), loan.tap, 0);
                if loan.source.is_null() {
                    return Err("Keyword run loop source unavailable");
                }
                (api.add_source)(loan.runloop, loan.source, loan.mode);
                if !ready() {
                    return Err("Keyword setup interrupted");
                }
                (api.tap_enable)(loan.tap, true);
                if !ready() || !(api.tap_enabled)(loan.tap) {
                    return Err("Keyword filter could not be enabled");
                }
                if !ready() {
                    return Err("Keyword setup interrupted");
                }
            }
            callback.observation_only.set(false);
            callback.live.set(true);
            Ok("Noninteractive")
        })();
        let status = match result {
            Ok(v) => v,
            Err(e) => {
                callback.live.set(false);
                if Self::retire(callback, &mut loan) {
                    e
                } else {
                    "Keyword cleanup pending"
                }
            }
        };
        if loan.empty() {
            Self::release_slot(callback, &loan);
        }
        *self.status.borrow_mut() = status.into();
        loan.commit();
    }
    pub fn recovery_pending(_main: MainThreadMarker) -> bool {
        RECOVERING.with(Cell::get) || RETIRED.with(|v| !v.borrow().is_empty())
    }
    pub fn recover(_main: MainThreadMarker) -> bool {
        if RECOVERING.with(|v| v.replace(true)) {
            return false;
        }
        let values = RETIRED.with(|v| std::mem::take(&mut *v.borrow_mut()));
        for (callback, mut resources) in values {
            if Self::retire(&callback, &mut resources) {
                Self::release_slot(&callback, &resources);
            } else {
                RETIRED.with(|v| v.borrow_mut().push((callback, resources)));
            }
        }
        RECOVERING.with(|v| v.set(false));
        RETIRED.with(|v| v.borrow().is_empty())
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if !self.stop() {
            RETIRED.with(|v| {
                v.borrow_mut().push((
                    self.callback.clone(),
                    std::mem::take(self.resources.get_mut()),
                ))
            });
        }
    }
}
thread_local! {static OWNED:Cell<bool>=const{Cell::new(false)};static RETIRED:RefCell<Vec<(Rc<Callback>,Resources)>>=const{RefCell::new(Vec::new())};static RECOVERING:Cell<bool>=const{Cell::new(false)};}

#[cfg(feature = "test-tools")]
thread_local! {
    static FIXTURE_SERVICE:RefCell<Weak<Service>>=const{RefCell::new(Weak::new())};
    static FIXTURE_ACTIVATIONS:RefCell<Vec<serde_json::Value>>=const{RefCell::new(vec![])};
}
#[cfg(feature = "test-tools")]
pub fn fixture_bind(service: &Rc<Service>) {
    FIXTURE_SERVICE.with(|slot| *slot.borrow_mut() = Rc::downgrade(service));
    FIXTURE_ACTIVATIONS.with(|slot| slot.borrow_mut().clear());
}

/// Own SDK controller only. Never falls back to native discovery/input.
#[cfg(feature = "test-tools")]
pub fn fixture(request: &serde_json::Value) -> Result<serde_json::Value, String> {
    use std::ffi::{c_void, CStr, CString};
    let main = MainThreadMarker::new().ok_or("Own main controller required")?;
    let _complete = Context::new(main, true)?;
    unsafe {
        macro_rules! method {
            ($name:literal,$ty:ty) => {{
                let p = libc::dlsym(
                    libc::RTLD_DEFAULT,
                    concat!("ruycast_keyword_fixture_", $name, "\0")
                        .as_ptr()
                        .cast(),
                );
                if p.is_null() {
                    return Err("Own complete keyword controller unavailable".into());
                }
                std::mem::transmute::<*mut c_void, $ty>(p)
            }};
        }
        match request["control"].as_str().unwrap_or("state") {
            "choice-schedule-click" => {
                let p = libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"ruycast_choice_fixture_schedule_click".as_ptr(),
                );
                if p.is_null() {
                    return Err("Own monitor scheduler required".into());
                }
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn()>(p)();
            }
            "choice-monitors" => {
                let p = libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"ruycast_choice_fixture_monitor_counts".as_ptr(),
                );
                if p.is_null() {
                    return Err("Own global-monitor adapter required".into());
                }
                let (mut created, mut removed, mut delivered, mut active) =
                    (0u64, 0u64, 0u64, 0u64);
                std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(*mut u64, *mut u64, *mut u64, *mut u64),
                >(p)(&mut created, &mut removed, &mut delivered, &mut active);
                return Ok(
                    serde_json::json!({"created":created,"removed":removed,"delivered":delivered,"active":active}),
                );
            }
            "choice" => {
                let service = FIXTURE_SERVICE
                    .with(|slot| slot.borrow().upgrade())
                    .ok_or("Own host keyword service required")?;
                let current = service.callback.choice.borrow().clone();
                return match current {
                    Some(choice) => choice.fixture_control(request),
                    None => Ok(
                        serde_json::json!({"phase":"None","lives":service.callback.choice_lives.get()}),
                    ),
                };
            }
            "text" => {
                let units = if let Some(values) = request["units"].as_array() {
                    values
                        .iter()
                        .map(|v| {
                            v.as_u64()
                                .filter(|n| *n <= u16::MAX as u64)
                                .map(|n| n as u16)
                                .ok_or("Own UTF16 unit required")
                        })
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    request["value"]
                        .as_str()
                        .unwrap_or("")
                        .encode_utf16()
                        .collect::<Vec<_>>()
                };
                method!(
                    "text",
                    unsafe extern "C" fn(*const u16, usize, isize, isize)
                )(
                    units.as_ptr(),
                    units.len(),
                    request["at"].as_i64().unwrap_or(units.len() as i64) as isize,
                    request["length"].as_i64().unwrap_or(0) as isize,
                );
            }
            "activation-listen" => {
                let service = FIXTURE_SERVICE
                    .with(|slot| slot.borrow().upgrade())
                    .ok_or("Own host keyword service required")?;
                let own = objc2_app_kit::NSRunningApplication::currentApplication();
                service.set_choice_activation_handler(Some(Rc::new(move |event| {
                    use objc2_foundation::NSObjectProtocol;
                    let typed = event.application().is_some();
                    let own = event.application().is_some_and(|app| app.isEqual(Some(&*own)));
                    FIXTURE_ACTIVATIONS.with(|slot|slot.borrow_mut().push(serde_json::json!({"sequence":event.sequence,"typed":typed,"ownInstance":own})));
                })));
            }
            "activation-state" => {
                return Ok(
                    FIXTURE_ACTIVATIONS.with(|slot| serde_json::json!({"events":*slot.borrow()}))
                )
            }
            "environment" | "change" => {
                let flags = request["flags"].as_u64().unwrap_or(0) as u32;
                let pid = if request["pid"].as_str() == Some("self") {
                    std::process::id() as i32
                } else {
                    request["pid"].as_i64().unwrap_or(42) as i32
                };
                let app = request["app"].as_u64().unwrap_or(1);
                let element = request["element"].as_u64().unwrap_or(1);
                if request["control"] == "change" {
                    let at = CString::new(request["at"].as_str().ok_or("Own phase required")?)
                        .map_err(|_| "Invalid own phase")?;
                    method!(
                        "change_after",
                        unsafe extern "C" fn(*const i8, u32, u32, i32, u64, u64)
                    )(
                        at.as_ptr(),
                        request["nth"].as_u64().unwrap_or(1) as u32,
                        flags,
                        pid,
                        app,
                        element,
                    );
                } else {
                    method!("environment", unsafe extern "C" fn(u32, i32, u64, u64))(
                        flags, pid, app, element,
                    );
                }
            }
            "failure" => {
                let at = CString::new(request["name"].as_str().unwrap_or(""))
                    .map_err(|_| "Invalid own failure")?;
                method!("failure", unsafe extern "C" fn(*const i8))(at.as_ptr());
            }
            "clipboard" | "board-external" => {
                let value = request["value"].as_str().unwrap_or("");
                let html = request["html"].as_str();
                method!(
                    "board_external",
                    unsafe extern "C" fn(*const u8, usize, *const u8, usize, bool, bool)
                )(
                    value.as_ptr(),
                    value.len(),
                    html.map_or(std::ptr::null(), |s| s.as_ptr()),
                    html.map_or(0, str::len),
                    html.is_some(),
                    request["extra"].as_bool().unwrap_or(false),
                );
            }
            "bundle" => {
                let units: Vec<u16> = request["value"]
                    .as_str()
                    .unwrap_or("")
                    .encode_utf16()
                    .collect();
                method!("set_bundle", unsafe extern "C" fn(*const u16, usize))(
                    units.as_ptr(),
                    units.len(),
                );
            }
            "clear" => method!("clear_output", unsafe extern "C" fn())(),
            "event-burst" => {
                let text = request["text"]
                    .as_str()
                    .filter(|v| v.chars().count() <= 64)
                    .ok_or("Own input burst required")?;
                let mut results = vec![];
                for ch in text.chars() {
                    let mut units = [0u16; 2];
                    let units = ch.encode_utf16(&mut units);
                    results.push(method!(
                        "emit",
                        unsafe extern "C" fn(u32, i64, u64, i64, i64, *const u16, usize) -> bool
                    )(
                        10, 0, 0, 0, 0, units.as_ptr(), units.len()
                    ));
                }
                return Ok(serde_json::json!({"consumed":results}));
            }
            "event" => {
                let units: Vec<u16> = if let Some(units) = request["units"].as_array() {
                    units
                        .iter()
                        .map(|v| v.as_u64().unwrap_or(0) as u16)
                        .collect()
                } else {
                    request["text"]
                        .as_str()
                        .unwrap_or("")
                        .encode_utf16()
                        .collect()
                };
                let consumed = method!(
                    "emit",
                    unsafe extern "C" fn(u32, i64, u64, i64, i64, *const u16, usize) -> bool
                )(
                    request["type"].as_u64().unwrap_or(10) as u32,
                    request["key"].as_i64().unwrap_or(0),
                    request["flags"].as_u64().unwrap_or(0),
                    request["repeat"].as_i64().unwrap_or(0),
                    request["tag"].as_i64().unwrap_or(0),
                    units.as_ptr(),
                    units.len(),
                );
                let text = CStr::from_ptr(method!("state", unsafe extern "C" fn() -> *const i8)());
                let state: serde_json::Value =
                    serde_json::from_slice(text.to_bytes()).map_err(|e| e.to_string())?;
                return Ok(serde_json::json!({"consumed":consumed,"state":state}));
            }
            "state" => {}
            _ => return Err("Unsupported own keyword operation".into()),
        }
        serde_json::from_slice(
            CStr::from_ptr(method!("state", unsafe extern "C" fn() -> *const i8)()).to_bytes(),
        )
        .map_err(|e| e.to_string())
    }
}
