//! Foundation Task/Pipe adapter state. This is provider simulation, not a second
//! production business-logic implementation. Launch rules/FDs stay typed/owned.
use super::{
    owned::{AliveLease, Result},
    process_dispatch::{FileAction, JobLease, PublishedJob, Registry},
};
use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Launch,
    Read,
    Write,
    Close,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FailureKind {
    Policy,
    Posix(i32),
    Unmapped,
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Plain immutable NSError payload data; this type grants no factory, receiver,
/// mapping qualification, execution or namespace authority.
pub struct ErrorFields {
    pub(crate) domain: String,
    pub(crate) code: isize,
    pub(crate) description: String,
    pub(crate) user_info: std::collections::BTreeMap<String, ErrorInfo>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorInfo {
    Text(String),
    Underlying(Box<ErrorFields>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) operation: Operation,
    pub(crate) origin: &'static str,
    pub(crate) kind: FailureKind,
    descriptor_access: Option<i32>,
    description: String,
}
impl Failure {
    pub(crate) fn policy(
        operation: Operation,
        origin: &'static str,
        description: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            origin,
            kind: FailureKind::Policy,
            descriptor_access: None,
            description: description.into(),
        }
    }
    fn unmapped(
        operation: Operation,
        origin: &'static str,
        description: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            origin,
            kind: FailureKind::Unmapped,
            descriptor_access: None,
            description: description.into(),
        }
    }
    fn io(
        operation: Operation,
        origin: &'static str,
        error: io::Error,
        description: &str,
        descriptor_access: Option<i32>,
    ) -> Self {
        Self {
            operation,
            origin,
            kind: error
                .raw_os_error()
                .map(FailureKind::Posix)
                .unwrap_or(FailureKind::Unmapped),
            descriptor_access,
            description: description.into(),
        }
    }
    fn from_launch(error: super::process_dispatch::LaunchFailure) -> Self {
        use super::process_dispatch::LaunchFailure;
        match error {
            LaunchFailure::Policy(description) => {
                Self::policy(Operation::Launch, "registry.admission", description)
            }
            LaunchFailure::Unmapped(description) => {
                Self::unmapped(Operation::Launch, "registry.setup", description)
            }
            LaunchFailure::Posix {
                origin,
                errno,
                description,
            } => Self {
                operation: Operation::Launch,
                origin,
                kind: FailureKind::Posix(errno),
                descriptor_access: None,
                description,
            },
        }
    }
    pub(crate) fn description(self) -> String {
        self.description
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn assembly_launch_stage(&self) -> &'static str {
        match self.description.as_str() {
            "Reference process-table Task route differs before dispatch" => "monitor command differs",
            "Reference process-table Task locale proof absent" => "monitor locale proof absent",
            "Reference file search Task launch and timeout reservation differ" => "file search reservation differs",
            "owned handle closed" => "stdio handle closed",
            "Task handle belongs to another process registry" => "stdio handle issuer differs",
            "ungranted process descriptor; no inherited capability" => "stdio descriptor ungranted",
            "stdio generation changed during admission" => "stdio generation changed",
            "own kernel identity watch not installed" => "child kernel watch refused",
            "own watch creation failed" => "child kernel watch unavailable",
            _ => self.origin,
        }
    }
    /// Exact once-observed wrong-direction cases only. No generic errno or
    /// policy-message promotion; other original mappings remain unqualified.
    pub(crate) fn ns_error_fields(&self) -> Result<ErrorFields> {
        let observed = match (
            self.operation,
            self.origin,
            &self.kind,
            self.descriptor_access,
        ) {
            (Operation::Read, "read", FailureKind::Posix(libc::EBADF), Some(libc::O_WRONLY)) => {
                Some((256, "The file couldn’t be opened."))
            }
            (Operation::Write, "write", FailureKind::Posix(libc::EBADF), Some(libc::O_RDONLY)) => {
                Some((512, "The file couldn’t be saved."))
            }
            _ => None,
        };
        if let Some((code, description)) = observed {
            let underlying = ErrorFields {
                domain: "NSPOSIXErrorDomain".into(),
                code: 9,
                description: "The operation couldn’t be completed. Bad file descriptor".into(),
                user_info: std::collections::BTreeMap::new(),
            };
            return Ok(ErrorFields {
                domain: "NSCocoaErrorDomain".into(),
                code,
                description: description.into(),
                user_info: [(
                    "NSUnderlyingError".into(),
                    ErrorInfo::Underlying(Box::new(underlying)),
                )]
                .into(),
            });
        }
        Err(format!(
            "unqualified Foundation NSError mapping for {:?} at {} ({:?})",
            self.operation, self.origin, self.kind
        ))
    }
}
type TaskResult<T> = std::result::Result<T, Failure>;
#[derive(Clone)]
pub struct Handle {
    inner: Arc<Mutex<Option<Arc<OwnedFd>>>>,
    registry: Arc<Registry>,
}
impl Handle {
    fn new(fd: OwnedFd, registry: &Arc<Registry>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(Arc::new(fd)))),
            registry: registry.clone(),
        }
    }
    pub fn null(registry: &Arc<Registry>) -> Result<Self> {
        Ok(Self::new(registry.null(libc::O_RDWR)?, registry))
    }
    /// Identity remains available after logical close or business retirement;
    /// this alone grants no new I/O and never renews cleanup authority.
    pub(crate) fn belongs_to_registry(&self, registry: &Registry) -> bool {
        std::ptr::eq(self.registry.as_ref(), registry)
    }
    pub(crate) fn same_handle(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner) && Arc::ptr_eq(&self.registry, &other.registry)
    }
    pub(crate) fn verify_registry(&self, registry: &Registry) -> Result<()> {
        if !self.belongs_to_registry(registry) {
            return Err("Task handle belongs to another process registry".into());
        }
        registry.before_effect()
    }
    pub fn loan(&self) -> Result<Arc<OwnedFd>> {
        self.inner
            .lock()
            .map_err(|_| "handle poisoned")?
            .clone()
            .ok_or_else(|| "owned handle closed".into())
    }
    pub fn close(&self) -> Result<()> {
        self.close_failure().map_err(Failure::description)
    }
    pub(crate) fn close_failure(&self) -> TaskResult<()> {
        self.inner
            .lock()
            .map_err(|_| Failure::unmapped(Operation::Close, "handle.state", "handle poisoned"))?
            .take();
        Ok(())
    }
    pub fn read(&self, limit: usize, life: &dyn AliveLease) -> Result<Vec<u8>> {
        self.read_failure(limit, life).map_err(Failure::description)
    }
    pub(crate) fn read_failure(&self, limit: usize, life: &dyn AliveLease) -> TaskResult<Vec<u8>> {
        life.before_effect()
            .map_err(|e| Failure::policy(Operation::Read, "caller.guardian", e))?;
        self.registry
            .before_effect()
            .map_err(|e| Failure::policy(Operation::Read, "registry.admission", e))?;
        if limit > 32 * 1024 * 1024 {
            return Err(Failure::policy(
                Operation::Read,
                "handle.bounds",
                "owned read limit",
            ));
        }
        let fd = self
            .loan()
            .map_err(|e| Failure::policy(Operation::Read, "handle.loan", e))?;
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        let access = (flags >= 0).then_some(flags & libc::O_ACCMODE);
        let mut bytes = vec![0; limit];
        loop {
            let n = unsafe { libc::read(fd.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
            if n >= 0 {
                bytes.truncate(n as usize);
                return Ok(bytes);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(Failure::io(
                    Operation::Read,
                    "read",
                    error,
                    "owned pipe read failed",
                    access,
                ));
            }
            life.before_effect()
                .map_err(|e| Failure::policy(Operation::Read, "caller.guardian", e))?;
            self.registry
                .before_effect()
                .map_err(|e| Failure::policy(Operation::Read, "registry.admission", e))?;
        }
    }
    pub fn read_to_end(&self, life: &dyn AliveLease) -> Result<Vec<u8>> {
        let mut out = vec![];
        loop {
            let next = self.read(16384, life)?;
            if next.is_empty() {
                return Ok(out);
            }
            if out.len() + next.len() > 32 * 1024 * 1024 {
                return Err("owned pipe output limit".into());
            }
            out.extend(next);
        }
    }
    pub fn write(&self, bytes: &[u8], life: &dyn AliveLease) -> Result<()> {
        self.write_failure(bytes, life)
            .map_err(Failure::description)
    }
    pub(crate) fn write_failure(&self, bytes: &[u8], life: &dyn AliveLease) -> TaskResult<()> {
        self.write_checked(bytes, life, || Ok(()))
    }

    pub(crate) fn write_checked(
        &self, bytes: &[u8], life: &dyn AliveLease, check: impl Fn() -> Result<()>,
    ) -> TaskResult<()> {
        life.before_effect()
            .map_err(|e| Failure::policy(Operation::Write, "caller.guardian", e))?;
        self.registry
            .before_effect()
            .map_err(|e| Failure::policy(Operation::Write, "registry.admission", e))?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(Failure::policy(
                Operation::Write,
                "handle.bounds",
                "owned pipe input limit",
            ));
        }
        let fd = self
            .loan()
            .map_err(|e| Failure::policy(Operation::Write, "handle.loan", e))?;
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        let access = (flags >= 0).then_some(flags & libc::O_ACCMODE);
        let mut rest = bytes;
        while !rest.is_empty() {
            life.before_effect()
                .map_err(|e| Failure::policy(Operation::Write, "caller.guardian", e))?;
            self.registry
                .before_effect()
                .map_err(|e| Failure::policy(Operation::Write, "registry.admission", e))?;
            check().map_err(|e| Failure::policy(Operation::Write, "caller.retirement", e))?;
            let n = unsafe { libc::write(fd.as_raw_fd(), rest.as_ptr().cast(), rest.len()) };
            if n > 0 {
                rest = &rest[n as usize..];
            } else if n == 0 {
                return Err(Failure::unmapped(
                    Operation::Write,
                    "write.zero",
                    "owned pipe write failed",
                ));
            } else {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(Failure::io(
                        Operation::Write,
                        "write",
                        error,
                        "owned pipe write failed",
                        access,
                    ));
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Pipe {
    pub read: Handle,
    pub write: Handle,
}
impl Pipe {
    pub fn new(registry: &Arc<Registry>) -> Result<Self> {
        Self::with_parent_reader(registry, false)
    }
    pub(crate) fn with_parent_reader(registry: &Arc<Registry>, parent_reader: bool) -> Result<Self> {
        let (read, write) = registry.task_pipe(parent_reader)?;
        Ok(Self {
            read: Handle::new(read, registry),
            write: Handle::new(write, registry),
        })
    }
    pub(crate) fn verify_registry(&self, registry: &Registry) -> Result<()> {
        self.read.verify_registry(registry)?;
        self.write.verify_registry(registry)
    }
}
#[derive(Clone)]
pub enum Stream {
    Null,
    Handle(Handle),
    Pipe(Pipe),
}
impl Stream {
    fn close_child_end(&self, index: i32) -> Result<()> {
        if let Self::Pipe(pipe) = self {
            if index == 0 {
                pipe.read.close()?;
            } else {
                pipe.write.close()?;
            }
        }
        Ok(())
    }
    fn prepare(
        &self,
        index: i32,
        registry: &Registry,
    ) -> Result<(FileAction, Option<Arc<OwnedFd>>)> {
        match self {
            Self::Null => Ok((
                FileAction::Null {
                    fd: index,
                    write: index != 0,
                },
                None,
            )),
            Self::Handle(handle) => {
                handle.verify_registry(registry)?;
                let loan = handle.loan()?;
                Ok((
                    FileAction::Dup {
                        from: loan.as_raw_fd(),
                        to: index,
                    },
                    Some(loan),
                ))
            }
            Self::Pipe(pipe) => {
                pipe.verify_registry(registry)?;
                let handle = if index == 0 { &pipe.read } else { &pipe.write };
                let loan = handle.loan()?;
                Ok((
                    FileAction::Dup {
                        from: loan.as_raw_fd(),
                        to: index,
                    },
                    Some(loan),
                ))
            }
        }
    }
}
#[derive(Clone)]
pub struct Config {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) monitor_locale_verified: bool,
    pub input: Stream,
    pub output: Stream,
    pub error: Stream,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            executable: PathBuf::new(),
            arguments: vec![],
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            monitor_locale_verified: false,
            input: Stream::Null,
            output: Stream::Null,
            error: Stream::Null,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimeoutLaunchRoute {
    Monitor,
    Files,
}

#[derive(Default)]
struct Completion {
    pid: Option<i32>,
    status: Option<i32>,
    launching: bool,
    lease: Option<JobLease>,
    wait_error: bool,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    timeout_route: Option<TimeoutLaunchRoute>,
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    monitor_stdout: Option<std::sync::Weak<Mutex<Option<Arc<OwnedFd>>>>>,
}
pub struct Task {
    pub config: Mutex<Config>,
    completion: Mutex<Completion>,
    changed: Condvar,
    life: Arc<dyn AliveLease>,
}
struct LaunchingReset<'a> {
    task: &'a Task,
    armed: bool,
}
impl Drop for LaunchingReset<'_> {
    fn drop(&mut self) {
        if self.armed {
            let mut state = self
                .task
                .completion
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.launching = false;
            self.task.changed.notify_all();
        }
    }
}
struct UnpublishedTask {
    pid: i32,
    lease: JobLease,
    registered: bool,
    armed: bool,
}
impl Drop for UnpublishedTask {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.lease.cancel();
        let mut status = 0;
        loop {
            let result = unsafe {
                if self.registered {
                    super::process_abi::waitpid(self.pid, &mut status, 0)
                } else {
                    libc::waitpid(self.pid, &mut status, 0)
                }
            };
            if result == self.pid {
                break;
            }
            if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            super::process_abi::record_task_wait_failure();
            break;
        }
    }
}
impl Task {
    pub fn new(life: Arc<dyn AliveLease>) -> Result<Arc<Self>> {
        life.before_effect()?;
        Ok(Arc::new(Self {
            config: Mutex::new(Config::default()),
            completion: Mutex::new(Completion::default()),
            changed: Condvar::new(),
            life,
        }))
    }
    fn begin_launch(&self) -> Result<LaunchingReset<'_>> {
        let mut completion = self.completion.lock().map_err(|_| "task poisoned")?;
        if completion.launching || completion.pid.is_some() {
            return Err("owned task can launch only once".into());
        }
        completion.launching = true;
        drop(completion);
        Ok(LaunchingReset {
            task: self,
            armed: true,
        })
    }
    pub fn launch(self: &Arc<Self>, registry: &Registry) -> Result<()> {
        self.launch_failure(registry, None).map_err(Failure::description)
    }
    pub(crate) fn launch_failure(
        self: &Arc<Self>, registry: &Registry, timeout_route: Option<TimeoutLaunchRoute>,
    ) -> TaskResult<()> {
        self.life
            .before_effect()
            .map_err(|e| Failure::policy(Operation::Launch, "caller.guardian", e))?;
        if !Arc::ptr_eq(
            &self.life,
            &super::process_abi::installed_context()
                .map_err(|e| Failure::policy(Operation::Launch, "registry.context", e))?
                .life,
        ) {
            return Err(Failure::policy(
                Operation::Launch,
                "task.issuer",
                "Task guardian differs from installed process owner",
            ));
        }
        super::process_abi::verify_registry(registry)
            .map_err(|e| Failure::policy(Operation::Launch, "registry.issuer", e))?;
        let job_launch = Arc::new(super::process_abi::reserve_job_launch()
            .map_err(|e| Failure::policy(Operation::Launch, "registry.launch", e))?);
        let mut launching = self
            .begin_launch()
            .map_err(|e| Failure::policy(Operation::Launch, "task.reservation", e))?;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        let mut monitor_stdout = None;
        let result: std::result::Result<_, super::process_dispatch::LaunchFailure> = (|| {
            // Registry/source guards may reenter a Task property. Hold only a
            // pure copied snapshot, never either Task mutex across dispatch.
            let config = self
                .config
                .lock()
                .map_err(|_| "task config poisoned")?
                .clone();
            let monitor_route = super::process_router::is_reference_monitor_command(
                &config.executable, &config.arguments,
            );
            let require_monitor_route = timeout_route == Some(TimeoutLaunchRoute::Monitor);
            if require_monitor_route && !monitor_route {
                return Err("Reference process-table Task route differs before dispatch".into());
            }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            if monitor_route {
                if !require_monitor_route {
                    return Err("Reference process-table Task launch has no caller admission".into());
                }
                if !config.monitor_locale_verified {
                    return Err("Reference process-table Task locale proof absent".into());
                }
                let Stream::Pipe(pipe) = &config.output else {
                    return Err("Reference process-table Task requires a stdout pipe".into());
                };
                monitor_stdout = Some(Arc::downgrade(&pipe.read.inner));
            }
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            if Self::file_search_config(&config) != (timeout_route == Some(TimeoutLaunchRoute::Files)) {
                return Err("Reference file search Task launch and timeout reservation differ".into());
            }
            let prepared = [
                config.input.prepare(0, registry)?,
                config.output.prepare(1, registry)?,
                config.error.prepare(2, registry)?,
            ];
            let actions = prepared.iter().map(|v| v.0.clone()).collect::<Vec<_>>();
            #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
            let discovery_stop = if config.executable == std::path::Path::new("/usr/bin/shortcuts")
                && config.arguments == ["list", "--show-identifiers"]
            {
                let (admission, stopped) = super::reference_runtime::worker_dispatch_admission()?;
                if !crate::workspace::callbacks::reference_live_search_dispatch_active(&admission) {
                    return Err("Reference Shortcuts discovery launch has no active search turn".into());
                }
                Some(stopped)
            } else {
                None
            };
            #[cfg(not(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")))]
            let discovery_stop = None;
            let job = registry.dispatch_task(
                &config.executable, &config.arguments, &actions, require_monitor_route,
                discovery_stop,
            )?;
            let mut ownership = UnpublishedTask {
                pid: job.pid,
                lease: job.lease.clone(),
                registered: false,
                armed: true,
            };
            // Foundation relinquishes the parent's child-side pipe ends after
            // launch. Keeping these writers open would make read-to-EOF hang.
            let closed = config
                .input
                .close_child_end(0)
                .and_then(|_| config.output.close_child_end(1))
                .and_then(|_| config.error.close_child_end(2))
                .and_then(|_| super::process_abi::register_owned_task(registry, &job));
            closed?;
            ownership.registered = true;
            Ok((job, ownership))
        })();
        let (PublishedJob { pid, lease }, mut ownership) =
            result.map_err(Failure::from_launch)?;
        let mut completion = self
            .completion
            .lock()
            .map_err(|_| Failure::unmapped(Operation::Launch, "task.state", "task poisoned"))?;
        completion.pid = Some(pid);
        completion.lease = Some(lease.clone());
        let this = self.clone();
        let wait_reservation = job_launch.clone();
        // Exactly this task worker owns waitpid after successful dispatch. Foundation
        // callbacks only read/await the completion; no original NSTask runs or reaps.
        let started = thread::Builder::new()
            .name("f1-owned-task-wait".into())
            .spawn(move || {
                // Keep the existing drain reservation after waitpid removes
                // the child from the registry, through completion publication.
                let _wait_reservation = wait_reservation;
                let mut status = 0;
                let failed = loop {
                    let n = unsafe { super::process_abi::waitpid(pid, &mut status, 0) };
                    if n == pid {
                        break false;
                    }
                    if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    break true;
                };
                #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
                if failed {
                    super::process_abi::record_task_wait_failure();
                }
                let mut state = this.completion.lock().unwrap_or_else(|e| e.into_inner());
                state.status = (!failed).then_some(status);
                state.wait_error = failed;
                state.launching = false;
                this.changed.notify_all();
            });
        if let Err(e) = started {
            drop(completion);
            drop(ownership);
            let mut state = self
                .completion
                .lock()
                .map_err(|_| Failure::unmapped(Operation::Launch, "task.state", "task poisoned"))?;
            state.wait_error = true;
            state.launching = false;
            let description = e.to_string();
            return Err(Failure::io(
                Operation::Launch,
                "thread.spawn.waiter",
                e,
                &description,
                None,
            ));
        }
        ownership.armed = false;
        #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
        {
            completion.timeout_route = timeout_route;
            completion.monitor_stdout = monitor_stdout;
        }
        completion.launching = false;
        launching.armed = false;
        Ok(())
    }
    pub fn terminate(&self) -> Result<()> {
        self.life.before_effect()?;
        if let Some(lease) = &self.completion.lock().map_err(|_| "task poisoned")?.lease {
            lease.cancel();
        }
        Ok(())
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    fn file_search_config(config: &Config) -> bool {
        config.executable == std::path::Path::new("/usr/bin/mdfind")
            && config.arguments == super::process_router::reference_file_search_args()
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn is_reference_file_search(&self) -> Result<bool> {
        let config = self.config.lock().map_err(|_| "Task config poisoned")?;
        Ok(Self::file_search_config(&config))
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_timeout_launch(&self, route: TimeoutLaunchRoute) -> Result<()> {
        self.life.before_effect()?;
        let completed = self.completion.lock().map_err(|_| "Reference monitor Task state poisoned")?;
        if completed.timeout_route != Some(route) || completed.pid.is_none() || completed.wait_error {
            return Err("Reference timeout captured a different launched Task".into());
        }
        Ok(())
    }
    #[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
    pub(crate) fn verify_monitor_stdout(&self, reader: &Handle) -> Result<()> {
        self.life.before_effect()?;
        let completed = self.completion.lock().map_err(|_| "Reference monitor Task state poisoned")?;
        if completed.timeout_route != Some(TimeoutLaunchRoute::Monitor)
            || completed.pid.is_none()
            || completed.wait_error
            || !completed.monitor_stdout.as_ref().is_some_and(|stdout| {
                stdout.ptr_eq(&Arc::downgrade(&reader.inner))
            })
        {
            return Err("Reference monitor reader is not the launched Task's stdout pipe".into());
        }
        Ok(())
    }
    pub fn running(&self) -> Result<bool> {
        self.life.before_effect()?;
        let c = self.completion.lock().map_err(|_| "task poisoned")?;
        Ok(c.pid.is_some() && c.status.is_none() && !c.wait_error)
    }
    pub fn pid(&self) -> Result<i32> {
        self.life.before_effect()?;
        self.completion
            .lock()
            .map_err(|_| "task poisoned")?
            .pid
            .ok_or_else(|| "task has not launched".into())
    }
    pub fn wait(&self) -> Result<i32> {
        self.life.before_effect()?;
        let mut c = self.completion.lock().map_err(|_| "task poisoned")?;
        if c.pid.is_none() {
            return Err("task has not launched".into());
        }
        loop {
            if c.wait_error {
                return Err("owned task wait failed".into());
            }
            if let Some(status) = c.status {
                return Ok(status);
            }
            drop(c);
            self.life.before_effect()?;
            c = self.completion.lock().map_err(|_| "task poisoned")?;
            if c.wait_error || c.status.is_some() {
                continue;
            }
            c = self
                .changed
                .wait_timeout(c, std::time::Duration::from_millis(10))
                .map_err(|_| "task poisoned")?
                .0;
        }
    }
    pub fn status(&self) -> Result<(i32, i64)> {
        self.life.before_effect()?;
        let c = self.completion.lock().map_err(|_| "task poisoned")?;
        let raw = c.status.ok_or("task has not finished")?;
        if libc::WIFEXITED(raw) {
            Ok((libc::WEXITSTATUS(raw), 1))
        } else if libc::WIFSIGNALED(raw) {
            Ok((libc::WTERMSIG(raw), 2))
        } else {
            Err("unknown task completion".into())
        }
    }
}

#[cfg(test)]
mod module_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Life(AtomicBool);
    impl AliveLease for Life {
        fn is_alive(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
        fn before_effect(&self) -> Result<()> {
            if self.is_alive() {
                Ok(())
            } else {
                Err("own Task fixture retired".into())
            }
        }
    }

    #[test]
    fn task_fd_failures_capture_kernel_errno_without_reusing_it_for_policy() {
        let owner = Arc::new(Life(AtomicBool::new(true)));
        let life: Arc<dyn AliveLease> = owner.clone();
        let registry = Arc::new(
            Registry::new(
                life.clone(),
                super::super::descriptors::DescriptorTable::new(),
            )
            .unwrap(),
        );
        let pipe = Pipe::new(&registry).unwrap();
        let loan = pipe.read.loan().unwrap();
        let flags = unsafe { libc::fcntl(loan.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(loan.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        unsafe { *libc::__error() = libc::EACCES };
        let read = pipe.read.read_failure(1, life.as_ref()).unwrap_err();
        assert_eq!(read.operation, Operation::Read);
        assert_eq!(read.origin, "read");
        assert_eq!(read.kind, FailureKind::Posix(libc::EAGAIN));
        unsafe { *libc::__error() = libc::ENOENT };
        assert_eq!(read.kind, FailureKind::Posix(libc::EAGAIN));
        let write = pipe.read.write_failure(b"own", life.as_ref()).unwrap_err();
        assert_eq!(write.operation, Operation::Write);
        assert_eq!(write.origin, "write");
        assert_eq!(write.kind, FailureKind::Posix(libc::EBADF));
        let mapped = write.ns_error_fields().unwrap();
        assert_eq!(
            (mapped.domain.as_str(), mapped.code),
            ("NSCocoaErrorDomain", 512)
        );
        let ErrorInfo::Underlying(underlying) = &mapped.user_info["NSUnderlyingError"] else {
            panic!("original NSError must retain its actual underlying error object");
        };
        assert_eq!(
            (underlying.domain.as_str(), underlying.code),
            ("NSPOSIXErrorDomain", 9)
        );
        assert!(underlying.user_info.is_empty());
        assert!(read.ns_error_fields().is_err());
        owner.0.store(false, Ordering::Release);
        let denied = pipe.read.read_failure(1, life.as_ref()).unwrap_err();
        assert_eq!(denied.kind, FailureKind::Policy);
        assert!(denied.ns_error_fields().is_err());
        pipe.read.close_failure().unwrap();
        pipe.write.close_failure().unwrap();
    }

    #[test]
    fn task_launch_reservation_releases_mutexes_and_unwind_resets_without_a_child() {
        let life: Arc<dyn AliveLease> = Arc::new(Life(AtomicBool::new(true)));
        let task = Task::new(life).unwrap();
        let reservation = task.begin_launch().unwrap();
        assert!(task.config.try_lock().is_ok());
        assert!(task.completion.try_lock().unwrap().launching);
        assert!(task.begin_launch().is_err());
        assert!(!task.running().unwrap());
        drop(reservation);
        assert!(!task.completion.lock().unwrap().launching);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _reservation = task.begin_launch().unwrap();
            panic!("own launch-reservation unwind");
        }));
        assert!(result.is_err());
        assert!(!task.completion.lock().unwrap().launching);
        drop(task.begin_launch().unwrap());
    }

    #[test]
    fn task_owned_pipe_roundtrips_and_clones_share_logical_close() {
        let owner = Arc::new(Life(AtomicBool::new(true)));
        let life: Arc<dyn AliveLease> = owner.clone();
        let registry = Arc::new(
            Registry::new(
                life.clone(),
                super::super::descriptors::DescriptorTable::new(),
            )
            .unwrap(),
        );
        let pipe = Pipe::new(&registry).unwrap();
        let foreign = Registry::new(
            life.clone(),
            super::super::descriptors::DescriptorTable::new(),
        )
        .unwrap();
        assert!(pipe.verify_registry(&registry).is_ok());
        assert!(pipe.verify_registry(&foreign).is_err());
        assert!(Stream::Handle(pipe.read.clone())
            .prepare(0, &foreign)
            .is_err());
        assert!(Stream::Pipe(pipe.clone()).prepare(1, &foreign).is_err());
        let payload = "owned-λ".as_bytes();
        pipe.write.write(payload, life.as_ref()).unwrap();
        assert_eq!(
            pipe.read.read(payload.len(), life.as_ref()).unwrap(),
            payload
        );
        assert!(pipe.read.read(33 * 1024 * 1024, life.as_ref()).is_err());
        let null = Handle::null(&registry).unwrap();
        assert!(null.read(1, life.as_ref()).unwrap().is_empty());
        null.write(b"owned-null", life.as_ref()).unwrap();
        null.close().unwrap();
        let alias = pipe.write.clone();
        let held = alias.loan().unwrap();
        let fd = held.as_raw_fd();
        pipe.write.close().unwrap();
        assert!(alias.loan().is_err());
        assert!(unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0);
        owner.0.store(false, Ordering::Release);
        assert!(pipe.read.verify_registry(&registry).is_err());
        assert!(alias.belongs_to_registry(&registry));
        assert!(!alias.belongs_to_registry(&foreign));
        assert!(pipe.read.read(1, life.as_ref()).is_err());
        // Last held ownership can close without renewed business admission.
        drop(held);
        assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
        pipe.read.close().unwrap();
        assert!(pipe.read.loan().is_err());
    }

    #[test]
    fn task_foreign_or_missing_process_authority_refuses_before_launch() {
        let owner = Arc::new(Life(AtomicBool::new(true)));
        let life: Arc<dyn AliveLease> = owner.clone();
        let registry = Registry::new(
            life.clone(),
            super::super::descriptors::DescriptorTable::new(),
        )
        .unwrap();
        let task = Task::new(life).unwrap();
        task.config.lock().unwrap().executable = "/owned/unregistered-task".into();
        assert!(task.launch(&registry).is_err());
        let completion = task.completion.lock().unwrap();
        assert!(completion.pid.is_none());
        assert!(completion.lease.is_none());
        assert!(!completion.launching);
        drop(completion);
        assert!(task.pid().is_err());
        owner.0.store(false, Ordering::Release);
        assert!(Task::new(owner).is_err());
    }
}
