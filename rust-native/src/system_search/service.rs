//! At most three native workers: main Safari, dedicated OS discovery and action.
//! No unbounded task queue; supersession revokes authority before admission.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Mutex,
};
use std::thread::JoinHandle;

struct Lane {
    busy: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}
impl Default for Lane {
    fn default() -> Self {
        Self {
            busy: Arc::new(AtomicBool::new(false)),
            workers: vec![],
        }
    }
}
impl Lane {
    fn spawn<T: Send + 'static>(
        &mut self,
        ticket: Ticket,
        wake: Arc<dyn Fn() + Send + Sync>,
        job: impl FnOnce(Ticket) -> Result<T, String> + Send + 'static,
    ) -> Result<Pending<T>, String> {
        let mut i = 0;
        while i < self.workers.len() {
            if self.workers[i].is_finished() {
                let h = self.workers.swap_remove(i);
                let _ = h.join();
            } else {
                i += 1;
            }
        }
        ticket.check()?;
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err("System search is busy. Refresh to retry.".into());
        }
        let busy = self.busy.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let check = stop.clone();
        let parent = ticket.live.clone();
        let deadline = ticket.deadline;
        let live = Arc::new(move || !check.load(Ordering::Acquire) && parent());
        let handle = std::thread::Builder::new()
            .name("ruycast-os-search".into())
            .spawn(move || {
                struct Release(Arc<AtomicBool>);
                impl Drop for Release {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }
                let _release = Release(busy);
                let ticket = Ticket { deadline, live };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(ticket)))
                    .unwrap_or_else(|_| {
                        Err("System search stopped unexpectedly. Refresh to retry.".into())
                    });
                let _ = tx.try_send(result);
                wake();
            });
        match handle {
            Ok(h) => {
                self.workers.push(h);
                Ok(Pending {
                    receive: rx,
                    stop,
                    deadline,
                    timeout_error: CANCELLED,
                })
            }
            Err(_) => {
                self.busy.store(false, Ordering::Release);
                Err("Could not start system search. Refresh to retry.".into())
            }
        }
    }
}
pub struct Pending<T> {
    receive: mpsc::Receiver<Result<T, String>>,
    stop: Arc<AtomicBool>,
    pub deadline: Instant,
    timeout_error: &'static str,
}
impl<T> Pending<T> {
    pub fn poll(&self) -> Option<Result<T, String>> {
        if Instant::now() >= self.deadline {
            self.stop.store(true, Ordering::Release);
            return Some(Err(self.timeout_error.into()));
        }
        match self.receive.try_recv() {
            Ok(v) => Some(v),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("System search stopped. Refresh to retry.".into())),
        }
    }
}
impl<T> Drop for Pending<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
pub struct Service {
    layouts: Arc<Mutex<crate::window_layout::Manager>>,
    resource_lane:Mutex<Lane>,
    pub main: Arc<providers::Provider>,
    pub dedicated: Arc<providers::Provider>,
    main_lane: Mutex<Lane>,
    dedicated_lane: Mutex<Lane>,
    action_lane: Mutex<Lane>,
    consent_lane: Mutex<Lane>,
    main_retry: Arc<Mutex<Option<Instant>>>,
}
impl Service {
    pub fn effect(&self,ticket:Ticket,wake:Arc<dyn Fn()+Send+Sync>,job:impl FnOnce(Ticket)->Result<(),String>+Send+'static)->Result<Pending<()>,String>{self.action_lane.lock().unwrap_or_else(|p|p.into_inner()).spawn(ticket,wake,job)}
    pub fn resource<T:Send+'static>(&self,ticket:Ticket,wake:Arc<dyn Fn()+Send+Sync>,job:impl FnOnce(Ticket)->Result<T,String>+Send+'static)->Result<Pending<T>,String>{self.resource_lane.lock().unwrap_or_else(|p|p.into_inner()).spawn(ticket,wake,job)}
    pub fn layout(
        &self,
        layout: String,
        previous: Option<Application>,
        foreground: Option<main_bridge::Client>,
        ticket: Ticket,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Pending<rustcast_core::Frame>, String> {
        let platform = self.dedicated.platform.clone();
        let layouts = self.layouts.clone();
        self.action_lane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .spawn(ticket, wake, move |ticket| {
                let previous = match previous {Some(previous)=>Some(previous),None=>foreground
                        .as_ref()
                        .map(|f| f.foreground(&ticket))
                        .transpose()?
                        .flatten()}
                    .ok_or("Select a window first.")?;
                let screens = foreground
                    .as_ref()
                    .ok_or("Native screen service unavailable.")?
                    .screens(&ticket)?;
                layouts
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .move_window(&layout, previous.identity, &screens, &*platform, &ticket)
            })
    }
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self {
            layouts: Default::default(),
            resource_lane:Default::default(),
            main: Arc::new(providers::Provider::new(platform.clone(), true)),
            dedicated: Arc::new(providers::Provider::new(platform, false)),
            main_lane: Default::default(),
            dedicated_lane: Default::default(),
            action_lane: Default::default(),
            consent_lane: Default::default(),
            main_retry: Default::default(),
        }
    }
    pub fn search(
        &self,
        scope: Scope,
        query: String,
        previous: Option<Application>,
        ticket: Ticket,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Pending<Results>, String> {
        self.search_with_foreground(scope, query, previous, None, ticket, wake)
    }
    pub fn search_with_foreground(
        &self,
        scope: Scope,
        query: String,
        previous: Option<Application>,
        foreground: Option<main_bridge::Client>,
        ticket: Ticket,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Pending<Results>, String> {
        let root = scope == Scope::MainTabs;
        let provider = if root {
            self.main.clone()
        } else {
            self.dedicated.clone()
        };
        let retry = self.main_retry.clone();
        let lane = if root {
            &self.main_lane
        } else {
            &self.dedicated_lane
        };
        lane.lock()
            .unwrap_or_else(|p| p.into_inner())
            .spawn(ticket, wake, move |ticket| {
                if root
                    && retry
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .is_some_and(|d| Instant::now() < d)
                {
                    return Ok(Results::default());
                }
                let previous = if scope == Scope::Menus && previous.is_none() {
                    foreground
                        .map(|f| f.foreground(&ticket))
                        .transpose()?
                        .flatten()
                } else {
                    previous
                };
                let result = provider.search(scope, &query, previous, &ticket);
                if root {
                    match result {
                        Ok(mut r) => {
                            r.truncate(if query.trim().is_empty() { 5 } else { 50 });
                            Ok(r)
                        }
                        Err(_) => {
                            // A superseded consumer cannot throttle the next
                            // query. Match the original ticket.current guard.
                            if (ticket.live)() {
                                *retry.lock().unwrap_or_else(|p| p.into_inner()) =
                                    Some(Instant::now() + std::time::Duration::from_secs(1));
                            }
                            Ok(Results::default())
                        }
                    }
                } else {
                    result
                }
            })
    }
    pub fn execute(
        &self,
        token: String,
        previous: Option<Identity>,
        ticket: Ticket,
        activate: Activate,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Pending<()>, String> {
        self.execute_prepared(token, previous, ticket, activate, None, wake)
    }
    pub fn execute_prepared(
        &self,
        token: String,
        previous: Option<Identity>,
        ticket: Ticket,
        activate: Activate,
        before: Option<Prepare>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Pending<()>, String> {
        let provider = if token.starts_with("system.tab:root.") {
            self.main.clone()
        } else {
            self.dedicated.clone()
        };
        if !provider.recognizes(&token) {
            return Err(EXPIRED.into());
        }
        self.action_lane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .spawn(ticket, wake, move |t| {
                if let Some(before) = before {
                    before(&t)?;
                }
                provider.execute(&token, previous, &t, &activate)
            })
    }
    pub fn run_shortcut(
        &self,
        id: String,
        ticket: Ticket,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Pending<()>, String> {
        self.run_shortcut_prepared(id, ticket, None, wake)
    }
    pub fn run_shortcut_prepared(
        &self,
        id: String,
        ticket: Ticket,
        before: Option<Prepare>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Pending<()>, String> {
        let platform = self.dedicated.platform.clone();
        self.action_lane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .spawn(ticket, wake, move |t| {
                if let Some(before) = before {
                    before(&t)?;
                }
                platform.run_shortcut(&id, &t)
            })
            .map(|mut pending| {
                pending.timeout_error = apple_shortcuts::TIMEOUT;
                pending
            })
    }
    pub fn consent(
        &self,
        ticket: Ticket,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Pending<()>, String> {
        let provider = self.dedicated.clone();
        self.consent_lane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .spawn(ticket, wake, move |t| provider.platform.safari_consent(&t))
    }
    pub fn refresh(&self) {
        self.main.refresh();
        self.dedicated.refresh();
        *self.main_retry.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    /// No new work is admitted by a dropped owner. Native SDK calls are capped
    /// by the original ticket deadline; joins happen only once they are done.
    pub fn reap(&self) {
        for lane in [
            &self.resource_lane,
            &self.main_lane,
            &self.dedicated_lane,
            &self.action_lane,
            &self.consent_lane,
        ] {
            let mut lane = lane.lock().unwrap_or_else(|p| p.into_inner());
            let mut i = 0;
            while i < lane.workers.len() {
                if lane.workers[i].is_finished() {
                    let h = lane.workers.swap_remove(i);
                    let _ = h.join();
                } else {
                    i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_worker_bounds_admission_and_dropped_receipt_revokes_late_work() {
        let mut lane = Lane::default();
        let (entered, wait) = mpsc::channel();
        let (release, hold) = mpsc::channel();
        let t = Ticket {
            deadline: Instant::now() + std::time::Duration::from_secs(2),
            live: Arc::new(|| true),
        };
        let pending = lane
            .spawn(t.clone(), Arc::new(|| {}), move |ticket| {
                entered.send(()).unwrap();
                hold.recv().unwrap();
                ticket.check()?;
                Ok(())
            })
            .unwrap();
        wait.recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(lane
            .spawn(t, Arc::new(|| {}), |_| -> Result<(), String> {
                panic!("No second worker")
            })
            .is_err());
        assert_eq!(lane.workers.len(), 1);
        drop(pending);
        release.send(()).unwrap();
        for thread in lane.workers.drain(..) {
            thread.join().unwrap();
        }
        assert!(!lane.busy.load(Ordering::Acquire));
    }
}
