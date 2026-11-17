//! Bounded shutdown and failure reporting for owned benchmark host processes.
use serde::Serialize;
use std::{
    io,
    process::{Child, ExitStatus},
    time::{Duration, Instant},
};
pub mod rpc;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Cleanup {
    pub pid: u32,
    pub graceful: bool,
    pub forced_cleanup: bool,
    pub method: &'static str,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quit_error: Option<String>,
}

/// Request a bounded graceful quit, then TERM and KILL only this owned child.
/// A successful quit acknowledgment is not enough: the process must exit 0.
pub fn shutdown<F>(mut child: Child, timeout: Duration, request_quit: F) -> Cleanup
where
    F: FnOnce(Instant) -> Result<(), String>,
{
    let pid = child.id();
    let deadline = Instant::now() + timeout;
    let mut quit_error = match request_quit(deadline) {
        Ok(()) if Instant::now() < deadline => None,
        Ok(()) => Some("Quit acknowledgment arrived after its deadline".to_owned()),
        Err(error) => Some(error),
    };
    let acknowledged = quit_error.is_none();
    if acknowledged {
        match wait_until(&mut child, deadline) {
            Ok(Some(status)) if status.success() => {
                return cleanup(pid, true, false, "quit", status, None);
            }
            Ok(Some(status)) => return cleanup(pid, false, false, "quit", status, None),
            Err(_) => return cleanup_unknown(pid, "wait-error", false, quit_error.clone(), child),
            Ok(None) => quit_error = Some("Host did not exit within the quit deadline".into()),
        }
    }

    // `try_wait` reaps an exited child. Never signal its numeric PID after
    // reaping: the operating system may already have reused that identifier.
    match child.try_wait() {
        Ok(Some(status)) => return cleanup(pid, false, false, "exited", status, quit_error),
        Err(_) => return cleanup_unknown(pid, "wait-error", false, quit_error, child),
        Ok(None) => {}
    }

    #[cfg(unix)]
    let term_sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) == 0 };
    #[cfg(not(unix))]
    let term_sent = false;
    let term_deadline = Instant::now() + Duration::from_millis(150);
    if term_sent {
        match wait_until(&mut child, term_deadline) {
            Ok(Some(status)) => return cleanup(pid, false, true, "terminate", status, quit_error),
            Err(_) => return cleanup_unknown(pid, "wait-error", true, quit_error, child),
            Ok(None) => {}
        }
    }
    let _ = child.kill();
    let status = match wait_until(&mut child, Instant::now() + Duration::from_secs(2)) {
        Ok(status) => status,
        Err(_) => return cleanup_unknown(pid, "wait-error", true, quit_error.clone(), child),
    };
    if status.is_none() {
        return cleanup_unknown(pid, "kill-reap-timeout", true, quit_error, child);
    }
    Cleanup {
        pid,
        graceful: false,
        forced_cleanup: true,
        method: "kill",
        exit_code: status.as_ref().and_then(ExitStatus::code),
        signal: status.as_ref().and_then(signal),
        quit_error,
    }
}

fn wait_until(child: &mut Child, deadline: Instant) -> io::Result<Option<ExitStatus>> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => return Ok(None),
            Err(error) => return Err(error),
        }
    }
}

fn cleanup_unknown(
    pid: u32,
    method: &'static str,
    forced_cleanup: bool,
    quit_error: Option<String>,
    child: Child,
) -> Cleanup {
    // Retain ownership until it exits, without risking a signal to a reused PID.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Cleanup {
        pid,
        graceful: false,
        forced_cleanup,
        method,
        exit_code: None,
        signal: None,
        quit_error,
    }
}

fn cleanup(
    pid: u32,
    graceful: bool,
    forced_cleanup: bool,
    method: &'static str,
    status: ExitStatus,
    quit_error: Option<String>,
) -> Cleanup {
    Cleanup {
        pid,
        graceful,
        forced_cleanup,
        method,
        exit_code: status.code(),
        signal: signal(&status),
        quit_error,
    }
}

#[cfg(unix)]
fn signal(status: &ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}
#[cfg(not(unix))]
fn signal(_: &ExitStatus) -> Option<i32> {
    None
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    phase: &'a str,
    #[serde(rename = "errorType")]
    error_type: &'a str,
    error: &'a str,
    cleanup: &'a Cleanup,
}

