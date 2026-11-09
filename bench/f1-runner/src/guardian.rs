//! Pinned-fake containment only. No arbitrary process/tree authority.
//! Collector retains the only writer; descendants inherit read capability only.
use std::{
    ffi::CString,
    io,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

unsafe extern "C" {
    fn posix_spawn_file_actions_addinherit_np(
        actions: *mut libc::posix_spawn_file_actions_t,
        fd: libc::c_int,
    ) -> libc::c_int;
}
fn cv(code: i32) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code))
    }
}
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let r = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let w = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    for fd in [&r, &w] {
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok((r, w))
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct GrantIdentity {
    device: u64,
    inode: u64,
    kind: u32,
    access: i32,
    uid: u32,
}
impl GrantIdentity {
    fn of(fd: RawFd) -> io::Result<Self> {
        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fstat(fd, info.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let info = unsafe { info.assume_init() };
        Ok(Self {
            device: info.st_dev as u64,
            inode: info.st_ino,
            kind: u32::from(info.st_mode & libc::S_IFMT),
            access: flags & libc::O_ACCMODE,
            uid: info.st_uid,
        })
    }
    fn aliases(self, other: Self) -> bool {
        self.device == other.device && self.inode == other.inode && self.kind == other.kind
    }
}
/// Fresh private read capability. Only this module's existing pipe factory can
/// create it; no raw/OwnedFd adoption or clone grants inheritance authority.
pub struct OwnedReadGrant {
    read: OwnedFd,
    pin: GrantIdentity,
}
impl AsRawFd for OwnedReadGrant {
    fn as_raw_fd(&self) -> RawFd {
        self.read.as_raw_fd()
    }
}
impl AsFd for OwnedReadGrant {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.read.as_fd()
    }
}
impl OwnedReadGrant {
    fn verify(&self) -> io::Result<GrantIdentity> {
        let fd = self.as_raw_fd();
        let actual = GrantIdentity::of(fd)?;
        let descriptor_flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if fd <= 2
            || actual != self.pin
            || actual.kind != u32::from(libc::S_IFIFO)
            || actual.access != libc::O_RDONLY
            || descriptor_flags < 0
            || descriptor_flags & libc::FD_CLOEXEC == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "private read grant identity/access/owner/CLOEXEC differs",
            ));
        }
        Ok(actual)
    }
}
pub fn private_grant_pipe() -> io::Result<(OwnedReadGrant, OwnedFd)> {
    let (read, writer) = pipe()?;
    let pin = GrantIdentity::of(read.as_raw_fd())?;
    let grant = OwnedReadGrant { read, pin };
    grant.verify()?;
    Ok((grant, writer))
}
fn validate_read_grants(
    grants: &[&OwnedReadGrant],
    io: &SpawnIo<'_>,
    life: RawFd,
    admit: RawFd,
) -> io::Result<()> {
    if grants.len() > 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at most two private read grants",
        ));
    }
    let life_identity = GrantIdentity::of(life)?;
    let admit_identity = GrantIdentity::of(admit)?;
    let mut identities = vec![(life, life_identity), (admit, admit_identity)];
    for fd in io.descriptors {
        identities.push((fd, GrantIdentity::of(fd)?));
    }
    for grant in grants {
        let actual = grant.verify()?;
        let fd = grant.as_raw_fd();
        if actual.uid != life_identity.uid
            || actual.uid != admit_identity.uid
            || identities
                .iter()
                .any(|(other, identity)| *other == fd || identity.aliases(actual))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "private read grant aliases reserved capability or differs in owner",
            ));
        }
        identities.push((fd, actual));
    }
    Ok(())
}

