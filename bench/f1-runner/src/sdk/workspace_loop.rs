//! Owned application loop and lifecycle ordering. No Cocoa/global runloop,
//! desktop input, sleep observer, menu registration or original app run method.
//! The endpoint feeds this queue and owns bounded transport/guardian teardown.
use super::*;
use std::collections::{BTreeMap, VecDeque};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    WillFinishLaunching,
    DidFinishLaunching,
    WillTerminate,
}
#[derive(Clone)]
pub enum Event {
    Action {
        target: Handle,
        selector: String,
    },
    ActionWithSender {
        target: Handle,
        callback: Arc<dyn Callback>,
        selector: String,
        sender: Arc<super::receiver::SenderLease>,
    },
    Stop,
    Terminate,
    Native(Arc<super::objc::EventLease>),
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    ReferenceOpen(Arc<super::callbacks::ReferenceOpenReply>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Created,
    Launching,
    Running,
    Stopped,
    Terminated,
    Retired,
}
/// A callback owns its target and source-bound identity through dispatch. Real
/// Objective-C adapters must additionally check the complete preentry registry;
/// owned Rust test delegates are not represented as original AppDelegates.
pub trait Callback: Send + Sync {
    fn lifecycle(&self, event: Lifecycle) -> Result<()>;
    fn action(&self, selector: &str) -> Result<()>;
    /// Only an explicitly source-qualified window callback may dispatch this.
    /// An absent implementation is an error, never a successful no-op.
    fn native_event(&self, _event: &super::objc::EventLease) -> Result<()> {
        Err("window native-event callback is not source-qualified".into())
    }
    fn action_with_sender(
        &self,
        control: Handle,
        selector: &str,
        _sender: &super::receiver::SenderLease,
    ) -> Result<()> {
        super::objc::controls::verify_action_control(control)?;
        self.action(selector)
    }
}
struct LoopState {
    phase: Phase,
    delegate: Option<Arc<dyn Callback>>,
    targets: BTreeMap<Handle, Arc<dyn Callback>>,
    queue: VecDeque<Event>,
    current_event: Option<Arc<super::objc::EventLease>>,
}
impl LoopState {
    fn terminal_queued(&self) -> bool {
        cfg!(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))
            && self.queue.iter().any(|event| matches!(event, Event::Terminate))
    }
}
pub struct Engine {
    model: Arc<Workspace>,
    state: Mutex<LoopState>,
    owner: std::thread::ThreadId,
    wake: Mutex<Option<Arc<super::cf_loop::Wake>>>,
}
struct CurrentEvent<'a> {
    engine: &'a Engine,
    previous: Option<Arc<super::objc::EventLease>>,
}
impl Drop for CurrentEvent<'_> {
    fn drop(&mut self) {
        let removed = {
            let mut state = match self.engine.state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            if matches!(state.phase, Phase::Retired | Phase::Terminated) {
                state.current_event.take()
            } else {
                std::mem::replace(&mut state.current_event, self.previous.take())
            }
        };
        // Both current and a retired previous frame release outside the lock.
        drop(removed);
        drop(self.previous.take());
    }
}
impl Engine {
    pub fn new(model: Arc<Workspace>) -> Result<Arc<Self>> {
        model.with(|_| Ok(()))?;
        Ok(Arc::new(Self {
            model,
            state: Mutex::new(LoopState {
                phase: Phase::Created,
                delegate: None,
                targets: BTreeMap::new(),
                queue: VecDeque::new(),
                current_event: None,
            }),
            owner: std::thread::current().id(),
            wake: Mutex::new(None),
        }))
    }
    fn own_thread(&self) -> Result<()> {
        if std::thread::current().id() != self.owner {
            return Err("application loop on foreign owner thread".into());
        }
        Ok(())
    }
    pub(crate) fn check_owner(&self) -> Result<()> {
        self.own_thread()
    }
    pub(crate) fn model(&self) -> &Arc<Workspace> {
        &self.model
    }
    pub(crate) fn has_events(&self) -> bool {
        self.state.lock().is_ok_and(|s| !s.queue.is_empty())
    }
    pub(crate) fn attach_wake(&self, wake: Arc<super::cf_loop::Wake>) -> Result<()> {
        self.own_thread()?;
        let mut slot = self.wake.lock().map_err(|_| "CF wake slot poisoned")?;
        if slot.is_some() {
            return Err("nested application run refused".into());
        }
        *slot = Some(wake);
        Ok(())
    }
    pub(crate) fn detach_wake(&self) {
        if let Ok(mut slot) = self.wake.lock() {
            *slot = None;
        }
    }
    pub fn wake_handle(&self) -> Result<Arc<super::cf_loop::Wake>> {
        self.model.with(|_| Ok(()))?;
        self.wake
            .lock()
            .map_err(|_| "CF wake slot poisoned")?
            .clone()
            .ok_or("application CF source not attached".into())
    }
    pub fn signal(&self) -> Result<()> {
        self.wake_handle()?.signal()
    }
    pub fn set_delegate(&self, delegate: Arc<dyn Callback>) -> Result<()> {
        self.own_thread()?;
        self.model.with(|_| Ok(()))?;
        let mut s = self.state.lock().map_err(|_| "loop poisoned")?;
        if s.phase != Phase::Created || s.delegate.is_some() {
            return Err("delegate replacement after selection refused".into());
        }
        s.delegate = Some(delegate);
        Ok(())
    }
    pub fn clear_delegate(&self) -> Result<()> {
        self.own_thread()?;
        self.model.with(|_| Ok(()))?;
        let delegate = {
            let mut s = self.state.lock().map_err(|_| "loop poisoned")?;
            if matches!(s.phase, Phase::Launching | Phase::Running) {
                return Err("cannot clear a live application delegate".into());
            }
            s.delegate.take()
        };
        drop(delegate);
        Ok(())
    }
    pub fn register_target(&self, target: Handle, callback: Arc<dyn Callback>) -> Result<()> {
        self.own_thread()?;
        self.model.with(|s| {
            if target.generation != s.generation {
                return Err("stale menu/control target".into());
            }
            Ok(())
        })?;
        let mut s = self.state.lock().map_err(|_| "loop poisoned")?;
        if matches!(s.phase, Phase::Terminated | Phase::Retired) || s.targets.contains_key(&target)
        {
            return Err("closed/duplicate control target".into());
        }
        s.targets.insert(target, callback);
        Ok(())
    }
    pub fn unregister_target(&self, target: Handle) -> Result<()> {
        self.own_thread()?;
        self.model.with(|_| Ok(()))?;
        let callback = {
            self.state
                .lock()
                .map_err(|_| "loop poisoned")?
                .targets
                .remove(&target)
                .ok_or("unknown control target")?
        };
        drop(callback);
        Ok(())
    }
    pub fn enqueue(&self, event: Event) -> Result<()> {
        self.enqueue_checked(event, || Ok(()))
    }
    pub(super) fn enqueue_checked(&self, event: Event, check: impl FnOnce() -> Result<()>) -> Result<()> {
        self.model.with(|_| Ok(()))?;
        check()?;
        let mut s = self.state.lock().map_err(|_| "loop poisoned")?;
        if matches!(s.phase, Phase::Terminated | Phase::Retired) {
            return Err("loop retired".into());
        }
        if s.terminal_queued() {
            // Drop the submitted lease after releasing state. Terminate
            // retains its position and duplicate terminal events coalesce.
            drop(s);
            return Ok(());
        }
        if s.queue.len() >= 4096 {
            return Err("owned loop queue bound reached".into());
        }
        s.queue.push_back(event);
        drop(s);
        // An event queued while stopped remains queued for a future run. A
        // concurrently invalidated wake never causes the event to be replayed.
        let _ = self.signal();
        Ok(())
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn enqueue_reference_open(
        &self, path: &Path, url: &str, opens_url: bool,
        publish: impl FnOnce(Handle, &Completion) -> Result<super::callbacks::ReferenceOpenReply>,
    ) -> Result<()> {
        self.own_thread()?;
        let mut cancelled = false;
        let result = self.model.begin_owned_application_open(path, url, opens_url, |app, completion| {
            let mut state = self.state.lock().map_err(|_| "loop poisoned")?;
            if state.terminal_queued() {
                cancelled = true;
                return Err("Reference open cancelled by queued termination".into());
            }
            if state.phase != Phase::Running || state.queue.len() >= 4096 {
                return Err("Reference open completion queue unavailable".into());
            }
            state.queue.push_back(Event::ReferenceOpen(Arc::new(publish(app, completion)?)));
            Ok(())
        });
        if cancelled {
            // The refused publisher commits no model effect and leaves the
            // caller's original retain available for qualified release.
            return Ok(());
        }
        result?;
        let _ = self.signal();
        Ok(())
    }
    pub fn post_native_event(&self, event: Arc<super::objc::EventLease>, at_start: bool) -> Result<()> {
        self.own_thread()?;
        if !event.handle().belongs_to(&self.model) {
            return Err("event belongs to another application model".into());
        }
        event.handle().check()?;
        self.model.with(|_| Ok(()))?;
        event.handle().check_pure()?;
        {
            let mut state = self.state.lock().map_err(|_| "loop poisoned")?;
            if matches!(state.phase, Phase::Terminated | Phase::Retired) {
                return Err("loop retired".into());
            }
            if state.terminal_queued() {
                // atStart must never overtake a queued Reference Terminate.
                return Ok(());
            }
            if state.queue.len() >= 4096 { return Err("owned loop queue bound reached".into()); }
            if at_start { state.queue.push_front(Event::Native(event)); }
            else { state.queue.push_back(Event::Native(event)); }
        }
        let _ = self.signal();
        Ok(())
    }
    pub fn current_event(&self) -> Result<Option<Arc<super::objc::EventLease>>> {
        self.own_thread()?;
        self.model.with(|_| Ok(()))?;
        let current = self.state.lock().map_err(|_| "loop poisoned")?.current_event.clone();
        if let Some(event) = &current { event.handle().check()?; }
        let state = self.state.lock().map_err(|_| "loop poisoned")?;
        if matches!(state.phase, Phase::Retired | Phase::Terminated) {
            return Err("application loop retired".into());
        }
        if state.current_event.as_ref().map(Arc::as_ptr) != current.as_ref().map(Arc::as_ptr) {
            return Err("current event changed during admission".into());
        }
        Ok(current)
    }
    /// Pure final check after current-event admission, with no guardian/callback
    /// between confirming the scope and returning its borrowed +0 identity.
    pub(super) fn verify_current_event(&self, event: &Arc<super::objc::EventLease>) -> Result<()> {
        self.own_thread()?;
        event.handle().check_pure()?;
        let state = self.state.lock().map_err(|_| "loop poisoned")?;
        if matches!(state.phase, Phase::Retired | Phase::Terminated)
            || !state.current_event.as_ref().is_some_and(|current| Arc::ptr_eq(current, event))
        { return Err("current event scope expired".into()); }
        Ok(())
    }
    pub fn launch(&self) -> Result<()> {
        self.own_thread()?;
        self.model.with(|_| Ok(()))?;
        let delegate = {
            let mut s = self.state.lock().map_err(|_| "loop poisoned")?;
            if s.phase != Phase::Created {
                return Err("launch replay refused".into());
            }
            let d = s
                .delegate
                .clone()
                .ok_or("missing selected application delegate")?;
            s.phase = Phase::Launching;
            d
        };
        delegate.lifecycle(Lifecycle::WillFinishLaunching)?;
        self.model.with(|_| Ok(()))?;
        delegate.lifecycle(Lifecycle::DidFinishLaunching)?;
        self.model.with(|_| Ok(()))?;
        let mut s = self.state.lock().map_err(|_| "loop poisoned")?;
        if s.phase != Phase::Launching {
            return Err("delegate changed loop during launch".into());
        }
        s.phase = Phase::Running;
        Ok(())
    }
    /// Executes at most one owned event. No default event source or real Cocoa
    /// fallback exists. Stop returns from this run; terminate delivers the actual
    /// selected shutdown callback once. Neither silently substitutes the other.
    pub fn pump(&self) -> Result<bool> {
        self.own_thread()?;
        self.model.with(|_| Ok(()))?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let terminal_wake = {
            let terminal = {
                let state = self.state.lock().map_err(|_| "loop poisoned")?;
                state.phase == Phase::Running && matches!(state.queue.front(), Some(Event::Terminate))
            };
            if terminal {
                // Qualify the provider and the wake outside both the engine
                // state lock and the work registry lock.
                crate::sdk::reference_runtime::close_terminal_ingress(self)?;
                Some(self.wake_handle()?)
            } else {
                None
            }
        };
        let (event, delegate, target) = {
            let mut s = self.state.lock().map_err(|_| "loop poisoned")?;
            if s.phase != Phase::Running {
                return Err("application is not running".into());
            }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            if matches!(s.queue.front(), Some(Event::Stop | Event::Terminate)) {
                let terminal = matches!(s.queue.front(), Some(Event::Terminate));
                if terminal && terminal_wake.is_none() {
                    // An enqueue raced with preflight. Leave the event queued
                    // for the next CF turn to perform terminal qualification.
                    return Ok(false);
                }
                if super::callbacks::begin_reference_open_shutdown(terminal_wake) {
                    // Leave the event queued until callbacks and admitted
                    // reply waits have acknowledged cancellation.
                    return Ok(false);
                }
            }
            let Some(e) = s.queue.pop_front() else {
                return Ok(false);
            };
            let target =
                if let Event::Action { target, .. } = &e {
                    s.targets.get(target).cloned()
                } else {
                    None
                };
            (e, s.delegate.clone(), target)
        };
        match event {
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            Event::ReferenceOpen(reply) => reply.deliver()?,
            Event::Native(event) => {
                if !event.handle().belongs_to(&self.model) { return Err("foreign event model".into()); }
                let data = event.data()?;
                let window = data.window.ok_or("native event has no qualified target window")?;
                let (target, previous) = {
                    let mut state = self.state.lock().map_err(|_| "loop poisoned")?;
                    if state.phase != Phase::Running { return Err("event dispatch no longer running".into()); }
                    let target = state.targets.get(&window).cloned().ok_or("window event target is not source-qualified")?;
                    let previous = state.current_event.replace(event.clone());
                    (target, previous)
                };
                let current = CurrentEvent { engine: self, previous };
                target.native_event(&event)?;
                drop(current);
            }
            Event::ActionWithSender {
                target, callback, selector, sender,
            } => {
                self.model.with(|_| Ok(()))?;
                sender.before_effect()?;
                callback.action_with_sender(target, &selector, &sender)?;
            }
            Event::Action { selector, .. } => {
                self.model.with(|_| Ok(()))?;
                target
                    .ok_or("missing control callback")?
                    .action(&selector)?;
            }
            Event::Stop => {
                self.state.lock().map_err(|_| "loop poisoned")?.phase = Phase::Stopped;
            }
            Event::Terminate => {
                self.model.with(|_| Ok(()))?;
                let retired = {
                    let mut s = self.state.lock().map_err(|_| "loop poisoned")?;
                    s.phase = Phase::Terminated;
                    (
                        s.delegate.take(),
                        std::mem::take(&mut s.targets),
                        std::mem::take(&mut s.queue),
                        s.current_event.take(),
                    )
                };
                delegate
                    .ok_or("missing shutdown delegate")?
                    .lifecycle(Lifecycle::WillTerminate)?;
                drop(retired);
            }
        }
        Ok(true)
    }
    pub fn resume(&self) -> Result<()> {
        self.own_thread()?;
        self.model.with(|_| Ok(()))?;
        let mut s = self.state.lock().map_err(|_| "loop poisoned")?;
        if s.phase != Phase::Stopped {
            return Err("only a stopped loop may resume".into());
        }
        s.phase = Phase::Running;
        Ok(())
    }
    pub fn phase(&self) -> Result<Phase> {
        self.model.with(|_| Ok(()))?;
        self.phase_snapshot()
    }

    pub(super) fn phase_snapshot(&self) -> Result<Phase> {
        Ok(self.state.lock().map_err(|_| "loop poisoned")?.phase)
    }
    pub fn retire(&self) -> Result<()> {
        self.own_thread()?;
        let retired = {
            let mut s = self.state.lock().map_err(|_| "loop poisoned")?;
            s.phase = Phase::Retired;
            (
                s.delegate.take(),
                std::mem::take(&mut s.targets),
                std::mem::take(&mut s.queue),
                s.current_event.take(),
            )
        };
        drop(retired);
        Ok(())
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.phase = Phase::Retired;
        let retired = (
            state.delegate.take(),
            std::mem::take(&mut state.targets),
            std::mem::take(&mut state.queue),
            state.current_event.take(),
        );
        if std::thread::current().id() == self.owner {
            drop(retired);
        } else {
            // No foreign-thread Arc finalizer may dispatch Objective-C cleanup.
            std::mem::forget(retired);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retire_drops_delegate_and_targets_outside_mutex_on_owner() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Reenter {
            engine: std::sync::Weak<Engine>,
            drops: Arc<AtomicUsize>,
            unlocked: Arc<AtomicUsize>,
        }
        impl Callback for Reenter {
            fn lifecycle(&self, _: Lifecycle) -> Result<()> {
                Ok(())
            }
            fn action(&self, _: &str) -> Result<()> {
                Ok(())
            }
        }
        impl Drop for Reenter {
            fn drop(&mut self) {
                self.drops.fetch_add(1, Ordering::SeqCst);
                if let Some(engine) = self.engine.upgrade() {
                    if engine.state.try_lock().is_ok() {
                        self.unlocked.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
        }
        let f = super::super::tests::Fixture::new();
        let engine = Engine::new(f.model.clone()).unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let unlocked = Arc::new(AtomicUsize::new(0));
        let make = || {
            Arc::new(Reenter {
                engine: Arc::downgrade(&engine),
                drops: drops.clone(),
                unlocked: unlocked.clone(),
            })
        };
        engine.set_delegate(make()).unwrap();
        engine
            .register_target(
                Handle {
                    id: 808,
                    generation: 1,
                },
                make(),
            )
            .unwrap();
        engine.retire().unwrap();
        assert_eq!(
            drops.load(Ordering::SeqCst),
            2,
            "retire must release delegate and targets"
        );
        assert_eq!(
            unlocked.load(Ordering::SeqCst),
            2,
            "callback destructors must run outside state mutex"
        );
    }
    #[test]
    fn foreign_retire_does_not_drop_selected_callbacks() {
        let f = super::super::tests::Fixture::new();
        let engine = Engine::new(f.model.clone()).unwrap();
        let foreign = engine.clone();
        assert!(std::thread::spawn(move || foreign.retire())
            .join()
            .unwrap()
            .is_err());
        assert_eq!(engine.phase().unwrap(), Phase::Created);
    }
    #[test]
    fn clear_unregister_terminate_and_engine_drop_release_without_mutex_reentry() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Reenter {
            engine: std::sync::Weak<Engine>,
            calls: Arc<AtomicUsize>,
        }
        impl Callback for Reenter {
            fn lifecycle(&self, _: Lifecycle) -> Result<()> {
                Ok(())
            }
            fn action(&self, _: &str) -> Result<()> {
                Ok(())
            }
        }
        impl Drop for Reenter {
            fn drop(&mut self) {
                if let Some(engine) = self.engine.upgrade() {
                    assert!(
                        engine.state.try_lock().is_ok(),
                        "destructor called under mutex"
                    );
                    // Actually reenter the public owner state API after the
                    // nonblocking lock witness; a regression cannot hang here.
                    engine.phase().unwrap();
                }
                self.calls.fetch_add(1, Ordering::SeqCst);
            }
        }
        for lane in 0..3 {
            let f = super::super::tests::Fixture::new();
            let engine = Engine::new(f.model.clone()).unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let callback = || {
                Arc::new(Reenter {
                    engine: Arc::downgrade(&engine),
                    calls: calls.clone(),
                })
            };
            engine.set_delegate(callback()).unwrap();
            let h = Handle {
                id: 809,
                generation: 1,
            };
            engine.register_target(h, callback()).unwrap();
            match lane {
                0 => {
                    engine.clear_delegate().unwrap();
                    engine.unregister_target(h).unwrap();
                }
                1 => {
                    engine.launch().unwrap();
                    engine.enqueue(Event::Terminate).unwrap();
                    engine.pump().unwrap();
                }
                _ => {
                    drop(engine);
                }
            }
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
    }
    struct Trace(Arc<Mutex<Vec<String>>>);
    impl Callback for Trace {
        fn lifecycle(&self, e: Lifecycle) -> Result<()> {
            self.0.lock().unwrap().push(format!("{e:?}"));
            Ok(())
        }
        fn action(&self, s: &str) -> Result<()> {
            if !["toggle:", "openSettings:"].contains(&s) {
                return Err("unknown control selector".into());
            }
            self.0.lock().unwrap().push(s.into());
            Ok(())
        }
    }
    #[test]
    fn real_owned_loop_dispatch_preserves_launch_control_stop_resume_terminate_order() {
        let f = super::super::tests::Fixture::new();
        let engine = Engine::new(f.model.clone()).unwrap();
        let trace = Arc::new(Mutex::new(vec![]));
        let callback = Arc::new(Trace(trace.clone()));
        engine.set_delegate(callback.clone()).unwrap();
        let h = Handle {
            id: 300,
            generation: 1,
        };
        engine.register_target(h, callback).unwrap();
        engine
            .enqueue(Event::Action {
                target: h,
                selector: "toggle:".into(),
            })
            .unwrap();
        engine.enqueue(Event::Stop).unwrap();
        engine.launch().unwrap();
        assert!(engine.launch().is_err());
        assert!(engine.pump().unwrap());
        assert!(engine.pump().unwrap());
        assert_eq!(engine.phase().unwrap(), Phase::Stopped);
        assert_eq!(
            *trace.lock().unwrap(),
            vec!["WillFinishLaunching", "DidFinishLaunching", "toggle:"]
        );
        engine.resume().unwrap();
        engine
            .enqueue(Event::Action {
                target: h,
                selector: "openSettings:".into(),
            })
            .unwrap();
        engine.enqueue(Event::Terminate).unwrap();
        assert!(engine.pump().unwrap());
        assert!(engine.pump().unwrap());
        assert_eq!(
            *trace.lock().unwrap(),
            vec![
                "WillFinishLaunching",
                "DidFinishLaunching",
                "toggle:",
                "openSettings:",
                "WillTerminate"
            ]
        );
        assert_eq!(engine.phase().unwrap(), Phase::Terminated);
        assert!(engine.enqueue(Event::Terminate).is_err());
        assert!(engine.pump().is_err());
    }
    #[test]
    fn missing_delegate_unknown_control_and_guardian_loss_do_not_fall_back() {
        use std::sync::atomic::Ordering;
        let f = super::super::tests::Fixture::new();
        let engine = Engine::new(f.model.clone()).unwrap();
        assert!(engine.launch().is_err());
        let trace = Arc::new(Mutex::new(vec![]));
        engine.set_delegate(Arc::new(Trace(trace.clone()))).unwrap();
        engine.launch().unwrap();
        engine
            .enqueue(Event::Action {
                target: Handle {
                    id: 99,
                    generation: 1,
                },
                selector: "toggle:".into(),
            })
            .unwrap();
        assert!(engine.pump().is_err());
        assert_eq!(trace.lock().unwrap().len(), 2);
        f.life.0.store(false, Ordering::Release);
        assert!(engine.enqueue(Event::Terminate).is_err());
        assert!(engine.pump().is_err());
        assert_eq!(trace.lock().unwrap().len(), 2);
    }
}
