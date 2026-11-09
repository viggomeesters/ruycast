//! Process-only C routes. No unknown executable, descriptor or null-device open
//! reaches an original provider. Main-cell selection is owned by the integrator.
use super::{
    owned::{AliveLease, Result},
    process_dispatch::{FileAction, JobLease, Options, Registry},
};
use std::{
    collections::BTreeMap,
    ffi::{c_void, CStr},
    os::fd::IntoRawFd,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
};
pub struct Context {
    pub registry: Arc<Registry>,
    pub life: Arc<dyn AliveLease>,
    pub owner_uid: libc::uid_t,
}
static CONTEXT: OnceLock<Context> = OnceLock::new();
static ACTIONS: OnceLock<Mutex<BTreeMap<usize, Vec<FileAction>>>> = OnceLock::new();
struct Job {
    lease: JobLease,
    generation: u64,
}
static JOBS: OnceLock<Mutex<BTreeMap<i32, Job>>> = OnceLock::new();
static JOBS_CLOSING: AtomicBool = AtomicBool::new(false);
static JOB_LAUNCHES: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
static TASK_WAIT_FAILED: AtomicBool = AtomicBool::new(false);
static JOBS_CHANGED: Condvar = Condvar::new();
static GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT: AtomicUsize = AtomicUsize::new(1);
static FAULTED: AtomicBool = AtomicBool::new(false);
/// Parent callbacks may invoke a dynamic lease and lock mutable fixture state.
/// Neither callback panics nor poisoned locks may unwind across a C caller.
/// The post-fork exec byte bridge never enters this helper. A successful fork
/// child only returns its scalar result through catch_unwind (no heap/lock).
pub(crate) fn parent_boundary<T: Copy>(denied: T, callback: impl FnOnce() -> T) -> T {
    callback_boundary(denied, callback, false)
}
fn cleanup_boundary<T: Copy>(denied: T, callback: impl FnOnce() -> T) -> T {
    callback_boundary(denied, callback, true)
}
fn callback_boundary<T: Copy>(denied: T, callback: impl FnOnce() -> T, cleanup: bool) -> T {
    if super::process_fork::is_child() || (!cleanup && FAULTED.load(Ordering::Acquire)) {
        unsafe { *libc::__error() = libc::EPERM };
        return denied;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)) {
        Ok(value) => value,
        Err(payload) => {
            FAULTED.store(true, Ordering::Release);
            // The sole process consumer keeps its wait ownership. Cancellation
            // closes each registered job's writer; no numeric PID is signalled.
            if let Some(jobs) = JOBS.get() {
                for job in jobs.lock().unwrap_or_else(|e| e.into_inner()).values() {
                    job.lease.cancel();
                }
            }
            // A custom panic payload may itself panic when dropped.
            std::mem::forget(payload);
            unsafe { *libc::__error() = libc::EPERM };
            denied
        }
    }
}
pub fn install(context: Context) -> Result<()> {
    context.life.before_effect()?;
    CONTEXT
        .set(context)
        .map_err(|_| "process ABI already installed".into())
}
fn context() -> Option<&'static Context> {
    let c = CONTEXT.get()?;
    c.registry.before_effect().ok()?;
    Some(c)
}
pub(crate) fn installed_context() -> Result<&'static Context> {
    context().ok_or_else(|| "process context absent/retired".into())
}
pub(crate) fn verify_registry(registry: &Registry) -> Result<()> {
    let current = installed_context()?;
    if !std::ptr::eq(current.registry.as_ref(), registry) {
        return Err("Task process registry differs from installed owner".into());
    }
    registry.before_effect()
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn cancel_reference_jobs(registry: &Registry) -> Result<()> {
    verify_registry(registry)?;
    let jobs = JOBS.get_or_init(Default::default).lock().map_err(|_| "owned Task registry poisoned")?;
    JOBS_CLOSING.store(true, Ordering::Release);
    for job in jobs.values() {
        job.lease.cancel();
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn verify_reference_jobs_drained(registry: &Registry) -> Result<()> {
    verify_registry(registry)?;
    let jobs = JOBS.get_or_init(Default::default).lock().map_err(|_| "owned Task registry poisoned")?;
    if !JOBS_CLOSING.load(Ordering::Acquire) {
        return Err("Reference process cancellation has not begun".into());
    }
    let (jobs, _) = JOBS_CHANGED.wait_timeout_while(jobs, std::time::Duration::from_secs(4), |jobs| {
        !jobs.is_empty() || JOB_LAUNCHES.load(Ordering::Acquire) != 0
    }).map_err(|_| "owned Task registry poisoned during shutdown")?;
    if !jobs.is_empty() || JOB_LAUNCHES.load(Ordering::Acquire) != 0 {
        return Err("Reference process jobs remain unacknowledged at shutdown".into());
    }
    if TASK_WAIT_FAILED.load(Ordering::Acquire) {
        return Err("Reference Task waiter lost its owned child result".into());
    }
    Ok(())
}

#[cfg(all(rcf1_reference_sdk, not(test), target_arch = "aarch64"))]
pub(crate) fn record_task_wait_failure() {
    TASK_WAIT_FAILED.store(true, Ordering::Release);
}

// Called under JOBS so a launch already crossing the process boundary when
// shutdown begins still receives cancellation and keeps its sole wait owner.
fn cancel_if_closing(lease: &JobLease) {
    if JOBS_CLOSING.load(Ordering::Acquire) {
        lease.cancel();
    }
}

pub(crate) fn verify_new_job_admission() -> Result<()> {
    if JOBS_CLOSING.load(Ordering::Acquire) {
        return Err("Reference process launch admission closed".into());
    }
    Ok(())
}

pub(crate) struct JobLaunchReservation(bool);
impl Drop for JobLaunchReservation {
    fn drop(&mut self) {
        if self.0 {
            let _jobs = JOBS.get_or_init(Default::default).lock().unwrap_or_else(|error| error.into_inner());
            JOB_LAUNCHES.fetch_sub(1, Ordering::AcqRel);
            JOBS_CHANGED.notify_all();
        }
    }
}

pub(crate) fn reserve_job_launch() -> Result<JobLaunchReservation> {
    if !cfg!(all(rcf1_reference_sdk, not(test), target_arch = "aarch64")) {
        return Ok(JobLaunchReservation(false));
    }
    let _jobs = JOBS.get_or_init(Default::default).lock().map_err(|_| "owned Task registry poisoned")?;
    verify_new_job_admission()?;
    JOB_LAUNCHES.fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| active.checked_add(1))
        .map_err(|_| "Reference launch reservation exhausted")?;
    Ok(JobLaunchReservation(true))
}
/// Register the actual just-launched owned Task before its PID is exposed.
/// The Task's sole wait worker uses the existing cleanup wait/ack route.
pub(crate) fn register_owned_task(
    registry: &Registry,
    job: &super::process_dispatch::PublishedJob,
) -> Result<()> {
    verify_registry(registry)?;
    if job.pid <= 1 {
        return Err("owned Task PID invalid".into());
    }
    let generation = GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| "owned Task generation exhausted")?;
    let mut jobs = JOBS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "owned Task registry poisoned")?;
    if jobs.contains_key(&job.pid) {
        return Err("owned Task PID already registered".into());
    }
    cancel_if_closing(&job.lease);
    jobs.insert(
        job.pid,
        Job {
            lease: job.lease.clone(),
            generation,
        },
    );
    Ok(())
}
/// Selection only; the shared router must enter parent_boundary before using
/// these helpers. Presence is never permission to inspect an arbitrary PID.
pub fn registered_job(pid: i32) -> bool {
    JOBS.get()
        .is_some_and(|jobs| jobs.lock().unwrap().contains_key(&pid))
}
/// No guardian or job/descriptor code may run in commit. Hold classification
/// stable through a virtual-only effect instead of a stale boolean lookup.
pub(crate) fn with_no_registered_job<T>(pid:i32,commit:impl FnOnce()->Result<T>)->Result<T> {
    if pid<=1 {
        return Err("virtual route PID invalid".into());
    }
    with_job_classification(|registered| {
        if registered(pid) {return Err("virtual target aliases a registered job".into());}
        commit()
    })
}

