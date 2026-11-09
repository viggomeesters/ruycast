//! Physical execution is restricted to a source/artifact-pinned owned Rust fake.
//! A successful dispatch transfers the sole wait owner to the existing caller;
//! our kernel watcher owns only a lease writer and never waitpid/reaps it.
use super::descriptors::{DescriptorTable, StdioSnapshot};
use super::owned::{AliveLease, OwnedRoot, Result};
use crate::guardian;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::CString,
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
unsafe extern "C" {
    fn posix_spawn_file_actions_addinherit_np(
        a: *mut libc::posix_spawn_file_actions_t,
        fd: i32,
    ) -> i32;
}
pub(crate) enum LaunchFailure {
    Policy(String),
    Unmapped(String),
    Posix {
        origin: &'static str,
        errno: i32,
        description: String,
    },
}
impl LaunchFailure {
    fn description(&self) -> &str {
        match self {
            Self::Policy(value) | Self::Unmapped(value) => value,
            Self::Posix { description, .. } => description,
        }
    }
    fn io(origin: &'static str, error: io::Error) -> Self {
        match error.raw_os_error() {
            Some(errno) => Self::Posix {
                origin,
                errno,
                description: error.to_string(),
            },
            None => Self::Unmapped(error.to_string()),
        }
    }
}
impl From<String> for LaunchFailure {
    fn from(value: String) -> Self {
        Self::Unmapped(value)
    }
}
impl From<&str> for LaunchFailure {
    fn from(value: &str) -> Self {
        Self::Unmapped(value.into())
    }
}
fn cv(n: i32, origin: &'static str) -> std::result::Result<(), LaunchFailure> {
    if n == 0 {
        Ok(())
    } else {
        Err(LaunchFailure::io(origin, io::Error::from_raw_os_error(n)))
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest(root: &OwnedRoot, path: &Path) -> Result<(String, u64, u64)> {
    let fd = root.checked(path)?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } != 0 {
        return Err("own fake fstat failed".into());
    }
    if st.st_mode & libc::S_IFMT != libc::S_IFREG
        || st.st_mode & 0o022 != 0
        || st.st_nlink != 1
        || st.st_size < 0
        || st.st_size > 32 * 1024 * 1024
    {
        return Err("unqualified own fake file".into());
    }
    let mut bytes = vec![];
    std::fs::File::from(fd)
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as i64 != st.st_size {
        return Err("own fake changed while hashing".into());
    }
    Ok((hash(&bytes), st.st_dev as u64, st.st_ino))
}
#[derive(Clone)]
pub struct PinnedFake {
    root: OwnedRoot,
    path: PathBuf,
    artifact: (String, u64, u64),
    source_path: PathBuf,
    source: (String, u64, u64),
}
impl PinnedFake {
    /// Source hash must be the integrator's hardcoded own Rust fixture source,
    /// never a value accepted from a product profile, SDK receipt or environment.
    pub fn bind(
        root: OwnedRoot,
        path: PathBuf,
        artifact_sha: &str,
        source_path: PathBuf,
        source_sha: &str,
    ) -> Result<Self> {
        let artifact = digest(&root, &path)?;
        let source = digest(&root, &source_path)?;
        if artifact.0 != artifact_sha || source.0 != source_sha {
            return Err("own fake source/artifact pin differs".into());
        }
        Ok(Self {
            root,
            path,
            artifact,
            source_path,
            source,
        })
    }
    pub(crate) fn verify(&self) -> Result<()> {
        if digest(&self.root, &self.path)? != self.artifact
            || digest(&self.root, &self.source_path)? != self.source
        {
            return Err("own fake source/artifact/identity changed".into());
        }
        Ok(())
    }
}
pub(crate) struct ExecPlan {
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub pin: Arc<PinnedFake>,
}
#[derive(Clone, Debug)]
pub enum FileAction {
    Dup { from: i32, to: i32 },
    Close(i32),
    Inherit(i32),
    Null { fd: i32, write: bool },
}
#[derive(Clone)]
pub struct Rule {
    pub fake: Arc<PinnedFake>,
    pub fake_args: Vec<String>,
}
#[derive(Clone, Default)]
pub struct Options {
    pub signal_mask: Option<libc::sigset_t>,
    pub signal_defaults: Option<libc::sigset_t>,
}
pub struct Registry {
    rules: BTreeMap<(Vec<u8>, Vec<Vec<u8>>), Rule>,
    life: Arc<dyn AliveLease>,
    descriptors: Arc<DescriptorTable>,
    retired: Arc<AtomicBool>,
}
struct DispatchLife {
    parent: Arc<dyn AliveLease>,
    retired: Arc<AtomicBool>,
}
fn process_live(parent: &dyn AliveLease, retired: &AtomicBool) -> Result<()> {
    if retired.load(Ordering::Acquire) {
        return Err("process module retired".into());
    }
    if let Err(error) = parent.before_effect() {
        retired.store(true, Ordering::Release);
        return Err(error);
    }
    let alive = parent.is_alive();
    if !alive {
        retired.store(true, Ordering::Release);
    }
    if retired.load(Ordering::Acquire) {
        Err("process module retired during guardian check".into())
    } else {
        Ok(())
    }
}
impl DispatchLife {
    fn not_retired(&self) -> Result<()> {
        if self.retired.load(Ordering::Acquire) {
            Err("process module retired".into())
        } else {
            Ok(())
        }
    }
}
impl AliveLease for DispatchLife {
    fn is_alive(&self) -> bool {
        self.before_effect().is_ok()
    }
    fn before_effect(&self) -> Result<()> {
        process_live(self.parent.as_ref(), &self.retired)
    }
}
impl Registry {
    pub fn new(life: Arc<dyn AliveLease>, descriptors: Arc<DescriptorTable>) -> Result<Self> {
        life.before_effect()?;
        Ok(Self {
            rules: BTreeMap::new(),
            life,
            descriptors,
            retired: Arc::new(AtomicBool::new(false)),
        })
    }
    pub(crate) fn before_effect(&self) -> Result<()> {
        process_live(self.life.as_ref(), &self.retired)
    }
    fn dispatch_life(&self) -> Arc<DispatchLife> {
        Arc::new(DispatchLife {
            parent: self.life.clone(),
            retired: self.retired.clone(),
        })
    }
    pub(crate) fn job_life(&self) -> Arc<dyn AliveLease> {
        self.dispatch_life()
    }
    pub(crate) fn not_retired(&self) -> Result<()> {
        if self.retired.load(Ordering::Acquire) {
            Err("process module retired".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }
    pub(crate) fn matches_issuer(
        &self,
        life: &Arc<dyn AliveLease>,
        descriptors: &Arc<DescriptorTable>,
    ) -> Result<()> {
        if !Arc::ptr_eq(&self.life, life) || !Arc::ptr_eq(&self.descriptors, descriptors) {
            return Err("process guardian/descriptor issuer differs".into());
        }
        self.before_effect()
    }
    /// The only public descriptor grant creates a new own pipe. Arbitrary raw
    /// FDs, inherited stdio and guardian writers cannot be granted by a receipt.
    pub fn pipe(&self) -> Result<(OwnedFd, OwnedFd)> {
        self.task_pipe(false)
    }
    pub(crate) fn task_pipe(&self, parent_reader: bool) -> Result<(OwnedFd, OwnedFd)> {
        self.before_effect()?;
        self.descriptors.process_pipe(self.life.clone(), parent_reader)
    }
    /// A source-selected null-device open is the only non-pipe grant. Flags
    /// cannot create, truncate, append or reach a caller-supplied pathname.
    pub fn null(&self, flags: i32) -> Result<OwnedFd> {
        self.before_effect()?;
        self.descriptors.null_for_process(flags, self.life.clone())
    }
    pub(crate) fn stdio_snapshot(&self) -> Result<StdioSnapshot> {
        self.before_effect()?;
        self.descriptors.stdio_snapshot()
    }
    pub(crate) fn prepare_exec(&self, path: &Path, args: &[String]) -> Result<ExecPlan> {
        self.before_effect()?;
        if cfg!(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))
            && super::process_router::is_reference_monitor_command(path, args)
        {
            return Err("Reference monitor command requires an admitted Task launch".into());
        }
        let key = (
            path.as_os_str().as_bytes().to_vec(),
            args.iter().map(|v| v.as_bytes().to_vec()).collect(),
        );
        let rule = self
            .rules
            .get(&key)
            .ok_or("unregistered fork exec; no fallback")?;
        rule.fake.verify()?;
        Ok(ExecPlan {
            program: rule.fake.path.clone(),
            arguments: rule.fake_args.clone(),
            pin: rule.fake.clone(),
        })
    }
    fn actions(&self, actions: &[FileAction], snapshot: &StdioSnapshot) -> Result<Vec<FileAction>> {
        if actions.len() > 256 {
            return Err("file action bounds".into());
        }
        let source = |fd: i32| -> Result<i32> {
            let row = snapshot
                .rows()
                .iter()
                .find(|row| row.fd == fd)
                .ok_or("ungranted process descriptor; no inherited capability")?;
            snapshot.pin_for(fd, row.generation)
        };
        let mut resolved = Vec::with_capacity(actions.len());
        for action in actions {
            match action {
                FileAction::Dup { from, to } if (0..=2).contains(to) => {
                    resolved.push(FileAction::Dup {
                        from: source(*from)?,
                        to: *to,
                    });
                }
                FileAction::Close(fd) if (0..=2).contains(fd) => resolved.push(action.clone()),
                FileAction::Close(fd) => {
                    source(*fd)?;
                    resolved.push(action.clone());
                }
                FileAction::Inherit(fd) => {
                    // Transfer the held OFD, never a possibly recycled original
                    // caller descriptor, into this child's requested slot.
                    resolved.push(FileAction::Dup {
                        from: source(*fd)?,
                        to: *fd,
                    });
                    resolved.push(FileAction::Inherit(*fd));
                }
                FileAction::Null { fd, .. } if (0..=2).contains(fd) => {
                    resolved.push(action.clone())
                }
                _ => return Err("unqualified descriptor action/target".into()),
            }
        }
        Ok(resolved)
    }
    pub fn register(&mut self, path: &Path, args: &[String], rule: Rule) -> Result<()> {
        self.before_effect()?;
        rule.fake.verify()?;
        if !path.is_absolute()
            || args.len() > 64
            || args.iter().any(|a| a.len() > 8192 || a.contains('\0'))
            || self.rules.len() >= 10000
        {
            return Err("invalid own fake route".into());
        }
        let key = (
            path.as_os_str().as_bytes().to_vec(),
            args.iter().map(|s| s.as_bytes().to_vec()).collect(),
        );
        if self.rules.contains_key(&key) {
            return Err("duplicate own fake route".into());
        }
        self.rules.insert(key, rule);
        Ok(())
    }
    pub fn dispatch(
        &self,
        path: &Path,
        args: &[String],
        actions: &[FileAction],
    ) -> Result<PublishedJob> {
        self.dispatch_with_options(path, args, actions, &Options::default())
    }
    pub fn dispatch_with_options(
        &self,
        path: &Path,
        args: &[String],
        actions: &[FileAction],
        options: &Options,
    ) -> Result<PublishedJob> {
        self.dispatch_inner(path, args, actions, options, false, None)
            .map_err(|error| error.description().to_owned())
    }
    pub(crate) fn dispatch_task(
        &self,
        path: &Path,
        args: &[String],
        actions: &[FileAction],
        monitor_admitted: bool,
        discovery_stop: Option<&AtomicBool>,
    ) -> std::result::Result<PublishedJob, LaunchFailure> {
        self.dispatch_inner(path, args, actions, &Options::default(), monitor_admitted, discovery_stop)
    }
    fn dispatch_inner(
        &self,
        path: &Path,
        args: &[String],
        actions: &[FileAction],
        options: &Options,
        monitor_admitted: bool,
        discovery_stop: Option<&AtomicBool>,
    ) -> std::result::Result<PublishedJob, LaunchFailure> {
        self.before_effect().map_err(LaunchFailure::Policy)?;
        super::process_abi::verify_new_job_admission().map_err(LaunchFailure::Policy)?;
        let key = (
            path.as_os_str().as_bytes().to_vec(),
            args.iter().map(|s| s.as_bytes().to_vec()).collect(),
        );
        let rule = self.rules.get(&key).ok_or_else(|| {
            LaunchFailure::Policy("unregistered executable/argv; no fallback".into())
        })?;
        rule.fake.verify().map_err(LaunchFailure::Policy)?;
        let snapshot = self.stdio_snapshot()?;
        let actions = self.actions(actions, &snapshot)?;
        let monitor_command = super::process_router::is_reference_monitor_command(path, args);
        let monitor_locale = monitor_command && rule.fake_args == ["monitor-ps"];
        if cfg!(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))
            && monitor_command
            && !(monitor_admitted && monitor_locale)
        {
            return Err(LaunchFailure::Policy(
                "Reference monitor command requires an admitted Task and its fixture".into(),
            ));
        }
        let job = launch(
            rule,
            &actions,
            self.dispatch_life(),
            options,
            &self.descriptors,
            &snapshot,
            monitor_locale,
            discovery_stop,
        )?;
        // Last transfer/admission has completed. No pipe-writer pin follows
        // the job lease or waits for process termination.
        drop(snapshot);
        Ok(job)
    }
}
struct Actions(libc::posix_spawn_file_actions_t);
impl Drop for Actions {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut self.0);
        }
    }
}
struct Attr(libc::posix_spawnattr_t);
impl Drop for Attr {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawnattr_destroy(&mut self.0);
        }
    }
}
// A not-yet-published child still belongs to this producer. On admission failure
// it is terminated and reaped here. On success disarm() transfers wait ownership.
struct Unpublished(Option<i32>);
impl Drop for Unpublished {
    fn drop(&mut self) {
        if let Some(pid) = self.0.take() {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
                let mut status = 0;
                while libc::waitpid(pid, &mut status, 0) < 0
                    && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                {
                }
            }
        }
    }
}
#[derive(Clone)]
pub struct JobLease {
    cancel: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}
