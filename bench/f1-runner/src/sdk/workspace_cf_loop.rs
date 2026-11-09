//! Process-local CF loop/source, never NSApplication.run or a desktop source.
//! Invalidation/removal precedes context destruction; retained late wake handles
//! cannot enter a dead callback. No threads/processes are spawned here.
use super::*;
use event_loop::{Engine, Phase};
use objc2_core_foundation::{
    kCFRunLoopCommonModes, kCFRunLoopDefaultMode, CFRetained, CFRunLoop, CFRunLoopMode,
    CFRunLoopSource, CFRunLoopSourceContext,
};
use std::{
    cell::Cell,
    ffi::c_void,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy)]
pub enum SourceMode {
    Default,
    Common,
}
/// One genuine caller-owned CF source on the creating thread's genuine loop.
/// No table of raw CF pointers or alternate run-loop implementation is used.
pub struct OwnedSource {
    model: Arc<Workspace>,
    run_loop: CFRetained<CFRunLoop>,
    mode: CFRetained<CFRunLoopMode>,
    default_callback_mode: CFRetained<CFRunLoopMode>,
    source: Option<CFRetained<CFRunLoopSource>>,
    owner: std::thread::ThreadId,
    used: bool,
    closed: bool,
    _owner_only: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl OwnedSource {
    pub fn current(model: Arc<Workspace>, mode: SourceMode) -> Result<Self> {
        model.with(|_| Ok(()))?;
        let run_loop = CFRunLoop::current().ok_or("missing owned current loop")?;
        let mode = unsafe {
            match mode {
                SourceMode::Default => kCFRunLoopDefaultMode,
                SourceMode::Common => kCFRunLoopCommonModes,
            }
        }
        .ok_or("missing standard CF mode")?;
        let default_callback_mode =
            unsafe { kCFRunLoopDefaultMode }.ok_or("missing concrete default CF mode")?;
        Ok(Self {
            model,
            run_loop,
            mode: unsafe { CFRetained::retain(std::ptr::NonNull::from(mode)) },
            default_callback_mode: unsafe {
                CFRetained::retain(std::ptr::NonNull::from(default_callback_mode))
            },
            source: None,
            owner: std::thread::current().id(),
            used: false,
            closed: false,
            _owner_only: std::marker::PhantomData,
        })
    }
    fn check(&self) -> Result<()> {
        if self.owner != std::thread::current().id() {
            return Err("owned CF source on foreign thread".into());
        }
        if self.closed {
            return Err("owned CF source registration retired".into());
        }
        self.model.with(|_| Ok(()))
    }
    /// Requested add/remove identities, pinned by this value. Common is a
    /// pseudo-mode and must never authorize a concrete schedule/cancel mode.
    pub fn guarded_add_remove_identities(&self) -> Result<(usize, usize)> {
        self.check()?;
        Ok((
            &*self.run_loop as *const CFRunLoop as usize,
            &*self.mode as *const CFRunLoopMode as usize,
        ))
    }
    /// The sole qualified concrete callback mode is Default. Even when attached
    /// via Common, other modes (including native event tracking) are unproven
    /// and must be refused without decoding their raw pointers.
    pub fn guarded_default_callback_identities(&self) -> Result<(usize, usize)> {
        self.check()?;
        Ok((
            &*self.run_loop as *const CFRunLoop as usize,
            &*self.default_callback_mode as *const CFRunLoopMode as usize,
        ))
    }
    pub fn verify_default_callback_identities(&self, run_loop: usize, mode: usize) -> Result<()> {
        if self.guarded_default_callback_identities()? != (run_loop, mode) {
            return Err("unqualified concrete CF callback loop/mode".into());
        }
        Ok(())
    }
    /// # Safety
    /// Source and every CF callback/context are own synthetic resources. Their
    /// context outlives source invalidation/finalization, contains panics at the
    /// foreign boundary, and permits cleanup after authority retirement.
    pub unsafe fn attach(&mut self, source: CFRetained<CFRunLoopSource>) -> Result<()> {
        self.check()?;
        if self.used {
            return Err("owned CF source attachment already consumed".into());
        }
        self.used = true;
        self.source = Some(source);
        self.run_loop
            .add_source(self.source.as_deref(), Some(&self.mode));
        Ok(())
    }
    pub fn remove(&mut self) -> Result<()> {
        if self.owner != std::thread::current().id() {
            return Err("owned CF source removal on foreign thread".into());
        }
        self.remove_owned();
        Ok(())
    }
    fn remove_owned(&mut self) {
        self.closed = true;
        if let Some(source) = self.source.take() {
            source.invalidate();
            self.run_loop.remove_source(Some(&source), Some(&self.mode));
        }
    }
}
impl Drop for OwnedSource {
    fn drop(&mut self) {
        self.remove_owned();
    }
}
pub struct Wake {
    source: CFRetained<CFRunLoopSource>,
    run_loop: CFRetained<CFRunLoop>,
    model: Arc<Workspace>,
    closed: AtomicBool,
}
// Only thread-safe CF retain/signal/wake/invalidate-state operations are exposed;
// no callback context or owner-thread-only Engine dispatch can be dereferenced.
unsafe impl Send for Wake {}
unsafe impl Sync for Wake {}
impl Wake {
    // The retained owner wake is qualified outside the monitor queue lock.
    // This tail performs only thread-safe CF operations under that lock.
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(super) fn signal_worker_drain(&self) {
        if !self.closed.load(Ordering::Acquire) {
            self.source.signal();
            self.run_loop.wake_up();
        }
    }

