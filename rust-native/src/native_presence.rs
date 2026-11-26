//! Running-application invalidation. The observer performs no AppKit work.
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, define_class, msg_send, rc::Retained,
    runtime::AnyObject,
};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{
    NSDictionary, NSKeyValueObservingOptions, NSObject, NSObjectNSKeyValueObserverRegistration,
    NSString,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

struct ObserverState {
    epoch: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
    context: usize,
    wake: Arc<dyn Fn() + Send + Sync>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = AnyThread]
    #[ivars = ObserverState]
    struct Observer;
    impl Observer {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        unsafe fn observe(&self, _key: Option<&NSString>, _object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSString, AnyObject>>, context: *mut std::ffi::c_void) {
            let state = self.ivars();
            if context as usize == state.context && !state.closed.load(Ordering::Acquire) {
                let _ = state.epoch.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| value.checked_add(1));
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (state.wake)()));
            }
        }
    }
);

/// Owned and dropped on the AppKit thread. Late wake hints must be harmless.
pub struct Observation {
    observed: Retained<NSObject>,
    observer: Retained<Observer>,
    key: Retained<NSString>,
    closed: Arc<AtomicBool>,
    context: usize,
    _main_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Observation {
    pub fn start(
        _main: MainThreadMarker,
        epoch: Arc<AtomicU64>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self::on(
            NSWorkspace::sharedWorkspace().into_super(),
            "runningApplications",
            epoch,
            wake,
        )
    }

    fn on(
        observed: Retained<NSObject>,
        name: &str,
        epoch: Arc<AtomicU64>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let key = NSString::from_str(name);
        let closed = Arc::new(AtomicBool::new(false));
        let context = Arc::as_ptr(&closed) as usize;
        let observer = Observer::alloc().set_ivars(ObserverState {
            epoch,
            closed: closed.clone(),
            context,
            wake,
        });
        let observer: Retained<Observer> = unsafe { msg_send![super(observer), init] };
        unsafe {
            observed.addObserver_forKeyPath_options_context(
                &observer,
                &key,
                NSKeyValueObservingOptions::empty(),
                context as *mut std::ffi::c_void,
            );
        }
        Self {
            observed,
            observer,
            key,
            closed,
            context,
            _main_thread: std::marker::PhantomData,
        }
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        unsafe {
            self.observed.removeObserver_forKeyPath_context(
                &self.observer,
                &self.key,
                self.context as *mut std::ffi::c_void,
            );
        }
    }
}

pub fn raycast_absent() -> Option<bool> {
    ruycast_native::performance_trace::shared().measure(ruycast_native::performance_trace::Stage::MenuHealthRaycastLookup, || objc2::rc::autoreleasepool(|_| {
        let apps = NSWorkspace::sharedWorkspace().runningApplications();
        let expected = NSString::from_str("com.raycast.macos");
        Some(!apps.iter().any(|app| {
            app.bundleIdentifier()
                .is_some_and(|id| *id == *expected)
        }))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSObjectNSKeyValueObserverNotification;
    use std::sync::atomic::AtomicUsize;

    define_class!(
        #[unsafe(super = NSObject)]
        #[thread_kind = AnyThread]
        #[ivars = ()]
        struct Fixture;
        impl Fixture {
            #[unsafe(method(fixtureValue))]
            fn value(&self) -> usize { 0 }
        }
    );

    #[test]
    fn kvo_invalidates_and_stops_after_observer_is_removed() {
        objc2::rc::autoreleasepool(|_| {
            let fixture = Fixture::alloc().set_ivars(());
            let fixture: Retained<Fixture> = unsafe { msg_send![super(fixture), init] };
            let epoch = Arc::new(AtomicU64::new(0));
            let wakes = Arc::new(AtomicUsize::new(0));
            let worker_wakes = wakes.clone();
            let observer = Observation::on(
                fixture.clone().into_super(),
                "fixtureValue",
                epoch.clone(),
                Arc::new(move || {
                    worker_wakes.fetch_add(1, Ordering::SeqCst);
                }),
            );
            let key = NSString::from_str("fixtureValue");
            fixture.willChangeValueForKey(&key);
            fixture.didChangeValueForKey(&key);
            assert_eq!(epoch.load(Ordering::Acquire), 1);
            assert_eq!(wakes.load(Ordering::SeqCst), 1);
            unsafe { observer.observer.observe(objc2::sel!(observeValueForKeyPath:ofObject:change:context:), None, None, None, std::ptr::null_mut()) };
            assert_eq!(epoch.load(Ordering::Acquire), 1);
            assert_eq!(wakes.load(Ordering::SeqCst), 1);
            epoch.store(u64::MAX - 1, Ordering::Release);
            fixture.willChangeValueForKey(&key);
            fixture.didChangeValueForKey(&key);
            fixture.willChangeValueForKey(&key);
            fixture.didChangeValueForKey(&key);
            assert_eq!(epoch.load(Ordering::Acquire), u64::MAX);
            assert_eq!(wakes.load(Ordering::SeqCst), 3);
            drop(observer);
            fixture.willChangeValueForKey(&key);
            fixture.didChangeValueForKey(&key);
            assert_eq!(epoch.load(Ordering::Acquire), u64::MAX);
            assert_eq!(wakes.load(Ordering::SeqCst), 3);
        });
    }
}