/// Hold the job table for a callback-free virtual process read or commit.
pub(crate) fn with_job_classification<T>(commit: impl FnOnce(&dyn Fn(i32) -> bool) -> Result<T>) -> Result<T> {
    verify_virtual_route_live()?;
    let jobs = JOBS.get_or_init(Default::default).lock().map_err(|_| "registered job table poisoned")?;
    verify_virtual_route_live()?;
    commit(&|pid| jobs.contains_key(&pid))
}
pub(crate) fn verify_virtual_route_live() -> Result<()> {
    if FAULTED.load(Ordering::Acquire) || super::process_fork::is_child() {
        return Err("virtual job classification faulted or post-fork".into());
    }
    Ok(())
}
pub fn routes_live() -> bool {
    context().is_some()
}

#[cfg(test)]
mod virtual_commit_tests {
    use super::*;
    #[test]
    fn virtual_commit_holds_job_classification_and_rejects_invalid_pid() {
        with_no_registered_job(730001,|| {
            assert!(JOBS.get().unwrap().try_lock().is_err(),"registered jobs must not change during virtual commit");Ok(())
        }).unwrap();
        let mut committed=false;
        assert!(with_no_registered_job(0,||{committed=true;Ok(())}).is_err());
        assert!(!committed);
    }
}

