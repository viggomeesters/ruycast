//! Owned test-process fcntl fault injection; no foreign source compiler.
use libc::{c_char, c_int, c_void};
use std::mem::MaybeUninit;

mod fcntl_abi;
use fcntl_abi::test_fcntl;

unsafe extern "C" fn checked(fd: c_int, command: c_int, argument: usize) -> c_int {
    if command == libc::F_FULLFSYNC || command == libc::F_BARRIERFSYNC {
        let directory = libc::getenv(c"RUYCAST_TEST_FSYNC_DIRECTORY".as_ptr());
        let mut path = [0 as c_char; 4096];
        let mut info = MaybeUninit::<libc::stat>::uninit();
        if !directory.is_null()
            && libc::fstat(fd, info.as_mut_ptr()) == 0
            && info.assume_init().st_mode & libc::S_IFMT == libc::S_IFDIR
            && libc::fcntl(fd, libc::F_GETPATH, path.as_mut_ptr()) == 0
            && libc::strcmp(path.as_ptr(), directory) == 0
        {
            let marker = libc::getenv(c"RUYCAST_TEST_FSYNC_MARKER".as_ptr());
            let first = !marker.is_null() && libc::access(marker, libc::F_OK) != 0;
            if !marker.is_null() {
                write_file(marker, b"injected", true);
            }
            let release = libc::getenv(c"RUYCAST_TEST_FSYNC_RELEASE".as_ptr());
            if first && !release.is_null() {
                let mut attempts = 0;
                while libc::access(release, libc::F_OK) != 0 {
                    attempts += 1;
                    if attempts > 5000 {
                        *libc::__error() = libc::ETIMEDOUT;
                        return -1;
                    }
                    libc::usleep(1000);
                }
            }
            let delay = libc::getenv(c"RUYCAST_TEST_FSYNC_DELAY_MS".as_ptr());
            if first && !delay.is_null() {
                let ms = libc::strtol(delay, std::ptr::null_mut(), 10);
                if ms > 0 && ms <= 1000 {
                    libc::usleep(ms as libc::useconds_t * 1000);
                }
            }
            if !libc::getenv(c"RUYCAST_TEST_FSYNC_DELAY_ONLY".as_ptr()).is_null() {
                return libc::fcntl(fd, command);
            }
            let mutation = libc::getenv(c"RUYCAST_TEST_FSYNC_MUTATE".as_ptr());
            if first && !mutation.is_null() {
                let mut target = [0 as c_char; 4096];
                let mut backup = [0 as c_char; 4096];
                if libc::strcmp(mutation, c"selector".as_ptr()) == 0 {
                    libc::snprintf(
                        target.as_mut_ptr(),
                        target.len(),
                        c"%s/config-location.json".as_ptr(),
                        directory,
                    );
                    let json = libc::getenv(c"RUYCAST_TEST_SELECTOR_JSON".as_ptr());
                    let payload = if json.is_null() {
                        &[][..]
                    } else {
                        std::slice::from_raw_parts(json.cast::<u8>(), libc::strlen(json))
                    };
                    write_file(target.as_ptr(), payload, true);
                } else if libc::strcmp(mutation, c"target".as_ptr()) == 0 {
                    let name = libc::getenv(c"RUYCAST_TEST_MUTATION_TARGET".as_ptr());
                    if !name.is_null() {
                        write_file(name, b"external fixture edit", false);
                    }
                } else {
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
                    write_file(target.as_ptr(), b"external fixture edit", true);
                }
            }
            *libc::__error() = libc::EIO;
            return -1;
        }
        return libc::fcntl(fd, command);
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

unsafe fn write_file(path: *const c_char, payload: &[u8], create: bool) {
    let flags = libc::O_WRONLY | libc::O_TRUNC | if create { libc::O_CREAT } else { 0 };
    let fd = libc::open(path, flags, 0o600);
    if fd >= 0 {
        libc::write(fd, payload.as_ptr().cast::<c_void>(), payload.len());
        libc::close(fd);
    }
}

#[repr(C)]
struct Interpose {
    replacement: *const c_void,
    original: *const c_void,
}
// Immutable loader metadata. Rust never dereferences these pointers.
unsafe impl Sync for Interpose {}
#[used]
#[link_section = "__DATA,__interpose"]
static INTERPOSE: Interpose = Interpose {
    replacement: test_fcntl as *const c_void,
    original: libc::fcntl as *const c_void,
};