/// Only successful posix_spawn creates this unreaped identity. Never construct
/// it from an arbitrary PID and never signal after waitpid has consumed it.
pub struct Process {
    pid: Option<i32>,
    status: Option<i32>,
}
impl Process {
    pub fn id(&self) -> i32 {
        self.pid.expect("live own identity")
    }
    pub fn wait(&mut self, budget: Duration) -> io::Result<i32> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        let end = Instant::now() + budget;
        loop {
            let mut status = 0;
            let pid = self.pid.unwrap();
            let n = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if n == pid {
                self.pid = None;
                self.status = Some(status);
                return Ok(status);
            }
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            if Instant::now() >= end {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "own process did not exit",
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
    pub fn terminate(&mut self) -> io::Result<i32> {
        if self.pid.is_some() {
            unsafe {
                libc::kill(-self.id(), libc::SIGKILL);
            }
        }
        self.wait(Duration::from_secs(3))
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

/// No pre_exec/fork callback exists. Kernel file actions admit only explicitly
/// listed fds; all writer/cancel descriptors stay CLOEXEC in the collector.
pub fn spawn(
    program: &Path,
    args: &[String],
    env: &[(String, String)],
    inherit: &[RawFd],
) -> io::Result<Process> {
    spawn_configured(program, args, env, inherit, None)
}

/// Borrowed collector descriptors. Spawn duplicates these to stdio in the child;
/// the collector retains ownership and its process-wide cwd is never changed.
pub struct SpawnIo<'a> {
    pub cwd: &'a Path,
    pub descriptors: [RawFd; 3],
}

fn spawn_configured(
    program: &Path,
    args: &[String],
    env: &[(String, String)],
    inherit: &[RawFd],
    io: Option<&SpawnIo<'_>>,
) -> io::Result<Process> {
    let program = CString::new(program.as_os_str().as_bytes())?;
    let mut strings = vec![program.clone()];
    strings.extend(
        args.iter()
            .map(|s| CString::new(s.as_bytes()))
            .collect::<Result<Vec<_>, _>>()?,
    );
    let mut argv = strings
        .iter()
        .map(|s| s.as_ptr() as *mut libc::c_char)
        .collect::<Vec<_>>();
    argv.push(std::ptr::null_mut());
    let environment = env
        .iter()
        .map(|(k, v)| CString::new(format!("{k}={v}")))
        .collect::<Result<Vec<_>, _>>()?;
    let mut envp = environment
        .iter()
        .map(|s| s.as_ptr() as *mut libc::c_char)
        .collect::<Vec<_>>();
    envp.push(std::ptr::null_mut());
    let mut actions: libc::posix_spawn_file_actions_t = std::ptr::null_mut();
    let mut attr: libc::posix_spawnattr_t = std::ptr::null_mut();
    unsafe {
        cv(libc::posix_spawn_file_actions_init(&mut actions))?;
    }
    struct Actions(*mut libc::c_void);
    impl Drop for Actions {
        fn drop(&mut self) {
            unsafe {
                libc::posix_spawn_file_actions_destroy(&mut self.0);
            }
        }
    }
    let mut actions = Actions(actions);
    unsafe {
        cv(libc::posix_spawnattr_init(&mut attr))?;
    }
    struct Attr(*mut libc::c_void);
    impl Drop for Attr {
        fn drop(&mut self) {
            unsafe {
                libc::posix_spawnattr_destroy(&mut self.0);
            }
        }
    }
    let mut attr = Attr(attr);
    unsafe {
        cv(libc::posix_spawnattr_setpgroup(&mut attr.0, 0))?;
        cv(libc::posix_spawnattr_setflags(
            &mut attr.0,
            (libc::POSIX_SPAWN_SETPGROUP | libc::POSIX_SPAWN_CLOEXEC_DEFAULT) as i16,
        ))?;
        if let Some(io) = io {
            let cwd = CString::new(io.cwd.as_os_str().as_bytes())?;
            cv(libc::posix_spawn_file_actions_addchdir_np(
                &mut actions.0,
                cwd.as_ptr(),
            ))?;
            for (target, source) in io.descriptors.iter().copied().enumerate() {
                if source <= 2 || inherit.contains(&source) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "stdio source aliases reserved capability",
                    ));
                }
                cv(libc::posix_spawn_file_actions_adddup2(
                    &mut actions.0,
                    source,
                    target as i32,
                ))?;
            }
        } else {
            for fd in [0, 1, 2] {
                cv(posix_spawn_file_actions_addinherit_np(&mut actions.0, fd))?;
            }
        }
        for fd in inherit.iter().copied() {
            cv(posix_spawn_file_actions_addinherit_np(&mut actions.0, fd))?;
        }
    }
    let mut pid = 0;
    unsafe {
        cv(libc::posix_spawn(
            &mut pid,
            program.as_ptr(),
            &actions.0,
            &attr.0,
            argv.as_ptr(),
            envp.as_ptr(),
        ))?;
    }
    if pid <= 1 {
        return Err(io::Error::other(
            "spawn returned no valid own child identity",
        ));
    }
    Ok(Process {
        pid: Some(pid),
        status: None,
    })
}

