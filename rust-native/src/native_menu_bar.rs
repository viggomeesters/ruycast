//! Native status item. Hidden fixtures substitute registration before NSStatusBar.
use crate::native::Controller;
use objc2::rc::{Retained, Weak};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};
use ruycast_native::performance_trace::{shared, MenuHealthTrigger};
use ruycast_native::menu_bar::{PresenceRefresh, PresenceWorker, Presentation};
use std::cell::{Cell, RefCell};
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
#[cfg(feature = "test-tools")]
struct FixtureState { starts: u64, permits: u64, answer: bool }
#[cfg(feature = "test-tools")]
struct Fixture {
    ax: std::sync::atomic::AtomicBool,
    input: std::sync::atomic::AtomicBool,
    visible: std::sync::atomic::AtomicBool,
    ax_calls: AtomicU64,
    input_calls: AtomicU64,
    completed: AtomicU64,
    hold: std::sync::atomic::AtomicBool,
    state: std::sync::Mutex<FixtureState>,
    gate: std::sync::Condvar,
}
struct Binding {
    owner: Weak<Controller>,
    serial: u64,
}
define_class!(
    #[unsafe(super=NSObject)] #[thread_kind=MainThreadOnly] #[ivars=Binding]
    struct Target;
    impl Target{
        #[unsafe(method(invoke:))]fn invoke(&self,sender:&NSMenuItem){if let Some(owner)=self.ivars().owner.load(){owner.menu_bar_action(self.ivars().serial,sender.tag());}}
    }
);
pub struct Service {
    item: RefCell<Option<Retained<NSStatusItem>>>,
    menu: RefCell<Option<Retained<NSMenu>>>,
    target: RefCell<Option<Retained<Target>>>,
    serial: Cell<u64>,
    requested: RefCell<Option<ruycast_native::menu_bar::State>>,
    state: RefCell<Option<ruycast_native::menu_bar::State>>,
    hidden: Cell<bool>,
    registrations: Cell<u64>,
    presence: RefCell<PresenceRefresh>,
    worker: RefCell<Option<PresenceWorker>>,
    observation: RefCell<Option<crate::native_presence::Observation>>,
    epoch: Arc<AtomicU64>,
    seen_epoch: Cell<u64>,
    submitted_epoch: Cell<Option<(u64, u64)>>,
    stopped: Cell<bool>,
    last_eligible: Cell<bool>,
    checking: Cell<bool>,
    presented: Cell<Option<Presentation>>,
    #[cfg(feature = "test-tools")]
    fixture: RefCell<Option<Arc<Fixture>>>,
    #[cfg(feature = "test-tools")]
    fixture_wake: RefCell<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(feature = "test-tools")]
    stale: RefCell<Option<Retained<NSMenuItem>>>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            item: RefCell::new(None),
            menu: RefCell::new(None),
            target: RefCell::new(None),
            serial: Cell::new(1),
            requested: RefCell::new(None),
            state: RefCell::new(None),
            hidden: Cell::new(true),
            registrations: Cell::new(0),
            presence: RefCell::new(PresenceRefresh::default()),
            worker: RefCell::new(None),
            observation: RefCell::new(None),
            epoch: Arc::new(AtomicU64::new(0)),
            seen_epoch: Cell::new(0),
            submitted_epoch: Cell::new(None),
            stopped: Cell::new(false),
            last_eligible: Cell::new(false),
            checking: Cell::new(false),
            presented: Cell::new(None),
            #[cfg(feature = "test-tools")]
            fixture: RefCell::new(None),
            #[cfg(feature = "test-tools")]
            fixture_wake: RefCell::new(None),
            #[cfg(feature = "test-tools")]
            stale: RefCell::new(None),
        }
    }
}
impl Service {
    pub fn start(&self, owner: &Controller, wake: Arc<dyn Fn() + Send + Sync>, hidden_fixture: bool) {
        if self.stopped.get() { return; }
        if hidden_fixture {
            #[cfg(feature = "test-tools")]
            { *self.fixture_wake.borrow_mut() = Some(wake); }
            return;
        }
        // Subscribe before dispatching the first lookup; this epoch is never
        // shared with another controller lifecycle.
        let worker = match PresenceWorker::start(Arc::new(crate::native_presence::raycast_absent), wake.clone()) {
            Ok(worker) => worker,
            Err(_) => { self.presence.borrow_mut().close(); return; }
        };
        *self.observation.borrow_mut() = Some(crate::native_presence::Observation::start(owner.mtm(), self.epoch.clone(), wake));
        *self.worker.borrow_mut() = Some(worker);
        self.last_eligible.set(false);
        self.refresh(owner, MenuHealthTrigger::State);
    }

