//! Bounded execution workers. Result delivery does not wait for thread exit.
use crate::control::HostEvent;
use rustcast_core::Command;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::SyncSender,
        Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::Instant,
};

pub const LIMIT: usize = crate::control::CLIENT_LIMIT;
pub const CLI_LIMIT: usize = LIMIT - 1;
pub type Live = Arc<dyn Fn() -> bool + Send + Sync>;
pub type BeforeCommit = Box<dyn FnOnce() -> Result<(), String> + Send>;
pub type OpenPrepare = Box<dyn FnOnce() -> Result<crate::workspace::OpenRequest, String> + Send>;
pub struct CommitRequest {
    pub token: String,
    pub reply: std::sync::mpsc::SyncSender<Result<(), String>>,
}
/// Provider data cannot create a trusted script capture or workspace outcome.
pub enum Outcome {
    Script(crate::supervision::Outcome),
    Workspace(crate::workspace::Outcome),
    Snippet(crate::paste::Outcome),
}
type ResultSlot = Arc<Mutex<Slot>>;
enum Slot {
    Pending,
    Ready(Result<Outcome, String>),
    Retired,
}
#[derive(Default)]
struct Budget {
    total: AtomicUsize,
    cli: AtomicUsize,
}
struct Lease {
    budget: Arc<Budget>,
    cli: bool,
}
impl Budget {
    fn reserve(self: &Arc<Self>, cli: bool) -> Option<Lease> {
        self.total
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < LIMIT).then_some(n + 1)
            })
            .ok()?;
        if cli
            && self
                .cli
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < CLI_LIMIT).then_some(n + 1)
                })
                .is_err()
        {
            self.total.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Lease {
            budget: self.clone(),
            cli,
        })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.cli {
            self.budget.cli.fetch_sub(1, Ordering::AcqRel);
        }
        self.budget.total.fetch_sub(1, Ordering::AcqRel);
    }
}
fn process_budget() -> Arc<Budget> {
    static BUDGET: OnceLock<Arc<Budget>> = OnceLock::new();
    BUDGET.get_or_init(|| Arc::new(Budget::default())).clone()
}
#[derive(Clone)]
enum Kind {
    Script,
    Workspace(Arc<crate::workspace::Dispatch>),
    Snippet(Arc<crate::paste::Dispatch>),
}
impl Kind {
    fn fallback(&self) -> Result<Outcome, String> {
        match self {
            Self::Script => Err(
                "Script execution failed unexpectedly; check its result before retrying.".into(),
            ),
            Self::Workspace(dispatch) => Ok(Outcome::Workspace(dispatch.retire())),
            Self::Snippet(dispatch) => Ok(Outcome::Snippet(dispatch.retire())),
        }
    }
    fn join_on_shutdown(&self) -> bool {
        matches!(self, Self::Script)
    }
}
pub enum WorkspaceMode {
    Production,
    #[cfg(feature = "test-tools")]
    Fixture(PathBuf),
}
struct Job<C> {
    id: u64,
    context: Option<C>,
    stop: Arc<AtomicBool>,
    result: ResultSlot,
    kind: Kind,
    deadline: Option<Instant>,
    live: Live,
    thread: JoinHandle<()>,
}
pub struct Finished<C> {
    pub id: u64,
    pub context: C,
    pub result: Result<Outcome, String>,
}
pub struct Manager<C> {
    jobs: Vec<Job<C>>,
    serial: u64,
    home: PathBuf,
    sender: SyncSender<HostEvent>,
    budget: Arc<Budget>,
}
impl<C> Manager<C> {
    pub fn new(home: PathBuf, sender: SyncSender<HostEvent>) -> Self {
        Self {
            jobs: vec![],
            serial: 0,
            home,
            sender,
            budget: process_budget(),
        }
    }
    pub fn count(&self) -> usize {
        self.jobs.iter().filter(|j| j.context.is_some()).count()
    }
    pub fn script_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| j.context.is_some() && matches!(j.kind, Kind::Script))
            .count()
    }
    pub fn snippet_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| j.context.is_some() && matches!(j.kind, Kind::Snippet(_)))
            .count()
    }
    pub fn ready_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| {
                j.context.is_some()
                    && matches!(
                        *j.result.lock().unwrap_or_else(|p| p.into_inner()),
                        Slot::Ready(_)
                    )
            })
            .count()
    }
    /// Includes detached work from older managers in this process.
    pub fn live_workers(&self) -> usize {
        self.budget.total.load(Ordering::Acquire)
    }
    pub fn any(&self, predicate: impl Fn(&C) -> bool) -> bool {
        self.jobs
            .iter()
            .filter_map(|j| j.context.as_ref())
            .any(predicate)
    }
    fn reap_finished(&mut self) {
        let mut i = 0;
        while i < self.jobs.len() {
            if self.jobs[i].context.is_none() && self.jobs[i].thread.is_finished() {
                let job = self.jobs.swap_remove(i);
                let _ = job.thread.join();
            } else {
                i += 1;
            }
        }
    }
    pub fn admit(
        &mut self,
        command: Command,
        cli: bool,
        live: Live,
        deadline: Option<Instant>,
        context: C,
    ) -> Result<u64, (String, C)> {
        if let Err(error) = budget(&command, None) {
            return Err((error, context));
        }
        self.reap_finished();
        if self.jobs.len() >= LIMIT
            || self.budget.total.load(Ordering::Acquire) >= LIMIT
            || (cli && self.budget.cli.load(Ordering::Acquire) >= CLI_LIMIT)
        {
            return Err((
                "Script execution is busy. Try again after an active command finishes.".into(),
                context,
            ));
        }
        if !live() || deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(("Request expired or was cancelled.".into(), context));
        }
        let helper = match crate::supervision::discover() {
            Ok(path) => path,
            Err(error) => return Err((error, context)),
        };
        let home = self.home.clone();
        self.admit_job(
            cli,
            live,
            deadline,
            context,
            Kind::Script,
            true,
            move |live| {
                crate::supervision::run(&helper, &command, &home, deadline, || !live())
                    .map(Outcome::Script)
            },
        )
    }
    /// Prepared literal target only. The real runtime must still provide bound
    /// origin/revalidation and native intent-specific hide acknowledgement.
    #[allow(clippy::too_many_arguments)]
    pub fn admit_open(
        &mut self,
        request: crate::workspace::OpenRequest,
        mode: WorkspaceMode,
        cli: bool,
        live: Live,
        deadline: Instant,
        before_commit: BeforeCommit,
        context: C,
    ) -> Result<u64, (String, C)> {
        self.admit_open_prepared(
            Box::new(move || Ok(request)),
            mode,
            cli,
            live,
            deadline,
            before_commit,
            context,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn admit_open_prepared(
        &mut self,
        prepare: OpenPrepare,
        mode: WorkspaceMode,
        cli: bool,
        live: Live,
        deadline: Instant,
        before_commit: BeforeCommit,
        context: C,
    ) -> Result<u64, (String, C)> {
        self.admit_open_prepared_tracked(
            prepare,
            mode,
            cli,
            live,
            deadline,
            before_commit,
            Arc::new(crate::workspace::Dispatch::default()),
            context,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn admit_open_prepared_tracked(
        &mut self,
        prepare: OpenPrepare,
        mode: WorkspaceMode,
        cli: bool,
        live: Live,
        deadline: impl Into<Option<Instant>>,
        before_commit: BeforeCommit,
        dispatch: Arc<crate::workspace::Dispatch>,
        context: C,
    ) -> Result<u64, (String, C)> {
        let deadline = deadline.into();
        self.admit_job(
            cli,
            live,
            deadline,
            context,
            Kind::Workspace(dispatch.clone()),
            false,
            move |live| {
                #[cfg(feature = "test-tools")]
                let prepared = (|| {
                    if let WorkspaceMode::Fixture(home) = &mode {
                        if !crate::workspace::fixture_installed(home) {
                            return Err(
                                "Native workspace fixture boundary is not installed.".into()
                            );
                        }
                    }
                    prepare()
                })();
                #[cfg(not(feature = "test-tools"))]
                let prepared = prepare();
                let request = match prepared {
                    Ok(request) => request,
                    Err(error) => {
                        return Ok(Outcome::Workspace(crate::workspace::Outcome {
                            response: serde_json::json!({"ok":false,"error":error}),
                            committed: false,
                        }))
                    }
                };
                let outcome = match mode {
                    WorkspaceMode::Production => crate::workspace::execute_authorized(
                        &request,
                        live.as_ref(),
                        deadline,
                        before_commit,
                        &dispatch,
                    ),
                    #[cfg(feature = "test-tools")]
                    WorkspaceMode::Fixture(home) => crate::workspace::execute_fixture_authorized(
                        &request,
                        &home,
                        live.as_ref(),
                        deadline,
                        before_commit,
                        &dispatch,
                    ),
                };
                Ok(Outcome::Workspace(outcome))
            },
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn admit_snippet(
        &mut self,
        cli: bool,
        live: Live,
        deadline: Option<Instant>,
        dispatch: Arc<crate::paste::Dispatch>,
        context: C,
        work: impl FnOnce(Live) -> crate::paste::Outcome + Send + 'static,
    ) -> Result<u64, (String, C)> {
        self.admit_job(
            cli,
            live,
            deadline,
            context,
            Kind::Snippet(dispatch),
            false,
            move |live| Ok(Outcome::Snippet(work(live))),
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn admit_job<F>(
        &mut self,
        cli: bool,
        live: Live,
        deadline: Option<Instant>,
        context: C,
        kind: Kind,
        live_checked: bool,
        work: F,
    ) -> Result<u64, (String, C)>
    where
        F: FnOnce(Live) -> Result<Outcome, String> + Send + 'static,
    {
        self.reap_finished();
        if (!live_checked && !live()) || deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(("Request expired or was cancelled.".into(), context));
        }
        if self.jobs.len() >= LIMIT {
            return Err((
                "Script execution is busy. Try again after an active command finishes.".into(),
                context,
            ));
        }
        let Some(lease) = self.budget.reserve(cli) else {
            return Err((
                "Script execution is busy. Try again after an active command finishes.".into(),
                context,
            ));
        };
        let stop = Arc::new(AtomicBool::new(false));
        let result = Arc::new(Mutex::new(Slot::Pending));
        let output = result.clone();
        let job_live = live.clone();
        let cancel = stop.clone();
        let sender = self.sender.clone();
        let worker_kind = kind.clone();
        let worker = thread::Builder::new()
            .name("ruycast-execution".into())
            .spawn(move || {
                let _lease = lease; // Held through result publication and actual work completion.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(Arc::new(move || !cancel.load(Ordering::Acquire) && live()))
                }))
                .unwrap_or_else(|_| worker_kind.fallback());
                let mut slot = output.lock().unwrap_or_else(|p| p.into_inner());
                if matches!(*slot, Slot::Pending) {
                    *slot = Slot::Ready(outcome);
                    drop(slot);
                    let _ = sender.try_send(HostEvent::UiWake);
                }
            });
        let thread = match worker {
            Ok(thread) => thread,
            Err(_) => return Err(("Could not start the script worker.".into(), context)),
        };
        self.serial = self.serial.wrapping_add(1);
        let id = self.serial;
        self.jobs.push(Job {
            id,
            context: Some(context),
            stop,
            result,
            kind,
            deadline,
            live: job_live,
            thread,
        });
        Ok(id)
    }
    pub fn harvest(&mut self) -> Vec<Finished<C>> {
        let mut output = self.retire_open_if(|job| {
            job.deadline.is_some_and(|d| Instant::now() >= d)
                || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| !(job.live)()))
                    .unwrap_or(true)
        });
        for job in &mut self.jobs {
            if job.context.is_none() {
                continue;
            }
            let mut slot = job.result.lock().unwrap_or_else(|p| p.into_inner());
            if matches!(*slot, Slot::Pending) && job.thread.is_finished() {
                *slot = Slot::Ready(job.kind.fallback());
            }
            if matches!(*slot, Slot::Ready(_)) {
                if let Slot::Ready(result) = std::mem::replace(&mut *slot, Slot::Retired) {
                    output.push(Finished {
                        id: job.id,
                        context: job.context.take().expect("undelivered context"),
                        result,
                    });
                }
            }
        }
        self.reap_finished();
        output
    }
    /// Owner-driven deadline/cancellation checks continue while native calls
    /// block. No background polling when no workspace request is pending.
    pub fn next_wake(&self) -> Option<Instant> {
        self.jobs
            .iter()
            .filter(|j| {
                j.context.is_some() && matches!(j.kind, Kind::Workspace(_) | Kind::Snippet(_))
            })
            .map(|j| {
                j.deadline
                    .unwrap_or_else(|| Instant::now() + crate::deadline::POLL)
                    .min(Instant::now() + crate::deadline::POLL)
            })
            .min()
    }
    pub fn find_open(&self, predicate: impl Fn(&C) -> bool) -> Option<&C> {
        self.jobs
            .iter()
            .filter(|j| {
                matches!(j.kind, Kind::Workspace(_) | Kind::Snippet(_))
                    && matches!(
                        *j.result.lock().unwrap_or_else(|p| p.into_inner()),
                        Slot::Pending
                    )
            })
            .filter_map(|j| j.context.as_ref())
            .find(|c| predicate(c))
    }
    pub fn retire_open_where(&mut self, predicate: impl Fn(&C) -> bool) -> Vec<Finished<C>> {
        self.retire_open_if(|job| job.context.as_ref().is_some_and(&predicate))
    }
    fn retire_open_if(&mut self, predicate: impl Fn(&Job<C>) -> bool) -> Vec<Finished<C>> {
        let mut output = vec![];
        for job in &mut self.jobs {
            if job.context.is_none()
                || !matches!(job.kind, Kind::Workspace(_) | Kind::Snippet(_))
                || !predicate(job)
            {
                continue;
            }
            let fallback = job.kind.fallback(); // Linearized dispatch revocation.
            job.stop.store(true, Ordering::Release);
            let mut slot = job.result.lock().unwrap_or_else(|p| p.into_inner());
            let result = match std::mem::replace(&mut *slot, Slot::Retired) {
                Slot::Ready(result) => result,
                _ => fallback,
            };
            output.push(Finished {
                id: job.id,
                context: job.context.take().expect("undelivered context"),
                result,
            });
        }
        output
    }
    pub fn cancel_all(&self) {
        for job in &self.jobs {
            if let Kind::Workspace(dispatch) = &job.kind {
                let _ = dispatch.retire();
            }
            if let Kind::Snippet(dispatch) = &job.kind {
                let _ = dispatch.retire();
            }
            job.stop.store(true, Ordering::Release);
        }
    }
    pub fn shutdown(&mut self) -> Vec<Finished<C>> {
        self.cancel_all();
        let mut output = vec![];
        for job in self.jobs.drain(..) {
            if job.kind.join_on_shutdown() || job.thread.is_finished() {
                let _ = job.thread.join();
            }
            let mut slot = job.result.lock().unwrap_or_else(|p| p.into_inner());
            let result = match std::mem::replace(&mut *slot, Slot::Retired) {
                Slot::Ready(result) => result,
                _ => job.kind.fallback(),
            };
            if let Some(context) = job.context {
                output.push(Finished {
                    id: job.id,
                    context,
                    result,
                });
            }
        }
        output
    }
}
impl<C> Drop for Manager<C> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

pub(crate) fn budget(command: &Command, input: Option<&str>) -> Result<(), String> {
    let mut size = 128usize;
    let strings = [
        Some(command.id.as_str()),
        Some(&command.title),
        Some(&command.kind),
        Some(&command.target),
        command.hotkey.as_deref(),
        command.input_prompt.as_deref(),
        command.keyword.as_deref(),
        command.application_id.as_deref(),
        command.snippet_output.as_deref(),
    ];
    for value in strings
        .into_iter()
        .flatten()
        .chain(command.aliases.iter().map(String::as_str))
        .chain(command.arguments.iter().map(|a| {
            if a == "{input}" {
                input.unwrap_or(a)
            } else if a == "{uuid}" && input.is_some() {
                "00000000-0000-0000-0000-000000000000"
            } else {
                a
            }
        }))
    {
        size = size
            .checked_add(value.len())
            .and_then(|s| s.checked_add(32))
            .filter(|s| *s <= isize::MAX as usize)
            .ok_or("Command data exceeds the supported size.")?;
    }
    Ok(())
}
/// Compute expansion size before allocating copies of potentially repeated input.
pub fn eligible(command: &Command) -> Result<(), String> {
    if command.enabled == Some(false) {
        return Err("Command is disabled.".into());
    }
    if command.kind != "script" {
        return Err("This command is unavailable in the development build.".into());
    }
    if command.id.starts_with("apple-shortcut.") {
        return Err("Result handling for this script is not available yet.".into());
    }
    Ok(())
}
pub fn prepare(command: &Command, values: &serde_json::Value) -> Result<Command, String> {
    expand(command, values["input"].as_str().unwrap_or(""), true)
}
/// Result selection/reload already owns the interaction. Empty literal values
/// and the empty reload input must not reopen the ordinary input prompt.
pub(crate) fn prepare_result(command: &Command, input: Option<&str>) -> Result<Command, String> {
    expand(command, input.unwrap_or(""), false)
}
fn expand(command: &Command, input: &str, require_input: bool) -> Result<Command, String> {
    eligible(command)?;
    if let Some(prompt) = command.input_prompt.as_ref().filter(|_| require_input) {
        if objc2::rc::autoreleasepool(|_| {
            use objc2_foundation::{NSCharacterSet, NSString};
            NSString::from_str(input).stringByTrimmingCharactersInSet(&NSCharacterSet::whitespaceAndNewlineCharacterSet()).is_empty()
        }) {
            return Err(format!("Enter {prompt}."));
        }
    }
    budget(command, Some(input))?;
    let mut ready = command.clone();
    for argument in &mut ready.arguments {
        match argument.as_str() {
            "{input}" => *argument = input.to_string(),
            "{uuid}" => {
                *argument = objc2::rc::autoreleasepool(|_| {
                    objc2_foundation::NSUUID::new().UUIDString().to_string()
                })
            }
            _ => {}
        }
    }
    Ok(ready)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{sync::mpsc, time::Duration};
    fn blocked(
        manager: &mut Manager<usize>,
        context: usize,
        live: Live,
        deadline: impl Into<Option<Instant>>,
    ) -> mpsc::Sender<()> {
        let (entered, ready) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::channel();
        manager
            .admit_job(
                false,
                live,
                deadline.into(),
                context,
                Kind::Workspace(Arc::new(crate::workspace::Dispatch::default())),
                false,
                move |_| {
                    entered.send(()).unwrap();
                    let _ = wait.recv_timeout(Duration::from_secs(5));
                    Err("Own late fixture result".into())
                },
            )
            .unwrap();
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        release
    }
    #[test]
    fn no_wall_deadline_keeps_context_until_cancel_without_immediate_spin() {
        let (tx, _) = mpsc::sync_channel(1);
        let mut manager = Manager::new(PathBuf::new(), tx);
        let alive = Arc::new(AtomicBool::new(true));
        let flag = alive.clone();
        let release = blocked(
            &mut manager,
            77,
            Arc::new(move || flag.load(Ordering::Acquire)),
            None,
        );
        let before = Instant::now();
        let wake = manager.next_wake().unwrap();
        assert!(wake >= before + crate::deadline::POLL);
        thread::sleep(Duration::from_millis(35));
        assert!(manager.harvest().is_empty());
        assert!(manager.jobs[0].deadline.is_none());
        alive.store(false, Ordering::Release);
        let result = manager.harvest();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].context, 77);
        assert!(manager.next_wake().is_none());
        release.send(()).unwrap();
    }
    #[test]
    fn owner_deadline_retires_context_without_waiting_for_blocked_work() {
        let (tx, _) = mpsc::sync_channel(1);
        let mut manager = Manager::new(PathBuf::from("/tmp"), tx);
        let deadline = Instant::now() + Duration::from_millis(100);
        let release = blocked(&mut manager, 9, Arc::new(|| true), deadline);
        assert!(manager.next_wake().is_some());
        thread::sleep(
            deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(2),
        );
        let result = manager.harvest();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].context, 9);
        assert!(matches!(&result[0].result,Ok(Outcome::Workspace(value)) if !value.committed));
        assert!(manager.next_wake().is_none());
        assert!(!manager.jobs[0].thread.is_finished());
        release.send(()).unwrap();
    }
    #[test]
    fn selected_retirement_preserves_unrelated_open_context_and_blocks_redelivery() {
        let (tx, _) = mpsc::sync_channel(1);
        let mut manager = Manager::new(PathBuf::from("/tmp"), tx);
        let first = blocked(
            &mut manager,
            1,
            Arc::new(|| true),
            Instant::now() + Duration::from_secs(3),
        );
        let second = blocked(
            &mut manager,
            2,
            Arc::new(|| true),
            Instant::now() + Duration::from_secs(3),
        );
        let retired = manager.retire_open_where(|c| *c == 1);
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].context, 1);
        assert_eq!(manager.count(), 1);
        first.send(()).unwrap();
        second.send(()).unwrap();
        let end = Instant::now() + Duration::from_secs(2);
        while manager.jobs.iter().any(|j| !j.thread.is_finished()) {
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(1));
        }
        let completed = manager.harvest();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].context, 2);
        assert!(manager.harvest().is_empty());
    }
    #[test]
    fn owner_observes_disconnected_liveness_before_an_unexpired_deadline() {
        let (tx, _) = mpsc::sync_channel(1);
        let mut manager = Manager::new(PathBuf::from("/tmp"), tx);
        let live = Arc::new(AtomicBool::new(true));
        let check = live.clone();
        let release = blocked(
            &mut manager,
            3,
            Arc::new(move || check.load(Ordering::Acquire)),
            Instant::now() + Duration::from_secs(3),
        );
        live.store(false, Ordering::Release);
        let result = manager.harvest();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0].result,Ok(Outcome::Workspace(value)) if !value.committed));
        release.send(()).unwrap();
    }
    #[test]
    fn lifetime_reservations_bound_cli_and_preserve_one_native_slot() {
        let budget = Arc::new(Budget::default());
        let mut leases = vec![];
        for _ in 0..CLI_LIMIT {
            leases.push(budget.reserve(true).unwrap());
        }
        assert!(budget.reserve(true).is_none());
        let native = budget.reserve(false).unwrap();
        assert!(budget.reserve(false).is_none());
        assert_eq!(budget.total.load(Ordering::Acquire), LIMIT);
        leases.pop();
        let replacement = budget.reserve(true).unwrap();
        drop(leases);
        drop(replacement);
        drop(native);
        assert_eq!(budget.total.load(Ordering::Acquire), 0);
        assert_eq!(budget.cli.load(Ordering::Acquire), 0);
    }
    fn command() -> Command {
        serde_json::from_value(
            json!({"id":"fixture","title":"Fixture","kind":"script","target":"/usr/bin/true"}),
        )
        .unwrap()
    }
    #[test]
    fn ready_slot_delivers_before_thread_exit_and_without_a_wake() {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.try_send(HostEvent::UiWake).unwrap();
        let mut manager = Manager::new(PathBuf::from("/tmp"), tx);
        let (release, wait) = mpsc::channel();
        let thread = thread::spawn(move || {
            let _ = wait.recv_timeout(Duration::from_secs(2));
        });
        manager.jobs.push(Job {
            id: 1,
            context: Some(7),
            kind: Kind::Script,
            deadline: None,
            live: Arc::new(|| true),
            stop: Arc::new(AtomicBool::new(false)),
            result: Arc::new(Mutex::new(Slot::Ready(Err("fixture result".into())))),
            thread,
        });
        let start = Instant::now();
        let completed = manager.harvest();
        assert!(start.elapsed() < Duration::from_millis(100));
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].context, 7);
        assert!(!manager.jobs[0].thread.is_finished());
        assert!(manager.harvest().is_empty());
        assert_eq!(manager.count(), 0);
        assert!(rx.try_recv().is_ok());
        release.send(()).unwrap();
    }
    #[test]
    fn expansion_is_literal_and_not_restricted_by_socket_packet_size() {
        let mut command = command();
        command.arguments = vec![
            "{input}".into(),
            "prefix{input}".into(),
            "{uuid}".into(),
            "{uuid}".into(),
        ];
        let ready = prepare(&command, &json!({"input":"$(literal)\nΩ"})).unwrap();
        assert_eq!(&ready.arguments[..2], &["$(literal)\nΩ", "prefix{input}"]);
        assert_eq!(ready.arguments[2].len(), 36);
        assert_ne!(ready.arguments[2], ready.arguments[3]);
        command.arguments = vec!["{input}".into(); 32];
        let ready = prepare(&command, &json!({"input":"x".repeat(65536)})).unwrap();
        assert_eq!(ready.arguments.len(), 32);
        assert!(ready.arguments.iter().all(|s| s.len() == 65536));
        command.input_prompt = Some("Text".into());
        assert!(prepare(&command, &json!({"input":" \n"})).is_err());
        command.enabled = Some(false);
        assert!(prepare(&command, &json!({})).is_err());
    }
    #[test]
    fn actual_worker_panic_yields_terminal_result_and_capacity() {
        let (tx, rx) = mpsc::sync_channel(1);
        let mut manager = Manager::new(PathBuf::from("/tmp"), tx);
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let live = Arc::new(move || {
            if count.fetch_add(1, Ordering::AcqRel) > 0 {
                panic!("fixture worker panic");
            }
            true
        });
        manager.admit(command(), true, live, None, ()).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let result = manager.harvest();
        assert_eq!(result.len(), 1);
        assert!(result[0].result.is_err());
        assert_eq!(manager.count(), 0);
    }
}
