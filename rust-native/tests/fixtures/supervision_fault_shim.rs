//! Fault only the disposable parent probe, never its Rust CLI helper.
use libc::{c_int, pid_t};
use std::sync::atomic::{AtomicBool, Ordering};

unsafe fn active() -> bool {
    libc::strcmp(libc::getprogname(), c"supervision-probe".as_ptr()) == 0
}
unsafe extern "C" fn test_waitpid(pid: pid_t, status: *mut c_int, options: c_int) -> pid_t {
    static INJECTED: AtomicBool = AtomicBool::new(false);
    let marker = libc::getenv(c"RUYCAST_SUPERVISION_WAIT_MARKER".as_ptr());
    if !marker.is_null()
        && active()
        && pid > 0
        && options == 0
        && !INJECTED.swap(true, Ordering::Relaxed)
    {
        let fd = libc::open(
            marker,
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o600,
        );
        if fd >= 0 {
            libc::write(fd, b"injected".as_ptr().cast(), 8);
            libc::close(fd);
        }
        *libc::__error() = libc::EIO;
        return -1;
    }
    libc::waitpid(pid, status, options)
}
unsafe extern "C" fn test_kill(pid: pid_t, signal: c_int) -> c_int {
    let path = libc::getenv(c"RUYCAST_SUPERVISION_KILL_TRACE".as_ptr());
    if !path.is_null() && active() {
        let mut line = [0 as libc::c_char; 80];
        let n = libc::snprintf(
            line.as_mut_ptr(),
            line.len(),
            c"%d %d\n".as_ptr(),
            pid,
            signal,
        );
        let fd = libc::open(path, libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND, 0o600);
        if fd >= 0 {
            libc::write(fd, line.as_ptr().cast(), n as usize);
            libc::close(fd);
        }
    }
    libc::kill(pid, signal)
}
type WaitPid = unsafe extern "C" fn(pid_t, *mut c_int, c_int) -> pid_t;
type Kill = unsafe extern "C" fn(pid_t, c_int) -> c_int;
#[repr(C)]
struct WaitPair {
    replacement: WaitPid,
    original: WaitPid,
}
#[repr(C)]
struct KillPair {
    replacement: Kill,
    original: Kill,
}
#[used]
#[link_section = "__DATA,__interpose"]
static WAIT: WaitPair = WaitPair {
    replacement: test_waitpid,
    original: libc::waitpid,
};
#[used]
#[link_section = "__DATA,__interpose"]
static KILL: KillPair = KillPair {
    replacement: test_kill,
    original: libc::kill,
};