fn error(code: i32) -> i32 {
    unsafe {
        *libc::__error() = code;
    }
    -1
}
unsafe fn actions_mut(
    p: *mut libc::posix_spawn_file_actions_t,
    f: impl FnOnce(&mut Vec<FileAction>) -> i32,
) -> i32 {
    if context().is_none() || p.is_null() {
        return libc::EPERM;
    }
    let token = unsafe { *p } as usize;
    let mut map = ACTIONS.get_or_init(Default::default).lock().unwrap();
    let Some(actions) = map.get_mut(&token) else {
        return libc::EINVAL;
    };
    if actions.len() >= 256 {
        return libc::E2BIG;
    }
    f(actions)
}
pub(crate) unsafe extern "C" fn action_init(p: *mut libc::posix_spawn_file_actions_t) -> i32 {
    parent_boundary(libc::EPERM, || {
        if context().is_none() || p.is_null() {
            return libc::EPERM;
        }
        let token = NEXT.fetch_add(1, Ordering::Relaxed);
        if token == 0 || token == usize::MAX {
            return libc::EOVERFLOW;
        }
        let mut map = ACTIONS.get_or_init(Default::default).lock().unwrap();
        if map.len() >= 4096 {
            return libc::EMFILE;
        }
        map.insert(token, vec![]);
        unsafe {
            *p = token as *mut c_void;
        }
        0
    })
}
pub(crate) unsafe extern "C" fn action_destroy(p: *mut libc::posix_spawn_file_actions_t) -> i32 {
    parent_boundary(libc::EPERM, || {
        if CONTEXT.get().is_none() {
            return libc::EPERM;
        }
        if super::process_fork::is_child() {
            return libc::EPERM;
        }
        if p.is_null() {
            return libc::EINVAL;
        }
        let token = unsafe { *p } as usize;
        let removed = ACTIONS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .remove(&token);
        if removed.is_none() {
            return libc::EINVAL;
        }
        unsafe {
            *p = std::ptr::null_mut();
        }
        0
    })
}
pub(crate) unsafe extern "C" fn action_dup(
    p: *mut libc::posix_spawn_file_actions_t,
    from: i32,
    to: i32,
) -> i32 {
    parent_boundary(libc::EPERM, || {
        if from < 0 || to < 0 {
            return libc::EBADF;
        }
        unsafe {
            actions_mut(p, |a| {
                a.push(FileAction::Dup { from, to });
                0
            })
        }
    })
}
pub(crate) unsafe extern "C" fn action_close(
    p: *mut libc::posix_spawn_file_actions_t,
    fd: i32,
) -> i32 {
    parent_boundary(libc::EPERM, || {
        if fd < 0 {
            return libc::EBADF;
        }
        unsafe {
            actions_mut(p, |a| {
                a.push(FileAction::Close(fd));
                0
            })
        }
    })
}
pub(crate) unsafe extern "C" fn action_inherit(
    p: *mut libc::posix_spawn_file_actions_t,
    fd: i32,
) -> i32 {
    parent_boundary(libc::EPERM, || {
        if fd < 0 {
            return libc::EBADF;
        }
        unsafe {
            actions_mut(p, |a| {
                a.push(FileAction::Inherit(fd));
                0
            })
        }
    })
}
pub(crate) unsafe extern "C" fn action_open(
    p: *mut libc::posix_spawn_file_actions_t,
    fd: i32,
    path: *const libc::c_char,
    flags: i32,
    mode: libc::mode_t,
) -> i32 {
    parent_boundary(libc::EPERM, || {
        if path.is_null()
            || unsafe { CStr::from_ptr(path) } != c"/dev/null"
            || mode != 0
            || !matches!(flags, libc::O_RDONLY | libc::O_WRONLY)
        {
            return libc::EPERM;
        }
        unsafe {
            actions_mut(p, |a| {
                a.push(FileAction::Null {
                    fd,
                    write: flags == libc::O_WRONLY,
                });
                0
            })
        }
    })
}
pub(crate) unsafe extern "C" fn action_chdir(
    _p: *mut libc::posix_spawn_file_actions_t,
    _path: *const libc::c_char,
) -> i32 {
    parent_boundary(libc::EPERM, || libc::EPERM)
}
pub(crate) unsafe extern "C" fn action_fchdir(
    _p: *mut libc::posix_spawn_file_actions_t,
    _fd: i32,
) -> i32 {
    parent_boundary(libc::EPERM, || libc::EPERM)
}
pub(crate) unsafe extern "C" fn pipe(out: *mut i32) -> i32 {
    parent_boundary(-1, || {
        let Some(c) = context() else {
            return error(libc::EPERM);
        };
        if out.is_null() {
            return error(libc::EINVAL);
        }
        match c.registry.pipe() {
            Ok((read, write)) => {
                unsafe {
                    out.write(read.into_raw_fd());
                    out.add(1).write(write.into_raw_fd());
                }
                0
            }
            Err(_) => error(libc::EPERM),
        }
    })
}
unsafe fn strings(argv: *const *mut libc::c_char) -> Result<Vec<String>> {
    if argv.is_null() {
        return Err("missing argv".into());
    }
    let mut out = vec![];
    let mut total = 0;
    for i in 0..66 {
        let p = unsafe { argv.add(i).read() };
        if p.is_null() {
            return Ok(out);
        }
        let bytes = unsafe { CStr::from_ptr(p) }.to_bytes();
        total += bytes.len();
        if bytes.len() > 8192 || total > 65536 {
            return Err("argv bounds".into());
        }
        out.push(std::str::from_utf8(bytes).map_err(|_| "argv UTF8")?.into());
    }
    Err("argv count".into())
}
pub(crate) unsafe extern "C" fn spawn(
    pid: *mut i32,
    path: *const libc::c_char,
    actions: *const libc::posix_spawn_file_actions_t,
    attr: *const libc::posix_spawnattr_t,
    argv: *const *mut libc::c_char,
    _env: *const *mut libc::c_char,
) -> i32 {
    parent_boundary(libc::EPERM, || {
        let Some(c) = context() else {
            return libc::EPERM;
        };
        if pid.is_null() || path.is_null() {
            return libc::EINVAL;
        }
        let Ok(_launch) = reserve_job_launch() else {
            return libc::EPERM;
        };
        let mut options = Options::default();
        if !attr.is_null() {
            let mut flags = 0;
            let mut group = 0;
            if unsafe { libc::posix_spawnattr_getflags(attr, &mut flags) } != 0
                || unsafe { libc::posix_spawnattr_getpgroup(attr, &mut group) } != 0
            {
                return libc::EINVAL;
            }
            let supported = libc::POSIX_SPAWN_SETPGROUP
                | libc::POSIX_SPAWN_CLOEXEC_DEFAULT
                | libc::POSIX_SPAWN_SETSIGDEF
                | libc::POSIX_SPAWN_SETSIGMASK;
            if flags as i32 & !supported != 0 || group != 0 {
                return libc::ENOTSUP;
            }
            if i32::from(flags) & libc::POSIX_SPAWN_SETSIGDEF != 0 {
                let mut value = unsafe { std::mem::zeroed() };
                if unsafe { libc::posix_spawnattr_getsigdefault(attr, &mut value) } != 0 {
                    return libc::EINVAL;
                }
                options.signal_defaults = Some(value);
            }
            if i32::from(flags) & libc::POSIX_SPAWN_SETSIGMASK != 0 {
                let mut value = unsafe { std::mem::zeroed() };
                if unsafe { libc::posix_spawnattr_getsigmask(attr, &mut value) } != 0 {
                    return libc::EINVAL;
                }
                options.signal_mask = Some(value);
            }
        }
        let Ok(path) = unsafe { CStr::from_ptr(path) }.to_str() else {
            return libc::EINVAL;
        };
        let Ok(mut arguments) = (unsafe { strings(argv) }) else {
            return libc::EINVAL;
        };
        if arguments.is_empty() {
            return libc::EINVAL;
        }
        arguments.remove(0);
        let list = if actions.is_null() {
            vec![]
        } else {
            let token = unsafe { *actions } as usize;
            let map = ACTIONS.get_or_init(Default::default).lock().unwrap();
            let Some(value) = map.get(&token) else {
                return libc::EINVAL;
            };
            value.clone()
        };
        let job =
            match c
                .registry
                .dispatch_with_options(Path::new(path), &arguments, &list, &options)
            {
                Ok(v) => v,
                Err(_) => return libc::EPERM,
            };
        let mut jobs = JOBS.get_or_init(Default::default).lock().unwrap();
        // New unreaped kernel child proves an older equal-number record is stale.
        cancel_if_closing(&job.lease);
        jobs.insert(
            job.pid,
            Job {
                lease: job.lease,
                generation: GENERATION.fetch_add(1, Ordering::Relaxed),
            },
        );
        unsafe {
            *pid = job.pid;
        }
        0
    })
}
pub(crate) unsafe extern "C" fn kill(pid: i32, signal: i32) -> i32 {
    parent_boundary(-1, || {
        if context().is_none() {
            return error(libc::EPERM);
        }
        if ![0, libc::SIGINT, libc::SIGTERM, libc::SIGKILL].contains(&signal) {
            return error(libc::EPERM);
        }
        let Some(key) = pid.checked_abs().filter(|p| *p > 1) else {
            return error(libc::EPERM);
        };
        let jobs = JOBS.get_or_init(Default::default).lock().unwrap();
        if let Some(job) = jobs.get(&key) {
            if job.lease.finished() {
                return error(libc::ESRCH);
            }
            if signal != 0 {
                job.lease.cancel();
            }
            return 0;
        }
        error(libc::EPERM)
    })
}
pub(crate) unsafe extern "C" fn fork() -> i32 {
    parent_boundary(-1, || {
        let Some(c) = context() else {
            return error(libc::EPERM);
        };
        let Ok(launch) = reserve_job_launch() else {
            return error(libc::EPERM);
        };
        match super::process_fork::fork(c.registry.clone(), c.registry.job_life()) {
            Ok(super::process_fork::Fork::Child) => {
                // The reservation is owned by the parent registry.
                std::mem::forget(launch);
                0
            }
            Ok(super::process_fork::Fork::Parent(job)) => {
                let pid = job.pid;
                let mut jobs = JOBS.get_or_init(Default::default).lock().unwrap();
                // Generation-bound acknowledgements cannot remove this new child.
                cancel_if_closing(&job.lease);
                jobs.insert(
                    pid,
                    Job {
                        lease: job.lease,
                        generation: GENERATION.fetch_add(1, Ordering::Relaxed),
                    },
                );
                pid
            }
            Err(_) => error(libc::EPERM),
        }
    })
}
pub(crate) unsafe extern "C" fn execve(
    path: *const libc::c_char,
    argv: *const *mut libc::c_char,
    _env: *const *mut libc::c_char,
) -> i32 {
    unsafe { super::process_fork::exec(path, argv.cast()) }
}
pub(crate) unsafe extern "C" fn execvp(
    path: *const libc::c_char,
    argv: *const *mut libc::c_char,
) -> i32 {
    unsafe { super::process_fork::exec(path, argv.cast()) }
}
#[no_mangle]
pub(crate) unsafe extern "C" fn rcf1_process_open_checked(
    path: *const libc::c_char,
    flags: i32,
    mode: u32,
) -> i32 {
    parent_boundary(-1, || {
        let Some(c) = context() else {
            return error(libc::EPERM);
        };
        if path.is_null() || unsafe { CStr::from_ptr(path) } != c"/dev/null" || mode != 0 {
            return error(libc::EPERM);
        }
        match c.registry.null(flags) {
            Ok(fd) => fd.into_raw_fd(),
            Err(_) => error(libc::EPERM),
        }
    })
}
#[no_mangle]
pub(crate) unsafe extern "C" fn rcf1_process_openat_checked(
    dir: i32,
    path: *const libc::c_char,
    flags: i32,
    mode: u32,
) -> i32 {
    parent_boundary(-1, || {
        if dir != libc::AT_FDCWD {
            return error(libc::EPERM);
        }
        unsafe { rcf1_process_open_checked(path, flags, mode) }
    })
}
// Apple arm64 passes variadic (not fixed) arguments on the stack. Do not cast
// open/openat to a fixed three/four-argument Rust function. These trampolines
// preserve all fixed registers, decode mode only for O_CREAT, then tail-call
// checked Rust. No path, policy, allocation or OS query exists in assembly.
#[cfg(target_arch = "aarch64")]
std::arch::global_asm!(
    r#"
.text
.p2align 2
.globl _rcf1_process_open
_rcf1_process_open:
 mov w2, #0
 tbz w1, #9, 1f
 ldr w2, [sp]
1:
 b _rcf1_process_open_checked
.p2align 2
.globl _rcf1_process_openat
_rcf1_process_openat:
 mov w3, #0
 tbz w2, #9, 2f
 ldr w3, [sp]
2:
 b _rcf1_process_openat_checked
"#
);
#[cfg(target_arch = "x86_64")]
std::arch::global_asm!(
    r#"
.text
.globl _rcf1_process_open
_rcf1_process_open:
 test $512, %esi
 jnz 1f
 xor %edx, %edx
1:
 jmp _rcf1_process_open_checked
.globl _rcf1_process_openat
_rcf1_process_openat:
 test $512, %edx
 jnz 2f
 xor %ecx, %ecx
2:
 jmp _rcf1_process_openat_checked
"#
);
unsafe extern "C" {
    pub fn rcf1_process_open(path: *const libc::c_char, flags: i32, ...) -> i32;
    pub fn rcf1_process_openat(dir: i32, path: *const libc::c_char, flags: i32, ...) -> i32;
    fn posix_spawn_file_actions_addinherit_np(
        a: *mut libc::posix_spawn_file_actions_t,
        fd: i32,
    ) -> i32;
    fn posix_spawn_file_actions_addchdir_np(
        a: *mut libc::posix_spawn_file_actions_t,
        path: *const libc::c_char,
    ) -> i32;
    fn posix_spawn_file_actions_addfchdir_np(
        a: *mut libc::posix_spawn_file_actions_t,
        fd: i32,
    ) -> i32;
}
pub fn import_specs() -> Vec<super::ImportSpec> {
    let rows: [(&CStr, usize, usize); 20] = [
        (
            c"posix_spawn",
            libc::posix_spawn as *const () as usize,
            spawn as *const () as usize,
        ),
        (
            c"posix_spawnp",
            libc::posix_spawnp as *const () as usize,
            spawn as *const () as usize,
        ),
        (
            c"posix_spawn_file_actions_init",
            libc::posix_spawn_file_actions_init as *const () as usize,
            action_init as *const () as usize,
        ),
        (
            c"posix_spawn_file_actions_destroy",
            libc::posix_spawn_file_actions_destroy as *const () as usize,
            action_destroy as *const () as usize,
        ),
        (
            c"posix_spawn_file_actions_adddup2",
            libc::posix_spawn_file_actions_adddup2 as *const () as usize,
            action_dup as *const () as usize,
        ),
        (
            c"posix_spawn_file_actions_addclose",
            libc::posix_spawn_file_actions_addclose as *const () as usize,
            action_close as *const () as usize,
        ),
        (
            c"posix_spawn_file_actions_addopen",
            libc::posix_spawn_file_actions_addopen as *const () as usize,
            action_open as *const () as usize,
        ),
        (
            c"posix_spawn_file_actions_addinherit_np",
            posix_spawn_file_actions_addinherit_np as *const () as usize,
            action_inherit as *const () as usize,
        ),
        (
            c"posix_spawn_file_actions_addchdir_np",
            posix_spawn_file_actions_addchdir_np as *const () as usize,
            action_chdir as *const () as usize,
        ),
        (
            c"posix_spawn_file_actions_addfchdir_np",
            posix_spawn_file_actions_addfchdir_np as *const () as usize,
            action_fchdir as *const () as usize,
        ),
        (
            c"pipe",
            libc::pipe as *const () as usize,
            pipe as *const () as usize,
        ),
        (
            c"kill",
            libc::kill as *const () as usize,
            kill as *const () as usize,
        ),
        (
            c"fork",
            libc::fork as *const () as usize,
            fork as *const () as usize,
        ),
        (
            c"execve",
            libc::execve as *const () as usize,
            execve as *const () as usize,
        ),
        (
            c"execvp",
            libc::execvp as *const () as usize,
            execvp as *const () as usize,
        ),
        (
            c"open",
            libc::open as *const () as usize,
            rcf1_process_open as *const () as usize,
        ),
        (
            c"waitpid",
            libc::waitpid as *const () as usize,
            waitpid as *const () as usize,
        ),
        (
            c"waitid",
            libc::waitid as *const () as usize,
            waitid as *const () as usize,
        ),
        (
            c"proc_listpids",
            libc::proc_listpids as *const () as usize,
            proc_listpids as *const () as usize,
        ),
        (
            c"proc_pidinfo",
            libc::proc_pidinfo as *const () as usize,
            job_proc_pidinfo as *const () as usize,
        ),
    ];
    rows.into_iter()
        .map(|(symbol, original, replacement)| super::ImportSpec {
            symbol,
            original,
            replacement,
        })
        .collect()
}

