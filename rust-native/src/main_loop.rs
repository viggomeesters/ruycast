//! Version-zero CF source: workers only signal/wake; callbacks and their Rust
//! owners remain on the main thread until source invalidation after loop exit.
use objc2::MainThreadMarker;
use objc2_core_foundation::{
    kCFRunLoopCommonModes, CFRetained, CFRunLoop, CFRunLoopSource, CFRunLoopSourceContext,
};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

struct Callback {
    perform: RefCell<Box<dyn FnMut()>>,
    failed: Cell<bool>,
    deferred: Cell<bool>,
    #[cfg(feature = "test-tools")]
    deferrals: Cell<u64>,
    signal: RefCell<Option<Arc<Signal>>>,
    failure: RefCell<Box<dyn FnMut()>>,
}
fn perform_failure(state: &Callback) {
    let Ok(mut callback) = state.failure.try_borrow_mut() else {
        state.deferred.set(true);
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        objc2::rc::autoreleasepool(|_| callback())
    }));
    drop(callback);
    if state.deferred.replace(false) {
        if let Some(signal) = state.signal.borrow().as_ref() {
            signal.wake();
        }
    }
}
unsafe extern "C-unwind" fn perform(info: *mut c_void) {
    // SAFETY: Box stays allocated in Source, registered exclusively on main.
    // Source::drop invalidates/removes it before dropping the callback owner.
    let state = unsafe { &*info.cast::<Callback>() };
    if state.failed.get() {
        perform_failure(state);
        return;
    }
    // AppKit can enter a nested run loop while ending field editing. Consume
    // the nested source delivery without re-entering mutable controller state;
    // the outer callback re-signals after its borrow has been released.
    let Ok(mut callback) = state.perform.try_borrow_mut() else {
        state.deferred.set(true);
        #[cfg(feature = "test-tools")]
        state.deferrals.set(state.deferrals.get().saturating_add(1));
        return;
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        objc2::rc::autoreleasepool(|_| callback())
    }));
    drop(callback);
    if outcome.is_err() {
        state.failed.set(true);
        perform_failure(state);
    } else if state.deferred.replace(false) {
        if let Some(signal) = state.signal.borrow().as_ref() {
            signal.wake();
        }
    }
}
pub struct Signal {
    source: CFRetained<CFRunLoopSource>,
    run_loop: CFRetained<CFRunLoop>,
}
// SAFETY: This private wrapper exposes only CFRunLoopSourceSignal and
// CFRunLoopWakeUp, which support cross-thread signaling, plus thread-safe CF
// reference counting. It never exposes/dereferences the main-only context.
// Invalidation precedes callback deallocation; late signaling is harmless.
unsafe impl Send for Signal {}
unsafe impl Sync for Signal {}
impl Signal {
    pub fn wake(&self) {
        self.source.signal();
        self.run_loop.wake_up();
    }
}
pub struct Source {
    signal: Arc<Signal>,
    callback: Box<Callback>,
    _main: MainThreadMarker,
}
impl Source {
    pub fn new(
        main: MainThreadMarker,
        callback: impl FnMut() + 'static,
        failure: impl FnMut() + 'static,
    ) -> Result<Self, String> {
        let mut callback = Box::new(Callback {
            perform: RefCell::new(Box::new(callback)),
            failed: Cell::new(false),
            deferred: Cell::new(false),
            #[cfg(feature = "test-tools")]
            deferrals: Cell::new(0),
            signal: RefCell::new(None),
            failure: RefCell::new(Box::new(failure)),
        });
        let mut context = CFRunLoopSourceContext {
            version: 0,
            info: (&mut *callback as *mut Callback).cast(),
            retain: None,
            release: None,
            copyDescription: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: Some(perform),
        };
        let run_loop = CFRunLoop::main().ok_or("Missing main run loop")?;
        let source = unsafe { CFRunLoopSource::new(None, 0, &mut context) }
            .ok_or("Native source allocation failed")?;
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
        let signal = Arc::new(Signal { source, run_loop });
        *callback.signal.borrow_mut() = Some(signal.clone());
        Ok(Self {
            signal,
            callback,
            _main: main,
        })
    }
    pub fn signal(&self) -> Arc<Signal> {
        self.signal.clone()
    }
    pub fn failed(&self) -> bool {
        self.callback.failed.get()
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.signal.source.invalidate();
        self.signal
            .run_loop
            .remove_source(Some(&self.signal.source), unsafe { kCFRunLoopCommonModes });
    }
}

