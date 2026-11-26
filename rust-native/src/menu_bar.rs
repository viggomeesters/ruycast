//! Plain menu state, refreshed with configuration and analytics health.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct State {
    pub visible: bool,
    pub healthy: bool,
    pub requires_input: bool,
}

/// Main-thread bookkeeping for a bounded, single-flight presence probe.
/// Invalidation immediately removes the old answer; stale replies are never
/// adopted and at most one replacement probe is queued.
#[derive(Default)]
pub struct PresenceRefresh {
    generation: u64,
    active: Option<u64>,
    pending: bool,
    known: Option<bool>,
    closed: bool,
}

impl PresenceRefresh {
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
            self.active = Some(next);
            Some(next)
        }
    }

    pub fn complete(&mut self, generation: u64, value: Option<bool>) -> Option<u64> {
        if self.active != Some(generation) {
            return None;
        }
        self.active = None;
        if self.closed {
            return None;
        }
        if generation == self.generation {
            self.known = value;
        }
        if self.pending {
            self.pending = false;
            self.active = Some(self.generation);
            Some(self.generation)
        } else {
            None
        }
    }

    pub fn known(&self) -> Option<bool> {
        self.known
    }
    /// A failed submission cannot produce a reply. Retire this owner rather
    /// than leave an active generation that can never finish.
    pub fn submission_failed(&mut self, generation: u64) {
        if self.active == Some(generation) {
            self.active = None;
            self.close();
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
}

impl PresenceWorker {
    pub fn start(
        probe: std::sync::Arc<dyn Fn() -> Option<bool> + Send + Sync>,
        wake: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        use std::sync::{Arc, atomic::Ordering, mpsc};
        let (request_tx, request_rx) = mpsc::sync_channel(1);
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_closed = closed.clone();
        std::thread::spawn(move || {
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
        Self {
            requests: Some(request_tx),
            replies: reply_rx,
            closed,
        }
    }

    pub fn request(&self, generation: u64) -> bool {
        self.requests
            .as_ref()
            .is_some_and(|sender| sender.try_send(generation).is_ok())
    }

    pub fn poll(&self) -> Option<(u64, Option<bool>)> {
        self.replies.try_recv().ok()
    }
}

impl Drop for PresenceWorker {
    fn drop(&mut self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        self.requests.take();
    }
}

#[cfg(test)]
mod tests {
    use super::{PresenceRefresh, PresenceWorker};

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
        );
        assert!(worker.request(1));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(worker.poll(), None);
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
        );
        assert!(worker.request(1));
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(worker.poll(), Some((1, None)));
        assert!(worker.request(2));
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(worker.poll(), Some((2, Some(false))));
    }
}
