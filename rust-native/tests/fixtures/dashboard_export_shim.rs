//! Faults only inside the marked disposable dashboard export directory.
use libc::{c_char, c_int, c_void};
use std::mem::MaybeUninit;
mod fcntl_abi;
use fcntl_abi::test_fcntl;

unsafe fn joined(home: *const c_char, name: *const c_char) -> [c_char; libc::PATH_MAX as usize] {
    let mut path = [0; libc::PATH_MAX as usize];
    libc::snprintf(path.as_mut_ptr(), path.len(), c"%s/%s".as_ptr(), home, name);
    path
}
unsafe fn own() -> *const c_char {
    let home = libc::getenv(c"RUYCAST_DASHBOARD_FIXTURE_HOME".as_ptr());
    if home.is_null() || *home != b'/' as c_char {
        return std::ptr::null();
    }
    let marker = joined(home, c".ruycast-test-fixture".as_ptr());
    if libc::access(marker.as_ptr(), libc::F_OK) == 0 {
        home
    } else {
        std::ptr::null()
    }
}
unsafe fn exists(home: *const c_char, name: *const c_char) -> bool {
    libc::access(joined(home, name).as_ptr(), libc::F_OK) == 0
}
unsafe fn mark(home: *const c_char, name: *const c_char) {
    let path = joined(home, name);
    let fd = libc::open(
        path.as_ptr(),
        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
        0o600,
    );
    if fd >= 0 {
        libc::write(fd, b"1".as_ptr().cast(), 1);
        libc::close(fd);
    }
}
unsafe fn wait_for(home: *const c_char, name: *const c_char) -> c_int {
    for _ in 0..10000 {
        if exists(home, name) {
            return 0;
        }
        libc::usleep(1000);
    }
    *libc::__error() = libc::ETIMEDOUT;
    -1
}
unsafe fn sync_fault(fd: c_int) -> c_int {
    let home = own();
    let mut path = [0 as c_char; libc::PATH_MAX as usize];
    let mut info = MaybeUninit::<libc::stat>::uninit();
    if !home.is_null()
        && libc::fcntl(fd, libc::F_GETPATH, path.as_mut_ptr()) == 0
        && libc::fstat(fd, info.as_mut_ptr()) == 0
    {
        let info = info.assume_init();
        let directory = joined(home, c"export".as_ptr());
        let length = libc::strlen(directory.as_ptr());
        if libc::strncmp(path.as_ptr(), directory.as_ptr(), length) == 0
            && path[length] == b'/' as c_char
            && info.st_mode & libc::S_IFMT == libc::S_IFREG
        {
            if exists(home, c"hold-export-file-sync".as_ptr()) {
                mark(home, c"export-file-sync-entered".as_ptr());
                if wait_for(home, c"export-file-sync-release".as_ptr()) != 0 {
                    return -1;
                }
            }
            if exists(home, c"fail-export-file-sync".as_ptr()) {
                mark(home, c"export-file-sync-failed".as_ptr());
                *libc::__error() = libc::EIO;
                return -1;
            }
        }
        if libc::strcmp(path.as_ptr(), directory.as_ptr()) == 0
            && info.st_mode & libc::S_IFMT == libc::S_IFDIR
            && exists(home, c"fail-export-parent-sync".as_ptr())
        {
            mark(home, c"export-parent-sync-failed".as_ptr());
            *libc::__error() = libc::EIO;
            return -1;
        }
    }
    0
}
unsafe extern "C" fn test_fsync(fd: c_int) -> c_int {
    if sync_fault(fd) != 0 {
        -1
    } else {
        libc::fsync(fd)
    }
}
unsafe extern "C" fn checked(fd: c_int, command: c_int, argument: usize) -> c_int {
    if command == libc::F_FULLFSYNC || command == libc::F_BARRIERFSYNC {
        return if sync_fault(fd) != 0 {
            -1
        } else {
            libc::fcntl(fd, command)
        };
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
unsafe extern "C" fn test_renameat(
    from: c_int,
    source: *const c_char,
    to: c_int,
    destination: *const c_char,
) -> c_int {
    let home = own();
    let mut path = [0 as c_char; libc::PATH_MAX as usize];
    if !home.is_null()
        && libc::strcmp(destination, c"diagnostics.json".as_ptr()) == 0
        && libc::fcntl(to, libc::F_GETPATH, path.as_mut_ptr()) == 0
    {
        let directory = joined(home, c"export".as_ptr());
        if libc::strcmp(path.as_ptr(), directory.as_ptr()) == 0 {
            if exists(home, c"hold-export-rename".as_ptr()) {
                mark(home, c"export-rename-entered".as_ptr());
                if wait_for(home, c"export-rename-release".as_ptr()) != 0 {
                    return -1;
                }
            }
            if exists(home, c"fail-export-rename".as_ptr()) {
                mark(home, c"export-rename-failed".as_ptr());
                *libc::__error() = libc::EIO;
                return -1;
            }
        }
    }
    libc::renameat(from, source, to, destination)
}
#[repr(C)]
struct Interpose {
    replacement: *const c_void,
    original: *const c_void,
}
// Immutable loader metadata, never dereferenced by Rust.
unsafe impl Sync for Interpose {}
#[used]
#[link_section = "__DATA,__interpose"]
static FSYNC: Interpose = Interpose {
    replacement: test_fsync as *const c_void,
    original: libc::fsync as *const c_void,
};
#[used]
#[link_section = "__DATA,__interpose"]
static FCNTL: Interpose = Interpose {
    replacement: test_fcntl as *const c_void,
    original: libc::fcntl as *const c_void,
};
#[used]
#[link_section = "__DATA,__interpose"]
static RENAME: Interpose = Interpose {
    replacement: test_renameat as *const c_void,
    original: libc::renameat as *const c_void,
};