fn event(ident: usize, filter: i16, fflags: u32) -> libc::kevent {
    libc::kevent {
        ident,
        filter,
        flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT | libc::EV_RECEIPT,
        fflags,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}
pub struct Endpoint {
    pub process: Process,
    cancel: Option<OwnedFd>,
    watcher: Option<thread::JoinHandle<()>>,
}
impl Endpoint {
    /// Admission happens only after kernel watching is installed on an unreaped
    /// own identity. The caller cannot copy/retain this function's life writer.
    pub fn start(
        program: &Path,
        args: &[String],
        base_env: &[(String, String)],
    ) -> io::Result<Self> {
        Self::start_configured(program, args, base_env, None, &[])
    }
    pub fn start_with_io(
        program: &Path,
        args: &[String],
        base_env: &[(String, String)],
        io: &SpawnIo<'_>,
    ) -> io::Result<Self> {
        Self::start_configured(program, args, base_env, Some(io), &[])
    }
    pub fn start_with_read_grants(
        program: &Path,
        args: &[String],
        base_env: &[(String, String)],
        io: &SpawnIo<'_>,
        grants: &[&OwnedReadGrant],
    ) -> io::Result<Self> {
        Self::start_configured(program, args, base_env, Some(io), grants)
    }
    fn start_configured(
        program: &Path,
        args: &[String],
        base_env: &[(String, String)],
        io: Option<&SpawnIo<'_>>,
        grants: &[&OwnedReadGrant],
    ) -> io::Result<Self> {
        if grants.len() > 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "at most two private read grants",
            ));
        }
        let (life_r, life_w) = pipe()?;
        let (admit_r, admit_w) = pipe()?;
        let mut env = base_env
            .iter()
            .cloned()
            .collect::<std::collections::BTreeMap<_, _>>();
        env.insert("F1_LIFE_FD".into(), life_r.as_raw_fd().to_string());
        env.insert("F1_ADMIT_FD".into(), admit_r.as_raw_fd().to_string());
        let env = env.into_iter().collect::<Vec<_>>();
        if !grants.is_empty() {
            validate_read_grants(
                grants,
                io.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "explicit private grants require owned stdio",
                    )
                })?,
                life_r.as_raw_fd(),
                admit_r.as_raw_fd(),
            )?;
        }
        let mut inherited = vec![life_r.as_raw_fd(), admit_r.as_raw_fd()];
        inherited.extend(grants.iter().map(|grant| grant.as_raw_fd()));
        let process = spawn_configured(program, args, &env, &inherited, io)?;
        for grant in grants {
            grant.verify()?;
        }
        drop(life_r);
        drop(admit_r);
        let (cancel_r, cancel_w) = pipe()?;
        let queue = unsafe { libc::kqueue() };
        if queue < 0 {
            return Err(io::Error::last_os_error());
        }
        let queue = unsafe { OwnedFd::from_raw_fd(queue) };
        if unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let changes = [
            event(process.id() as usize, libc::EVFILT_PROC, libc::NOTE_EXIT),
            event(cancel_r.as_raw_fd() as usize, libc::EVFILT_READ, 0),
        ];
        let mut receipts: [libc::kevent; 2] = unsafe { std::mem::zeroed() };
        let immediate = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let installed = unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                changes.as_ptr(),
                changes.len() as i32,
                receipts.as_mut_ptr(),
                2,
                &immediate,
            )
        };
        if installed < 0 {
            return Err(io::Error::last_os_error());
        }
        if installed != 2
            || receipts
                .iter()
                .any(|r| r.flags & libc::EV_ERROR == 0 || r.data != 0)
        {
            return Err(io::Error::other(
                "own endpoint/cancel kernel watches not synchronously installed",
            ));
        }
        let watcher = thread::spawn(move || {
            let _writer = life_w;
            let _cancel = cancel_r;
            let mut result: libc::kevent = unsafe { std::mem::zeroed() };
            loop {
                let n = unsafe {
                    libc::kevent(
                        queue.as_raw_fd(),
                        std::ptr::null(),
                        0,
                        &mut result,
                        1,
                        std::ptr::null(),
                    )
                };
                if n >= 0 {
                    #[cfg(test)]
                    if result.filter == libc::EVFILT_READ {
                        let mut byte = 0_u8;
                        let n = unsafe {
                            libc::read(_cancel.as_raw_fd(), (&mut byte as *mut u8).cast(), 1)
                        };
                        if n == 1 && byte == b'P' {
                            panic!("owned watcher fault injection");
                        }
                    }
                    break;
                }
                if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    break;
                }
            }
        });
        let endpoint = Self {
            process,
            cancel: Some(cancel_w),
            watcher: Some(watcher),
        };
        if unsafe { libc::write(admit_w.as_raw_fd(), b"A".as_ptr().cast(), 1) } != 1 {
            return Err(io::Error::last_os_error());
        }
        drop(admit_w);
        Ok(endpoint)
    }
    pub fn cancel(&mut self) {
        drop(self.cancel.take());
        if let Some(thread) = self.watcher.take() {
            let _ = thread.join();
        }
    }
    #[cfg(test)]
    pub fn crash_watcher(&self) -> io::Result<()> {
        let fd = self
            .cancel
            .as_ref()
            .ok_or_else(|| io::Error::other("watcher already closed"))?
            .as_raw_fd();
        if unsafe { libc::write(fd, b"P".as_ptr().cast(), 1) } != 1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.cancel();
        let _ = self.process.terminate();
    }
}