    pub fn signal(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err("CF wake generation retired".into());
        }
        self.model.with(|_| Ok(()))?;
        self.source.signal();
        self.run_loop.wake_up();
        Ok(())
    }
}
struct Context {
    engine: Arc<Engine>,
    entered: Cell<bool>,
    deferred: Cell<bool>,
    failure: std::cell::RefCell<Option<String>>,
}
unsafe extern "C-unwind" fn perform(info: *mut c_void) {
    let context = unsafe { &*info.cast::<Context>() };
    if catch_unwind(AssertUnwindSafe(|| perform_owned(context))).is_err() {
        context.entered.set(false);
        context.deferred.set(false);
        context
            .engine
            .model()
            .faulted
            .store(true, Ordering::Release);
        if let Ok(mut failure) = context.failure.try_borrow_mut() {
            *failure = Some("owned CF callback boundary panicked; authority retired".into());
        }
        // Target cleanup can itself drop callback-owned Rust data. It is part
        // of the same foreign boundary, including an unknown destructor panic.
        let _ = catch_unwind(AssertUnwindSafe(|| context.engine.retire()));
    }
}
#[cfg(all(rcf1_reference_sdk,not(test)))]
pub(crate) fn perform_address()->usize {
    perform as *const () as usize
}
fn perform_owned(context: &Context) {
    if context.entered.replace(true) {
        context.deferred.set(true);
        return;
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| ->Result<()> {
        context.engine.pump()?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        if super::callbacks::reference_open_shutdown_waiting() {
            return Ok(());
        }
        #[cfg(rcf1_reference_sdk)]
        if context.engine.phase()? == Phase::Running {
            // WillTerminate can invalidate original child custody. After
            // the terminal event, only the dedicated teardown may use it.
            crate::sdk::reference_runtime::poll_main()?;
        }
        Ok(())
    }));
    context.entered.set(false);
    match outcome {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            *context.failure.borrow_mut() = Some(e);
            let _ = context.engine.retire();
        }
        Err(_) => {
            *context.failure.borrow_mut() =
                Some("owned CF callback panicked; unknown outcome, no retry".into());
            let _ = context.engine.retire();
        }
    }
    let resignal = context.deferred.replace(false) || context.engine.has_events();
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    let resignal = resignal && !super::callbacks::reference_open_shutdown_waiting();
    if resignal {
        let _ = context.engine.signal();
    }
}
struct Source {
    wake: Arc<Wake>,
    context: Box<Context>,
}
impl Source {
    fn attach(engine: Arc<Engine>) -> Result<Self> {
        engine.model().with(|_| Ok(()))?;
        let mut context = Box::new(Context {
            engine: engine.clone(),
            entered: Cell::new(false),
            deferred: Cell::new(false),
            failure: std::cell::RefCell::new(None),
        });
        let mut setup = CFRunLoopSourceContext {
            version: 0,
            info: (&mut *context as *mut Context).cast(),
            retain: None,
            release: None,
            copyDescription: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: Some(perform),
        };
        let run_loop = CFRunLoop::current().ok_or("missing owned current run loop")?;
        let source = unsafe { CFRunLoopSource::new(None, 0, &mut setup) }
            .ok_or("owned CF source allocation failed")?;
        let wake = Arc::new(Wake {
            source,
            run_loop,
            model: engine.model().clone(),
            closed: AtomicBool::new(false),
        });
        engine.attach_wake(wake.clone())?;
        wake.run_loop
            .add_source(Some(&wake.source), unsafe { kCFRunLoopDefaultMode });
        if engine.has_events() {
            wake.signal()?;
        }
        Ok(Self { wake, context })
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.wake.closed.store(true, Ordering::Release);
        self.wake.source.invalidate();
        self.wake
            .run_loop
            .remove_source(Some(&self.wake.source), unsafe { kCFRunLoopDefaultMode });
        self.context.engine.detach_wake();
    }
}
pub fn run(engine: Arc<Engine>) -> Result<Phase> {
    engine.check_owner()?;
    let source = Source::attach(engine.clone())?;
    // Declared after Source: worker/owner teardown also precedes Source removal
    // when launch, a CF callback or ordinary Rust code returns early/unwinds.
    #[cfg(rcf1_reference_sdk)]
    struct ReferenceTeardown { engine:Arc<Engine>, armed:bool }
    #[cfg(rcf1_reference_sdk)]
    impl ReferenceTeardown {
        fn finish(&mut self)->Result<()> {
            self.armed=false;
            crate::sdk::reference_runtime::after_loop(self.engine.clone())
        }
    }
    #[cfg(rcf1_reference_sdk)]
    impl Drop for ReferenceTeardown {
        fn drop(&mut self) {
            if !self.armed { return; }
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||self.finish()));
            if !matches!(result,Ok(Ok(()))) {
                if let Err(payload)=result { std::mem::forget(payload); }
                self.engine.model().fault();
            }
        }
    }
    #[cfg(rcf1_reference_sdk)]
    let mut teardown=ReferenceTeardown {engine:engine.clone(),armed:true};
    match engine.phase()? {
        Phase::Created => engine.launch()?,
        Phase::Stopped => engine.resume()?,
        _ => return Err("application loop already entered/closed".into()),
    }
    #[cfg(rcf1_reference_sdk)]
    if let Err(startup) = crate::sdk::reference_runtime::after_launch(engine.clone()) {
        // Finish through the guard so its Drop cannot retry retirement.
        return match teardown.finish() {
            Ok(()) => Err(startup),
            Err(cleanup) => Err(format!("{startup}; Reference startup cleanup failed: {cleanup}")),
        };
    }
    while engine.phase()? == Phase::Running {
        engine.model().with(|_| Ok(()))?;
        // Native/legacy main-queue and owned timers remain genuine process-local CF
        // sources. No polling cadence is imposed on idle workloads.
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 60.0, true);
        if let Some(problem) = source.context.failure.borrow().as_ref() {
            return Err(problem.clone());
        }
    }
    let phase = engine.phase()?;
    #[cfg(rcf1_reference_sdk)]
    teardown.finish()?;
    drop(source);
    #[cfg(rcf1_reference_sdk)]
    crate::sdk::reference_runtime::report_field_probe_completion(&engine, phase)?;
    Ok(phase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use event_loop::{Callback, Event, Lifecycle};
    #[test]
    fn genuine_owned_source_pins_loop_mode_and_removes_after_model_retirement() {
        use std::sync::atomic::AtomicUsize;
        struct Info {
            identities: (usize, usize),
            scheduled: AtomicUsize,
            cancelled: AtomicUsize,
            bad: AtomicBool,
        }
        unsafe extern "C-unwind" fn schedule(
            info: *mut c_void,
            run_loop: *mut CFRunLoop,
            mode: *const CFRunLoopMode,
        ) {
            let info = unsafe { &*info.cast::<Info>() };
            if info.identities != (run_loop as usize, mode as usize) {
                info.bad.store(true, Ordering::Release);
            }
            info.scheduled.fetch_add(1, Ordering::SeqCst);
        }
        unsafe extern "C-unwind" fn cancel(
            info: *mut c_void,
            run_loop: *mut CFRunLoop,
            mode: *const CFRunLoopMode,
        ) {
            let info = unsafe { &*info.cast::<Info>() };
            if info.identities != (run_loop as usize, mode as usize) {
                info.bad.store(true, Ordering::Release);
            }
            info.cancelled.fetch_add(1, Ordering::SeqCst);
        }
        let f = super::super::tests::Fixture::new();
        let mut registration = OwnedSource::current(f.model.clone(), SourceMode::Default).unwrap();
        let identities = registration.guarded_default_callback_identities().unwrap();
        assert_eq!(
            registration.guarded_add_remove_identities().unwrap(),
            identities
        );
        let mut info = Box::new(Info {
            identities,
            scheduled: AtomicUsize::new(0),
            cancelled: AtomicUsize::new(0),
            bad: AtomicBool::new(false),
        });
        let mut context = CFRunLoopSourceContext {
            version: 0,
            info: (&mut *info as *mut Info).cast(),
            retain: None,
            release: None,
            copyDescription: None,
            equal: None,
            hash: None,
            schedule: Some(schedule),
            cancel: Some(cancel),
            perform: None,
        };
        let source = unsafe { CFRunLoopSource::new(None, 0, &mut context) }.unwrap();
        unsafe { registration.attach(source.clone()) }.unwrap();
        assert!(registration
            .run_loop
            .contains_source(Some(&source), Some(&registration.mode)));
        assert_eq!(info.scheduled.load(Ordering::SeqCst), 1);
        assert!(unsafe { registration.attach(source.clone()) }.is_err());
        f.model.retire().unwrap();
        assert!(registration.guarded_add_remove_identities().is_err());
        assert!(registration.guarded_default_callback_identities().is_err());
        registration.remove().unwrap();
        assert!(!source.is_valid());
        assert!(!registration
            .run_loop
            .contains_source(Some(&source), Some(&registration.mode)));
        assert_eq!(info.cancelled.load(Ordering::SeqCst), 1);
        assert!(!info.bad.load(Ordering::Acquire));
        assert!(unsafe { registration.attach(source.clone()) }.is_err());
        drop(registration);
        drop(source);
        assert_eq!(info.cancelled.load(Ordering::SeqCst), 1);
        let common_fixture = super::super::tests::Fixture::new();
        let mut common =
            OwnedSource::current(common_fixture.model.clone(), SourceMode::Common).unwrap();
        let requested = common.guarded_add_remove_identities().unwrap();
        let concrete = common.guarded_default_callback_identities().unwrap();
        assert_ne!(requested.1, concrete.1);
        assert_eq!(concrete, identities);
        assert!(common
            .verify_default_callback_identities(requested.0, requested.1)
            .is_err());
        assert!(common
            .verify_default_callback_identities(concrete.0, 1)
            .is_err());
        assert!(common
            .verify_default_callback_identities(1, concrete.1)
            .is_err());
        common
            .verify_default_callback_identities(concrete.0, concrete.1)
            .unwrap();
        let mut common_info = Box::new(Info {
            identities: concrete,
            scheduled: AtomicUsize::new(0),
            cancelled: AtomicUsize::new(0),
            bad: AtomicBool::new(false),
        });
        context.info = (&mut *common_info as *mut Info).cast();
        let common_source = unsafe { CFRunLoopSource::new(None, 0, &mut context) }.unwrap();
        unsafe { common.attach(common_source.clone()) }.unwrap();
        assert_eq!(common_info.scheduled.load(Ordering::SeqCst), 1);
        assert!(!common_info.bad.load(Ordering::Acquire));
        common.remove().unwrap();
        assert_eq!(common_info.cancelled.load(Ordering::SeqCst), 1);
        assert!(!common_info.bad.load(Ordering::Acquire));
        assert!(!common_source.is_valid());
        drop(common);
        drop(common_source);
    }
    struct Sink(Arc<Mutex<Vec<String>>>);
    impl Callback for Sink {
        fn lifecycle(&self, e: Lifecycle) -> Result<()> {
            self.0.lock().unwrap().push(format!("{e:?}"));
            Ok(())
        }
        fn action(&self, s: &str) -> Result<()> {
            self.0.lock().unwrap().push(s.into());
            Ok(())
        }
    }
    #[test]
    fn actual_cf_callback_contains_late_wake_lease_panic_and_retires_without_replay() {
        use crate::sdk::owned::AliveLease;
        use std::sync::atomic::AtomicUsize;
        struct Life {
            calls: AtomicUsize,
            panic_at: AtomicUsize,
        }
        impl AliveLease for Life {
            fn is_alive(&self) -> bool {
                let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
                assert_ne!(
                    call,
                    self.panic_at.load(Ordering::Acquire),
                    "own late wake panic"
                );
                true
            }
            fn before_effect(&self) -> Result<()> {
                Ok(())
            }
        }
        struct Arm(Arc<Life>);
        impl Callback for Arm {
            fn lifecycle(&self, _: Lifecycle) -> Result<()> {
                Ok(())
            }
            fn action(&self, _: &str) -> Result<()> {
                // wake_handle first admits through Workspace::with. The second
                // admission is Wake::signal, after pump has already returned.
                self.0
                    .panic_at
                    .store(self.0.calls.load(Ordering::Acquire) + 2, Ordering::Release);
                Ok(())
            }
        }
        let f = super::super::tests::Fixture::new();
        let life = Arc::new(Life {
            calls: AtomicUsize::new(0),
            panic_at: AtomicUsize::new(usize::MAX),
        });
        let mut context = f.model.context.clone();
        context.life = life.clone();
        let model = Workspace::new(
            context,
            f.model
                .applications()
                .unwrap()
                .into_iter()
                .map(|(_, app)| app)
                .collect(),
            f.model.screens().unwrap(),
            f.model.mouse().unwrap(),
        )
        .unwrap();
        let engine = Engine::new(model.clone()).unwrap();
        let arm = Arc::new(Arm(life.clone()));
        engine.set_delegate(arm.clone()).unwrap();
        let target = Handle {
            id: 507,
            generation: 1,
        };
        engine.register_target(target, arm).unwrap();
        engine
            .enqueue(Event::Action {
                target,
                selector: "ownedArm:".into(),
            })
            .unwrap();
        engine.enqueue(Event::Stop).unwrap();
        let error = run(engine.clone()).unwrap_err();
        assert!(error.contains("authority retired"), "{error}");
        assert!(life.calls.load(Ordering::Acquire) >= life.panic_at.load(Ordering::Acquire));
        life.panic_at.store(usize::MAX, Ordering::Release);
        assert!(model.screens().is_err());
        assert!(engine.resume().is_err());
        assert!(engine.signal().is_err());
    }
    #[test]
    fn actual_own_cf_source_dispatch_drains_before_context_drop_and_resume_does_not_relaunch() {
        let f = super::super::tests::Fixture::new();
        let engine = Engine::new(f.model.clone()).unwrap();
        let trace = Arc::new(Mutex::new(vec![]));
        let sink = Arc::new(Sink(trace.clone()));
        engine.set_delegate(sink.clone()).unwrap();
        let h = Handle {
            id: 503,
            generation: 1,
        };
        engine.register_target(h, sink).unwrap();
        engine
            .enqueue(Event::Action {
                target: h,
                selector: "ownedButton:".into(),
            })
            .unwrap();
        engine.enqueue(Event::Stop).unwrap();
        assert_eq!(run(engine.clone()).unwrap(), Phase::Stopped);
        assert!(engine.signal().is_err());
        assert_eq!(
            *trace.lock().unwrap(),
            vec!["WillFinishLaunching", "DidFinishLaunching", "ownedButton:"]
        );
        engine.enqueue(Event::Terminate).unwrap();
        assert_eq!(run(engine.clone()).unwrap(), Phase::Terminated);
        assert_eq!(
            *trace.lock().unwrap(),
            vec![
                "WillFinishLaunching",
                "DidFinishLaunching",
                "ownedButton:",
                "WillTerminate"
            ]
        );
        assert!(run(engine).is_err());
    }
}