fn generation(pid: i32) -> Option<u64> {
    JOBS.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&pid)
        .map(|job| job.generation)
}
fn acknowledged(pid: i32, generation: u64) {
    let mut jobs = JOBS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if jobs.get(&pid).is_some_and(|j| j.generation == generation) {
        jobs.remove(&pid);
        JOBS_CHANGED.notify_all();
    }
}
/// Delegates this original caller's single wait; the SDK never adds a second
/// independent reaper. Unknown/global waits cannot inspect or reap processes.
pub(crate) unsafe extern "C" fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32 {
    cleanup_boundary(-1, || {
        // Retired/faulted admission forbids new effects, but must not strand the
        // original consumer's sole wait for an already registered kernel child.
        // No dynamic lease is consulted on this held-identity cleanup route.
        if CONTEXT.get().is_none() {
            return error(libc::EPERM);
        }
        if pid <= 1 || options & !libc::WNOHANG != 0 {
            return error(libc::EPERM);
        }
        let Some(generation) = generation(pid) else {
            return error(libc::ECHILD);
        };
        let result = unsafe { libc::waitpid(pid, status, options) };
        if result == pid || result < 0 && unsafe { *libc::__error() } == libc::ECHILD {
            acknowledged(pid, generation);
        }
        result
    })
}
pub(crate) unsafe extern "C" fn waitid(
    kind: libc::idtype_t,
    id: libc::id_t,
    info: *mut libc::siginfo_t,
    options: i32,
) -> i32 {
    cleanup_boundary(-1, || {
        if CONTEXT.get().is_none() {
            return error(libc::EPERM);
        }
        if kind != libc::P_PID
            || id <= 1
            || id > i32::MAX as libc::id_t
            || info.is_null()
            || options & !(libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) != 0
            || options & libc::WEXITED == 0
        {
            return error(libc::EPERM);
        }
        let pid = id as i32;
        let Some(generation) = generation(pid) else {
            return error(libc::ECHILD);
        };
        let result = unsafe { libc::waitid(kind, id, info, options) };
        if result == 0 && options & libc::WNOWAIT == 0 && unsafe { (*info).si_pid() } == pid
            || result < 0 && unsafe { *libc::__error() } == libc::ECHILD
        {
            acknowledged(pid, generation);
        }
        result
    })
}
pub(crate) unsafe extern "C" fn proc_listpids(
    kind: u32,
    group: u32,
    out: *mut c_void,
    capacity: i32,
) -> i32 {
    parent_boundary(-1, || {
        if context().is_none() || kind != 2 || group <= 1 || group > i32::MAX as u32 || capacity < 0
        {
            return error(libc::EPERM);
        }
        if generation(group as i32).is_none() {
            return error(libc::EPERM);
        }
        if out.is_null() {
            return 4;
        }
        if capacity < 4 {
            return 0;
        }
        unsafe {
            out.cast::<i32>().write(group as i32);
        }
        4
    })
}
/// This entry is the owned-job half of the final single proc_pidinfo router.
/// The monitor's virtual process half must compose at the same symbol, never
/// install a competing import binding or fall back to the actual libproc API.
pub(crate) unsafe extern "C" fn job_proc_pidinfo(
    pid: i32,
    flavor: i32,
    arg: u64,
    out: *mut c_void,
    capacity: i32,
) -> i32 {
    parent_boundary(-1, || {
        let Some(c) = context() else {
            return error(libc::EPERM);
        };
        let size = std::mem::size_of::<libc::proc_bsdinfo>();
        if flavor != libc::PROC_PIDTBSDINFO || arg != 0 || out.is_null() || capacity < size as i32 {
            return error(libc::EPERM);
        }
        let jobs = JOBS.get_or_init(Default::default).lock().unwrap();
        let Some(job) = jobs.get(&pid) else {
            return error(libc::EPERM);
        };
        let p = out.cast::<libc::proc_bsdinfo>();
        unsafe {
            std::ptr::write_bytes(out, 0, size);
            (*p).pbi_pid = pid as u32;
            (*p).pbi_pgid = pid as u32;
            (*p).pbi_uid = c.owner_uid;
            (*p).pbi_ruid = c.owner_uid;
            (*p).pbi_svuid = c.owner_uid;
            (*p).pbi_start_tvsec = job.generation;
            (*p).pbi_status = if job.lease.finished() { 5 } else { 1 };
        }
        size as i32
    })
}
