//! Bounded read-only CLI search. Snapshot ownership remains separate from writes.
use crate::{clipboard_history::Matcher, control::Envelope};
use rustcast_core::Clip;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};

struct Job {
    request: Envelope,
    items: Arc<Vec<Clip>>,
    query: String,
}
pub struct Worker {
    sender: Option<SyncSender<Job>>,
    stopped: Arc<AtomicBool>,
    gate: Arc<Mutex<bool>>,
    worker: Option<JoinHandle<()>>,
    #[cfg(feature = "test-tools")]
    fixture_root: Option<std::path::PathBuf>,
}
impl Worker {
    pub fn new(fixture_root: Option<std::path::PathBuf>) -> Result<Self, String> {
        #[cfg(not(feature = "test-tools"))]
        let _ = &fixture_root;
        let (sender, receiver) = mpsc::sync_channel::<Job>(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let running = stopped.clone();
        let gate = Arc::new(Mutex::new(false));
        let worker_gate = gate.clone();
        #[cfg(feature = "test-tools")]
        let admitted_fixture = fixture_root.clone();
        let worker = std::thread::Builder::new()
            .name("ruycast-clipboard-query".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    if running.load(Ordering::Acquire) {
                        break;
                    }
                    #[cfg(feature = "test-tools")]
                    if let Some(root) = fixture_root.as_ref().filter(|root| {
                        root.parent().is_some_and(|p| {
                            std::fs::read(p.join(".ruycast-test-fixture"))
                                .ok()
                                .as_deref()
                                == Some(b"own-disposable-fixture-v1")
                        })
                    }) {
                        if root.join("clipboard-query-hold").exists() {
                            let _ = std::fs::write(
                                root.join("clipboard-query-entered"),
                                b"owned query boundary",
                            );
                            while root.join("clipboard-query-hold").exists()
                                && !running.load(Ordering::Acquire)
                                && job.request.context.is_live()
                            {
                                std::thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                    let matcher = (!job.query.is_empty()).then(|| Matcher::new(&job.query));
                    let mut indices = Vec::new();
                    for (index, clip) in job.items.iter().enumerate() {
                        if running.load(Ordering::Acquire) || !job.request.context.is_live() {
                            break;
                        }
                        if matcher
                            .as_ref()
                            .is_none_or(|matcher| matcher.matches(&clip.text))
                        {
                            indices.push(index);
                        }
                    }
                    let guard = worker_gate.lock().unwrap_or_else(|p| p.into_inner());
                    if *guard || running.load(Ordering::Acquire) || !job.request.context.is_live() {
                        continue;
                    }
                    let delivered = job.request.respond_clipboard(job.items, indices);
                    drop(guard);
                #[cfg(feature="test-tools")]
                if let Some(root) = fixture_root.as_ref().filter(|root| root.parent().is_some_and(|p| std::fs::read(p.join(".ruycast-test-fixture")).ok().as_deref() == Some(b"own-disposable-fixture-v1"))) {
                    let _ = std::fs::write(root.join("clipboard-query-reply"), b"owned reply boundary");
                }
                    // Keep the admission occupied through wire encoding/write, so
                    // slow clients cannot multiply large snapshot reply buffers.
                    loop {
                        match delivered.recv_timeout(Duration::from_millis(50)) {
                            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            Err(mpsc::RecvTimeoutError::Timeout)
                                if running.load(Ordering::Acquire) =>
                            {
                                break
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            sender: Some(sender),
            stopped,
            gate,
            worker: Some(worker),
            #[cfg(feature = "test-tools")]
            fixture_root: admitted_fixture,
        })
    }
    pub fn submit(&self, request: Envelope, items: Arc<Vec<Clip>>, query: String) {
        let Some(sender) = &self.sender else {
            let _ = request.respond(json!({"ok":false,"error":"Clipboard search is unavailable."}));
            return;
        };
        match sender.try_send(Job {
            request,
            items,
            query,
        }) {
            Ok(()) => {
                #[cfg(feature = "test-tools")]
                if let Some(root) = self.fixture_root.as_ref().filter(|root| {
                    root.parent().is_some_and(|p| {
                        std::fs::read(p.join(".ruycast-test-fixture"))
                            .ok()
                            .as_deref()
                            == Some(b"own-disposable-fixture-v1")
                    })
                }) {
                    let _ = std::fs::write(
                        root.join("clipboard-query-queued"),
                        b"owned queued boundary",
                    );
                }
            }
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                let _ = job
                    .request
                    .respond(json!({"ok":false,"error":"Clipboard search is busy. Try again."}));
            }
        }
    }
    pub fn stop(&mut self) {
        let mut guard = self.gate.lock().unwrap_or_else(|p| p.into_inner());
        *guard = true;
        self.stopped.store(true, Ordering::Release);
        self.sender.take();
    }
    pub fn finish(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.finish();
    }
}