#[derive(Clone)]
pub struct Guard {
    alive: Arc<AtomicBool>,
    fd: RawFd,
}
fn self_group_exit() -> ! {
    let pid = unsafe { libc::getpid() };
    if unsafe { libc::getpgrp() } == pid && unsafe { libc::kill(0, libc::SIGKILL) } == 0 {
        loop {
            unsafe {
                libc::pause();
            }
        }
    }
    unsafe { libc::_exit(89) }
}
impl Guard {
    pub fn is_alive(&self) -> bool {
        let mut poll = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        self.alive.load(Ordering::Acquire) && unsafe { libc::poll(&mut poll, 1, 0) } == 0
    }
    /// The fake calls this as its first operation, before FS/output/spawn effects.
    /// Every independently grouped fake grandchild must call the same bootstrap.
    pub fn bootstrap() -> io::Result<Self> {
        if unsafe { libc::getpid() } != unsafe { libc::getpgrp() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fake is not its own group leader",
            ));
        }
        let fd: RawFd = std::env::var("F1_LIFE_FD")
            .map_err(|_| {
                io::Error::new(io::ErrorKind::PermissionDenied, "life capability missing")
            })?
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "life fd invalid"))?;
        if fd <= 2 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "stdio is not an owned life capability",
            ));
        }
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if flags < 0
            || flags & libc::O_ACCMODE != libc::O_RDONLY
            || unsafe { libc::fstat(fd, &mut st) } != 0
            || st.st_mode & libc::S_IFMT != libc::S_IFIFO
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "life capability is not an own read pipe",
            ));
        }
        // Endpoint-only admission is consumed/closed before it can spawn descendants.
        if let Ok(value) = std::env::var("F1_ADMIT_FD") {
            let admit: RawFd = value
                .parse()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "admission fd invalid"))?;
            if admit <= 2 || admit == fd {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "admission capability aliases life/stdio",
                ));
            }
            let end = Instant::now() + Duration::from_secs(10);
            loop {
                let mut events = [
                    libc::pollfd {
                        fd,
                        events: libc::POLLIN | libc::POLLHUP,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: admit,
                        events: libc::POLLIN | libc::POLLHUP,
                        revents: 0,
                    },
                ];
                let remaining = end.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    self_group_exit();
                }
                let n = unsafe {
                    libc::poll(
                        events.as_mut_ptr(),
                        2,
                        remaining.as_millis().min(i32::MAX as u128) as i32,
                    )
                };
                if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if n <= 0 || events[0].revents != 0 {
                    self_group_exit();
                }
                if events[1].revents != 0 {
                    break;
                }
            }
            let mut byte = 0_u8;
            if unsafe { libc::read(admit, (&mut byte as *mut u8).cast(), 1) } != 1 || byte != b'A' {
                self_group_exit();
            }
            unsafe {
                libc::close(admit);
            }
            // No process-global environment mutation. Every authorised child
            // spawn overrides both lease keys with its new descriptor pair.
        }
        let alive = Arc::new(AtomicBool::new(true));
        let worker = alive.clone();
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, 0) } != 0 {
            self_group_exit();
        }
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(0);
        #[cfg(test)]
        let panic_on_start = std::env::var("F1_TEST_GUARDIAN_PANIC").ok().as_deref() == Some("yes");
        thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                #[cfg(test)]
                if panic_on_start {
                    panic!("owned fake guardian fault injection");
                }
                if ready_tx.send(()).is_err() {
                    return;
                }
                let mut byte = 0_u8;
                loop {
                    let n = unsafe { libc::read(fd, (&mut byte as *mut u8).cast(), 1) };
                    if n == 0 {
                        break;
                    }
                    if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
            });
            let _ = result;
            worker.store(false, Ordering::Release);
            self_group_exit();
        });
        if ready_rx.recv_timeout(Duration::from_secs(10)).is_err() {
            self_group_exit();
        }
        Ok(Self { alive, fd })
    }
    pub fn before_effect(&self) -> io::Result<()> {
        let mut poll = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, 0) } != 0 {
            self_group_exit();
        }
        if !self.alive.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "owner lease closed",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod read_grant_tests {
    use super::*;
    fn own_stdio() -> [OwnedFd; 3] {
        let (input, _writer) = pipe().unwrap();
        let (_reader, output) = pipe().unwrap();
        let error = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC) };
        assert!(error > 2);
        [input, output, unsafe { OwnedFd::from_raw_fd(error) }]
    }
    #[test]
    fn only_unique_private_read_grants_can_be_inherited() {
        let (one, _w1) = private_grant_pipe().unwrap();
        let (two, _w2) = private_grant_pipe().unwrap();
        let (life, _life_writer) = pipe().unwrap();
        let (admit, _admit_writer) = pipe().unwrap();
        let stdio = own_stdio();
        let io = SpawnIo {
            cwd: Path::new("/"),
            descriptors: stdio.each_ref().map(AsRawFd::as_raw_fd),
        };
        validate_read_grants(&[&one, &two], &io, life.as_raw_fd(), admit.as_raw_fd()).unwrap();
        assert!(validate_read_grants(&[&one], &io, one.as_raw_fd(), admit.as_raw_fd()).is_err());
        assert!(validate_read_grants(&[&one], &io, life.as_raw_fd(), one.as_raw_fd()).is_err());
        assert!(
            validate_read_grants(&[&one, &one], &io, life.as_raw_fd(), admit.as_raw_fd()).is_err()
        );
        assert!(validate_read_grants(
            &[&one, &two, &one],
            &io,
            life.as_raw_fd(),
            admit.as_raw_fd()
        )
        .is_err());
        let alias_io = SpawnIo {
            cwd: io.cwd,
            descriptors: [one.as_raw_fd(), io.descriptors[1], io.descriptors[2]],
        };
        assert!(
            validate_read_grants(&[&one], &alias_io, life.as_raw_fd(), admit.as_raw_fd()).is_err()
        );
        let duplicate = unsafe { libc::fcntl(one.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
        assert!(duplicate > 2);
        let duplicate = unsafe { OwnedFd::from_raw_fd(duplicate) };
        let duplicate = OwnedReadGrant {
            pin: GrantIdentity::of(duplicate.as_raw_fd()).unwrap(),
            read: duplicate,
        };
        assert!(validate_read_grants(
            &[&one, &duplicate],
            &io,
            life.as_raw_fd(),
            admit.as_raw_fd()
        )
        .is_err());
    }
    #[test]
    fn stale_rebound_wrong_owner_access_and_kind_are_refused() {
        let (mut grant, _writer) = private_grant_pipe().unwrap();
        let original = grant.pin;
        grant.pin.uid = grant.pin.uid.wrapping_add(1);
        assert!(grant.verify().is_err());
        grant.pin = original;
        assert_eq!(
            unsafe { libc::fcntl(grant.as_raw_fd(), libc::F_SETFD, 0) },
            0
        );
        assert!(grant.verify().is_err());
        assert_eq!(
            unsafe { libc::fcntl(grant.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
            0
        );
        grant.verify().unwrap();
        let (other, _writer2) = pipe().unwrap();
        assert_eq!(
            unsafe { libc::dup2(other.as_raw_fd(), grant.as_raw_fd()) },
            grant.as_raw_fd()
        );
        assert_eq!(
            unsafe { libc::fcntl(grant.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
            0
        );
        assert!(grant.verify().is_err());
        let (_reader, writer) = pipe().unwrap();
        let wrong_access = OwnedReadGrant {
            pin: GrantIdentity::of(writer.as_raw_fd()).unwrap(),
            read: writer,
        };
        assert!(wrong_access.verify().is_err());
        let regular =
            unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        assert!(regular > 2);
        let regular = unsafe { OwnedFd::from_raw_fd(regular) };
        let wrong_kind = OwnedReadGrant {
            pin: GrantIdentity::of(regular.as_raw_fd()).unwrap(),
            read: regular,
        };
        assert!(wrong_kind.verify().is_err());
    }
}