impl JobLease {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}
impl Drop for JobLease {
    fn drop(&mut self) {
        if Arc::strong_count(&self.cancel) == 2 {
            self.cancel();
        }
    }
}
pub struct PublishedJob {
    pub pid: i32,
    pub lease: JobLease,
}
fn event(ident: usize) -> libc::kevent {
    libc::kevent {
        ident,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT | libc::EV_RECEIPT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}
fn launch(
    rule: &Rule,
    actions: &[FileAction],
    parent: Arc<DispatchLife>,
    options: &Options,
    descriptors: &Arc<DescriptorTable>,
    snapshot: &StdioSnapshot,
    monitor_locale: bool,
    discovery_stop: Option<&AtomicBool>,
) -> std::result::Result<PublishedJob, LaunchFailure> {
    parent.before_effect().map_err(LaunchFailure::Policy)?;
    // An admitted Inherit expands to pinned dup2 plus inherit. The public
    // request remains bounded at 256; only this private resolved form can double.
    if actions.len() > 512 {
        return Err("file action bounds".into());
    }
    let (life_r, life_w) =
        guardian::pipe().map_err(|e| LaunchFailure::io("guardian.life_pipe", e))?;
    let (admit_r, admit_w) =
        guardian::pipe().map_err(|e| LaunchFailure::io("guardian.admission_pipe", e))?;
    if unsafe { libc::fcntl(admit_w.as_raw_fd(), 73, 1) } != 0 {
        let error = io::Error::last_os_error();
        return Err(match error.raw_os_error() {
            Some(errno) => LaunchFailure::Posix {
                origin: "fcntl.F_SETNOSIGPIPE",
                errno,
                description: "own admission no-SIGPIPE setup failed".into(),
            },
            None => LaunchFailure::Unmapped("own admission no-SIGPIPE setup failed".into()),
        });
    }
    let reserved = [
        life_r.as_raw_fd(),
        life_w.as_raw_fd(),
        admit_r.as_raw_fd(),
        admit_w.as_raw_fd(),
    ];
    if reserved.iter().any(|fd| *fd <= 2) {
        return Err("job capabilities must not alias stdio".into());
    }
    let mut a = std::ptr::null_mut();
    cv(
        unsafe { libc::posix_spawn_file_actions_init(&mut a) },
        "posix_spawn_file_actions_init",
    )?;
    let mut a = Actions(a);
    // The same central issuer creates even default child null stdio. Keep its
    // parent handles only through spawn/admission, never through job completion.
    let mut nulls = Vec::new();
    for target in 0..=2 {
        let fd = descriptors.null_for_process(
            if target == 0 {
                libc::O_RDONLY
            } else {
                libc::O_WRONLY
            },
            parent.clone(),
        )?;
        cv(
            unsafe { libc::posix_spawn_file_actions_adddup2(&mut a.0, fd.as_raw_fd(), target) },
            "posix_spawn_file_actions_adddup2",
        )?;
        nulls.push(fd);
    }
    for action in actions {
        let fds = match action {
            FileAction::Dup { from, to } => vec![*from, *to],
            FileAction::Close(fd) | FileAction::Inherit(fd) | FileAction::Null { fd, .. } => {
                vec![*fd]
            }
        };
        if fds.iter().any(|fd| *fd < 0 || reserved.contains(fd)) {
            return Err("file action aliases private lease capability".into());
        }
        match action {
            FileAction::Dup { from, to } => cv(
                unsafe { libc::posix_spawn_file_actions_adddup2(&mut a.0, *from, *to) },
                "posix_spawn_file_actions_adddup2",
            )?,
            FileAction::Close(fd) => cv(
                unsafe { libc::posix_spawn_file_actions_addclose(&mut a.0, *fd) },
                "posix_spawn_file_actions_addclose",
            )?,
            FileAction::Inherit(fd) => cv(
                unsafe { posix_spawn_file_actions_addinherit_np(&mut a.0, *fd) },
                "posix_spawn_file_actions_addinherit_np",
            )?,
            FileAction::Null { fd, write } => {
                let source = descriptors.null_for_process(
                    if *write {
                        libc::O_WRONLY
                    } else {
                        libc::O_RDONLY
                    },
                    parent.clone(),
                )?;
                cv(
                    unsafe {
                        libc::posix_spawn_file_actions_adddup2(&mut a.0, source.as_raw_fd(), *fd)
                    },
                    "posix_spawn_file_actions_adddup2",
                )?;
                nulls.push(source);
            }
        }
    }
    for fd in [life_r.as_raw_fd(), admit_r.as_raw_fd()] {
        cv(
            unsafe { posix_spawn_file_actions_addinherit_np(&mut a.0, fd) },
            "posix_spawn_file_actions_addinherit_np",
        )?;
    }
    let mut attr = std::ptr::null_mut();
    cv(
        unsafe { libc::posix_spawnattr_init(&mut attr) },
        "posix_spawnattr_init",
    )?;
    let mut attr = Attr(attr);
    cv(
        unsafe { libc::posix_spawnattr_setpgroup(&mut attr.0, 0) },
        "posix_spawnattr_setpgroup",
    )?;
    let mut flags = libc::POSIX_SPAWN_SETPGROUP | libc::POSIX_SPAWN_CLOEXEC_DEFAULT;
    if let Some(mask) = &options.signal_mask {
        cv(
            unsafe { libc::posix_spawnattr_setsigmask(&mut attr.0, mask) },
            "posix_spawnattr_setsigmask",
        )?;
        flags |= libc::POSIX_SPAWN_SETSIGMASK;
    }
    if let Some(defaults) = &options.signal_defaults {
        cv(
            unsafe { libc::posix_spawnattr_setsigdefault(&mut attr.0, defaults) },
            "posix_spawnattr_setsigdefault",
        )?;
        flags |= libc::POSIX_SPAWN_SETSIGDEF;
    }
    cv(
        unsafe { libc::posix_spawnattr_setflags(&mut attr.0, flags as i16) },
        "posix_spawnattr_setflags",
    )?;
    let program = CString::new(rule.fake.path.as_os_str().as_bytes()).map_err(|_| "NULfake")?;
    let mut argv = vec![program.clone()];
    for arg in &rule.fake_args {
        argv.push(CString::new(arg.as_bytes()).map_err(|_| "NULfakearg")?);
    }
    let mut av = argv
        .iter()
        .map(|s| s.as_ptr().cast_mut())
        .collect::<Vec<_>>();
    av.push(std::ptr::null_mut());
    // Deliberately no inherited DYLD/provider/path/profile/account environment.
    let env = [
        CString::new(format!("F1_LIFE_FD={}", life_r.as_raw_fd())).unwrap(),
        CString::new(format!("F1_ADMIT_FD={}", admit_r.as_raw_fd())).unwrap(),
    ];
    let mut ev = env
        .iter()
        .map(|s| s.as_ptr().cast_mut())
        .collect::<Vec<_>>();
    // The frozen monitor requests a stable C locale. The only extra entry
    // is fixed by this exact registered fixture route, never by the host.
    if monitor_locale {
        ev.push(c"LC_ALL=C".as_ptr().cast_mut());
    }
    ev.push(std::ptr::null_mut());
    parent.before_effect().map_err(LaunchFailure::Policy)?;
    snapshot.verify().map_err(LaunchFailure::Policy)?;
    rule.fake.verify().map_err(LaunchFailure::Policy)?;
    parent.not_retired().map_err(LaunchFailure::Policy)?;
    super::process_abi::verify_new_job_admission().map_err(LaunchFailure::Policy)?;
    if discovery_stop.is_some_and(|stopped| stopped.load(Ordering::Acquire)) {
        return Err("Reference Shortcuts discovery stopped before spawn".into());
    }
    let mut pid = 0;
    cv(
        unsafe {
            libc::posix_spawn(
                &mut pid,
                program.as_ptr(),
                &a.0,
                &attr.0,
                av.as_ptr(),
                ev.as_ptr(),
            )
        },
        "posix_spawn",
    )?;
    if pid <= 1 {
        return Err("invalid own child identity".into());
    }
    let mut unpublished = Unpublished(Some(pid));
    drop(life_r);
    drop(admit_r);
    let lease = watch(pid, life_w, parent.clone())?;
    // The kernel has transferred the selected OFDs into this held child.
    // Parent logical close is valid now; only saved pin/life identity remains.
    snapshot
        .verify_transferred()
        .map_err(LaunchFailure::Policy)?;
    parent.before_effect().map_err(LaunchFailure::Policy)?;
    rule.fake.verify().map_err(LaunchFailure::Policy)?;
    parent.not_retired().map_err(LaunchFailure::Policy)?;
    if discovery_stop.is_some_and(|stopped| stopped.load(Ordering::Acquire)) {
        return Err("Reference Shortcuts discovery stopped before child admission".into());
    }
    let written = unsafe { libc::write(admit_w.as_raw_fd(), b"A".as_ptr().cast(), 1) };
    if written != 1 {
        let failure = if written < 0 {
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(errno) => LaunchFailure::Posix {
                    origin: "write.admission",
                    errno,
                    description: "own child admission write failed".into(),
                },
                None => LaunchFailure::Unmapped("own child admission write failed".into()),
            }
        } else {
            LaunchFailure::Unmapped("own child admission write failed".into())
        };
        return Err(failure);
    }
    drop(admit_w);
    drop(nulls);
    unpublished.0 = None;
    Ok(PublishedJob { pid, lease })
}
/// Test/caller helper: no wait ownership is taken here. The original consumer
/// remains responsible for exactly one waitpid/Child.wait after this returns.
pub fn wait_lease_settled(lease: &JobLease, budget: Duration) -> bool {
    let end = Instant::now() + budget;
    while !lease.finished() {
        if Instant::now() >= end {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
    true
}

pub(crate) fn watch(pid: i32, life_w: OwnedFd, parent: Arc<dyn AliveLease>) -> Result<JobLease> {
    let queue = unsafe { libc::kqueue() };
    if queue < 0 {
        return Err("own watch creation failed".into());
    }
    let queue = unsafe { OwnedFd::from_raw_fd(queue) };
    if unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err("own watch CLOEXEC failed".into());
    }
    let change = event(pid as usize);
    let mut receipt: libc::kevent = unsafe { std::mem::zeroed() };
    let instant = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let n = unsafe { libc::kevent(queue.as_raw_fd(), &change, 1, &mut receipt, 1, &instant) };
    if n != 1 || receipt.flags & libc::EV_ERROR == 0 || receipt.data != 0 {
        return Err("own kernel identity watch not installed".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let cancel_worker = cancel.clone();
    let done = finished.clone();
    thread::Builder::new()
        .name("f1-own-job-lease".into())
        .spawn(move || {
            struct Done(Arc<AtomicBool>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                }
            }
            let _done = Done(done);
            let _writer = life_w;
            loop {
                if cancel_worker.load(Ordering::Acquire) || parent.before_effect().is_err() {
                    break;
                }
                let timeout = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 5_000_000,
                };
                let mut result: libc::kevent = unsafe { std::mem::zeroed() };
                let n = unsafe {
                    libc::kevent(
                        queue.as_raw_fd(),
                        std::ptr::null(),
                        0,
                        &mut result,
                        1,
                        &timeout,
                    )
                };
                if n > 0 {
                    break;
                }
                if n < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    break;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(JobLease { cancel, finished })
}

#[cfg(test)]
mod retirement_tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Mutex, Weak};
    struct Life {
        calls: AtomicUsize,
        retire: AtomicBool,
        fail: AtomicBool,
        registry: Mutex<Option<Weak<Registry>>>,
    }
    impl Life {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                retire: AtomicBool::new(false),
                fail: AtomicBool::new(false),
                registry: Mutex::new(None),
            })
        }
    }
    impl AliveLease for Life {
        fn is_alive(&self) -> bool {
            true
        }
        fn before_effect(&self) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.swap(false, Ordering::SeqCst) {
                return Err("own single guardian fault".into());
            }
            if self.retire.load(Ordering::SeqCst) {
                if let Some(registry) = self
                    .registry
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(Weak::upgrade)
                {
                    registry.retire();
                }
            }
            Ok(())
        }
    }
    #[test]
    fn process_job_life_shares_retirement_without_replacing_guardian_issuer() {
        let owner = Life::new();
        let original: Arc<dyn AliveLease> = owner.clone();
        let table = DescriptorTable::new();
        let registry = Arc::new(Registry::new(original.clone(), table.clone()).unwrap());
        *owner.registry.lock().unwrap() = Some(Arc::downgrade(&registry));
        let held_job_life = registry.job_life();
        assert!(Arc::ptr_eq(&registry.life, &original));
        assert!(registry.matches_issuer(&original, &table).is_ok());
        assert!(held_job_life.before_effect().is_ok());
        owner.retire.store(true, Ordering::SeqCst);
        assert!(held_job_life.before_effect().is_err());
        let calls = owner.calls.load(Ordering::SeqCst);
        assert!(!held_job_life.is_alive());
        assert!(registry.before_effect().is_err());
        assert_eq!(owner.calls.load(Ordering::SeqCst), calls);
        assert!(registry.pipe().is_err());
    }
    #[test]
    fn process_job_guardian_fault_is_sticky_even_when_original_recovers() {
        let owner = Life::new();
        let registry = Registry::new(owner.clone(), DescriptorTable::new()).unwrap();
        let held_job_life = registry.job_life();
        let (read, write) = registry.pipe().unwrap();
        owner.fail.store(true, Ordering::SeqCst);
        assert!(held_job_life.before_effect().is_err());
        assert!(owner.before_effect().is_ok());
        let calls = owner.calls.load(Ordering::SeqCst);
        assert!(registry.before_effect().is_err());
        assert!(!held_job_life.is_alive());
        assert_eq!(owner.calls.load(Ordering::SeqCst), calls);
        // Cleanup retains the actual already-issued descriptors; no new grant.
        assert!(unsafe { libc::fcntl(read.as_raw_fd(), libc::F_GETFD) } >= 0);
        drop(read);
        drop(write);
    }
}