// Main-owned one-shot scheduling; callers bound/coalesce timer ownership. The timer and callback are both
// owned by the main controller and invalidated before their Rust data drops.
use objc2_core_foundation::{CFAbsoluteTimeGetCurrent, CFRunLoopTimer, CFRunLoopTimerContext};
pub struct Timer {
    timer: CFRetained<CFRunLoopTimer>,
    _callback: Box<Option<Box<dyn FnOnce()>>>,
    _main: MainThreadMarker,
}
unsafe extern "C-unwind" fn timer_perform(_: *mut CFRunLoopTimer, info: *mut c_void) {
    let callback = unsafe { &mut *info.cast::<Option<Box<dyn FnOnce()>>>() };
    if let Some(callback) = callback.take() {
        let _ = catch_unwind(AssertUnwindSafe(callback));
    }
}
impl Timer {
    pub fn new_common(
        main: MainThreadMarker,
        seconds: f64,
        callback: impl FnOnce() + 'static,
    ) -> Result<Self, String> {
        Self::with_mode(main, seconds, callback, true)
    }
    #[cfg(feature = "test-tools")]
    pub fn new(
        main: MainThreadMarker,
        seconds: f64,
        callback: impl FnOnce() + 'static,
    ) -> Result<Self, String> {
        Self::with_mode(main, seconds, callback, true)
    }
    pub fn new_default(
        main: MainThreadMarker,
        seconds: f64,
        callback: impl FnOnce() + 'static,
    ) -> Result<Self, String> {
        Self::with_mode(main, seconds, callback, false)
    }
    fn with_mode(
        main: MainThreadMarker,
        seconds: f64,
        callback: impl FnOnce() + 'static,
        common: bool,
    ) -> Result<Self, String> {
        let mut callback: Box<Option<Box<dyn FnOnce()>>> = Box::new(Some(Box::new(callback)));
        let mut context = CFRunLoopTimerContext {
            version: 0,
            info: (&mut *callback as *mut Option<Box<dyn FnOnce()>>).cast(),
            retain: None,
            release: None,
            copyDescription: None,
        };
        let timer = unsafe {
            CFRunLoopTimer::new(
                None,
                CFAbsoluteTimeGetCurrent() + seconds,
                0.,
                0,
                0,
                Some(timer_perform),
                &mut context,
            )
        }
        .ok_or("Native timer allocation failed")?;
        CFRunLoop::main()
            .ok_or("Missing main loop")?
            .add_timer(Some(&timer), unsafe {
                if common {
                    kCFRunLoopCommonModes
                } else {
                    objc2_core_foundation::kCFRunLoopDefaultMode
                }
            });
        Ok(Self {
            timer,
            _callback: callback,
            _main: main,
        })
    }
    pub fn valid(&self) -> bool {
        self.timer.is_valid()
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        self.timer.invalidate();
    }
}

// Deterministic nested-run-loop regression through real CF source delivery.
#[cfg(feature = "test-tools")]
pub fn check_reentry(main: MainThreadMarker) -> Result<u64, String> {
    use objc2_core_foundation::kCFRunLoopDefaultMode;
    use std::rc::Rc;
    let calls = Rc::new(Cell::new(0));
    let nested = Rc::new(RefCell::new(None::<Arc<Signal>>));
    let calls_inside = calls.clone();
    let nested_inside = nested.clone();
    let source = Source::new(
        main,
        move || {
            calls_inside.set(calls_inside.get() + 1);
            if calls_inside.get() == 1 {
                nested_inside
                    .borrow()
                    .as_ref()
                    .expect("source installed")
                    .wake();
                CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 0.005, false);
            }
        },
        || {},
    )?;
    *nested.borrow_mut() = Some(source.signal());
    source.signal().wake();
    for _ in 0..8 {
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 0.002, true);
        if calls.get() >= 2 {
            break;
        }
    }
    if source.failed() || calls.get() != 2 || source.callback.deferrals.get() == 0 {
        return Err(format!(
            "Nested source failed or lost delivery: {}",
            calls.get()
        ));
    }
    Ok(source.callback.deferrals.get())
}