/// Atomically publish a failure receipt using the private-storage writer.
pub fn publish_failure(
    path: &std::path::Path,
    phase: &str,
    error_type: &str,
    error: &str,
    cleanup: &Cleanup,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(&Failure {
        ok: false,
        phase,
        error_type,
        error,
        cleanup,
    })
    .map_err(io::Error::other)?;
    crate::private_storage::write(path, &bytes, true).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        process::{Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::Duration,
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    #[cfg(unix)]
    static TERM_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    #[cfg(unix)]
    extern "C" fn record_term(_: libc::c_int) {
        TERM_SEEN.store(true, Ordering::Relaxed);
    }

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    impl Fixture {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    fn fixture() -> Fixture {
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("owned fixture parent"),
        );
        assert!(parent.is_absolute());
        let meta = fs::symlink_metadata(&parent).expect("fixture parent exists");
        assert!(meta.is_dir() && !meta.file_type().is_symlink());
        let root = parent.join(format!(
            "benchmark-host-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("create owned fixture");
        Fixture(root)
    }

    fn launch(root: &std::path::Path, mode: &str) -> Child {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "benchmark_host::tests::owned_host_entry",
                "--nocapture",
            ])
            .env("RUYCAST_BENCHMARK_OWNED_HOST", mode)
            .env("RUYCAST_BENCHMARK_FIXTURE", root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if root.join("ready").exists() {
                return child;
            }
            if let Ok(Some(status)) = child.try_wait() {
                panic!("owned host exited before ready: {status}");
            }
            thread::sleep(Duration::from_millis(4));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("owned host readiness timed out");
    }

    fn quit_request(root: PathBuf) -> impl FnOnce(Instant) -> Result<(), String> {
        move |deadline| {
            fs::write(root.join("quit"), b"quit").map_err(|e| e.to_string())?;
            while Instant::now() < deadline {
                if root.join("quit-ack").exists() {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(4));
            }
            Err("quit acknowledgment timed out".into())
        }
    }

    #[test]
    fn graceful_quit_requires_zero_exit() {
        let root = fixture();
        let child = launch(root.path(), "quit");
        let result = shutdown(
            child,
            Duration::from_secs(2),
            quit_request(root.path().to_owned()),
        );
        assert_eq!(
            (
                result.graceful,
                result.forced_cleanup,
                result.method,
                result.exit_code
            ),
            (true, false, "quit", Some(0))
        );
    }

    #[test]
    fn acknowledged_nonzero_exit_is_failed_without_forced_cleanup() {
        let root = fixture();
        let child = launch(root.path(), "quit-fails");
        let result = shutdown(
            child,
            Duration::from_secs(2),
            quit_request(root.path().to_owned()),
        );
        assert!(!result.graceful && !result.forced_cleanup);
        assert_eq!((result.method, result.exit_code), ("quit", Some(7)));
    }

    #[test]
    fn acknowledged_quit_that_stays_alive_is_forced() {
        let root = fixture();
        let child = launch(root.path(), "stays-alive");
        let result = shutdown(
            child,
            Duration::from_millis(80),
            quit_request(root.path().to_owned()),
        );
        assert!(!result.graceful && result.forced_cleanup);
        assert_eq!(result.method, "terminate");
        assert!(result
            .quit_error
            .as_deref()
            .unwrap()
            .contains("quit deadline"));
    }

    #[cfg(unix)]
    #[test]
    fn forced_termination_is_failure_even_when_child_exits_zero() {
        let root = fixture();
        let child = launch(root.path(), "term-exits-zero");
        let result = shutdown(
            child,
            Duration::from_millis(80),
            quit_request(root.path().to_owned()),
        );
        assert!(!result.graceful && result.forced_cleanup);
        assert_eq!((result.method, result.exit_code), ("terminate", Some(0)));
    }

    #[test]
    fn ignored_terminate_is_killed_and_never_reported_graceful() {
        let root = fixture();
        let child = launch(root.path(), "ignore-term");
        let result = shutdown(
            child,
            Duration::from_secs(2),
            quit_request(root.path().to_owned()),
        );
        assert!(!result.graceful && result.forced_cleanup);
        assert_eq!(result.method, "kill");
        #[cfg(unix)]
        assert_eq!(result.signal, Some(libc::SIGKILL));
    }

    #[test]
    fn failure_receipt_keeps_cleanup_outcome() {
        let root = fixture();
        let dest = root.path().join("failure.json");
        let cleanup = Cleanup {
            pid: 42,
            graceful: false,
            forced_cleanup: true,
            method: "kill",
            exit_code: None,
            signal: Some(9),
            quit_error: None,
        };
        publish_failure(&dest, "shutdown", "Timeout", "host did not stop", &cleanup).unwrap();
        let receipt: serde_json::Value = serde_json::from_slice(&fs::read(dest).unwrap()).unwrap();
        assert_eq!(receipt["ok"], false);
        assert_eq!(receipt["phase"], "shutdown");
        assert_eq!(receipt["cleanup"]["forcedCleanup"], true);
        assert_eq!(receipt["cleanup"]["signal"], 9);
    }

    #[test]
    #[ignore]
    fn owned_host_entry() {
        let Some(mode) = std::env::var_os("RUYCAST_BENCHMARK_OWNED_HOST") else {
            return;
        };
        let root = PathBuf::from(std::env::var_os("RUYCAST_BENCHMARK_FIXTURE").unwrap());
        if mode == "term-exits-zero" {
            TERM_SEEN.store(false, Ordering::Relaxed);
            #[cfg(unix)]
            unsafe {
                libc::signal(libc::SIGTERM, record_term as libc::sighandler_t);
            }
        }
        if mode == "ignore-term" {
            #[cfg(unix)]
            unsafe {
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
            }
        }
        fs::write(root.join("ready"), b"ready").unwrap();
        while !root.join("quit").exists() {
            thread::sleep(Duration::from_millis(3));
        }
        fs::write(root.join("quit-ack"), b"ack").unwrap();
        match mode.to_string_lossy().as_ref() {
            "quit" => return,
            "quit-fails" => std::process::exit(7),
            "stays-alive" => thread::sleep(Duration::from_secs(5)),
            "term-exits-zero" => {
                while !TERM_SEEN.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(3));
                }
            }
            "ignore-term" => thread::sleep(Duration::from_secs(5)),
            _ => panic!("unknown owned host mode"),
        }
    }
}
