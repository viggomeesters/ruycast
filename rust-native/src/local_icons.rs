//! Bounded, transient local icon resolution; no image objects cross threads.
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread,
    time::Instant,
};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum Key {
    App(String),
    Path(String),
}
pub struct Resolved {
    pub image: Option<Vec<u8>>,
    pub name: String,
}
pub struct Completion {
    pub key: Key,
    pub started: Instant,
    pub value: Resolved,
}
struct Job {
    key: Key,
    ticket: u64,
    started: Instant,
}
struct Finished {
    job: Job,
    value: Resolved,
}
type Resolver = dyn Fn(&Key) -> Resolved + Send + Sync;

pub struct Service {
    apps: SyncSender<Job>,
    paths: SyncSender<Job>,
    completed: Receiver<Finished>,
    pending: HashMap<Key, u64>,
    rejected: VecDeque<Key>,
    overflowed: bool,
    next: u64,
    alive: Arc<AtomicBool>,
}
impl Service {
    pub fn new(resolve: Arc<Resolver>, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (apps, app_rx) = mpsc::sync_channel::<Job>(32);
        let (paths, path_rx) = mpsc::sync_channel::<Job>(32);
        let (finished, completed) = mpsc::sync_channel::<Finished>(64);
        let alive = Arc::new(AtomicBool::new(true));
        for rx in [app_rx, path_rx] {
            let (resolve, wake, finished, alive) = (
                resolve.clone(),
                wake.clone(),
                finished.clone(),
                alive.clone(),
            );
            thread::spawn(move || {
                while let Ok(job) = rx.recv() {
                    if !alive.load(Ordering::Acquire) {
                        break;
                    }
                    let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        resolve(&job.key)
                    }))
                    .unwrap_or_else(|_| Resolved {
                        image: None,
                        name: match &job.key {
                            Key::App(id) => id.clone(),
                            Key::Path(_) => String::new(),
                        },
                    });
                    if !alive.load(Ordering::Acquire) {
                        break;
                    }
                    if finished.try_send(Finished { job, value }).is_ok() {
                        wake();
                    }
                }
            });
        }
        Self {
            apps,
            paths,
            completed,
            pending: HashMap::new(),
            rejected: VecDeque::new(),
            overflowed: false,
            next: 0,
            alive,
        }
    }
    pub fn request(&mut self, key: Key) {
        if self.pending.contains_key(&key) {
            return;
        }
        if self.pending.len() >= 64 {
            self.mark_retry(key);
            return;
        }
        self.next = self.next.wrapping_add(1);
        let ticket = self.next;
        let job = Job {
            key: key.clone(),
            ticket,
            started: Instant::now(),
        };
        let sender = match &key {
            Key::App(_) => &self.apps,
            Key::Path(_) => &self.paths,
        };
        if sender.try_send(job).is_ok() {
            self.rejected.retain(|item| item != &key);
            self.pending.insert(key, ticket);
        } else {
            self.mark_retry(key);
        }
    }
    pub fn poll(&mut self) -> Vec<Completion> {
        let mut rows = Vec::new();
        while let Ok(done) = self.completed.try_recv() {
            if self.pending.get(&done.job.key) == Some(&done.job.ticket) {
                self.pending.remove(&done.job.key);
                rows.push(Completion {
                    key: done.job.key,
                    started: done.job.started,
                    value: done.value,
                });
            }
        }
        rows
    }
    pub fn is_pending(&self, key: &Key) -> bool {
        self.pending.contains_key(key)
    }
    pub fn needs_retry(&self, key: &Key) -> bool {
        self.rejected.contains(key) || (self.overflowed && !self.pending.contains_key(key))
    }
    pub fn mark_retry(&mut self, key: Key) {
        self.rejected.retain(|item| item != &key);
        if self.rejected.len() == 256 {
            self.overflowed = true;
            self.rejected.pop_front();
        }
        self.rejected.push_back(key);
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{atomic::AtomicUsize, Condvar, Mutex},
        time::Duration,
    };
    #[test]
    fn blocked_lookup_does_not_block_request_or_shutdown_and_deduplicates() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let test_gate = gate.clone();
        let test_calls = calls.clone();
        let mut service = Service::new(
            Arc::new(move |_| {
                test_calls.fetch_add(1, Ordering::SeqCst);
                let (lock, signal) = &*test_gate;
                let _held = signal
                    .wait_while(lock.lock().unwrap(), |open| !*open)
                    .unwrap();
                Resolved {
                    image: None,
                    name: String::new(),
                }
            }),
            Arc::new(|| {}),
        );
        service.request(Key::App("fixture".into()));
        let deadline = Instant::now() + Duration::from_secs(2);
        while calls.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let start = Instant::now();
        for _ in 0..100 {
            service.request(Key::App("fixture".into()));
        }
        assert!(start.elapsed() < Duration::from_millis(100));
        assert_eq!(service.pending.len(), 1);
        drop(service);
        assert!(start.elapsed() < Duration::from_millis(100));
        let (lock, signal) = &*gate;
        *lock.lock().unwrap() = true;
        signal.notify_all();
    }
    #[test]
    fn blocked_path_does_not_delay_application_or_allow_unbounded_queue() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let test_gate = gate.clone();
        let mut service = Service::new(
            Arc::new(move |key| {
                if matches!(key, Key::Path(_)) {
                    let (lock, signal) = &*test_gate;
                    let _held = signal
                        .wait_while(lock.lock().unwrap(), |open| !*open)
                        .unwrap();
                }
                Resolved {
                    image: None,
                    name: "fixture".into(),
                }
            }),
            Arc::new(|| {}),
        );
        for i in 0..100 {
            service.request(Key::Path(format!("fixture-{i}")));
        }
        assert!(service.pending.len() <= 33);
        service.request(Key::App("owned".into()));
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut ready = false;
        while Instant::now() < deadline {
            if service
                .poll()
                .iter()
                .any(|item| item.key == Key::App("owned".into()))
            {
                ready = true;
                break;
            }
            thread::yield_now();
        }
        assert!(ready);
        drop(service);
        let (lock, signal) = &*gate;
        *lock.lock().unwrap() = true;
        signal.notify_all();
    }
    #[test]
    fn overflow_preserves_retry_for_evicted_visible_keys() {
        let mut service = Service::new(
            Arc::new(|_| Resolved {
                image: None,
                name: String::new(),
            }),
            Arc::new(|| {}),
        );
        for i in 0..300 {
            service.mark_retry(Key::Path(format!("owned-{i}")));
        }
        assert!(service.needs_retry(&Key::Path("owned-0".into())));
        assert!(service.rejected.len() <= 256);
    }
}
