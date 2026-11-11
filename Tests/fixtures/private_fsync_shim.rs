//! Fault injection only in an explicitly configured disposable test process.
use libc::{c_int, c_void};
use std::mem::MaybeUninit;

unsafe extern "C" fn private_test_fsync(fd: c_int) -> c_int {
    let directory = libc::getenv(c"RUYCAST_TEST_FSYNC_DIRECTORY".as_ptr());
    let mut path = [0 as libc::c_char; 4096];
    let mut info = MaybeUninit::<libc::stat>::uninit();
    if !directory.is_null()
        && libc::fstat(fd, info.as_mut_ptr()) == 0
        && info.assume_init().st_mode & libc::S_IFMT == libc::S_IFDIR
        && libc::fcntl(fd, libc::F_GETPATH, path.as_mut_ptr()) == 0
        && libc::strcmp(path.as_ptr(), directory) == 0
    {
        let marker = libc::getenv(c"RUYCAST_TEST_FSYNC_MARKER".as_ptr());
        if !marker.is_null() {
            let m = libc::open(
                marker,
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                0o600,
            );
            if m >= 0 {
                libc::write(m, b"injected".as_ptr().cast::<c_void>(), 8);
                libc::close(m);
            }
        }
        let mutation = libc::getenv(c"RUYCAST_TEST_FSYNC_MUTATE".as_ptr());
        if !mutation.is_null() {
            let mut target = [0 as libc::c_char; 4096];
            let mut backup = [0 as libc::c_char; 4096];
            libc::snprintf(
                target.as_mut_ptr(),
                target.len(),
                c"%s/config.json".as_ptr(),
                directory,
            );
            if libc::strcmp(mutation, c"replace".as_ptr()) == 0 {
                libc::snprintf(
                    backup.as_mut_ptr(),
                    backup.len(),
                    c"%s/published.copy".as_ptr(),
                    directory,
                );
                libc::rename(target.as_ptr(), backup.as_ptr());
            }
            let changed = libc::open(
                target.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                0o600,
            );
            if changed >= 0 {
                libc::write(
                    changed,
                    b"external fixture edit".as_ptr().cast::<c_void>(),
                    21,
                );
                libc::close(changed);
            }
        }
        *libc::__error() = libc::EIO;
        return -1;
    }
    libc::fsync(fd)
}

#[repr(C)]
struct Interpose {
    replacement: unsafe extern "C" fn(c_int) -> c_int,
    original: unsafe extern "C" fn(c_int) -> c_int,
}

#[used]
#[link_section = "__DATA,__interpose"]
static PRIVATE_INTERPOSE: Interpose = Interpose {
    replacement: private_test_fsync,
    original: libc::fsync,
};