    fn eligible(&self, reason: MenuHealthTrigger) -> bool {
        #[cfg(feature = "test-tools")]
        let simulated = self.fixture.borrow().is_some();
        #[cfg(not(feature = "test-tools"))]
        let simulated = false;
        let base = (!self.hidden.get() || simulated) && self.requested.borrow().as_ref()
            .is_some_and(|state| state.visible && state.requires_input && state.healthy);
        base && shared().measure_menu_permissions(reason,
            || {
                #[cfg(feature = "test-tools")]
                if let Some(fixture) = self.fixture.borrow().as_ref() { fixture.ax_calls.fetch_add(1, Ordering::Relaxed); return fixture.ax.load(Ordering::Acquire); }
                unsafe { AXIsProcessTrusted() != 0 }
            },
            || {
                #[cfg(feature = "test-tools")]
                if let Some(fixture) = self.fixture.borrow().as_ref() { fixture.input_calls.fetch_add(1, Ordering::Relaxed); return fixture.input.load(Ordering::Acquire); }
                unsafe { CGPreflightListenEventAccess() }
            })
    }

    fn submit(&self, generation: u64, eligible: bool) {
        if !eligible {
            self.last_eligible.set(false);
            self.presence.borrow_mut().submission_cancelled(generation);
            self.submitted_epoch.set(None);
            return;
        }
        let epoch = self.epoch.load(Ordering::Acquire);
        if epoch == u64::MAX {
            self.presence.borrow_mut().submission_failed(generation);
            return;
        }
        self.submitted_epoch.set(Some((generation, epoch)));
        if !self.worker.borrow().as_ref().is_some_and(|worker| worker.request(generation)) {
            self.presence.borrow_mut().submission_failed(generation);
            self.submitted_epoch.set(None);
        }
    }

    fn invalidate(&self, eligible: bool) {
        if self.worker.borrow().is_none() { return; }
        self.last_eligible.set(eligible);
        if !eligible { self.presence.borrow_mut().suspend(); return; }
        let next = self.presence.borrow_mut().invalidate();
        if let Some(generation) = next { self.submit(generation, eligible); }
    }

    pub fn pump(&self, owner: &Controller) {
        if self.stopped.get() { return; }
        #[cfg(feature = "test-tools")]
        if self.fixture.borrow().as_ref().is_some_and(|fixture| fixture.hold.load(Ordering::Acquire)) { return; }
        let epoch = self.epoch.load(Ordering::Acquire);
        let reply = self.worker.borrow().as_ref().map(|worker| worker.poll());
        let epoch_changed = epoch != self.seen_epoch.get();
        let limit = self.worker.borrow().as_ref().map(|worker| worker.limit());
        let expired = limit.is_some_and(|limit| self.presence.borrow_mut().expire_overdue(std::time::Instant::now(), limit));
        if !epoch_changed && !expired && matches!(&reply, None | Some(Ok(None))) { return; }
        let eligible = self.eligible(MenuHealthTrigger::State);
        let mut invalidated = false;
        if self.last_eligible.replace(eligible) != eligible {
            self.invalidate(eligible);
            invalidated = true;
        }
        if epoch == u64::MAX {
            self.presence.borrow_mut().close();
        } else if epoch_changed {
            self.seen_epoch.set(epoch);
            if !invalidated { self.invalidate(eligible); invalidated = true; }
        }
        match reply {
            Some(Ok(Some((generation, answer)))) => {
                let current = self.epoch.load(Ordering::Acquire);
                if self.submitted_epoch.get() != Some((generation, current)) || current == u64::MAX {
                    if !invalidated { self.invalidate(eligible); }
                }
                self.submitted_epoch.set(None);
                let next = self.presence.borrow_mut().complete(generation, answer);
                if let Some(next) = next { self.submit(next, eligible); }
            }
            Some(Err(_)) => {
                let observation = self.observation.borrow_mut().take();
                drop(observation);
                self.presence.borrow_mut().close();
                let worker = self.worker.borrow_mut().take();
                drop(worker);
            }
            _ => {}
        }
        let state = self.requested.borrow().clone();
        self.render(owner, state, self.hidden.get(), eligible);
    }

