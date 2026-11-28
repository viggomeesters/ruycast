//! Plain menu state, refreshed with configuration and analytics health.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct State {
    pub visible: bool,
    pub healthy: bool,
    pub requires_input: bool,
}

/// What the status item shows for a visible menu. A pending lookup keeps the
/// neutral template icon instead of reporting a false error; a previously
/// confirmed conflict remains a warning until a fresh lookup clears it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Presentation {
    Checking,
    Running,
    NeedsAttention,
}

impl Presentation {
    pub fn of(healthy: bool, checking: bool) -> Self {
        if checking {
            Self::Checking
        } else if healthy {
            Self::Running
        } else {
            Self::NeedsAttention
        }
    }
    pub fn tool_tip(self) -> &'static str {
        match self {
            Self::Checking => "Ruycast · Checking…",
            Self::Running => "Ruycast · Running",
            Self::NeedsAttention => "Ruycast · Needs attention",
        }
    }
    pub fn accessibility_label(self) -> &'static str {
        match self {
            Self::Checking => "Ruycast, checking",
            Self::Running => "Ruycast, running",
            Self::NeedsAttention => "Ruycast, needs attention",
        }
    }
    /// The template (non-warning) icon is used unless attention is needed.
    pub fn neutral_icon(self) -> bool {
        !matches!(self, Self::NeedsAttention)
    }
}

/// How long a submitted lookup may stay unanswered before the status item
/// stops showing "Checking…" and reports that attention is needed.
pub const LOOKUP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);

/// Main-thread bookkeeping for a bounded, single-flight presence probe.
/// Invalidation immediately removes the old answer; stale replies are never
/// adopted and at most one replacement probe is queued. A lookup that misses
/// its deadline no longer counts as waiting, so it is shown as unhealthy; a
/// late reply for the current generation can still be adopted.
#[derive(Default)]
pub struct PresenceRefresh {
    generation: u64,
    active: Option<u64>,
    pending: bool,
    known: Option<bool>,
    confirmed_conflict: bool,
    closed: bool,
    started: Option<std::time::Instant>,
    expired: bool,
}

impl PresenceRefresh {
    fn begin(&mut self, generation: u64) {
        self.active = Some(generation);
        self.started = Some(std::time::Instant::now());
        self.expired = false;
    }
    /// Marks an unanswered lookup as overdue once `limit` has passed since it
    /// was submitted. Returns true only when this call newly expired it.
    pub fn expire_overdue(&mut self, now: std::time::Instant, limit: std::time::Duration) -> bool {
        let overdue = !self.expired
            && self.active.is_some()
            && self.started.is_some_and(|started| now.saturating_duration_since(started) >= limit);
        if overdue {
            self.expired = true;
        }
        overdue
    }
    pub fn invalidate(&mut self) -> Option<u64> {
        self.known = None;
        let Some(next) = self.generation.checked_add(1) else {
            self.closed = true;
            self.pending = false;
            return None;
        };
        if self.closed {
            return None;
        }
        self.generation = next;
        if self.active.is_some() {
            self.pending = true;
            None
        } else {
            self.begin(next);
            Some(next)
        }
    }

    pub fn complete(&mut self, generation: u64, value: Option<bool>) -> Option<u64> {
        if self.active != Some(generation) {
            return None;
        }
        self.active = None;
        self.started = None;
        self.expired = false;
        if self.closed {
            return None;
        }
        if generation == self.generation {
            self.known = value;
            if let Some(absent) = value { self.confirmed_conflict = !absent; }
        }
        if self.pending {
            self.pending = false;
            let next = self.generation;
            self.begin(next);
            Some(next)
        } else {
            None
        }
    }

    pub fn known(&self) -> Option<bool> {
        self.known
    }
    /// Preserve a confirmed warning until a current lookup proves it resolved.
    pub fn confirmed_conflict(&self) -> bool {
        self.confirmed_conflict
    }
    pub fn waiting(&self) -> bool {
        !self.closed && !self.expired && (self.active.is_some() || self.pending)
    }
    /// Retire the current answer and any queued replacement without waiting for
    /// an in-flight lookup. A later invalidation can schedule a fresh result.
    pub fn suspend(&mut self) {
        self.known = None;
        self.pending = false;
        match self.generation.checked_add(1) {
            Some(next) => self.generation = next,
            None => self.close(),
        }
    }
    /// A failed submission cannot produce a reply. Retire this owner rather
    /// than leave an active generation that can never finish.
    pub fn submission_failed(&mut self, generation: u64) {
        if self.active == Some(generation) {
            self.active = None;
            self.close();
        }
    }
    pub fn submission_cancelled(&mut self, generation: u64) {
        if self.active == Some(generation) {
            self.active = None;
            self.suspend();
        }
    }
    pub fn close(&mut self) {
        self.closed = true;
        self.known = None;
        self.pending = false;
    }
}

