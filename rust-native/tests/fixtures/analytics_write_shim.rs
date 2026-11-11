//! Real renameat interception, restricted to the marked disposable analytics profile.
use libc::{c_char, c_int};

type RenameAt = unsafe extern "C" fn(c_int, *const c_char, c_int, *const c_char) -> c_int;

unsafe extern "C" fn test_renameat(
    from: c_int,
    source: *const c_char,
    to: c_int,
    destination: *const c_char,
) -> c_int {
    let home = libc::getenv(c"RUYCAST_ANALYTICS_FIXTURE_HOME".as_ptr());
    let mut path = [0 as c_char; libc::PATH_MAX as usize];
    if !home.is_null()
        && *home == b'/' as c_char
        && libc::strcmp(destination, c"analytics.json".as_ptr()) == 0
        && libc::fcntl(to, libc::F_GETPATH, path.as_mut_ptr()) == 0
    {
        let profile = joined(home, c"%s/p".as_ptr());
        let marker = joined(home, c"%s/.ruycast-test-fixture".as_ptr());
        if libc::strcmp(path.as_ptr(), profile.as_ptr()) == 0
            && libc::access(marker.as_ptr(), libc::F_OK) == 0
        {
            let hold = joined(home, c"%s/hold-analytics-write".as_ptr());
            let entered = joined(home, c"%s/analytics-write-entered".as_ptr());
            let release = joined(home, c"%s/analytics-write-release".as_ptr());
            let fail = joined(home, c"%s/fail-analytics-write".as_ptr());
            if libc::access(hold.as_ptr(), libc::F_OK) == 0 {
                write_marker(entered.as_ptr(), b"held");
                let mut attempts = 0;
                while libc::access(release.as_ptr(), libc::F_OK) != 0 {
                    attempts += 1;
                    if attempts > 15000 {
                        *libc::__error() = libc::ETIMEDOUT;
                        return -1;
                    }
                    libc::usleep(1000);
                }
            }
            if libc::access(fail.as_ptr(), libc::F_OK) == 0 {
                let marker = joined(home, c"%s/analytics-write-failed".as_ptr());
                write_marker(marker.as_ptr(), b"EIO");
                *libc::__error() = libc::EIO;
                return -1;
            }
        }
    }
    libc::renameat(from, source, to, destination)
}

unsafe fn joined(home: *const c_char, format: *const c_char) -> [c_char; libc::PATH_MAX as usize] {
    let mut path = [0; libc::PATH_MAX as usize];
    libc::snprintf(path.as_mut_ptr(), path.len(), format, home);
    path
}

unsafe fn write_marker(path: *const c_char, bytes: &[u8]) {
    let fd = libc::open(path, libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC, 0o600);
    if fd >= 0 {
        libc::write(fd, bytes.as_ptr().cast(), bytes.len());
        libc::close(fd);
    }
}

// Typed fields also enforce identical signatures for replacement and original.
#[repr(C)]
struct Interpose {
    replacement: RenameAt,
    original: RenameAt,
}
#[used]
#[link_section = "__DATA,__interpose"]
static INTERPOSE: Interpose = Interpose {
    replacement: test_renameat,
    original: libc::renameat,
};
