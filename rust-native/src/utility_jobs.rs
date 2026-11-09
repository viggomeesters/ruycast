//! Bounded Utilities I/O jobs. SDK/Keychain latency never blocks AppKit or the
//! serialized runtime. A timed-out worker keeps its capacity/profile lease
//! until its actual syscall returns; no replacement thread is launched for it.
use crate::{
    control::HostEvent,
    execution::Live,
    utilities::Utilities,
    utility_storage::{Authority, NativeStorage},
};
use rustcast_core::Command;
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::SyncSender,
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Instant,
};

pub type Store = Arc<Mutex<Utilities<NativeStorage>>>;
const LIMIT: usize = 2;
static WORKERS: AtomicUsize = AtomicUsize::new(0);
struct Lease;
impl Lease {
    fn acquire() -> Result<Self, String> {
        WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < LIMIT).then_some(n + 1)
            })
            .map(|_| Self)
            .map_err(|_| {
                "Utilities are busy. Try again after the current operation finishes.".into()
            })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}
pub enum Operation {
    Search { mode: String, query: String },
    Import(Value, Arc<std::sync::atomic::AtomicU8>),
}
pub enum Output {
    Rows(Vec<Command>),
    Imported(Value, Option<String>),
}
type Slot = Arc<Mutex<Option<Result<Output, String>>>>;
struct Job<C> {
    context: Option<C>,
    thread: JoinHandle<()>,
    result: Slot,
    stop: Arc<AtomicBool>,
    live: Live,
    deadline: Instant,
}
pub struct Jobs<C> {
    jobs: Vec<Job<C>>,
    store: Store,
    wake: SyncSender<HostEvent>,
}
impl<C> Jobs<C> {
    #[cfg(feature = "test-tools")]
    pub(crate) fn owned_pending(&self) -> Value {
        serde_json::json!({"retained_jobs":self.jobs.len(),"active_contexts":self.jobs.iter().filter(|j|j.context.is_some()).count(),"global_workers":WORKERS.load(Ordering::Acquire)})
    }
    pub fn new(store: Store, wake: SyncSender<HostEvent>) -> Self {
        Self {
            jobs: vec![],
            store,
            wake,
        }
    }
    pub fn busy(&self) -> bool {
        !self.jobs.is_empty()
    }
    pub fn start(
        &mut self,
        operation: Operation,
        live: Live,
        deadline: Instant,
        context: C,
    ) -> Result<(), (String, C)> {
        if self.jobs.len() >= LIMIT || !live() || Instant::now() >= deadline {
            return Err((
                "Utilities are busy or this request expired. Try again.".into(),
                context,
            ));
        }
        let lease = match Lease::acquire() {
            Ok(v) => v,
            Err(e) => return Err((e, context)),
        };
        let store = self.store.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = stop.clone();
        let check = live.clone();
        let wake = self.wake.clone();
        let result: Slot = Default::default();
        let output = result.clone();
        let thread = thread::Builder::new()
            .name("ruycast-utilities-io".into())
            .spawn(move || {
                let _lease = lease;
                let live: Live = Arc::new(move || {
                    !cancel.load(Ordering::Acquire) && Instant::now() < deadline && check()
                });
                let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if !live() {
                        return Err("Utilities request expired or was cancelled.".into());
                    }
                    let mut store = loop {
                        if !live(){return Err("Utilities request expired or was cancelled.".into());}
                        match store.try_lock(){Ok(store)=>break store,Err(std::sync::TryLockError::WouldBlock)=>std::thread::sleep(std::time::Duration::from_millis(10)),Err(_)=>return Err("Utilities storage stopped unexpectedly. Restart Ruycast before retrying.".into())}
                    };
                    let mut authority=Authority::read_write(live.clone(),Arc::new(||Ok(())));
                    if let Operation::Import(_,publication)=&operation{authority.changed=publication.clone();}
                    store.storage_mut().set_authority(Some(authority));
                    let value = match operation {
                        Operation::Search { mode, query } => {
                            store.commands(&mode, &query).map(Output::Rows)
                        }
                        Operation::Import(value,_) => store
                            .import_profile(value)
                            .map(|value| Output::Imported(value, store.take_publication_warning())),
                    };
                    store.storage_mut().set_authority(None);
                    value
                }))
                .unwrap_or_else(|_| Err("Utilities operation stopped unexpectedly.".into()));
                if live() {
                    *output.lock().unwrap_or_else(|p| p.into_inner()) = Some(value);
                }
                let _ = wake.try_send(HostEvent::UiWake);
            });
        match thread {
            Ok(thread) => {
                self.jobs.push(Job {
                    context: Some(context),
                    thread,
                    result,
                    stop,
                    live,
                    deadline,
                });
                Ok(())
            }
            Err(_) => Err(("Could not start Utilities operation.".into(), context)),
        }
    }
    pub fn poll(&mut self) -> Vec<(C, Result<Output, String>)> {
        let mut done = vec![];
        for job in &mut self.jobs {
            if job.context.is_none() {
                continue;
            }
            let value = job.result.lock().unwrap_or_else(|p| p.into_inner()).take();
            if let Some(value) = value {
                done.push((job.context.take().unwrap(), value));
            } else if !(job.live)() || Instant::now() >= job.deadline {
                job.stop.store(true, Ordering::Release);
                done.push((
                    job.context.take().unwrap(),
                    Err("Utilities request expired or was cancelled.".into()),
                ));
            }
        }
        let mut i = 0;
        while i < self.jobs.len() {
            if self.jobs[i].context.is_none() && self.jobs[i].thread.is_finished() {
                let job = self.jobs.swap_remove(i);
                let _ = job.thread.join();
            } else {
                i += 1;
            }
        }
        done
    }
    pub fn next_wake(&self) -> Option<Instant> {
        self.jobs
            .iter()
            .filter(|j| j.context.is_some())
            .map(|j| j.deadline)
            .min()
    }
}
impl<C> Drop for Jobs<C> {
    fn drop(&mut self) {
        for job in &self.jobs {
            job.stop.store(true, Ordering::Release);
        }
        for job in self.jobs.drain(..) {
            if job.thread.is_finished() {
                let _ = job.thread.join();
            }
        }
    }
}
