//! Retained, exactly-once synthetic workspace replies. No dispatch to a real
//! workspace, notification center or queue. The endpoint pumps on its owner
//! thread; guardian retirement prevents every subsequent callback.
use super::*;
use block2::{Block, RcBlock};
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSError, NSObject, NSString};
use std::{
    collections::BTreeMap,
    thread::{self, ThreadId},
};
pub(super) type OpenReply = Block<dyn Fn(*mut AnyObject, *mut NSError)>;

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
#[derive(Clone, Copy)]
pub(crate) enum SafariEffect {
    Send,
    Consent,
    Activate,
}

// The shared Swift import also serves unrelated callers. A later import
// installation may select this frontdoor only after the open owner can take
// custody of a validated block. The fast path preserves the Swift ABI.
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
mod reference_dispatch {
    use super::*;
    use std::cell::RefCell;
    use std::{ffi::c_void, sync::{Mutex, MutexGuard, atomic::{AtomicBool, AtomicUsize, Ordering}}};

    pub(super) static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    pub(super) static APPLICATION_CALLER: AtomicUsize = AtomicUsize::new(0);
    pub(super) static URL_CALLER: AtomicUsize = AtomicUsize::new(0);
    pub(super) static DEADLINE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    pub(super) static DEADLINE_CALLERS: [AtomicUsize; 5] = [const { AtomicUsize::new(0) }; 5];
    pub(super) static CONTROL_WAIT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    pub(super) static CONTROL_WAIT_CALLER: AtomicUsize = AtomicUsize::new(0);
    const CONTROL_ROUTE: usize = 11;
    static QUEUED: AtomicUsize = AtomicUsize::new(0);
    static CANCELLING: AtomicBool = AtomicBool::new(false);
    static HANDOFF_FAILED: AtomicBool = AtomicBool::new(false);
    pub(super) static WORKER_CALLERS: [AtomicUsize; 23] = [const { AtomicUsize::new(0) }; 23];
    const CONFIGURATION_REPLY_ROUTE: usize = 22;
    const CONFIGURATION_DEBOUNCE_ROUTE: usize = 23;
    const CONFIGURATION_ACK_ROUTE: usize = 24;
    pub(super) static SHORTCUTS_DESCRIPTOR_CALLERS: [AtomicUsize; 3] =
        [const { AtomicUsize::new(0) }; 3];
    struct WorkerQueue {
        pending: usize,
        closed: bool,
        stopping: bool,
        control_waits: BTreeMap<usize, Arc<ControlReply>>,
        control_drain_wake: Option<Arc<super::super::cf_loop::Wake>>,
    }
    static WORKER_QUEUE: Mutex<WorkerQueue> = Mutex::new(WorkerQueue {
        pending: 0, closed: false, stopping: false, control_waits: BTreeMap::new(),
        control_drain_wake: None,
    });
    fn worker_queue() -> MutexGuard<'static, WorkerQueue> {
        WORKER_QUEUE.lock().unwrap_or_else(|_| unsafe { libc::_exit(78) })
    }
    thread_local! {
        static RUNNING_WORKER: RefCell<Option<RunningWorker>> =
            const { RefCell::new(None) };
        static CONTROL_REPLY: RefCell<Option<(Arc<ControlReply>, bool)>> =
            const { RefCell::new(None) };
    }

    struct ControlReply {
        semaphore: usize,
        cancelled: AtomicBool,
    }

    struct RunningWorker {
        route: usize,
        stopped: &'static AtomicBool,
        admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        queue: usize,
        session: usize,
        monitor_reply: Option<MonitorReply>,
        wake: Arc<super::super::cf_loop::Wake>,
        shortcuts_descriptor: Option<Arc<std::os::fd::OwnedFd>>,
    }

    pub(super) fn hold_shortcuts_descriptor(
        caller: usize, descriptor: Arc<std::os::fd::OwnedFd>,
    ) -> Result<()> {
        if caller == 0 || !SHORTCUTS_DESCRIPTOR_CALLERS.iter().any(|entry| entry.load(Ordering::Acquire) == caller) {
            return Err("Reference Task descriptor caller differs".into());
        }
        let workers = worker_queue();
        if workers.closed {
            return Err("Reference Shortcuts descriptor ingress closed".into());
        }
        let mut descriptor = Some(descriptor);
        let previous = RUNNING_WORKER.with(|slot| -> Result<_> {
            let mut slot = slot.try_borrow_mut().map_err(|_| "Reference Shortcuts worker busy")?;
            let worker = slot.as_mut().filter(|worker| {
                matches!(worker.route, 4 | 5) && !worker.stopped.load(Ordering::Acquire)
            })
                .ok_or("Reference Shortcuts descriptor has no active search worker")?;
            Ok(std::mem::replace(&mut worker.shortcuts_descriptor, descriptor.take()))
        });
        drop(workers);
        drop(previous?);
        Ok(())
    }

    pub(super) static SHORTCUTS_PIPE_CALLERS: [AtomicUsize; 2] =
        [const { AtomicUsize::new(0) }; 2];

    pub(super) fn shortcuts_pipe_reader(caller: usize) -> Result<bool> {
        if caller == 0 || !SHORTCUTS_PIPE_CALLERS.iter().any(|entry| entry.load(Ordering::Acquire) == caller) {
            return Ok(false);
        }
        let workers = worker_queue();
        if workers.closed || !RUNNING_WORKER.with(|slot| {
            slot.borrow().as_ref().is_some_and(|worker| matches!(worker.route, 4 | 5))
        }) {
            return Err("Reference Shortcuts pipe has no active search worker".into());
        }
        Ok(true)
    }

    type Continuation = Block<dyn Fn()>;

    struct Handoff {
        model: Arc<Workspace>,
        admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        completion: Completion,
        outer: (usize, usize),
        opens_url: bool,
        error: usize,
        _error_owner: Option<super::super::factory::OwnedReceiver>,
        expected_outer: (usize, [usize; 4]),
        outcome: Option<Result<Captured>>,
        entered: bool,
    }
    struct Captured {
        group: usize,
        block: usize,
        queue: usize,
        caller: usize,
        context: usize,
    }

    struct Scheduled {
        handoff: Handoff,
        captured: Captured,
    }
    impl Scheduled {
        fn verify(&self) -> Result<(&'static AtomicBool, Arc<super::super::factory::AppStateCustody>)> {
            let handoff = &self.handoff;
            let captured = &self.captured;
            let (admission, wire_stopped, owner) =
                crate::sdk::reference_runtime::open_application_cleanup_admission(&handoff.model)?;
            if !Arc::ptr_eq(&admission, &handoff.admission) || owner.pointer_on_creator()? != handoff.expected_outer.1[3] {
                return Err("Reference open continuation issuer changed".into());
            }
            let context = unsafe {
                admission.verify_open_continuation(
                    handoff.outer, handoff.expected_outer, handoff.opens_url, captured.caller,
                    captured.group, captured.queue, captured.block, handoff.error,
                )
            }?;
            if context != captured.context {
                return Err("Reference open continuation context changed".into());
            }
            Ok((wire_stopped, owner))
        }

        fn run(&self) -> Result<()> {
            let engine = super::objc::application::engine()?;
            if engine.phase()? != super::event_loop::Phase::Running {
                return Err("Reference open continuation owner loop is not running".into());
            }
            let (wire_stopped, owner) = self.verify()?;
            if !wire_stopped.load(Ordering::Acquire) && !CANCELLING.load(Ordering::Acquire) {
                let handoff = &self.handoff;
                let captured = &self.captured;
                let committed = handoff.model.complete_if(&handoff.completion, || {
                    if engine.phase_snapshot()? != super::event_loop::Phase::Running {
                        return Err("Reference open continuation owner loop stopped before invocation".into());
                    }
                    let context = unsafe {
                        handoff.admission.verify_open_continuation_capture(
                            handoff.outer, handoff.expected_outer, handoff.opens_url, captured.caller,
                            captured.group, captured.queue, captured.block, handoff.error,
                        )
                    }?;
                    if context != captured.context {
                        return Err("Reference open continuation context changed".into());
                    }
                    crate::sdk::reference_runtime::verify_current_open_application_state(
                        &owner, handoff.expected_outer.1[3], handoff.completion.generation,
                    )?;
                    Ok(!wire_stopped.load(Ordering::Acquire) && !CANCELLING.load(Ordering::Acquire))
                })?;
                if committed {
                    unsafe { (&*(captured.block as *const Continuation)).call(()); }
                }
            }
            self.verify()?;
            unsafe { drop(RcBlock::from_raw(self.captured.block as *mut Continuation)); }
            let handoff = &self.handoff;
            let (admission, _, owner) =
                crate::sdk::reference_runtime::open_application_cleanup_admission(&handoff.model)?;
            if !Arc::ptr_eq(&admission, &handoff.admission) || owner.pointer_on_creator()? != handoff.expected_outer.1[3]
                || unsafe { admission.verify_open_block(handoff.outer.0, handoff.outer.1, handoff.opens_url) }?
                    != handoff.expected_outer
            {
                return Err("Reference outer open capture changed during continuation cleanup".into());
            }
            unsafe { drop(RcBlock::from_raw(handoff.outer.1 as *mut OpenReply)); }
            Ok(())
        }
    }

    #[link(name = "System")]
    unsafe extern "C" {
        fn dispatch_async_f(queue: *mut c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
        fn dispatch_after_f(time: u64, queue: *mut c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
        fn dispatch_time(when: u64, delta: i64) -> u64;
        fn dispatch_semaphore_signal(semaphore: *mut c_void) -> libc::c_long;
    }

    #[unsafe(naked)]
    pub(super) unsafe extern "C" fn deadline_entry() {
        core::arch::naked_asm!(
            "adrp x16, {caller}@PAGE",
            "add x16, x16, {caller}@PAGEOFF",
            "ldar x17, [x16]",
            "cmp x30, x17",
            "b.eq 2f",
            ".rept 4",
            "add x16, x16, #8",
            "ldar x17, [x16]",
            "cmp x30, x17",
            "b.eq 2f",
            ".endr",
            "adrp x16, {original}@PAGE",
            "add x16, x16, {original}@PAGEOFF",
            "ldar x16, [x16]",
            "br x16",
            "2:",
            "mov x1, x3",
            "mov x2, x20",
            "mov x3, x30",
            "mov x4, x29",
            "b {capture}",
            caller = sym DEADLINE_CALLERS,
            original = sym DEADLINE_ORIGINAL,
            capture = sym capture_deadline,
        );
    }

    #[unsafe(naked)]
    pub(super) unsafe extern "C" fn control_wait_entry() {
        core::arch::naked_asm!(
            "adrp x16, {caller}@PAGE",
            "add x16, x16, {caller}@PAGEOFF",
            "ldar x16, [x16]",
            "cmp x30, x16",
            "b.eq 2f",
            "adrp x16, {original}@PAGE",
            "add x16, x16, {original}@PAGEOFF",
            "ldar x16, [x16]",
            "br x16",
            "2:",
            "mov x1, x20",
            "b {wait}",
            caller = sym CONTROL_WAIT_CALLER,
            original = sym CONTROL_WAIT_ORIGINAL,
            wait = sym wait_for_control_reply,
        );
    }

    #[unsafe(naked)]
    unsafe extern "C" fn swift_wait(original: usize, semaphore: usize, deadline: usize) -> u8 {
        core::arch::naked_asm!(
            "stp x20, x30, [sp, #-16]!", "mov x9, x0", "mov x20, x1",
            "mov x0, x2", "blr x9", "ldp x20, x30, [sp], #16", "ret",
        );
    }

    extern "C" fn wait_for_control_reply(deadline: usize, semaphore: usize) -> u8 {
        let reply = CONTROL_REPLY.with(|slot| slot.take());
        let Some((reply, registered)) = reply.filter(|(reply, _)| reply.semaphore == semaphore) else {
            unsafe { libc::_exit(78) }
        };
        if !registered {
            return 1;
        }
        let result = unsafe { swift_wait(CONTROL_WAIT_ORIGINAL.load(Ordering::Acquire), semaphore, deadline) };
        let mut workers = worker_queue();
        if !workers.control_waits.remove(&semaphore).is_some_and(|wait| Arc::ptr_eq(&wait, &reply)) {
            unsafe { libc::_exit(78) }
        }
        if workers.control_waits.is_empty() {
            if let Some(wake) = workers.control_drain_wake.take() {
                wake.signal_worker_drain();
            }
        }
        if reply.cancelled.load(Ordering::Acquire) { 1 } else { result }
    }

    extern "C" fn run_scheduled(context: *mut c_void) {
        if unsafe { libc::pthread_main_np() } != 1 {
            unsafe { libc::_exit(78) }
        }
        let result = std::panic::catch_unwind(|| {
            let scheduled = unsafe { Box::from_raw(context.cast::<Scheduled>()) };
            let model = scheduled.handoff.model.clone();
            scheduled.run()?;
            drop(scheduled);
            model.with(|_| Ok(()))?;
            let previous = QUEUED.fetch_sub(1, Ordering::AcqRel);
            if previous == 1 && CANCELLING.load(Ordering::Acquire) {
                super::objc::application::engine()?.signal()?;
            }
            Ok::<_, String>(())
        });
        if !matches!(result, Ok(Ok(()))) {
            // Unknown outcome or loss of qualification leaves raw retains
            // quarantined. Never let the child report a clean exit.
            unsafe { libc::_exit(78) }
        }
    }

    pub(super) fn safari_dispatch_active() -> bool {
        RUNNING_WORKER.with(|slot| {
            slot.borrow().as_ref().is_some_and(|worker| matches!(worker.route, 4 | 5 | 12 | 13))
        }) && !worker_queue().closed
    }

    pub(super) fn safari_discovery_worker(admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>) -> bool {
        RUNNING_WORKER.with(|slot| {
            slot.try_borrow().ok().is_some_and(|slot| {
                slot.as_ref().is_some_and(|worker| {
                    matches!(worker.route, 4 | 5 | 12 | 13) && Arc::ptr_eq(&worker.admission, admission)
                })
            })
        })
    }

    pub(super) fn search_dispatch_active(
        admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        require_open_ingress: bool,
    ) -> bool {
        let stopped = RUNNING_WORKER.with(|slot| {
            let slot = slot.try_borrow().ok()?;
            let worker = slot.as_ref()?;
            (matches!(worker.route, 4 | 5) && Arc::ptr_eq(&worker.admission, admission))
                .then_some(worker.stopped)
        });
        stopped.is_some_and(|stopped| {
            !require_open_ingress || {
                let queue = worker_queue();
                !queue.closed && !stopped.load(Ordering::Acquire)
            }
        })
    }

    pub(super) fn safari_effect<T>(
        effect: SafariEffect, stopped: &AtomicBool, commit: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let active = RUNNING_WORKER.with(|slot| {
            slot.borrow().as_ref().is_some_and(|worker| {
                match effect {
                    SafariEffect::Send => matches!(worker.route, 4 | 5),
                    SafariEffect::Consent => worker.route == 13,
                    SafariEffect::Activate => worker.route == 12,
                }
            })
        });
        let queue = worker_queue();
        if !active || queue.closed || stopped.load(Ordering::Acquire) {
            return Err("Reference Safari effect outside an active dispatch turn".into());
        }
        commit()
    }

    pub(super) fn waiting_for_shutdown() -> bool {
        (CANCELLING.load(Ordering::Acquire) && QUEUED.load(Ordering::Acquire) != 0)
            || {
                let workers = worker_queue();
                (workers.closed || workers.stopping) && workers.pending != 0
                    || workers.closed && !workers.control_waits.is_empty()
            }
    }

    pub(super) fn enqueue_configuration_drain(queue: usize, wake: Arc<super::super::cf_loop::Wake>) -> Result<()> {
        extern "C" fn acknowledge(pointer: *mut c_void) {
            let wake = unsafe { Box::from_raw(pointer.cast::<Arc<super::super::cf_loop::Wake>>()) };
            let mut workers = worker_queue();
            workers.pending -= 1;
            if (workers.closed || workers.stopping) && workers.pending == 0 {
                wake.signal_worker_drain();
            }
        }
        let context = Box::new(wake);
        {
            let mut workers = worker_queue();
            if workers.closed || workers.pending >= 64 {
                return Err("Reference configuration drain ingress unavailable".into());
            }
            workers.pending += 1;
        }
        unsafe { dispatch_async_f(queue as *mut c_void, Box::into_raw(context).cast(), acknowledge) };
        Ok(())
    }

    pub(super) fn begin_shutdown(terminal_wake: Option<Arc<super::super::cf_loop::Wake>>) -> bool {
        let terminal = terminal_wake.is_some();
        CANCELLING.store(true, Ordering::Release);
        {
            let mut workers = worker_queue();
            if terminal && !workers.closed {
                if !workers.control_waits.is_empty() {
                    workers.control_drain_wake = terminal_wake;
                }
                workers.closed = true;
                for reply in workers.control_waits.values() {
                    // Swift owns the semaphore until its wait returns. The
                    // lock serializes this signal with that wait's removal.
                    if !reply.cancelled.swap(true, Ordering::AcqRel) {
                        unsafe { dispatch_semaphore_signal(reply.semaphore as *mut c_void); }
                    }
                }
            }
            // A nonterminal Stop lets admitted Swift turns finish and clear
            // their session's running state, including any queued next turn.
            workers.stopping = !terminal && workers.pending != 0;
        }
        waiting_for_shutdown()
    }

    pub(super) fn is_cancelling() -> bool {
        CANCELLING.load(Ordering::Acquire)
    }

    pub(super) fn mark_cleanup_unconfirmed() {
        HANDOFF_FAILED.store(true, Ordering::Release);
    }

    pub(super) fn verify_drained() -> Result<()> {
        if HANDOFF_FAILED.load(Ordering::Acquire) {
            return Err("Reference open continuation handoff failed before confirmed release".into());
        }
        if QUEUED.load(Ordering::Acquire) != 0 || HANDOFF.with(|slot| slot.borrow().is_some()) {
            return Err("Reference open continuations remain queued or in flight".into());
        }
        let workers = worker_queue();
        if !workers.control_waits.is_empty() || workers.control_drain_wake.is_some() {
            return Err("Reference ControlServer reply waits remain in flight".into());
        }
        if workers.pending != 0 {
            return Err("Reference worker dispatch remains queued or in flight".into());
        }
        Ok(())
    }

    struct QueueReservation(bool);
    impl Drop for QueueReservation {
        fn drop(&mut self) {
            if self.0 {
                // A reservation can end without dispatching a retained
                // continuation. A zero count then cannot certify drain.
                HANDOFF_FAILED.store(true, Ordering::Release);
                QUEUED.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }

    thread_local! {
        static HANDOFF: RefCell<Option<Handoff>> = const { RefCell::new(None) };
    }

    pub(super) struct Active {
        armed: bool,
        reservation: QueueReservation,
    }
    impl Active {
        pub(super) fn enter(
            reply: &ReferenceOpenReply, address: usize,
            error_owner: Option<super::super::factory::OwnedReceiver>,
        ) -> Result<Self> {
            if CANCELLING.load(Ordering::Acquire) || HANDOFF_FAILED.load(Ordering::Acquire) {
                return Err("Reference open continuation admission closed".into());
            }
            HANDOFF.with(|slot| {
                let mut slot = slot.try_borrow_mut().map_err(|_| "Reference open handoff slot busy")?;
                if slot.is_some() {
                    return Err("Reference open handoff reentered".into());
                }
                // Count the outer Swift call before invoking it. A reentrant
                // terminal event must wait for it as well as queued work.
                QUEUED.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    (count < 256).then_some(count + 1)
                }).map_err(|_| "Reference open continuation queue bound reached")?;
                *slot = Some(Handoff {
                    model: reply.model.clone(),
                    admission: reply.admission.clone(),
                    completion: reply.completion.clone(),
                    outer: (reply.return_pc, address),
                    opens_url: reply.opens_url,
                    error: error_owner.as_ref().map_or(0, |owner| owner.as_ptr() as usize),
                    _error_owner: error_owner,
                    expected_outer: reply.capture,
                    outcome: None,
                    entered: false,
                });
                Ok(Self { armed: true, reservation: QueueReservation(true) })
            })
        }

        pub(super) fn finish(mut self) -> Result<()> {
            let mut handoff = HANDOFF.with(|slot| slot.borrow_mut().take())
                .ok_or("Reference open handoff slot absent")?;
            self.armed = false;
            let captured = handoff.outcome.take().ok_or("Reference open continuation was not captured")??;
            let scheduled = Box::new(Scheduled { handoff, captured });
            scheduled.verify()?;
            let queue = scheduled.captured.queue;
            unsafe { dispatch_async_f(queue as *mut c_void, Box::into_raw(scheduled).cast(), run_scheduled); }
            self.reservation.0 = false;
            Ok(())
        }
    }
    impl Drop for Active {
        fn drop(&mut self) {
            // An unwind cannot prove the Swift captures are safe to release.
            if self.armed {
                let abandoned = HANDOFF.with(|slot| slot.borrow_mut().take());
                drop(abandoned);
            }
        }
    }

    #[unsafe(naked)]
    pub(super) unsafe extern "C" fn entry() {
        core::arch::naked_asm!(
            "adrp x16, {app}@PAGE",
            "add x16, x16, {app}@PAGEOFF",
            "ldar x16, [x16]",
            "cmp x30, x16",
            "b.eq 2f",
            "adrp x17, {monitor}@PAGE",
            "add x17, x17, {monitor}@PAGEOFF",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            ".rept 12",
            "add x17, x17, #8",
            "ldar x16, [x17]",
            "cmp x30, x16",
            "b.eq 3f",
            ".endr",
            "adrp x16, {url}@PAGE",
            "add x16, x16, {url}@PAGEOFF",
            "ldar x16, [x16]",
            "cmp x30, x16",
            "b.eq 2f",
            "adrp x16, {original}@PAGE",
            "add x16, x16, {original}@PAGEOFF",
            "ldar x16, [x16]",
            "br x16",
            "2:",
            "mov x1, x3",
            "mov x2, x20",
            "mov x3, x30",
            "b {capture}",
            "3:",
            "mov x1, x3",
            "mov x2, x20",
            "mov x3, x30",
            "mov x4, x29",
            "b {capture_worker}",
            app = sym APPLICATION_CALLER,
            url = sym URL_CALLER,
            original = sym ORIGINAL,
            capture = sym capture,
            monitor = sym WORKER_CALLERS,
            capture_worker = sym capture_worker,
        );
    }

    struct WorkerDispatch {
        admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
        wake: Option<Arc<super::super::cf_loop::Wake>>,
        route: usize,
        queue: usize,
        block: usize,
        context: usize,
        session: usize,
        reply_revision: Option<usize>,
        monitor_reply: Option<MonitorReply>,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum MonitorReply {
        Cli([usize; 2]),
        WindowGeneration(usize),
    }

    // The pinned AppState CLI worker captures its state before ReplySink.
    // Its main-queue continuation copies the same two words before the snapshot.
    // The window worker copies the generation but creates a separate weak box.
    // The caller must have just verified the live block and its capture.
    unsafe fn monitor_reply_capture(route: usize, context: usize) -> Option<MonitorReply> {
        let (offset, cli) = match route {
            0 => (24, true),
            1 => (16, true),
            2 => (32, false),
            3 => (24, false),
            _ => return None,
        };
        let address = (context + offset) as *const usize;
        Some(if cli {
            MonitorReply::Cli(unsafe { address.cast::<[usize; 2]>().read_volatile() })
        } else {
            MonitorReply::WindowGeneration(unsafe { address.read_volatile() })
        })
    }

    extern "C" fn capture_worker(group: usize, block: usize, queue: usize, caller: usize, frame: usize) {
        let result = std::panic::catch_unwind(|| -> Result<()> {
            let route = WORKER_CALLERS.iter().position(|slot| slot.load(Ordering::Acquire) == caller)
                .ok_or("Reference worker Dispatch caller absent")?;
            let route = match route {
                18 => CONFIGURATION_REPLY_ROUTE,
                19 => CONFIGURATION_ACK_ROUTE,
                20..=22 => route + 5,
                10..=17 => route + 1,
                _ => route,
            };
            if route == CONFIGURATION_ACK_ROUTE {
                if unsafe { libc::pthread_main_np() } != 1 {
                    return Err("Reference configuration acknowledgement outside owner".into());
                }
                unsafe { crate::sdk::core_preentry::ReferenceAdmission::verify_dispatch_queue_frame(frame, queue, 0x98) }?;
            }
            if route == 27 {
                if unsafe { libc::pthread_main_np() } != 1 {
                    return Err("Reference system cache submitted off owner".into());
                }
                unsafe { crate::sdk::core_preentry::ReferenceAdmission::verify_dispatch_queue_frame(frame, queue, 0xa0) }?;
            }
            schedule_worker(route, group, block, queue, None)
        });
        match result {
            Ok(Ok(())) => (),
            Ok(Err(error)) => {
                crate::sdk::private_network::report_reference_website_dispatch_refusal(&error);
                unsafe { libc::_exit(78) };
            }
            Err(payload) => {
                std::mem::forget(payload);
                crate::sdk::private_network::report_reference_website_dispatch_refusal("worker dispatch panicked");
                unsafe { libc::_exit(78) };
            }
        }
    }

    extern "C" fn capture_deadline(deadline: usize, block: usize, queue: usize, caller: usize, frame: usize) {
        let result = std::panic::catch_unwind(|| -> Result<()> {
            let index = DEADLINE_CALLERS.iter().position(|slot| slot.load(Ordering::Acquire) == caller)
                .ok_or("Reference deadline caller absent")?;
            let route = [10, 19, 20, 21, CONFIGURATION_DEBOUNCE_ROUTE][index];
            if (unsafe { libc::pthread_main_np() } == 1) == (route == CONFIGURATION_DEBOUNCE_ROUTE) {
                return Err("Reference deadline submitted on wrong thread".into());
            }
            if route == CONFIGURATION_DEBOUNCE_ROUTE {
                unsafe { crate::sdk::core_preentry::ReferenceAdmission::verify_dispatch_queue_frame(frame, queue, 0xa8) }?;
            }
            let time = unsafe { crate::sdk::core_preentry::ReferenceAdmission::read_main_tabs_deadline(deadline) }?;
            let bound = if index == 0 { 1_500_000_000 } else { 250_000_000 };
            if time > unsafe { dispatch_time(0, bound) } {
                return Err("Reference deadline exceeds its source bound".into());
            }
            schedule_worker(route, 0, block, queue, Some(time))
        });
        if !matches!(result, Ok(Ok(()))) {
            unsafe { libc::_exit(78) }
        }
    }

    fn schedule_worker(route: usize, group: usize, block: usize, queue: usize, deadline: Option<u64>) -> Result<()> {
        if matches!(route, 15 | 16) && deadline.is_none()
            && crate::sdk::private_network::submit_reference_website_reply(route, group, block, queue)?
        {
            return Ok(());
        }
        if route == CONFIGURATION_REPLY_ROUTE && unsafe { libc::pthread_main_np() } == 1 {
            return Err("Reference configuration reply submitted on the owner".into());
        }
        let (admission, stopped) = crate::sdk::reference_runtime::worker_dispatch_admission()?;
        if matches!(route, 15 | 16) && unsafe { libc::pthread_main_np() } == 1 {
            return Err("Reference website result submitted on the owner".into());
        }
        let control_reply = if route == CONTROL_ROUTE {
            if unsafe { libc::pthread_main_np() } == 1 {
                return Err("Reference ControlServer receive ran on the owner".into());
            }
            let context = unsafe { admission.verify_worker_dispatch_block(route, group, queue, block) }?;
            let semaphore = unsafe { admission.control_reply_semaphore(context) }?;
            Some(Arc::new(ControlReply { semaphore, cancelled: AtomicBool::new(false) }))
        } else {
            None
        };
        let accepted = {
            let mut workers = worker_queue();
            if (workers.closed || stopped.load(Ordering::Acquire)) && !matches!(route, 12 | 13) {
                false
            } else {
                if workers.pending >= 64 {
                    return Err("Reference worker Dispatch bound reached".into());
                }
                if let Some(reply) = &control_reply {
                    // The original receive owns its semaphore from before
                    // submission through the return of its paired wait.
                    if workers.control_waits.len() >= 64 || workers.control_waits.contains_key(&reply.semaphore) {
                        return Err("Reference ControlServer reply wait registration differs".into());
                    }
                    workers.control_waits.insert(reply.semaphore, reply.clone());
                }
                workers.pending += 1;
                true
            }
        };
        if let Some(reply) = control_reply {
            if CONTROL_REPLY.with(|slot| slot.replace(Some((reply, accepted)))).is_some() {
                return Err("Reference ControlServer reply wait reentered".into());
            }
        }
        if !accepted {
            // An initial submit has already stored work in the Swift
            // session before reaching Dispatch. A requeue is still covered
            // by its counted worker, which cancels before releasing.
            if route == 4 {
                if unsafe { libc::pthread_main_np() } != 1 {
                    return Err("Reference stopped search submitted off owner".into());
                }
                let context = unsafe { admission.verify_worker_dispatch_block(route, group, queue, block) }?;
                let session = unsafe { (context as *const usize).add(2).read_volatile() };
                unsafe { admission.cancel_search_worker_session(session) }?;
            }
            return Ok(());
        }
        let context = unsafe { admission.verify_worker_dispatch_block(route, group, queue, block) }?;
        let captured = unsafe { (context as *const usize).add(2).read_volatile() };
        // Search and process-details replies hold the original Swift Ticket.
        // Swift remains responsible for deciding if that revision is current.
        let reply_revision = matches!(route, 6 | 7)
            .then(|| unsafe { (context as *const usize).add(3).read_volatile() });
        let monitor_reply = unsafe { monitor_reply_capture(route, context) };
        let session = match route {
            4..=7 | 13 | 14 | CONFIGURATION_REPLY_ROUTE | CONFIGURATION_ACK_ROUTE => captured,
            12 => unsafe { crate::sdk::core_preentry::ReferenceAdmission::safari_activation_session(context) }?,
            _ => 0,
        };
        let wake = if route == CONFIGURATION_DEBOUNCE_ROUTE {
            Some(crate::sdk::reference_runtime::configuration_worker_wake(&admission)?)
        } else if matches!(route, CONTROL_ROUTE | 15 | 16 | CONFIGURATION_REPLY_ROUTE) {
            None
        } else if matches!(route, 1 | 3 | 5..=9 | 12 | 14) {
            Some(RUNNING_WORKER.with(|slot| {
                let slot = slot.borrow();
                let worker = slot.as_ref().filter(|worker| Arc::ptr_eq(&worker.admission, &admission))?;
                let matches = match route {
                    1 | 3 => worker.route == route - 1 && worker.monitor_reply == monitor_reply,
                    5 => matches!(worker.route, 4 | 5) && worker.queue == queue && worker.session == session,
                    6 | 7 => matches!(worker.route, 4 | 5) && worker.session == session,
                    // Tab replies capture the callback and result;
                    // the executing turn supplies the worker identity.
                    8 | 9 => matches!(worker.route, 4 | 5),
                    12 => matches!(worker.route, 4 | 5) && worker.session == session,
                    14 => worker.route == 13 && worker.session == session,
                    _ => worker.route == route - 1,
                };
                matches.then(|| worker.wake.clone())
            }).ok_or("Reference Dispatch continuation has no matching running worker")?)
        } else {
            if unsafe { libc::pthread_main_np() } != 1 {
                return Err("Reference worker submitted off the owner thread".into());
            }
            Some(super::objc::application::engine()?.wake_handle()?)
        };
        if matches!(route, 0 | 2) {
            crate::sdk::reference_runtime::verify_monitor_worker_queue(route == 2, queue, captured)?;
        }
        if matches!(route, 25 | 26) {
            crate::sdk::reference_runtime::verify_safari_cache_owner(context)?;
        }
        if matches!(route, 17 | 18) {
            crate::sdk::reference_runtime::verify_hyper_action_owner(&admission, route, context)?;
        }
        let retained = unsafe { RcBlock::copy(block as *mut Continuation) }
            .ok_or("Reference worker block retain failed")?;
        if RcBlock::into_raw(retained) as usize != block
            || unsafe { admission.verify_worker_dispatch_block(route, group, queue, block) }? != context
            || reply_revision.is_some_and(|revision| unsafe {
                (context as *const usize).add(3).read_volatile() != revision
            })
            || unsafe { monitor_reply_capture(route, context) } != monitor_reply
        {
            return Err("Reference worker block identity changed during retain".into());
        }
        let work = Box::new(WorkerDispatch {
            admission, wake, route, queue, block, context, session, reply_revision, monitor_reply,
        });
        let context = Box::into_raw(work).cast();
        unsafe {
            if let Some(time) = deadline {
                dispatch_after_f(time, queue as *mut c_void, context, run_worker);
            } else {
                dispatch_async_f(queue as *mut c_void, context, run_worker);
            }
        }
        Ok(())
    }

    extern "C" fn run_worker(pointer: *mut c_void) {
        let result = std::panic::catch_unwind(|| -> Result<()> {
            let work = unsafe { Box::from_raw(pointer.cast::<WorkerDispatch>()) };
            let main_reply = matches!(work.route, 1 | 3 | 6..=12 | 14..=CONFIGURATION_REPLY_ROUTE);
            if (unsafe { libc::pthread_main_np() } == 1) != main_reply {
                return Err("Reference worker callback executed on the wrong thread".into());
            }
            let wake = match &work.wake {
                Some(wake) => wake.clone(),
                None => super::objc::application::engine()?.wake_handle()?,
            };
            let verify = || -> Result<&'static AtomicBool> {
                let (admission, stopped) = crate::sdk::reference_runtime::worker_dispatch_admission()?;
                if !Arc::ptr_eq(&admission, &work.admission)
                    || unsafe { admission.verify_worker_dispatch_block(work.route, 0, work.queue, work.block) }?
                        != work.context
                    || (matches!(work.route, 4..=7 | 13 | 14 | CONFIGURATION_REPLY_ROUTE | CONFIGURATION_ACK_ROUTE)
                        && unsafe { (work.context as *const usize).add(2).read_volatile() } != work.session)
                    || work.reply_revision.is_some_and(|revision| unsafe {
                        (work.context as *const usize).add(3).read_volatile() != revision
                    })
                    || unsafe { monitor_reply_capture(work.route, work.context) } != work.monitor_reply
                    || (work.route == 12 && unsafe {
                        crate::sdk::core_preentry::ReferenceAdmission::safari_activation_session(work.context)
                    }? != work.session)
                {
                    return Err("Reference worker callback lost block custody".into());
                }
                Ok(stopped)
            };
            let stopped = verify()?;
            let closed = worker_queue().closed;
            if closed || stopped.load(Ordering::Acquire) {
                // Invalidate the original guard before allowing the exact
                // retained callback to acknowledge cancellation in Swift.
                match work.route {
                    12 => unsafe {
                        work.admission.cancel_search_worker_session(work.session)?;
                        verify()?;
                        (&*(work.block as *const Continuation)).call(());
                    },
                    13 => unsafe {
                        crate::sdk::core_preentry::ReferenceAdmission::cancel_safari_consent(work.session)?;
                        verify()?;
                        (&*(work.block as *const Continuation)).call(());
                    },
                    _ => (),
                }
            } else {
                if !main_reply || work.route == 12 {
                    let running = RunningWorker { route: work.route, stopped, queue: work.queue,
                        admission: work.admission.clone(),
                        session: work.session, monitor_reply: work.monitor_reply,
                        wake: wake.clone(), shortcuts_descriptor: None };
                    if RUNNING_WORKER.with(|slot| slot.replace(Some(running))).is_some() {
                        return Err("Reference worker reentered".into());
                    }
                    unsafe { (&*(work.block as *const Continuation)).call(()); }
                    RUNNING_WORKER.with(|slot| slot.replace(None));
                } else if matches!(work.route, 17 | 18) {
                    crate::sdk::reference_runtime::verify_hyper_action_owner(&work.admission, work.route, work.context)?;
                    if !worker_queue().closed && !stopped.load(Ordering::Acquire) {
                        unsafe { (&*(work.block as *const Continuation)).call(()); }
                    }
                } else {
                    unsafe { (&*(work.block as *const Continuation)).call(()); }
                }
            }
            let stopped = verify()?;
            let closed = worker_queue().closed;
            if matches!(work.route, 4 | 5) && (closed || stopped.load(Ordering::Acquire)) {
                unsafe { work.admission.cancel_search_worker_session(work.session) }?;
                verify()?;
            }
            unsafe { drop(RcBlock::from_raw(work.block as *mut Continuation)); }
            drop(work);
            let mut workers = worker_queue();
            workers.pending -= 1;
            if (workers.closed || workers.stopping) && workers.pending == 0 {
                // Serialize the final wake with the owner checking the count.
                wake.signal_worker_drain();
            }
            Ok(())
        });
        if !matches!(result, Ok(Ok(()))) {
            unsafe { libc::_exit(78) }
        }
    }

    extern "C" fn capture(group: usize, block: usize, queue: usize, caller: usize) {
        let result = std::panic::catch_unwind(|| {
            let (admission, outer, opens_url, error, expected_outer) = HANDOFF.with(|slot| {
                let mut slot = slot.borrow_mut();
                let handoff = slot.as_mut().ok_or("Reference open capture has no outer owner")?;
                if handoff.entered {
                    return Err("Reference open capture reentered");
                }
                handoff.entered = true;
                Ok((handoff.admission.clone(), handoff.outer, handoff.opens_url, handoff.error, handoff.expected_outer))
            })?;
            let outcome = (|| -> Result<Captured> {
                let verify = |address| unsafe {
                    admission.verify_open_continuation(outer, expected_outer, opens_url, caller, group, queue, address, error)
                };
                let context = verify(block)?;
                let copied = unsafe { RcBlock::copy(block as *mut Continuation) }
                    .ok_or("Reference open continuation retain failed")?;
                let copied = RcBlock::into_raw(copied) as usize;
                // A malloc block's copy must retain the same allocation. On
                // divergence quarantine the +1 rather than running ARC.
                if copied != block {
                    return Err("Reference open continuation retain changed identity".into());
                }
                if verify(copied)? != context {
                    return Err("Reference open continuation capture changed during retain".into());
                }
                Ok(Captured { group, block: copied, queue, caller, context })
            })();
            HANDOFF.with(|slot| {
                slot.borrow_mut().as_mut().ok_or("Reference open capture owner disappeared")?
                    .outcome = Some(outcome);
                Ok::<_, &str>(())
            })
        });
        if !matches!(result, Ok(Ok(()))) {
            unsafe { libc::_exit(78) }
        }
    }
}

/// Exact replacement address for pre-install source qualification.
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub fn reference_dispatch_frontdoor_address() -> usize {
    reference_dispatch::entry as *const () as usize
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_main_tabs_deadline_address() -> usize {
    reference_dispatch::deadline_entry as *const () as usize
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn prepare_reference_main_tabs_deadline(original: usize, callers: [usize; 5]) -> Result<()> {
    bind_dispatch_address(&reference_dispatch::DEADLINE_ORIGINAL, original)?;
    for (slot, address) in reference_dispatch::DEADLINE_CALLERS.iter().zip(callers) {
        bind_dispatch_address(slot, address)?;
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_control_wait_address() -> usize {
    reference_dispatch::control_wait_entry as *const () as usize
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn prepare_reference_control_wait(original: usize, caller: usize) -> Result<()> {
    bind_dispatch_address(&reference_dispatch::CONTROL_WAIT_ORIGINAL, original)?;
    bind_dispatch_address(&reference_dispatch::CONTROL_WAIT_CALLER, caller)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_safari_dispatch_active() -> bool {
    reference_dispatch::safari_dispatch_active()
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_safari_discovery_worker(admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>) -> bool {
    reference_dispatch::safari_discovery_worker(admission)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_search_dispatch_active(admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>) -> bool {
    reference_dispatch::search_dispatch_active(admission, false)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_live_search_dispatch_active(admission: &Arc<crate::sdk::core_preentry::ReferenceAdmission>) -> bool {
    reference_dispatch::search_dispatch_active(admission, true)
}

/// Pure tail only. The caller must qualify all potentially reentrant work first.
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_safari_effect<T>(
    effect: SafariEffect, stopped: &std::sync::atomic::AtomicBool, commit: impl FnOnce() -> Result<T>,
) -> Result<T> {
    reference_dispatch::safari_effect(effect, stopped, commit)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn mark_reference_open_cleanup_unconfirmed() {
    reference_dispatch::mark_cleanup_unconfirmed();
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_open_drained() -> Result<()> {
    reference_dispatch::verify_drained()
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn begin_reference_open_shutdown(terminal_wake: Option<Arc<super::cf_loop::Wake>>) -> bool {
    reference_dispatch::begin_shutdown(terminal_wake)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(super) fn reference_open_shutdown_waiting() -> bool {
    reference_dispatch::waiting_for_shutdown()
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn enqueue_reference_configuration_drain(queue: usize, wake: Arc<super::cf_loop::Wake>) -> Result<()> {
    reference_dispatch::enqueue_configuration_drain(queue, wake)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn prepare_reference_dispatch_frontdoor(
    original: usize, application_caller: usize, url_caller: usize, worker_callers: [usize; 23],
) -> Result<()> {
    for (slot, address) in [
        (&reference_dispatch::ORIGINAL, original),
        (&reference_dispatch::APPLICATION_CALLER, application_caller),
        (&reference_dispatch::URL_CALLER, url_caller),
    ].into_iter().chain(reference_dispatch::WORKER_CALLERS.iter().zip(worker_callers)) {
        bind_dispatch_address(slot, address)?;
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
fn bind_dispatch_address(slot: &std::sync::atomic::AtomicUsize, address: usize) -> Result<()> {
    use std::sync::atomic::Ordering;
    if address == 0 {
        return Err("Reference Dispatch frontdoor address absent".into());
    }
    match slot.compare_exchange(0, address, Ordering::Release, Ordering::Relaxed) {
        Ok(_) => Ok(()),
        Err(previous) if previous == address => Ok(()),
        Err(_) => Err("Reference Dispatch frontdoor binding changed".into()),
    }
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn prepare_reference_shortcuts_descriptors(callers: [usize; 3], pipes: [usize; 2]) -> Result<()> {
    for (slot, caller) in reference_dispatch::SHORTCUTS_DESCRIPTOR_CALLERS.iter().zip(callers) {
        bind_dispatch_address(slot, caller)?;
    }
    for (slot, caller) in reference_dispatch::SHORTCUTS_PIPE_CALLERS.iter().zip(pipes) {
        bind_dispatch_address(slot, caller)?;
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn reference_shortcuts_pipe_reader(caller: usize) -> Result<bool> {
    reference_dispatch::shortcuts_pipe_reader(caller)
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn hold_reference_shortcuts_descriptor(
    caller: usize, descriptor: Arc<std::os::fd::OwnedFd>,
) -> Result<()> {
    reference_dispatch::hold_shortcuts_descriptor(caller, descriptor)
}

/// One actual compiler block held for the Reference owner loop. A block which
/// can no longer be requalified stays quarantined after retirement.
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub struct ReferenceOpenReply {
    pub(super) model: Arc<Workspace>,
    pub(super) admission: Arc<crate::sdk::core_preentry::ReferenceAdmission>,
    pub(super) return_pc: usize,
    pub(super) opens_url: bool,
    pub(super) capture: (usize, [usize; 4]),
    pub(super) block: std::sync::atomic::AtomicUsize,
    pub(super) app: Handle,
    pub(super) completion: Completion,
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
impl ReferenceOpenReply {
    fn verify_block(&self, block: usize) -> Result<&'static std::sync::atomic::AtomicBool> {
        let (admission, stopped, owner) = crate::sdk::reference_runtime::open_application_cleanup_admission(&self.model)?;
        if !Arc::ptr_eq(&admission, &self.admission) || owner.pointer_on_creator()? != self.capture.1[3]
            || unsafe { admission.verify_open_block(self.return_pc, block, self.opens_url) }? != self.capture
        {
            return Err("Reference open block owner or capture changed".into());
        }
        crate::sdk::reference_runtime::verify_current_open_application_state(
            &owner, self.capture.1[3], self.completion.generation,
        )?;
        Ok(stopped)
    }

    fn release_without_delivery(&self, address: usize) -> Result<()> {
        use std::sync::atomic::Ordering;
        self.block.compare_exchange(address, 0, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Reference open block already consumed")?;
        drop(unsafe { RcBlock::from_raw(address as *mut OpenReply) });
        Ok(())
    }

    pub(super) fn deliver(&self) -> Result<()> {
        use std::sync::atomic::Ordering;
        let address = self.block.load(Ordering::Acquire);
        let stopped = self.verify_block(address)?;
        if self.completion.outcome == Outcome::Unknown
            || stopped.load(Ordering::Acquire) || reference_dispatch::is_cancelling()
        {
            return self.release_without_delivery(address);
        }
        let (app, error_owner) = super::objc::reference_open_result(&self.model, self.app, self.completion.outcome)?;
        let error = error_owner.as_ref().map_or(std::ptr::null_mut(), |object| object.as_ptr().cast());
        let stopped = self.verify_block(address)?;
        if stopped.load(Ordering::Acquire) || reference_dispatch::is_cancelling() {
            return self.release_without_delivery(address);
        }
        let handoff = reference_dispatch::Active::enter(self, address, error_owner)?;
        self.block.compare_exchange(address, 0, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Reference open block already consumed")?;
        // Keep the raw +1 across invocation. A callback can retire its issuer.
        let block = unsafe { &*(address as *const OpenReply) };
        block.call((
            app.as_ref().map_or(std::ptr::null_mut(), |object| object.as_ptr()),
            error,
        ));
        // If the callback retired or changed its issuer, quarantine the raw
        // retain and report the failed cleanup. Delivery cannot be retried.
        self.verify_block(address)?;
        if let Err(error) = handoff.finish() {
            self.verify_block(address)?;
            drop(unsafe { RcBlock::from_raw(address as *mut OpenReply) });
            return Err(error);
        }
        // The queued continuation now owns both raw retains.
        Ok(())
    }
}
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
impl Drop for ReferenceOpenReply {
    fn drop(&mut self) {
        let address = *self.block.get_mut();
        if address != 0 {
            if self.verify_block(address).is_ok() {
                drop(unsafe { RcBlock::from_raw(address as *mut OpenReply) });
            } else {
                mark_reference_open_cleanup_unconfirmed();
            }
        }
    }
}
struct Held {
    completion: Completion,
    block: usize,
}
pub struct Replies {
    model: Arc<Workspace>,
    owner: ThreadId,
    held: Mutex<BTreeMap<u64, Held>>,
    success_application: usize,
}
impl Replies {
    fn check_owner(&self) -> Result<()> {
        if thread::current().id() != self.owner {
            return Err("completion operation on foreign owner thread".into());
        }
        Ok(())
    }
    pub fn new(model: Arc<Workspace>) -> Result<Self> {
        model.lease.before_effect()?;
        Ok(Self {
            model,
            owner: thread::current().id(),
            held: Mutex::new(BTreeMap::new()),
            // Own data-only non-null completion object; the complete native
            // type/downcast contract is separately hard-open in the roster.
            success_application: Retained::into_raw(NSObject::new()) as usize,
        })
    }
    /// # Safety
    /// block is a valid live Objective-C block with the NSWorkspace completion
    /// ABI. A retained copy is kept until delivered; retirement deliberately
    /// retains remaining copies for process lifetime rather than risking a late
    /// callback's captured state. Unknown outcomes are never replayed.
    pub unsafe fn enqueue(&self, url: &str, block: *mut OpenReply) -> Result<u64> {
        self.check_owner()?;
        self.model.lease.before_effect()?;
        if block.is_null() {
            return Err("missing workspace completion block".into());
        }
        let mut copied = Some(unsafe { RcBlock::copy(block) }.ok_or("failed to retain completion block")?);
        let completion = self.model.begin_open_with(url, |completion| {
            let mut held = self.held.lock().map_err(|_| "reply queue poisoned")?;
            if held.len() >= 4096 || held.contains_key(&completion.id) {
                return Err("reply queue full or completion ID reused".into());
            }
            let block = copied.take().ok_or("retained completion block absent")?;
            held.insert(completion.id, Held {
                completion: completion.clone(),
                block: RcBlock::into_raw(block) as usize,
            });
            Ok(())
        })?;
        Ok(completion.id)
    }
    pub fn drain(&self) -> Result<usize> {
        self.check_owner()?;
        self.model.lease.before_effect()?;
        let ids: Vec<_> = self
            .held
            .lock()
            .map_err(|_| "reply queue poisoned")?
            .keys()
            .copied()
            .collect();
        let mut done = 0;
        for id in ids {
            self.model.lease.before_effect()?;
            let h = {
                let mut held = self.held.lock().map_err(|_| "reply queue poisoned")?;
                if held
                    .get(&id)
                    .is_some_and(|h| h.completion.outcome == Outcome::Unknown)
                {
                    continue;
                }
                held.remove(&id).ok_or("reply disappeared")?
            };
            // Rejection keeps the retained raw block alive. It is never invoked
            // after retirement and never converted into a retry.
            self.model.complete(&h.completion)?;
            self.model.lease.before_effect()?;
            let error = if h.completion.outcome == Outcome::Failure {
                Some(unsafe {
                    NSError::errorWithDomain_code_userInfo(
                        &NSString::from_str("RuycastOwnedWorkspace"),
                        1,
                        None,
                    )
                })
            } else {
                None
            };
            let block = unsafe { RcBlock::from_raw(h.block as *mut OpenReply) }
                .ok_or("lost retained completion block")?;
            let error = error
                .as_ref()
                .map_or(std::ptr::null_mut(), |e| Retained::as_ptr(e).cast_mut());
            let app = if h.completion.outcome == Outcome::Success {
                self.success_application as *mut AnyObject
            } else {
                std::ptr::null_mut()
            };
            block.call((app, error));
            done += 1;
        }
        Ok(done)
    }
    pub fn pending(&self) -> Result<usize> {
        self.model.lease.before_effect()?;
        Ok(self.held.lock().map_err(|_| "reply queue poisoned")?.len())
    }
}