    pub fn apply(
        &self,
        owner: &Controller,
        state: Option<ruycast_native::menu_bar::State>,
        hidden: bool,
    ) {
        if self.stopped.get() { return; }
        #[cfg(feature = "test-tools")]
        let state = if hidden && self.fixture.borrow().is_some() {
            Some(ruycast_native::menu_bar::State { visible: self.fixture.borrow().as_ref().unwrap().visible.load(Ordering::Acquire), healthy: true, requires_input: true })
        } else { state };
        if self.hidden.get() == hidden && *self.requested.borrow() == state {
            return;
        }
        self.update(owner, state, hidden, MenuHealthTrigger::State);
    }
    fn update(
        &self,
        owner: &Controller,
        state: Option<ruycast_native::menu_bar::State>,
        hidden: bool,
        reason: MenuHealthTrigger,
    ) {
        let previously_eligible = self.last_eligible.get();
        self.hidden.set(hidden);
        *self.requested.borrow_mut() = state.clone();
        let eligible = self.eligible(reason);
        self.last_eligible.set(eligible);
        let retry_unknown = eligible && matches!(reason, MenuHealthTrigger::Activation)
            && self.presence.borrow().known().is_none() && !self.presence.borrow().waiting();
        if previously_eligible != eligible || retry_unknown { self.invalidate(eligible); }
        self.render(owner, state, hidden, eligible);
    }
    fn render(&self, owner: &Controller, state: Option<ruycast_native::menu_bar::State>, hidden: bool, eligible: bool) {
        let observed_epoch = self.epoch.load(Ordering::Acquire);
        let mut checking = eligible && observed_epoch != u64::MAX
            && self.worker.borrow().is_some()
            && (self.presence.borrow().waiting() || observed_epoch != self.seen_epoch.get());
        let mut state = state;
        #[cfg(feature = "test-tools")]
        let simulated = self.fixture.borrow().is_some();
        #[cfg(not(feature = "test-tools"))]
        let simulated = false;
        if !hidden || simulated {
            if let Some(s) = &mut state {
                if s.visible && s.requires_input && s.healthy {
                    s.healthy = eligible && shared().measure(ruycast_native::performance_trace::Stage::MenuHealthRaycast,
                            || self.epoch.load(Ordering::Acquire) == observed_epoch
                                && observed_epoch == self.seen_epoch.get()
                                && self.presence.borrow().known().unwrap_or(false),
                        );
                }
            }
        }
        let latest_epoch = self.epoch.load(Ordering::Acquire);
        if latest_epoch != observed_epoch {
            checking = eligible && latest_epoch != u64::MAX && self.worker.borrow().is_some();
            if let Some(s) = &mut state {
                if s.visible && s.requires_input { s.healthy = false; }
            }
        }
        if self.presence.borrow().confirmed_conflict() { checking = false; }
        if *self.state.borrow() == state && self.checking.get() == checking {
            return;
        }
        self.checking.set(checking);
        let visible = state.as_ref().is_some_and(|s| s.visible);
        if !visible {
            self.retire();
            *self.state.borrow_mut() = state;
            return;
        }
        if self.menu.borrow().is_none() {
            let main = owner.mtm();
            let target = Target::alloc(main).set_ivars(Binding {
                owner: Weak::new(owner),
                serial: self.serial.get(),
            });
            let target: Retained<Target> = unsafe { msg_send![super(target), init] };
            let menu = NSMenu::new(main);
            for (index, title) in ["Open Launcher", "Settings…", "Quit"].iter().enumerate() {
                let row = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(main),
                        &NSString::from_str(title),
                        Some(sel!(invoke:)),
                        &NSString::from_str(""),
                    )
                };
                row.setTag(index as isize);
                unsafe {
                    row.setTarget(Some(&*target));
                }
                menu.addItem(&row);
            }
            *self.menu.borrow_mut() = Some(menu.clone());
            *self.target.borrow_mut() = Some(target);
            if !hidden {
                // Never call systemStatusBar/statusItemWithLength in fixtures.
                let item =
                    NSStatusBar::systemStatusBar().statusItemWithLength(NSSquareStatusItemLength);
                item.setMenu(Some(&menu));
                *self.item.borrow_mut() = Some(item);
                self.registrations.set(self.registrations.get() + 1);
            }
        }
        let presentation =
            Presentation::of(state.as_ref().is_some_and(|s| s.healthy), checking);
        // Recorded for fixtures too, which never register a status item, so the
        // presentation a visible menu would show is directly observable.
        self.presented.set(Some(presentation));
        if let Some(item) = &*self.item.borrow() {
            if let Some(button) = item.button(owner.mtm()) {
                button.setImage(Some(&icon(presentation.neutral_icon())));
                button.setToolTip(Some(&NSString::from_str(presentation.tool_tip())));
                button.setAccessibilityLabel(Some(&NSString::from_str(
                    presentation.accessibility_label(),
                )));
            }
        }
        *self.state.borrow_mut() = state;
    }
    pub fn refresh(&self, owner: &Controller, reason: MenuHealthTrigger) {
        if self.stopped.get() { return; }
        let state = owner.menu_bar_state();
        self.update(owner, state, self.hidden.get(), reason);
    }
    pub fn live(&self, serial: u64) -> bool {
        serial != 0
            && self.serial.get() == serial
            && self.state.borrow().as_ref().is_some_and(|s| s.visible)
    }
    pub fn retire(&self) {
        self.serial
            .set(self.serial.get().checked_add(1).unwrap_or(0));
        self.presented.set(None);
        if let Some(menu) = self.menu.take() {
            for row in menu.itemArray().iter() {
                unsafe {
                    row.setTarget(None);
                    row.setAction(None);
                }
            }
        }
        self.target.take();
        let item = self.item.take();
        if let Some(item) = item {
            NSStatusBar::systemStatusBar().removeStatusItem(&item);
        }
    }
    pub fn stop(&self) {
        if self.stopped.replace(true) { return; }
        let observation = self.observation.borrow_mut().take();
        drop(observation);
        self.presence.borrow_mut().close();
        let worker = self.worker.borrow_mut().take();
        drop(worker);
    }
    pub fn cleanup(&self) {
        self.stop();
        self.requested.borrow_mut().take();
        self.state.borrow_mut().take();
        self.retire();
    }
    #[cfg(feature = "test-tools")]
    pub fn fixture_enable(&self, owner: &Controller) -> bool {
        if !self.hidden.get() || self.stopped.get() || self.worker.borrow().is_some() { return false; }
        let Some(wake) = self.fixture_wake.borrow().clone() else { return false; };
        let fixture = Arc::new(Fixture {
            ax: std::sync::atomic::AtomicBool::new(true),
            input: std::sync::atomic::AtomicBool::new(true),
            visible: std::sync::atomic::AtomicBool::new(true),
            ax_calls: AtomicU64::new(0),
            input_calls: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            hold: std::sync::atomic::AtomicBool::new(false),
            state: std::sync::Mutex::new(FixtureState { starts: 0, permits: 0, answer: true }),
            gate: std::sync::Condvar::new(),
        });
        let worker_fixture = fixture.clone();
        let probe = Arc::new(move || {
            let mut state = worker_fixture.state.lock().ok()?;
            state.starts += 1;
            let (next, timeout) = worker_fixture.gate.wait_timeout_while(
                state, std::time::Duration::from_secs(10), |state| state.permits == 0).ok()?;
            state = next;
            if timeout.timed_out() && state.permits == 0 { return None; }
            state.permits -= 1;
            worker_fixture.completed.fetch_add(1, Ordering::Release);
            Some(state.answer)
        });
        let Ok(worker) = PresenceWorker::start(probe, wake) else { return false; };
        *self.fixture.borrow_mut() = Some(fixture);
        *self.worker.borrow_mut() = Some(worker);
        self.last_eligible.set(false);
        self.update(owner, Some(ruycast_native::menu_bar::State { visible: true, healthy: true, requires_input: true }), true, MenuHealthTrigger::State);
        true
    }
    #[cfg(feature = "test-tools")]
    pub fn fixture_change(&self, owner: &Controller, request: &serde_json::Value) -> bool {
        let Some(fixture) = self.fixture.borrow().clone() else { return false; };
        if let Some(value) = request["ax"].as_bool() { fixture.ax.store(value, Ordering::Release); }
        if let Some(value) = request["input"].as_bool() { fixture.input.store(value, Ordering::Release); }
        if let Some(value) = request["visible"].as_bool() { fixture.visible.store(value, Ordering::Release); }
        if let Some(value) = request["hold"].as_bool() { fixture.hold.store(value, Ordering::Release); }
        if request["event"] == true { let _ = self.epoch.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1)); }
        if request["release"] == true {
            let Ok(mut state) = fixture.state.lock() else { return false; };
            if let Some(answer) = request["answer"].as_bool() { state.answer = answer; }
            state.permits += 1;
            fixture.gate.notify_one();
        }
        if self.stopped.get() { return true; }
        self.update(owner, Some(ruycast_native::menu_bar::State { visible: fixture.visible.load(Ordering::Acquire), healthy: true, requires_input: true }), true, MenuHealthTrigger::State);
        self.pump(owner);
        true
    }
    #[cfg(feature = "test-tools")]
    pub fn probe(&self) -> serde_json::Value {
        let starts = self.fixture.borrow().as_ref().and_then(|fixture| fixture.state.lock().ok().map(|state| state.starts));
        let calls = self.fixture.borrow().as_ref().map(|fixture| (fixture.ax_calls.load(Ordering::Relaxed), fixture.input_calls.load(Ordering::Relaxed)));
        let completed = self.fixture.borrow().as_ref().map(|fixture| fixture.completed.load(Ordering::Acquire));
        let presentation = self.presented.get().map(|p| serde_json::json!({"state":p,"toolTip":p.tool_tip(),"accessibilityLabel":p.accessibility_label(),"neutralIcon":p.neutral_icon()}));
        serde_json::json!({"state":*self.state.borrow(),"checking":self.checking.get(),"presentation":presentation,"fixtureStarts":starts,"fixtureCompleted":completed,"fixtureBusy":self.presence.borrow().waiting(),"fixtureCalls":calls,"serial":self.serial.get(),"hiddenSubstitution":self.hidden.get(),"nativeRegistrations":self.registrations.get(),"items":self.menu.borrow().as_ref().map(|m|m.itemArray().iter().map(|r|r.title().to_string()).collect::<Vec<_>>())})
    }
    #[cfg(feature = "test-tools")]
    pub fn click(&self, index: usize) -> bool {
        let row = self
            .menu
            .borrow()
            .as_ref()
            .and_then(|m| m.itemAtIndex(index as isize));
        if let Some(row) = row {
            if let Some(target) = self.target.borrow().clone() {
                unsafe {
                    let _: () = msg_send![&*target,invoke:&*row];
                }
                return true;
            }
        }
        false
    }
    #[cfg(feature = "test-tools")]
    pub fn retain(&self) {
        *self.stale.borrow_mut() = self.menu.borrow().as_ref().and_then(|m| m.itemAtIndex(0));
    }
    #[cfg(feature = "test-tools")]
    pub fn stale(&self) {
        if let Some(row) = &*self.stale.borrow() {
            if let Some(target) = row.target() {
                unsafe {
                    let _: () = msg_send![&*target,invoke:&**row];
                }
            }
        }
    }
}
fn icon(healthy: bool) -> Retained<NSImage> {
    let draw = block2::RcBlock::new(move |_: NSRect| -> objc2::runtime::Bool {
        let color = if healthy {
            NSColor::blackColor()
        } else {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.78, 0.37, 0.10, 1.)
        };
        color.setFill();
        let p = NSBezierPath::bezierPath();
        let point = |x: f64, y: f64| NSPoint::new(1. + x * 0.16, 1. + y * 0.16);
        p.moveToPoint(point(17., 15.));
        for (x, y) in [
            (17., 85.),
            (58., 85.),
            (83., 61.),
            (59., 38.),
            (34., 38.),
            (34., 15.),
        ] {
            p.lineToPoint(point(x, y));
        }
        p.closePath();
        p.moveToPoint(point(34., 69.));
        for (x, y) in [(53., 69.), (62., 61.), (53., 53.), (34., 53.)] {
            p.lineToPoint(point(x, y));
        }
        p.closePath();
        p.setWindingRule(NSWindingRule::EvenOdd);
        p.fill();
        let ray = NSBezierPath::bezierPath();
        ray.moveToPoint(point(39., 43.));
        for (x, y) in [(59., 43.), (85., 15.), (62., 15.)] {
            ray.lineToPoint(point(x, y));
        }
        ray.closePath();
        ray.fill();
        true.into()
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(18., 18.), false, &draw);
    image.setTemplate(healthy);
    image.setAccessibilityDescription(Some(&NSString::from_str(if healthy {
        "Ruycast"
    } else {
        "Ruycast needs attention"
    })));
    image
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn CGPreflightListenEventAccess() -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn genuine_health_images_preserve_template_state_and_accessible_description() {
        objc2::rc::autoreleasepool(|_| {
            let healthy = icon(true);
            let unhealthy = icon(false);
            assert!(healthy.isTemplate());
            assert!(!unhealthy.isTemplate());
            assert_eq!(healthy.accessibilityDescription().unwrap().to_string(), "Ruycast");
            assert_eq!(unhealthy.accessibilityDescription().unwrap().to_string(), "Ruycast needs attention");
        });
    }
}
