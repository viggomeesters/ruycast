//! ABI and side-effect checks against the actually loaded fcntl interpose.
use std::{
    fs,
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
    path::PathBuf,
    time::{Duration, Instant},
};
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn clear() {
    for name in [
        "DIRECTORY",
        "MARKER",
        "RELEASE",
        "DELAY_MS",
        "DELAY_ONLY",
        "MUTATE",
    ] {
        std::env::remove_var(format!("RUYCAST_TEST_FSYNC_{name}"));
    }
    std::env::remove_var("RUYCAST_TEST_SELECTOR_JSON");
    std::env::remove_var("RUYCAST_TEST_MUTATION_TARGET");
}

fn main() {
    clear();
    let parent = PathBuf::from(std::env::args_os().nth(1).expect("owned parent"))
        .canonicalize()
        .unwrap();
    let home = parent.join(format!("fcntl-probe-{}", std::process::id()));
    fs::create_dir(&home).unwrap();
    let _cleanup = Cleanup(home.clone());
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(home.join("file"))
        .unwrap();
    let fd = file.as_raw_fd();
    assert!(
        unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0,
        "no-argument forwarding"
    );
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
        0
    );
    assert_ne!(
        unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
    let mut path = [0 as libc::c_char; 4096];
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_GETPATH, path.as_mut_ptr()) },
        0,
        "pointer forwarding"
    );
    assert_eq!(
        unsafe { std::ffi::CStr::from_ptr(path.as_ptr()) }.to_bytes(),
        home.join("file").as_os_str().as_encoded_bytes()
    );
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 64) };
    assert!(duplicate >= 64, "integer forwarding");
    unsafe {
        libc::close(duplicate);
    }
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_DUPFD, -1 as libc::c_int) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EINVAL)
    );
    assert_eq!(unsafe { libc::fcntl(fd, i32::MAX) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EINVAL)
    );

    for mode in [
        "plain",
        "edit",
        "replace",
        "selector",
        "target",
        "release",
        "delay",
        "timeout",
        "delay-only",
    ] {
        clear();
        let directory = home.join(mode);
        fs::create_dir(&directory).unwrap();
        let config = directory.join("config.json");
        fs::write(&config, b"original").unwrap();
        let marker = home.join(format!("{mode}.marker"));
        let dir = fs::File::open(&directory).unwrap();
        let original = unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_FULLFSYNC) };
        let original_errno = std::io::Error::last_os_error().raw_os_error();
        std::env::set_var("RUYCAST_TEST_FSYNC_DIRECTORY", &directory);
        std::env::set_var("RUYCAST_TEST_FSYNC_MARKER", &marker);
        if ["edit", "replace", "selector", "target"].contains(&mode) {
            std::env::set_var("RUYCAST_TEST_FSYNC_MUTATE", mode);
        }
        std::env::set_var("RUYCAST_TEST_SELECTOR_JSON", r#"{"directory":"/owned"}"#);
        std::env::set_var("RUYCAST_TEST_MUTATION_TARGET", &config);
        let release = home.join("release.signal");
        let releaser = if mode == "release" {
            std::env::set_var("RUYCAST_TEST_FSYNC_RELEASE", &release);
            let (release, marker) = (release.clone(), marker.clone());
            Some(std::thread::spawn(move || {
                let end = Instant::now() + Duration::from_secs(2);
                while !marker.exists() {
                    assert!(Instant::now() < end);
                    std::thread::sleep(Duration::from_millis(1));
                }
                fs::write(release, b"go").unwrap();
            }))
        } else {
            None
        };
        if mode == "delay" {
            std::env::set_var("RUYCAST_TEST_FSYNC_DELAY_MS", "30");
        }
        if mode == "timeout" {
            std::env::set_var("RUYCAST_TEST_FSYNC_RELEASE", home.join("never-released"));
        }
        if mode == "delay-only" {
            std::env::set_var("RUYCAST_TEST_FSYNC_DELAY_ONLY", "1");
            std::env::set_var("RUYCAST_TEST_FSYNC_DELAY_MS", "30");
        }
        let started = Instant::now();
        assert_eq!(
            unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_FULLFSYNC) },
            if mode == "delay-only" { original } else { -1 },
            "loaded injection"
        );
        if mode != "delay-only" || original == -1 {
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                if mode == "delay-only" {
                    original_errno
                } else {
                    Some(if mode == "timeout" {
                        libc::ETIMEDOUT
                    } else {
                        libc::EIO
                    })
                }
            );
        }
        if mode == "timeout" {
            assert!(started.elapsed() >= Duration::from_secs(5));
        }
        if mode == "delay" || mode == "delay-only" {
            assert!(started.elapsed() >= Duration::from_millis(25));
        }
        if let Some(thread) = releaser {
            thread.join().unwrap();
        }
        assert_eq!(fs::read(&marker).unwrap(), b"injected");
        if ["edit", "replace", "target"].contains(&mode) {
            assert_eq!(fs::read(&config).unwrap(), b"external fixture edit");
        } else {
            assert_eq!(fs::read(&config).unwrap(), b"original");
        }
        if mode == "replace" {
            assert_eq!(
                fs::read(directory.join("published.copy")).unwrap(),
                b"original"
            );
        }
        if mode == "selector" {
            assert_eq!(
                fs::read(directory.join("config-location.json")).unwrap(),
                br#"{"directory":"/owned"}"#
            );
        }
        // Mutations apply only once, even though later directory synchronizations still fail.
        fs::write(&config, b"after-first").unwrap();
        std::env::remove_var("RUYCAST_TEST_FSYNC_DELAY_MS");
        std::env::remove_var("RUYCAST_TEST_FSYNC_DELAY_ONLY");
        assert_eq!(
            unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_BARRIERFSYNC) },
            -1
        );
        assert_eq!(fs::read(&config).unwrap(), b"after-first");
    }
    clear();
    println!("PASS: fcntl argumentless/integer/pointer/signed/unknown ABI; EIO/marker/edit/replace/selector/target/release/delay/timeout/delay-only/first-only");
}