/// The worker owns no AppKit objects. The injected probe must establish any
/// platform-specific autorelease scope. Dropping this handle never waits for a
/// platform call that may be stuck. Wake is a nonpanicking hint, may race
/// teardown, and must not retain or dereference an AppKit object.
pub struct PresenceWorker {
    requests: Option<std::sync::mpsc::SyncSender<u64>>,
    replies: std::sync::mpsc::Receiver<(u64, Option<bool>)>,
    closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    deadline: std::sync::Arc<Deadline>,
    limit: std::time::Duration,
}

/// The next instant at which the owner must be woken to notice an overdue
/// lookup. A wake for a lookup that already finished is a harmless no-op.
#[derive(Default)]
struct Deadline {
    next: std::sync::Mutex<Option<std::time::Instant>>,
    changed: std::sync::Condvar,
}

impl PresenceWorker {
    pub fn start(
        probe: std::sync::Arc<dyn Fn() -> Option<bool> + Send + Sync>,
        wake: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Self> {
        Self::start_with_deadline(probe, wake, LOOKUP_DEADLINE)
    }

    pub fn start_with_deadline(
        probe: std::sync::Arc<dyn Fn() -> Option<bool> + Send + Sync>,
        wake: std::sync::Arc<dyn Fn() + Send + Sync>,
        limit: std::time::Duration,
    ) -> std::io::Result<Self> {
        use std::sync::{Arc, atomic::Ordering, mpsc};
        let (request_tx, request_rx) = mpsc::sync_channel(1);
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_closed = closed.clone();
        let deadline = Arc::new(Deadline::default());
        let watchdog_deadline = deadline.clone();
        let watchdog_closed = closed.clone();
        let watchdog_wake = wake.clone();
        std::thread::Builder::new()
            .name("menu-health-deadline".into())
            .spawn(move || {
                let Ok(mut next) = watchdog_deadline.next.lock() else { return };
                loop {
                    if watchdog_closed.load(Ordering::Acquire) {
                        return;
                    }
                    let now = std::time::Instant::now();
                    match *next {
                        None => match watchdog_deadline.changed.wait(next) {
                            Ok(guard) => next = guard,
                            Err(_) => return,
                        },
                        Some(due) if due <= now => {
                            *next = None;
                            drop(next);
                            if !watchdog_closed.load(Ordering::Acquire) {
                                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| watchdog_wake()));
                            }
                            next = match watchdog_deadline.next.lock() {
                                Ok(guard) => guard,
                                Err(_) => return,
                            };
                        }
                        Some(due) => match watchdog_deadline.changed.wait_timeout(next, due - now) {
                            Ok((guard, _)) => next = guard,
                            Err(_) => return,
                        },
                    }
                }
            })?;
        let spawned = std::thread::Builder::new()
            .name("menu-health".into())
            .spawn(move || {
                while let Ok(generation) = request_rx.recv() {
                    if worker_closed.load(Ordering::Acquire) {
                        break;
                    }
                    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| probe()))
                        .unwrap_or(None);
                    if worker_closed.load(Ordering::Acquire) {
                        break;
                    }
                    if reply_tx.send((generation, answer)).is_err() {
                        break;
                    }
                    if !worker_closed.load(Ordering::Acquire) {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wake()));
                    }
                }
            });
        if let Err(error) = spawned {
            // Without an owner, Drop never runs: retire the watchdog here.
            closed.store(true, Ordering::Release);
            if let Ok(_guard) = deadline.next.lock() {
                deadline.changed.notify_one();
            }
            return Err(error);
        }
        Ok(Self {
            requests: Some(request_tx),
            replies: reply_rx,
            closed,
            deadline,
            limit,
        })
    }

    pub fn request(&self, generation: u64) -> bool {
        let sent = self
            .requests
            .as_ref()
            .is_some_and(|sender| sender.try_send(generation).is_ok());
        if sent {
            if let Ok(mut next) = self.deadline.next.lock() {
                *next = Some(std::time::Instant::now() + self.limit);
                self.deadline.changed.notify_one();
            }
        }
        sent
    }

    pub fn limit(&self) -> std::time::Duration {
        self.limit
    }

    pub fn poll(&self) -> Result<Option<(u64, Option<bool>)>, std::sync::mpsc::TryRecvError> {
        match self.replies.try_recv() {
            Ok(reply) => Ok(Some(reply)),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl Drop for PresenceWorker {
    fn drop(&mut self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        self.requests.take();
        if let Ok(_guard) = self.deadline.next.lock() {
            self.deadline.changed.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Presentation, PresenceRefresh, PresenceWorker};

    #[test]
    fn pending_check_is_neutral_but_never_reported_as_running() {
        assert_eq!(Presentation::of(true, false), Presentation::Running);
        assert_eq!(Presentation::of(false, false), Presentation::NeedsAttention);
        assert_eq!(Presentation::of(false, true), Presentation::Checking);
        assert_eq!(Presentation::of(true, true), Presentation::Checking);
        assert!(Presentation::Checking.neutral_icon());
        assert!(Presentation::Running.neutral_icon());
        assert!(!Presentation::NeedsAttention.neutral_icon());
        assert_eq!(Presentation::Checking.tool_tip(), "Ruycast · Checking…");
        assert_eq!(Presentation::Checking.accessibility_label(), "Ruycast, checking");
        assert_eq!(Presentation::Running.accessibility_label(), "Ruycast, running");
        assert_eq!(Presentation::NeedsAttention.accessibility_label(), "Ruycast, needs attention");
        assert_eq!(serde_json::to_value(Presentation::NeedsAttention).unwrap(), "needsAttention");
    }

    #[test]
    fn coalesces_invalidations_and_rejects_stale_replies() {
        let mut state = PresenceRefresh::default();
        let first = state.invalidate().unwrap();
        assert_eq!(state.known(), None);
        assert_eq!(state.invalidate(), None);
        assert_eq!(state.invalidate(), None);
        let latest = state.complete(first, Some(true)).unwrap();
        assert_eq!(state.known(), None);
        assert_ne!(first, latest);
        assert_eq!(state.complete(first, Some(true)), None);
        assert_eq!(state.complete(latest, Some(false)), None);
        assert_eq!(state.known(), Some(false));
        let next = state.invalidate().unwrap();
        assert_eq!(state.known(), None);
        state.close();
        assert_eq!(state.complete(next, Some(true)), None);
        assert_eq!(state.invalidate(), None);
        assert_eq!(state.known(), None);
    }

    #[test]
    fn failed_probe_and_generation_exhaustion_fail_closed() {
        let mut state = PresenceRefresh::default();
        let current = state.invalidate().unwrap();
        assert_eq!(state.complete(current, None), None);
        assert_eq!(state.known(), None);
        state.generation = u64::MAX;
        assert_eq!(state.invalidate(), None);
        assert_eq!(state.known(), None);
        assert!(state.closed);
    }

    #[test]
    fn submission_failure_retires_unfinishable_generation() {
        let mut state = PresenceRefresh::default();
        let current = state.invalidate().unwrap();
        state.submission_failed(current);
        assert_eq!(state.known(), None);
        assert_eq!(state.complete(current, Some(true)), None);
        assert_eq!(state.invalidate(), None);
    }

    #[test]
    fn cancelled_unsubmitted_generation_can_be_retried_after_grant() {
        let mut state = PresenceRefresh::default();
        let current = state.invalidate().unwrap();
        state.submission_cancelled(current);
        assert_eq!(state.known(), None);
        let next = state.invalidate().unwrap();
        assert_ne!(next, current);
        assert_eq!(state.complete(next, Some(true)), None);
        assert_eq!(state.known(), Some(true));
    }

    #[test]
    fn suspended_in_flight_result_cannot_restore_a_hidden_owner() {
        let mut state = PresenceRefresh::default();
        let first = state.invalidate().unwrap();
        state.suspend();
        assert_eq!(state.complete(first, Some(true)), None);
        assert_eq!(state.known(), None);
        let next = state.invalidate().unwrap();
        assert_ne!(next, first);
        assert_eq!(state.complete(next, Some(false)), None);
        assert_eq!(state.known(), Some(false));
    }

    #[test]
    fn blocked_probe_never_blocks_owner_and_late_result_is_discarded_on_drop() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };
        use std::time::Duration;
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        struct Done(mpsc::Sender<()>);
        impl Drop for Done {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let done = Done(done_tx);
        let release_rx = Mutex::new(release_rx);
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_for_worker = wakes.clone();
        let worker = PresenceWorker::start(
            Arc::new(move || {
                let _keep_until_worker_stops = &done;
                started_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
                Some(true)
            }),
            Arc::new(move || {
                wakes_for_worker.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        assert!(worker.request(1));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(worker.poll().unwrap(), None);
        let (dropped_tx, dropped_rx) = mpsc::channel();
        let drop_thread = std::thread::spawn(move || {
            drop(worker);
            let _ = dropped_tx.send(());
        });
        let dropped_while_blocked = dropped_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        release_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop_thread.join().unwrap();
        assert!(dropped_while_blocked, "owner waited for a stuck probe");
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn overdue_lookup_stops_checking_and_late_current_reply_is_adopted() {
        use std::time::{Duration, Instant};
        let limit = Duration::from_secs(15);
        let mut state = PresenceRefresh::default();
        let first = state.invalidate().unwrap();
        let submitted = state.started.unwrap();
        assert!(!state.expire_overdue(submitted + limit - Duration::from_millis(1), limit));
        assert!(state.waiting());
        assert!(state.expire_overdue(submitted + limit, limit));
        assert!(!state.expire_overdue(submitted + limit * 2, limit), "expiry is reported once");
        assert!(!state.waiting());
        assert_eq!(state.known(), None);
        assert_eq!(Presentation::of(state.known().unwrap_or(false), state.waiting()), Presentation::NeedsAttention);
        assert_eq!(state.invalidate(), None, "a stuck lookup still blocks a second probe");
        assert!(!state.waiting());
        let next = state.complete(first, Some(true)).unwrap();
        assert!(state.waiting(), "the queued replacement starts a fresh deadline");
        assert!(!state.expire_overdue(Instant::now(), limit));
        assert_eq!(state.complete(next, Some(true)), None);
        assert_eq!(state.known(), Some(true));
        assert!(!state.waiting());
        assert!(!state.expire_overdue(Instant::now() + limit * 2, limit), "no active lookup, nothing to expire");
    }

    #[test]
    fn stuck_probe_wakes_owner_at_deadline_and_not_after_drop() {
        use std::sync::{Arc, Mutex, mpsc};
        use std::time::{Duration, Instant};
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let (wake_tx, wake_rx) = mpsc::channel();
        let limit = Duration::from_millis(150);
        let worker = PresenceWorker::start_with_deadline(
            Arc::new(move || {
                let _ = release_rx.lock().unwrap().recv_timeout(Duration::from_secs(5));
                Some(true)
            }),
            Arc::new(move || {
                let _ = wake_tx.send(Instant::now());
            }),
            limit,
        )
        .unwrap();
        let submitted = Instant::now();
        assert!(worker.request(1));
        let woke = wake_rx.recv_timeout(Duration::from_secs(2)).expect("deadline wake");
        assert!(woke.duration_since(submitted) >= limit);
        assert_eq!(worker.poll().unwrap(), None, "the probe is still stuck");
        drop(worker);
        release_tx.send(()).unwrap();
        assert!(wake_rx.recv_timeout(Duration::from_millis(400)).is_err(), "no wake after drop");
    }

    #[test]
    fn worker_reports_panic_as_unknown_and_accepts_next_probe() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };
        use std::time::Duration;
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_worker = attempts.clone();
        let (wake_tx, wake_rx) = mpsc::channel();
        let worker = PresenceWorker::start(
            Arc::new(move || {
                if attempts_for_worker.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("owned probe failure");
                }
                Some(false)
            }),
            Arc::new(move || {
                let _ = wake_tx.send(());
            }),
        )
        .unwrap();
        assert!(worker.request(1));
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(worker.poll().unwrap(), Some((1, None)));
        assert!(worker.request(2));
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(worker.poll().unwrap(), Some((2, Some(false))));
    }

    #[test]
    fn disconnected_reply_channel_is_distinct_from_empty() {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = PresenceWorker {
            requests: None,
            replies: rx,
            closed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            deadline: std::sync::Arc::default(),
            limit: super::LOOKUP_DEADLINE,
        };
        assert_eq!(worker.poll().unwrap(), None);
        drop(tx);
        assert!(matches!(
            worker.poll(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }
}
