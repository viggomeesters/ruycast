//! Recoverable Caps latch ownership for the HID service. No SDK call runs while
//! a state borrow is held. A failed write may have changed the latch; restore it
//! before closing, and retain the connection if restoration cannot be proved.
use std::cell::{Cell, RefCell};

/// Acquisition must own/clean intermediate resources until its successful return,
/// including if a Rust implementation unwinds. Close consumes ownership before
/// any possible unwind. Abandoned must retain Recovery and must not unwind.
/// Implementations must use nonzero handles, release the matching service even
/// when open fails, and never fall back from a fixture to real IOKit calls.
pub trait Io {
    fn open(&self) -> Result<u32, i32>;
    fn get(&self, connection: u32) -> Result<bool, i32>;
    fn set(&self, connection: u32, value: bool) -> Result<(), i32>;
    /// Consumes the connection, including on error (do not retry a stale port).
    fn close(&self, connection: u32) -> Result<(), i32>;
    /// Last-chance cleanup failed on owner destruction. No success may be
    /// reported; implementations must expose this failure without input data.
    fn abandoned(&self, error: Error, recovery: Option<Recovery>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Busy,
    Cancelled,
    Open(i32),
    Read(i32),
    Clear(i32),
    Restore(i32),
    Close(i32),
    Inactive,
    RecoveryPending,
    Panic(&'static str),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Inactive,
    Active,
    Busy,
    RecoveryPending(Error),
    Failed(Error),
}
#[derive(Debug)]
pub struct Recovery {
    pub(crate) handle: u32,
    pub(crate) original: Option<bool>,
    pub(crate) dirty: bool,
}
#[derive(Clone, Copy)]
struct Lease {
    handle: u32,
    original: Option<bool>,
    dirty: bool,
}

/// Main-owner component, deliberately !Sync. Reentrant callbacks may request
/// retirement but cannot reacquire or mutate the in-flight transaction.
pub struct Control<I: Io> {
    io: I,
    lease: RefCell<Option<Lease>>,
    busy: Cell<bool>,
    retire_requested: Cell<bool>,
    health: Cell<Health>,
}
impl<I: Io> Control<I> {
    pub fn new(io: I) -> Self {
        Self {
            io,
            lease: RefCell::new(None),
            busy: Cell::new(false),
            retire_requested: Cell::new(false),
            health: Cell::new(Health::Inactive),
        }
    }
    pub fn health(&self) -> Health {
        if self.busy.get() {
            Health::Busy
        } else {
            self.health.get()
        }
    }
    pub fn retired(&self) -> bool {
        !self.busy.get() && self.lease.borrow().is_none()
    }
    fn invoke<T>(
        &self,
        operation: &'static str,
        call: impl FnOnce() -> Result<T, i32>,
        error: fn(i32) -> Error,
    ) -> Result<T, Error> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(call))
            .map_err(|_| Error::Panic(operation))?
            .map_err(error)
    }
    fn enter(&self) -> Result<Option<Lease>, Error> {
        if self.busy.replace(true) {
            return Err(Error::Busy);
        }
        Ok(self.lease.borrow_mut().take())
    }
    fn finish(&self, lease: Option<Lease>, health: Health) {
        *self.lease.borrow_mut() = lease;
        self.health.set(health);
        self.busy.set(false);
    }
    fn cleanup(&self, lease: &mut Option<Lease>) -> Result<(), Error> {
        if let Some(mut value) = *lease {
            if value.dirty {
                // dirty implies a known original: never guess a latch value.
                let original = value.original.expect("mutation requires a known latch");
                self.invoke(
                    "restore",
                    || self.io.set(value.handle, original),
                    Error::Restore,
                )?;
                value.dirty = false;
                *lease = Some(value);
            }
            // Take before closing: IOServiceClose destroys the connection, so
            // a failure must not cause a retry through a potentially reused ID.
            *lease = None;
            self.invoke("close", || self.io.close(value.handle), Error::Close)?;
        }
        Ok(())
    }
    fn fail(&self, mut lease: Option<Lease>, cause: Error) -> Result<(), Error> {
        let result = self.cleanup(&mut lease).err().unwrap_or(cause);
        let health = if lease.is_some() {
            Health::RecoveryPending(result)
        } else if result == Error::Cancelled {
            Health::Inactive
        } else {
            Health::Failed(result)
        };
        self.finish(lease, health);
        Err(result)
    }
    /// Acquire once. Repeated refreshes do not overwrite the original latch.
    /// A prior failed restoration must retire before a new connection opens.
    pub fn activate(&self) -> Result<(), Error> {
        let mut lease = self.enter()?;
        self.retire_requested.set(false);
        if self.health.get() == Health::Active {
            self.finish(lease, Health::Active);
            return Ok(());
        }
        if let Err(error) = self.cleanup(&mut lease) {
            return self.fail_without_retry(lease, error);
        }
        if self.retire_requested.get() {
            return self.fail(lease, Error::Cancelled);
        }
        let handle = match self.invoke("open", || self.io.open(), Error::Open) {
            Ok(0) => return self.fail(None, Error::Open(-1)),
            Ok(handle) => handle,
            Err(error) => return self.fail(None, error),
        };
        lease = Some(Lease {
            handle,
            original: None,
            dirty: false,
        });
        if self.retire_requested.get() {
            return self.fail(lease, Error::Cancelled);
        }
        let original = match self.invoke("get", || self.io.get(handle), Error::Read) {
            Ok(value) => value,
            Err(error) => return self.fail(lease, error),
        };
        lease.as_mut().unwrap().original = Some(original);
        if self.retire_requested.get() {
            return self.fail(lease, Error::Cancelled);
        }
        // Even a failed set can have taken effect. Mark before crossing SDK.
        lease.as_mut().unwrap().dirty = true;
        if let Err(error) = self.invoke("clear", || self.io.set(handle, false), Error::Clear) {
            return self.fail(lease, error);
        }
        if self.retire_requested.get() {
            return self.fail(lease, Error::Cancelled);
        }
        self.finish(lease, Health::Active);
        Ok(())
    }
    fn fail_without_retry(&self, lease: Option<Lease>, error: Error) -> Result<(), Error> {
        let health = if lease.is_some() {
            Health::RecoveryPending(error)
        } else {
            Health::Failed(error)
        };
        self.finish(lease, health);
        Err(error)
    }
    /// Commit only after the complete Hyper service has started. Until then,
    /// acquisition failures restore the initial latch. Once Hyper owns Caps,
    /// retirement must not switch it back on from that stale startup snapshot.
    pub fn commit_off(&self) -> Result<(), Error> {
        let mut lease = self.enter()?;
        if self.health.get() != Health::Active {
            self.finish(lease, self.health.get());
            return Err(Error::Inactive);
        }
        if self.retire_requested.get() {
            return self.fail(lease, Error::Cancelled);
        }
        lease
            .as_mut()
            .expect("active control owns a connection")
            .original = Some(false);
        self.finish(lease, Health::Active);
        Ok(())
    }
    /// Clear after a physical Caps transition. A failure retires immediately;
    /// a failed restore remains recoverable by the subsequent refresh/stop.
    pub fn clear(&self) -> Result<(), Error> {
        self.clear_latch(false)
    }
    /// The system can update the latch after the physical/event callbacks.
    /// Check it during the existing refresh without repeatedly writing when off.
    pub fn reconcile(&self) -> Result<(), Error> {
        self.clear_latch(true)
    }
    fn clear_latch(&self, check_first: bool) -> Result<(), Error> {
        let lease = self.enter()?;
        if self.health.get() != Health::Active {
            let error = match self.health.get() {
                Health::RecoveryPending(error) => error,
                _ => Error::Inactive,
            };
            self.finish(lease, self.health.get());
            return Err(error);
        }
        let handle = lease.expect("active control owns a connection").handle;
        if check_first {
            let enabled = match self.invoke("get", || self.io.get(handle), Error::Read) {
                Ok(value) => value,
                Err(error) => return self.fail(lease, error),
            };
            if self.retire_requested.get() {
                return self.fail(lease, Error::Cancelled);
            }
            if !enabled {
                self.finish(lease, Health::Active);
                return Ok(());
            }
        }
        if let Err(error) = self.invoke("clear", || self.io.set(handle, false), Error::Clear) {
            return self.fail(lease, error);
        }
        if self.retire_requested.get() {
            return self.fail(lease, Error::Cancelled);
        }
        self.finish(lease, Health::Active);
        Ok(())
    }
    /// Safe during SDK reentry: records cancellation and returns Busy; the
    /// outer transaction checks it before reporting any active state.
    pub fn retire(&self) -> Result<(), Error> {
        self.retire_requested.set(true);
        let mut lease = self.enter()?;
        match self.cleanup(&mut lease) {
            Ok(()) => {
                self.finish(None, Health::Inactive);
                Ok(())
            }
            Err(error) => self.fail_without_retry(lease, error),
        }
    }
}
impl<I: Io> Drop for Control<I> {
    fn drop(&mut self) {
        if let Err(error) = self.retire() {
            // Transfer both the live handle and its original state. A warning
            // alone would lose recovery authority when this owner is dropped.
            let recovery = self.lease.get_mut().take().map(|lease| Recovery {
                handle: lease.handle,
                original: lease.original,
                dirty: lease.dirty,
            });
            self.io.abandoned(error, recovery);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, rc::Rc};
    type Hook = Rc<dyn Fn(&str)>;
    #[derive(Default)]
    struct Fake {
        value: Cell<bool>,
        live: Cell<bool>,
        calls: RefCell<Vec<String>>,
        failures: RefCell<BTreeSet<usize>>,
        hook: RefCell<Option<Hook>>,
        abandoned: RefCell<Vec<Error>>,
        recovery: RefCell<Vec<Recovery>>,
    }
    impl Fake {
        fn call(&self, name: &str) -> Result<(), i32> {
            self.calls.borrow_mut().push(name.into());
            let index = self.calls.borrow().len();
            let hook = self.hook.borrow().clone();
            if let Some(hook) = hook {
                hook(name);
            }
            if self.failures.borrow().contains(&index) {
                Err(-(index as i32))
            } else {
                Ok(())
            }
        }
        fn trace(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        fn fail(&self, indexes: &[usize]) {
            *self.failures.borrow_mut() = indexes.iter().copied().collect();
        }
    }
    impl Io for Rc<Fake> {
        fn open(&self) -> Result<u32, i32> {
            self.call("open")?;
            assert!(!self.live.replace(true), "duplicate acquisition");
            Ok(7)
        }
        fn get(&self, connection: u32) -> Result<bool, i32> {
            assert_eq!(connection, 7);
            assert!(self.live.get());
            self.call("get")?;
            Ok(self.value.get())
        }
        fn set(&self, connection: u32, value: bool) -> Result<(), i32> {
            assert_eq!(connection, 7);
            assert!(self.live.get());
            // Deliberately mutate on error: callers cannot assume no effect.
            self.value.set(value);
            self.call(if value { "set:true" } else { "set:false" })
        }
        fn close(&self, connection: u32) -> Result<(), i32> {
            assert_eq!(connection, 7);
            assert!(self.live.replace(false), "stale close");
            self.call("close")
        }
        fn abandoned(&self, error: Error, recovery: Option<Recovery>) {
            self.abandoned.borrow_mut().push(error);
            if let Some(recovery) = recovery {
                self.recovery.borrow_mut().push(recovery);
            }
        }
    }
    fn owner(original: bool) -> (Rc<Fake>, Rc<Control<Rc<Fake>>>) {
        let io = Rc::new(Fake::default());
        io.value.set(original);
        (io.clone(), Rc::new(Control::new(io)))
    }
    #[test]
    fn preserve_original_across_refresh_clear_and_repeated_retirement() {
        for original in [false, true] {
            let (io, control) = owner(original);
            control.activate().unwrap();
            control.activate().unwrap();
            assert_eq!(control.health(), Health::Active);
            assert!(!io.value.get());
            control.clear().unwrap();
            control.retire().unwrap();
            control.retire().unwrap();
            assert_eq!(io.value.get(), original);
            assert!(!io.live.get());
            assert_eq!(
                io.trace(),
                [
                    "open",
                    "get",
                    "set:false",
                    "set:false",
                    if original { "set:true" } else { "set:false" },
                    "close"
                ]
            );
            assert_eq!(control.health(), Health::Inactive);
        }
    }
    #[test]
    fn committed_hyper_never_restores_a_stale_caps_on_snapshot() {
        let (io, control) = owner(true);
        control.activate().unwrap();
        control.commit_off().unwrap();
        control.activate().unwrap();
        io.value.set(true);
        control.reconcile().unwrap();
        control.retire().unwrap();
        assert!(!io.value.get());
        assert!(!io.live.get());
        assert!(!io.trace().iter().any(|call| call == "set:true"));
        assert_eq!(control.commit_off(), Err(Error::Inactive));
    }
    #[test]
    fn committed_read_failure_restores_off_and_closes() {
        let (io, control) = owner(true);
        control.activate().unwrap();
        control.commit_off().unwrap();
        io.fail(&[4]);
        assert_eq!(control.reconcile(), Err(Error::Read(-4)));
        assert!(!io.value.get());
        assert!(!io.live.get());
    }
    #[test]
    fn commit_during_reentrant_retirement_is_rejected() {
        let (io, control) = owner(true);
        let weak = Rc::downgrade(&control);
        *io.hook.borrow_mut() = Some(Rc::new(move |operation| {
            if operation == "get" {
                let control = weak.upgrade().unwrap();
                assert_eq!(control.retire(), Err(Error::Busy));
                assert_eq!(control.commit_off(), Err(Error::Busy));
            }
        }));
        assert_eq!(control.activate(), Err(Error::Cancelled));
        assert_eq!(control.commit_off(), Err(Error::Inactive));
        assert!(io.value.get());
        assert!(!io.live.get());
    }
    #[test]
    fn reconcile_repairs_a_late_system_toggle_without_repeated_writes() {
        let (io, control) = owner(false);
        control.activate().unwrap();
        control.clear().unwrap();
        // Model an OS latch update arriving after both input callbacks.
        io.value.set(true);
        control.reconcile().unwrap();
        assert!(!io.value.get());
        assert_eq!(control.health(), Health::Active);
        assert_eq!(
            io.trace(),
            ["open", "get", "set:false", "set:false", "get", "set:false"]
        );
        control.reconcile().unwrap();
        assert_eq!(io.trace().last().unwrap(), "get");
        assert_eq!(io.trace().len(), 7);
        control.retire().unwrap();
        assert!(!io.value.get());
    }
    #[test]
    fn reconcile_read_failure_retires_and_restores_the_original() {
        let (io, control) = owner(true);
        control.activate().unwrap();
        io.fail(&[4]);
        assert_eq!(control.reconcile(), Err(Error::Read(-4)));
        assert!(io.value.get());
        assert!(!io.live.get());
        assert_eq!(control.health(), Health::Failed(Error::Read(-4)));
    }
    #[test]
    fn retirement_during_reconcile_never_reports_active() {
        let (io, control) = owner(true);
        control.activate().unwrap();
        let weak = Rc::downgrade(&control);
        *io.hook.borrow_mut() = Some(Rc::new(move |operation| {
            if operation == "get" {
                assert_eq!(weak.upgrade().unwrap().retire(), Err(Error::Busy));
            }
        }));
        assert_eq!(control.reconcile(), Err(Error::Cancelled));
        assert_eq!(control.health(), Health::Inactive);
        assert!(io.value.get());
        assert!(!io.live.get());
    }
    #[test]
    fn unknown_original_is_never_guessed_or_mutated() {
        for (failed, expected, trace) in [
            (1, Error::Open(-1), vec!["open"]),
            (2, Error::Read(-2), vec!["open", "get", "close"]),
        ] {
            let (io, control) = owner(true);
            io.fail(&[failed]);
            assert_eq!(control.activate(), Err(expected));
            assert_eq!(io.trace(), trace);
            assert!(io.value.get());
            assert!(!io.live.get());
            assert_eq!(control.health(), Health::Failed(expected));
        }
    }
    #[test]
    fn failed_clear_can_mutate_and_must_restore_before_close() {
        let (io, control) = owner(true);
        io.fail(&[3]);
        assert_eq!(control.activate(), Err(Error::Clear(-3)));
        assert_eq!(
            io.trace(),
            ["open", "get", "set:false", "set:true", "close"]
        );
        assert!(io.value.get());
        assert!(!io.live.get());
    }
    #[test]
    fn failed_restore_retains_connection_and_cannot_reacquire_until_recovered() {
        let (io, control) = owner(true);
        io.fail(&[3, 4, 5]);
        assert_eq!(control.activate(), Err(Error::Restore(-4)));
        assert_eq!(
            control.health(),
            Health::RecoveryPending(Error::Restore(-4))
        );
        assert!(io.live.get());
        assert_eq!(control.activate(), Err(Error::Restore(-5)));
        assert_eq!(
            io.trace(),
            ["open", "get", "set:false", "set:true", "set:true"]
        );
        control.retire().unwrap();
        assert_eq!(control.health(), Health::Inactive);
        assert!(io.value.get());
        assert!(!io.live.get());
        control.activate().unwrap();
        control.retire().unwrap();
        assert!(io.value.get());
    }
    #[test]
    fn transition_clear_failure_retires_and_preserves_original() {
        let (io, control) = owner(true);
        control.activate().unwrap();
        io.fail(&[4, 5]);
        assert_eq!(control.clear(), Err(Error::Restore(-5)));
        assert_eq!(control.clear(), Err(Error::Restore(-5)));
        assert_eq!(
            io.trace().len(),
            5,
            "pending restoration must not clear again"
        );
        control.retire().unwrap();
        assert!(io.value.get());
        assert!(!io.live.get());
    }
    #[test]
    fn failed_close_is_visible_and_consumed_handle_is_never_reused() {
        let (io, control) = owner(true);
        control.activate().unwrap();
        io.fail(&[5]);
        assert_eq!(control.retire(), Err(Error::Close(-5)));
        assert_eq!(control.health(), Health::Failed(Error::Close(-5)));
        control.retire().unwrap();
        assert_eq!(
            io.trace(),
            ["open", "get", "set:false", "set:true", "close"]
        );
        control.activate().unwrap();
        control.retire().unwrap();
    }
    #[test]
    fn nested_stop_at_each_acquisition_boundary_cancels_and_cleans_up() {
        for boundary in ["open", "get", "set:false"] {
            let (io, control) = owner(true);
            let weak = Rc::downgrade(&control);
            *io.hook.borrow_mut() = Some(Rc::new(move |name| {
                if name == boundary {
                    let control = weak.upgrade().unwrap();
                    assert_eq!(control.health(), Health::Busy);
                    assert_eq!(control.activate(), Err(Error::Busy));
                    assert_eq!(control.clear(), Err(Error::Busy));
                    assert_eq!(control.retire(), Err(Error::Busy));
                }
            }));
            assert_eq!(control.activate(), Err(Error::Cancelled));
            assert_eq!(control.health(), Health::Inactive);
            assert!(io.value.get());
            assert!(!io.live.get());
            assert_eq!(io.trace().last().unwrap(), "close");
        }
    }
    #[test]
    fn nested_stop_during_restore_or_close_does_not_double_release() {
        for boundary in ["set:true", "close"] {
            let (io, control) = owner(true);
            control.activate().unwrap();
            let weak = Rc::downgrade(&control);
            *io.hook.borrow_mut() = Some(Rc::new(move |name| {
                if name == boundary {
                    assert_eq!(weak.upgrade().unwrap().retire(), Err(Error::Busy));
                }
            }));
            control.retire().unwrap();
            assert_eq!(control.health(), Health::Inactive);
            assert!(!io.live.get());
            assert_eq!(
                io.trace()
                    .iter()
                    .filter(|name| name.as_str() == "close")
                    .count(),
                1
            );
        }
    }
    #[test]
    fn destruction_reports_unrecoverable_restore_without_false_close() {
        let (io, control) = owner(true);
        control.activate().unwrap();
        io.fail(&[4]);
        drop(control);
        assert_eq!(*io.abandoned.borrow(), [Error::Restore(-4)]);
        assert!(io.live.get());
        let recovery = io.recovery.borrow_mut().pop().unwrap();
        assert_eq!(recovery.original, Some(true));
        assert!(recovery.dirty);
        io.set(recovery.handle, recovery.original.unwrap()).unwrap();
        io.close(recovery.handle).unwrap();
        assert!(!io.live.get());
        assert!(io.value.get());
    }
    #[test]
    fn unwinding_boundary_does_not_lose_known_lease_or_leave_owner_busy() {
        for boundary in ["open", "get", "set:false", "close"] {
            let (io, control) = owner(true);
            *io.hook.borrow_mut() = Some(Rc::new(move |name| {
                if name == boundary {
                    panic!("synthetic boundary unwind");
                }
            }));
            if boundary == "close" {
                control.activate().unwrap();
                assert_eq!(control.retire(), Err(Error::Panic("close")));
            } else {
                let expected = if boundary == "set:false" {
                    "clear"
                } else {
                    boundary
                };
                assert_eq!(control.activate(), Err(Error::Panic(expected)));
            }
            assert!(!io.live.get());
            assert!(io.value.get());
            assert_ne!(control.health(), Health::Busy);
            *io.hook.borrow_mut() = None;
            control.activate().unwrap();
            control.retire().unwrap();
        }
    }
    #[test]
    fn panicking_restore_retains_recovery_and_drop_transfers_it() {
        let (io, control) = owner(true);
        control.activate().unwrap();
        *io.hook.borrow_mut() = Some(Rc::new(|name| {
            if name == "set:true" {
                panic!("synthetic restore unwind");
            }
        }));
        assert_eq!(control.retire(), Err(Error::Panic("restore")));
        assert_eq!(
            control.health(),
            Health::RecoveryPending(Error::Panic("restore"))
        );
        drop(control);
        assert!(io.live.get());
        let recovery = io.recovery.borrow_mut().pop().unwrap();
        *io.hook.borrow_mut() = None;
        io.set(recovery.handle, recovery.original.unwrap()).unwrap();
        io.close(recovery.handle).unwrap();
        assert!(!io.live.get());
        assert!(io.value.get());
    }
}
