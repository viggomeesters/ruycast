//! Script-probe-only syscall faults, with mandatory real-call cleanup traces.
use libc::{c_char, c_int, c_long, c_void, pid_t};
use std::{
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, Ordering},
};
mod fcntl_abi;
use fcntl_abi::test_fcntl;

unsafe fn active() -> bool {
    libc::strcmp(libc::getprogname(), c"script-probe".as_ptr()) == 0
}
unsafe fn trace(kind: *const c_char, a: c_long, b: c_long) {
    let path = libc::getenv(c"RUYCAST_SCRIPT_TRACE".as_ptr());
    if path.is_null() || !active() {
        return;
    }
    let saved = *libc::__error();
    let mut line = [0 as c_char; 128];
    let n = libc::snprintf(
        line.as_mut_ptr(),
        line.len(),
        c"%s %ld %ld\n".as_ptr(),
        kind,
        a,
        b,
    );
    let fd = libc::open(path, libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND, 0o600);
    if fd >= 0 {
        libc::write(fd, line.as_ptr().cast(), n as usize);
        libc::close(fd);
    }
    *libc::__error() = saved;
}
unsafe fn fault(name: *const c_char) -> bool {
    let mode = libc::getenv(c"RUYCAST_SCRIPT_FAULT".as_ptr());
    active() && !mode.is_null() && libc::strcmp(mode, name) == 0
}
unsafe extern "C" fn checked(fd: c_int, command: c_int, argument: usize) -> c_int {
    let mut info = MaybeUninit::<libc::stat>::uninit();
    if active()
        && fd > 2
        && (command == libc::F_GETFL || command == libc::F_SETFL)
        && libc::fstat(fd, info.as_mut_ptr()) == 0
        && info.assume_init().st_mode & libc::S_IFMT == libc::S_IFIFO
        && ((command == libc::F_GETFL && fault(c"getfl".as_ptr()))
            || (command == libc::F_SETFL && fault(c"setfl".as_ptr())))
    {
        trace(c"FAULT".as_ptr(), command.into(), fd.into());
        *libc::__error() = libc::EIO;
        return -1;
    }
    match command {
        libc::F_GETFD | libc::F_GETFL | libc::F_GETOWN => libc::fcntl(fd, command),
        libc::F_GETPATH => libc::fcntl(fd, command, argument as *mut c_void),
        libc::F_DUPFD
        | libc::F_DUPFD_CLOEXEC
        | libc::F_SETFD
        | libc::F_SETFL
        | libc::F_SETOWN
        | libc::F_NOCACHE
        | libc::F_RDAHEAD
        | libc::F_SETNOSIGPIPE => libc::fcntl(fd, command, argument as c_int),
        _ => {
            *libc::__error() = libc::EINVAL;
            -1
        }
    }
}
unsafe extern "C" fn shim_kill(pid: pid_t, signal: c_int) -> c_int {
    trace(c"KILL".as_ptr(), pid.into(), signal.into());
    if pid < 0 && fault(c"kill-eperm".as_ptr()) {
        trace(c"FAULTKILL".as_ptr(), pid.into(), signal.into());
        *libc::__error() = libc::EPERM;
        return -1;
    }
    libc::kill(pid, signal)
}
unsafe extern "C" fn shim_proc_listpids(
    kind: u32,
    info: u32,
    buffer: *mut c_void,
    size: c_int,
) -> c_int {
    if fault(c"list-error".as_ptr()) {
        trace(c"FAULTLIST".as_ptr(), kind.into(), info.into());
        *libc::__error() = libc::EIO;
        return 0;
    }
    if fault(c"list-truncated".as_ptr()) {
        trace(c"FAULTLIST".as_ptr(), kind.into(), info.into());
        return size;
    }
    libc::proc_listpids(kind, info, buffer, size)
}
unsafe extern "C" fn shim_proc_pidinfo(
    pid: c_int,
    flavor: c_int,
    argument: u64,
    buffer: *mut c_void,
    size: c_int,
) -> c_int {
    if fault(c"info-error".as_ptr()) {
        trace(c"FAULTINFO".as_ptr(), pid.into(), flavor.into());
        *libc::__error() = libc::EPERM;
        return 0;
    }
    libc::proc_pidinfo(pid, flavor, argument, buffer, size)
}
unsafe extern "C" fn shim_waitpid(pid: pid_t, status: *mut c_int, options: c_int) -> pid_t {
    let result = libc::waitpid(pid, status, options);
    trace(c"WAIT".as_ptr(), pid.into(), result.into());
    result
}
unsafe extern "C" fn shim_waitid(
    kind: libc::idtype_t,
    id: libc::id_t,
    info: *mut libc::siginfo_t,
    options: c_int,
) -> c_int {
    static INJECTED: AtomicBool = AtomicBool::new(false);
    if (fault(c"wait-error".as_ptr()) || fault(c"wait-eintr".as_ptr()))
        && !INJECTED.swap(true, Ordering::Relaxed)
    {
        trace(c"FAULTWAIT".as_ptr(), id.into(), options.into());
        *libc::__error() = if fault(c"wait-error".as_ptr()) {
            libc::EIO
        } else {
            libc::EINTR
        };
        return -1;
    }
    libc::waitid(kind, id, info, options)
}
unsafe extern "C" fn shim_read(fd: c_int, buffer: *mut c_void, count: usize) -> libc::ssize_t {
    static INJECTED: AtomicBool = AtomicBool::new(false);
    let mut info = MaybeUninit::<libc::stat>::uninit();
    if fd > 2
        && active()
        && libc::fstat(fd, info.as_mut_ptr()) == 0
        && info.assume_init().st_mode & libc::S_IFMT == libc::S_IFIFO
        && libc::fcntl(fd, libc::F_GETFL) & libc::O_NONBLOCK != 0
        && (fault(c"read-error".as_ptr()) || fault(c"read-eintr".as_ptr()))
        && !INJECTED.swap(true, Ordering::Relaxed)
    {
        trace(c"FAULTREAD".as_ptr(), fd.into(), 0);
        *libc::__error() = if fault(c"read-error".as_ptr()) {
            libc::EIO
        } else {
            libc::EINTR
        };
        return -1;
    }
    libc::read(fd, buffer, count)
}
#[repr(C)]
struct Interpose {
    replacement: *const c_void,
    original: *const c_void,
}
// Immutable loader metadata; Rust never dereferences these addresses.
unsafe impl Sync for Interpose {}
macro_rules! interpose {
    ($name:ident, $replacement:ident, $original:ident) => {
        #[used]
        #[link_section = "__DATA,__interpose"]
        static $name: Interpose = Interpose {
            replacement: $replacement as *const c_void,
            original: libc::$original as *const c_void,
        };
    };
}
interpose!(FCNTL, test_fcntl, fcntl);
interpose!(KILL, shim_kill, kill);
interpose!(WAITPID, shim_waitpid, waitpid);
interpose!(WAITID, shim_waitid, waitid);
interpose!(READ, shim_read, read);
interpose!(LISTPIDS, shim_proc_listpids, proc_listpids);
interpose!(PIDINFO, shim_proc_pidinfo, proc_pidinfo);
