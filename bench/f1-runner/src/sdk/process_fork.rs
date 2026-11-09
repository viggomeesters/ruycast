//! Source-bound std fork/exec bridge. Child side is bounded async-signal-safe
//! byte transport only. Parent alone resolves an exact pinned own-fake route.
//! A child signal guardian follows a read-only local lease before fake exec;
//! the fake then takes over using the independently reviewed Guardian bootstrap.
use super::{
    descriptors::{StdioRow, StdioSnapshot},
    owned::{AliveLease, Result},
    process_dispatch::{self, ExecPlan, PublishedJob, Registry},
};
use crate::guardian;
use std::{
    ffi::c_char,
    io,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
const MAX: usize = 65536;
const LIMIT: i64 = 3_000_000_000;
static SERIAL: Mutex<()> = Mutex::new(());
static CHILD: AtomicBool = AtomicBool::new(false);
static LIFE: AtomicI32 = AtomicI32::new(-1);
static REQUEST: AtomicI32 = AtomicI32::new(-1);
static REPLY: AtomicI32 = AtomicI32::new(-1);
static ADMIT: AtomicI32 = AtomicI32::new(-1);
// This entire module is a bench SDK fixture, never product code. These typed
// own-fake faults cannot be set through a profile, command, receipt or wire op.
static BROKER_PANIC_ON_READY: AtomicBool = AtomicBool::new(false);
static PREEXEC_READY: AtomicBool = AtomicBool::new(false);
pub fn owned_fixture_broker_panic_on_ready() {
    BROKER_PANIC_ON_READY.store(true, Ordering::Release);
}
pub fn owned_fixture_preexec_ready() {
    PREEXEC_READY.store(true, Ordering::Release);
}
/// Must wrap the single common ModuleContext lifecycle before any original
/// callback. Forkchild can reach only its byte bridge, never another SDK leaf.
pub struct ForkAwareLease {
    inner: Arc<dyn AliveLease>,
}
impl ForkAwareLease {
    pub fn new(inner: Arc<dyn AliveLease>) -> Self {
        Self { inner }
    }
}
impl AliveLease for ForkAwareLease {
    fn is_alive(&self) -> bool {
        !CHILD.load(Ordering::Acquire) && self.inner.is_alive()
    }
    fn before_effect(&self) -> Result<()> {
        if CHILD.load(Ordering::Acquire) {
            // This may run after a multithreaded fork: no String allocation,
            // formatting, lock, Objective-C or inherited callback is safe here.
            self_exit();
        }
        self.inner.before_effect()
    }
}
pub fn is_child() -> bool {
    CHILD.load(Ordering::Acquire)
}
pub fn owned_fixture_child_gate() -> bool {
    if !is_child() {
        return false;
    }
    if !life_alive() {
        self_exit();
    }
    true
}
fn self_exit() -> ! {
    unsafe {
        if libc::getpid() == libc::getpgrp() && libc::kill(0, libc::SIGKILL) == 0 {
            loop {
                libc::pause();
            }
        }
        libc::_exit(89)
    }
}
fn life_alive() -> bool {
    let fd = LIFE.load(Ordering::Relaxed);
    if fd <= 2 {
        return false;
    }
    let mut p = libc::pollfd {
        fd,
        events: libc::POLLIN | libc::POLLHUP,
        revents: 0,
    };
    unsafe { libc::poll(&mut p, 1, 0) == 0 }
}
extern "C" fn alarm(_signal: i32) {
    if !life_alive() {
        self_exit();
    }
}
unsafe fn timer(enabled: bool) -> bool {
    let interval = libc::timeval {
        tv_sec: 0,
        tv_usec: if enabled { 10000 } else { 0 },
    };
    let value = libc::itimerval {
        it_interval: interval,
        it_value: interval,
    };
    unsafe { libc::setitimer(libc::ITIMER_REAL, &value, std::ptr::null_mut()) == 0 }
}
unsafe fn child_guard() {
    if unsafe { libc::setpgid(0, 0) } != 0 || unsafe { libc::getpid() != libc::getpgrp() } {
        self_exit();
    }
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    if unsafe { libc::sigemptyset(&mut set) } != 0
        || unsafe { libc::sigaddset(&mut set, libc::SIGALRM) } != 0
        || unsafe { libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()) } != 0
    {
        self_exit();
    }
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = alarm as *const () as usize;
    action.sa_flags = libc::SA_RESTART;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
    }
    if unsafe { libc::sigaction(libc::SIGALRM, &action, std::ptr::null_mut()) } != 0
        || !unsafe { timer(true) }
    {
        self_exit();
    }
    if !life_alive() {
        self_exit();
    }
}
fn clock() -> i64 {
    let mut t: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t) } != 0 {
        self_exit();
    }
    t.tv_sec
        .saturating_mul(1_000_000_000)
        .saturating_add(t.tv_nsec)
}
unsafe fn child_io(fd: i32, bytes: *mut u8, length: usize, write: bool, end: i64) -> bool {
    let mut at = 0;
    while at < length {
        if !life_alive() {
            self_exit();
        }
        let remaining = end.saturating_sub(clock());
        if remaining <= 0 {
            return false;
        }
        let mut p = libc::pollfd {
            fd,
            events: if write { libc::POLLOUT } else { libc::POLLIN },
            revents: 0,
        };
        let n = unsafe { libc::poll(&mut p, 1, ((remaining / 1_000_000).clamp(1, 50)) as i32) };
        if n < 0 {
            if unsafe { *libc::__error() } == libc::EINTR {
                continue;
            }
            return false;
        }
        if n == 0 {
            continue;
        }
        if p.revents & libc::POLLNVAL != 0 {
            return false;
        }
        let n = if write {
            unsafe { libc::write(fd, bytes.add(at).cast(), length - at) }
        } else {
            unsafe { libc::read(fd, bytes.add(at).cast(), length - at) }
        };
        if n > 0 {
            at += n as usize;
        } else if n == 0 {
            return false;
        } else {
            let e = unsafe { *libc::__error() };
            if e != libc::EINTR && e != libc::EAGAIN {
                return false;
            }
        }
    }
    true
}
struct Pipes {
    life_r: OwnedFd,
    life_w: OwnedFd,
    request_r: OwnedFd,
    request_w: OwnedFd,
    reply_r: OwnedFd,
    reply_w: OwnedFd,
    admit_r: OwnedFd,
    admit_w: OwnedFd,
}
impl Pipes {
    fn new() -> Result<Self> {
        let (life_r, life_w) = guardian::pipe().map_err(|e| e.to_string())?;
        let (request_r, request_w) = guardian::pipe().map_err(|e| e.to_string())?;
        let (reply_r, reply_w) = guardian::pipe().map_err(|e| e.to_string())?;
        let (admit_r, admit_w) = guardian::pipe().map_err(|e| e.to_string())?;
        if [
            &life_r, &life_w, &request_r, &request_w, &reply_r, &reply_w, &admit_r, &admit_w,
        ]
        .iter()
        .any(|fd| fd.as_raw_fd() <= 2)
        {
            return Err("fork capabilities must not alias stdio".into());
        }
        for fd in [&request_r, &request_w, &reply_r, &reply_w] {
            let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
            if flags < 0
                || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                    != 0
            {
                return Err("fork control nonblocking setup".into());
            }
        }
        for fd in [&request_w, &reply_w, &admit_w] {
            if unsafe { libc::fcntl(fd.as_raw_fd(), 73, 1) } != 0 {
                return Err("fork no-SIGPIPE setup".into());
            }
        }
        Ok(Self {
            life_r,
            life_w,
            request_r,
            request_w,
            reply_r,
            reply_w,
            admit_r,
            admit_w,
        })
    }
}
pub enum Fork {
    Parent(PublishedJob),
    Child,
}
/// Only two frozen empty pre_exec hooks are in the source admission roster.
/// Publication transfers wait ownership to the original std consumer. This
/// bridge never reaps a successfully published child or signals a numeric PGID.
pub fn fork(registry: Arc<Registry>, parent: Arc<dyn AliveLease>) -> Result<Fork> {
    if is_child() {
        return Err("nested pre-exec fork forbidden".into());
    }
    parent.before_effect()?;
    let serial = SERIAL.lock().map_err(|_| "fork coordinator poisoned")?;
    let snapshot = registry.stdio_snapshot()?;
    snapshot.verify()?;
    let pipes = Pipes::new()?;
    LIFE.store(pipes.life_r.as_raw_fd(), Ordering::Relaxed);
    REQUEST.store(pipes.request_w.as_raw_fd(), Ordering::Relaxed);
    REPLY.store(pipes.reply_r.as_raw_fd(), Ordering::Relaxed);
    ADMIT.store(pipes.admit_r.as_raw_fd(), Ordering::Relaxed);
    parent.before_effect()?;
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("own fork creation failed".into());
    }
    if pid == 0 {
        CHILD.store(true, Ordering::Release);
        unsafe {
            libc::close(pipes.life_w.as_raw_fd());
            libc::close(pipes.request_r.as_raw_fd());
            libc::close(pipes.reply_w.as_raw_fd());
            libc::close(pipes.admit_w.as_raw_fd());
            // These plain numbers are solely the central snapshot's private
            // duplicate pins. They are not child stdio or guardian capabilities.
            // Close them before pre_exec; no Rust destructor/allocator/lock runs.
            for fd in snapshot.pin_fds() {
                if *fd <= 2 || libc::close(*fd) != 0 {
                    self_exit()
                }
            }
            child_guard();
        }
        let mut byte = 0u8;
        let end = clock().saturating_add(LIMIT);
        if !unsafe { child_io(pipes.reply_r.as_raw_fd(), &mut byte, 1, false, end) } || byte != b'M'
        {
            self_exit();
        }
        // No destructor/allocator/Mutex operation occurs in this forkchild branch.
        std::mem::forget(serial);
        std::mem::forget(snapshot);
        std::mem::forget(registry);
        std::mem::forget(parent);
        std::mem::forget(pipes);
        return Ok(Fork::Child);
    }
    struct Unpublished(Option<i32>);
    impl Drop for Unpublished {
        fn drop(&mut self) {
            if let Some(pid) = self.0.take() {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                    let mut s = 0;
                    while libc::waitpid(pid, &mut s, 0) < 0
                        && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                    {
                    }
                }
            }
        }
    }
    let mut owner = Unpublished(Some(pid));
    // Both parent and child set only this held child into its own group before
    // admission. No unrelated PID, callback, account or inventory is consulted.
    if unsafe { libc::setpgid(pid, pid) } != 0 {
        return Err("own child group setup failed".into());
    }
    let Pipes {
        life_r,
        life_w,
        request_r,
        request_w,
        reply_r,
        reply_w,
        admit_r,
        admit_w,
    } = pipes;
    let life_fd = life_r.as_raw_fd();
    let admit_fd = admit_r.as_raw_fd();
    drop(life_r);
    drop(request_w);
    drop(reply_r);
    drop(admit_r);
    let lease = process_dispatch::watch(pid, life_w, parent.clone())?;
    let broker_lease = lease.clone();
    thread::Builder::new()
        .name("f1-own-exec-broker".into())
        .spawn(move || {
            struct CancelOnFailure {
                lease: process_dispatch::JobLease,
                completed: bool,
            }
            impl Drop for CancelOnFailure {
                fn drop(&mut self) {
                    if !self.completed {
                        self.lease.cancel();
                    }
                }
            }
            let mut cancellation = CancelOnFailure {
                lease: broker_lease,
                completed: false,
            };
            let result = broker(
                (&request_r, &reply_w, &admit_w),
                (life_fd, admit_fd),
                &snapshot,
                &registry,
                parent.as_ref(),
            );
            cancellation.completed = result.is_ok();
        })
        .map_err(|e| e.to_string())?;
    owner.0 = None;
    drop(serial);
    Ok(Fork::Parent(PublishedJob { pid, lease }))
}
fn parent_io(
    fd: i32,
    bytes: &mut [u8],
    write: bool,
    end: Instant,
    life: &dyn AliveLease,
) -> Result<()> {
    let mut at = 0;
    while at < bytes.len() {
        life.before_effect()?;
        if Instant::now() >= end {
            return Err("exec broker deadline".into());
        }
        let mut p = libc::pollfd {
            fd,
            events: if write { libc::POLLOUT } else { libc::POLLIN },
            revents: 0,
        };
        let n = unsafe { libc::poll(&mut p, 1, 10) };
        if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if n < 0 || p.revents & libc::POLLNVAL != 0 {
            return Err("exec broker poll failure".into());
        }
        if n == 0 {
            continue;
        }
        let n = if write {
            unsafe { libc::write(fd, bytes.as_ptr().add(at).cast(), bytes.len() - at) }
        } else {
            unsafe { libc::read(fd, bytes.as_mut_ptr().add(at).cast(), bytes.len() - at) }
        };
        if n > 0 {
            at += n as usize;
        } else if n == 0 {
            return Err("exec broker EOF".into());
        } else if !matches!(
            io::Error::last_os_error().kind(),
            io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
        ) {
            return Err("exec broker I/O failure".into());
        }
    }
    Ok(())
}
fn take<'a>(bytes: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8]> {
    let end = at
        .checked_add(n)
        .filter(|end| *end <= bytes.len())
        .ok_or("exec packet bounds")?;
    let value = &bytes[*at..end];
    *at = end;
    Ok(value)
}
fn u32_at(bytes: &[u8], at: &mut usize) -> Result<u32> {
    Ok(u32::from_ne_bytes(take(bytes, at, 4)?.try_into().unwrap()))
}
fn u64_at(bytes: &[u8], at: &mut usize) -> Result<u64> {
    Ok(u64::from_ne_bytes(take(bytes, at, 8)?.try_into().unwrap()))
}
fn text_at(bytes: &[u8], at: &mut usize) -> Result<String> {
    let n = u32_at(bytes, at)? as usize;
    if n > 8192 {
        return Err("exec text bounds".into());
    }
    let value = take(bytes, at, n)?;
    if value.contains(&0) {
        return Err("exec text NUL".into());
    }
    String::from_utf8(value.to_vec()).map_err(|_| "exec text UTF8".into())
}
fn decode(bytes: &[u8], snapshot: &[StdioRow], registry: &Registry) -> Result<ExecPlan> {
    let mut at = 0;
    if take(bytes, &mut at, 8)? != b"F1EXEC01" {
        return Err("exec packet magic".into());
    }
    for _ in 0..3 {
        let descriptor = (
            u64_at(bytes, &mut at)?,
            u64_at(bytes, &mut at)?,
            u32_at(bytes, &mut at)? as i32,
            u32_at(bytes, &mut at)? as libc::mode_t,
        );
        if !snapshot.iter().any(|row| row.identity == descriptor) {
            return Err("exec stdio lacks typed provenance".into());
        }
    }
    let path = text_at(bytes, &mut at)?;
    let count = u32_at(bytes, &mut at)? as usize;
    if count > 64 {
        return Err("exec argc bounds".into());
    }
    let mut args = vec![];
    for _ in 0..count {
        args.push(text_at(bytes, &mut at)?);
    }
    if at != bytes.len() {
        return Err("exec trailing bytes".into());
    }
    registry.prepare_exec(Path::new(&path), &args)
}
fn put_string(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    if bytes.len() > 8192 || bytes.contains(&0) || out.len() + bytes.len() + 5 > MAX {
        return Err("exec reply bounds".into());
    }
    out.extend((bytes.len() as u32).to_ne_bytes());
    out.extend(bytes);
    out.push(0);
    Ok(())
}
fn encode(plan: &ExecPlan, life_fd: i32, admit_fd: i32) -> Result<Vec<u8>> {
    let mut bytes = b"F1PLAN01".to_vec();
    if plan.arguments.len() > 64 {
        return Err("own fake argc bounds".into());
    }
    bytes.extend(((plan.arguments.len() + 1) as u32).to_ne_bytes());
    put_string(&mut bytes, plan.program.as_os_str().as_bytes())?;
    for arg in &plan.arguments {
        put_string(&mut bytes, arg.as_bytes())?;
    }
    bytes.extend(2_u32.to_ne_bytes());
    put_string(&mut bytes, format!("F1_LIFE_FD={life_fd}").as_bytes())?;
    put_string(&mut bytes, format!("F1_ADMIT_FD={admit_fd}").as_bytes())?;
    Ok(bytes)
}
fn broker(
    channels: (&OwnedFd, &OwnedFd, &OwnedFd),
    inherited: (i32, i32),
    snapshot: &StdioSnapshot,
    registry: &Registry,
    parent: &dyn AliveLease,
) -> Result<()> {
    let (request, reply, admit) = channels;
    let (life_fd, admit_fd) = inherited;
    let end = Instant::now() + Duration::from_secs(3);
    parent_io(reply.as_raw_fd(), &mut [b'M'], true, end, parent)?;
    if BROKER_PANIC_ON_READY.swap(false, Ordering::AcqRel) {
        while !PREEXEC_READY.load(Ordering::Acquire) {
            parent.before_effect()?;
            if Instant::now() >= end {
                return Err("owned fixture readiness deadline".into());
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("owned fixture exec-broker thread fault");
    }
    let mut length = [0u8; 4];
    parent_io(request.as_raw_fd(), &mut length, false, end, parent)?;
    let length = u32::from_ne_bytes(length) as usize;
    if length > MAX {
        return Err("exec request size".into());
    }
    let mut input = vec![0; length];
    parent_io(request.as_raw_fd(), &mut input, false, end, parent)?;
    let plan = match decode(&input, snapshot.rows(), registry) {
        Ok(plan) => plan,
        Err(_) => {
            // Let frozen std write its exec error and reap this child. Killing
            // it here would falsely turn a refused exec into a successful spawn.
            parent_io(
                reply.as_raw_fd(),
                &mut 0_u32.to_ne_bytes(),
                true,
                end,
                parent,
            )?;
            return Ok(());
        }
    };
    let mut output = encode(&plan, life_fd, admit_fd)?;
    plan.pin.verify()?;
    parent.before_effect()?;
    // fork already transferred the held descriptors to the child. Its actual
    // stdio identity was decoded above; a correct parent close must not mint a
    // new raw-FD grant or retroactively revoke this inherited capability.
    snapshot.verify_transferred()?;
    parent_io(admit.as_raw_fd(), &mut [b'A'], true, end, parent)?;
    parent_io(
        reply.as_raw_fd(),
        &mut (output.len() as u32).to_ne_bytes(),
        true,
        end,
        parent,
    )?;
    parent_io(reply.as_raw_fd(), &mut output, true, end, parent)
}
unsafe fn append(buffer: *mut u8, at: &mut usize, bytes: *const u8, n: usize) -> bool {
    let Some(end) = at.checked_add(n).filter(|end| *end <= MAX) else {
        return false;
    };
    unsafe {
        std::ptr::copy_nonoverlapping(bytes, buffer.add(*at), n);
    }
    *at = end;
    true
}
unsafe fn append_u32(buffer: *mut u8, at: &mut usize, value: u32) -> bool {
    let bytes = value.to_ne_bytes();
    unsafe { append(buffer, at, bytes.as_ptr(), 4) }
}
unsafe fn child_string(buffer: *mut u8, at: &mut usize, p: *const c_char) -> bool {
    if p.is_null() {
        return false;
    }
    let mut n = 0;
    while n <= 8192 && unsafe { p.add(n).read() } != 0 {
        n += 1;
    }
    if n > 8192 {
        return false;
    }
    unsafe { append_u32(buffer, at, n as u32) && append(buffer, at, p.cast(), n) }
}
unsafe fn request(
    buffer: *mut u8,
    path: *const c_char,
    argv: *const *const c_char,
) -> Option<usize> {
    let mut at = 0;
    if !unsafe { append(buffer, &mut at, b"F1EXEC01".as_ptr(), 8) } {
        return None;
    }
    for fd in 0..3 {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fstat(fd, &mut st) } != 0 {
            return None;
        }
        for value in [st.st_dev as u64, st.st_ino] {
            let bytes = value.to_ne_bytes();
            if !unsafe { append(buffer, &mut at, bytes.as_ptr(), 8) } {
                return None;
            }
        }
        if !unsafe {
            append_u32(buffer, &mut at, (flags & libc::O_ACCMODE) as u32)
                && append_u32(buffer, &mut at, (st.st_mode & libc::S_IFMT) as u32)
        } {
            return None;
        }
    }
    if !unsafe { child_string(buffer, &mut at, path) }
        || argv.is_null()
        || unsafe { argv.read() }.is_null()
    {
        return None;
    }
    let count_at = at;
    if !unsafe { append_u32(buffer, &mut at, 0) } {
        return None;
    }
    for (count, index) in (1..=65).enumerate() {
        let value = unsafe { argv.add(index).read() };
        if value.is_null() {
            let bytes = (count as u32).to_ne_bytes();
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.add(count_at), 4);
            }
            return Some(at);
        }
        if count >= 64 || !unsafe { child_string(buffer, &mut at, value) } {
            return None;
        }
    }
    None
}
unsafe fn read_u32(buffer: *const u8, n: usize, at: &mut usize) -> Option<u32> {
    let end = at.checked_add(4).filter(|end| *end <= n)?;
    let mut bytes = [0; 4];
    unsafe {
        std::ptr::copy_nonoverlapping(buffer.add(*at), bytes.as_mut_ptr(), 4);
    }
    *at = end;
    Some(u32::from_ne_bytes(bytes))
}
unsafe fn string_pointer(buffer: *mut u8, n: usize, at: &mut usize) -> Option<*const c_char> {
    let len = unsafe { read_u32(buffer, n, at) }? as usize;
    if len > 8192 {
        return None;
    }
    let end = at
        .checked_add(len)?
        .checked_add(1)
        .filter(|end| *end <= n)?;
    if unsafe { buffer.add(end - 1).read() } != 0 {
        return None;
    }
    for i in *at..end - 1 {
        if unsafe { buffer.add(i).read() } == 0 {
            return None;
        }
    }
    let p = unsafe { buffer.add(*at) };
    *at = end;
    Some(p.cast())
}
/// Only the forkchild branch may enter. No allocation, Mutex, Foundation,
/// producer fallback or background thread runs between fork and exec.
/// # Safety
/// `path` is a readable NUL-terminated C path and `argv` a readable
/// NULL-terminated array of readable NUL-terminated strings, as supplied by the
/// reviewed std exec call. No arbitrary/invalid pointer is accepted as data.
pub unsafe fn exec(path: *const c_char, argv: *const *const c_char) -> i32 {
    if !is_child() {
        unsafe {
            *libc::__error() = libc::EPERM;
        }
        return -1;
    }
    if !life_alive() {
        self_exit();
    }
    let end = clock().saturating_add(LIMIT);
    let mut bytes = [0u8; MAX];
    let Some(n) = (unsafe { request(bytes.as_mut_ptr(), path, argv) }) else {
        unsafe {
            *libc::__error() = libc::EPERM;
        }
        return -1;
    };
    let mut size = (n as u32).to_ne_bytes();
    if !unsafe {
        child_io(
            REQUEST.load(Ordering::Relaxed),
            size.as_mut_ptr(),
            4,
            true,
            end,
        ) && child_io(
            REQUEST.load(Ordering::Relaxed),
            bytes.as_mut_ptr(),
            n,
            true,
            end,
        )
    } {
        self_exit();
    }
    if !unsafe {
        child_io(
            REPLY.load(Ordering::Relaxed),
            size.as_mut_ptr(),
            4,
            false,
            end,
        )
    } {
        self_exit();
    }
    let n = u32::from_ne_bytes(size) as usize;
    if n == 0 {
        unsafe {
            *libc::__error() = libc::EPERM;
        }
        return -1;
    }
    if n > MAX
        || !unsafe {
            child_io(
                REPLY.load(Ordering::Relaxed),
                bytes.as_mut_ptr(),
                n,
                false,
                end,
            )
        }
    {
        self_exit();
    }
    if n < 12 || bytes[..8] != *b"F1PLAN01" {
        self_exit();
    }
    let mut at = 8;
    let Some(count) = (unsafe { read_u32(bytes.as_ptr(), n, &mut at) }) else {
        self_exit()
    };
    if !(1..=65).contains(&count) {
        self_exit();
    }
    let mut av = [std::ptr::null(); 66];
    for slot in av.iter_mut().take(count as usize) {
        let Some(p) = (unsafe { string_pointer(bytes.as_mut_ptr(), n, &mut at) }) else {
            self_exit()
        };
        *slot = p;
    }
    let Some(count) = (unsafe { read_u32(bytes.as_ptr(), n, &mut at) }) else {
        self_exit()
    };
    if count != 2 {
        self_exit();
    }
    let mut env = [std::ptr::null(); 3];
    for slot in env.iter_mut().take(2) {
        let Some(p) = (unsafe { string_pointer(bytes.as_mut_ptr(), n, &mut at) }) else {
            self_exit()
        };
        *slot = p;
    }
    if at != n || !life_alive() {
        self_exit();
    }
    for fd in [LIFE.load(Ordering::Relaxed), ADMIT.load(Ordering::Relaxed)] {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } != 0 {
            self_exit();
        }
    }
    if !unsafe { timer(false) } {
        self_exit();
    }
    let result = unsafe { libc::execve(av[0], av.as_ptr(), env.as_ptr()) };
    let errno = unsafe { *libc::__error() };
    if !unsafe { timer(true) } {
        self_exit();
    }
    unsafe {
        *libc::__error() = errno;
    }
    result
}
