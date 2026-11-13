//! Real script syscall failures and owned process cleanup.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
struct OwnedDir(PathBuf);
impl OwnedDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("owned fixture parent"),
        );
        assert!(parent.is_absolute() && parent.is_dir());
        let path = parent.join(format!(
            "script-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for OwnedDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run_fault(home: &Path, mode: &str, marker: &str) {
    run_fault_with(home, mode, marker, false, false);
}
fn run_fault_with(home: &Path, mode: &str, marker: &str, echo: bool, success: bool) {
    let probe = Path::new(env!("CARGO_BIN_EXE_script-probe"));
    let shim = std::env::var_os("RUYCAST_TEST_SCRIPT_FAULT_SHIM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            probe
                .parent()
                .unwrap()
                .join("examples/libscript_fault_shim.dylib")
        });
    assert!(
        shim.is_absolute() && shim.is_file(),
        "build script-fault-shim first"
    );
    let trace = home.join("syscalls.trace");
    let mut command = json!({"id":"script.fixture","title":"Script fixture","kind":"script","target":if echo {"/bin/echo"} else {"/bin/sleep"},"arguments":[if echo {"hello"} else {"10"}]});
    if mode == "kill-eperm" {
        command["timeout"] = json!(0.1);
    }
    let request = json!({"home":home,"command":command});
    let mut c = Command::new(probe);
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "DYLD_INSERT_LIBRARIES",
        "RUYCAST_HOME",
        "RUSTCAST_HOME",
        "RUYCAST_CONFIG_DIR",
        "RUSTCAST_CONFIG_DIR",
    ] {
        c.env_remove(name);
    }
    c.env("DYLD_INSERT_LIBRARIES", shim)
        .env("RUYCAST_SCRIPT_TRACE", &trace)
        .env("RUYCAST_SCRIPT_FAULT", mode);
    let mut child = c.spawn().unwrap();
    let written = child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes());
    let output = child_output::wait(child, Duration::from_secs(5));
    written.expect("send fixture request");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["noChildren"], true);
    if success {
        assert_eq!(v["result"]["response"]["ok"], true);
    } else {
        assert_eq!(v["result"]["ok"], false);
    }
    let text = fs::read_to_string(trace).unwrap();
    let lines: Vec<Vec<&str>> = text
        .lines()
        .map(|line| line.split_whitespace().collect())
        .collect();
    assert!(
        lines.iter().any(|line| line[0] == marker),
        "fault was not injected: {text}"
    );
    let reaped: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| {
            if line[0] == "WAIT" {
                let pid = line[2].parse::<i32>().unwrap();
                (pid > 0).then_some((i, pid))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(reaped.len(), 1, "{text}");
    let (index, leader) = reaped[0];
    assert!(
        lines[..index]
            .iter()
            .any(|line| line[0] == "KILL" && line[1].parse::<i32>().unwrap() == -leader),
        "{text}"
    );
    assert!(
        !lines[index + 1..]
            .iter()
            .any(|line| line[0] == "KILL" && line[1].parse::<i32>().unwrap().abs() == leader),
        "{text}"
    );
    assert_eq!(unsafe { libc::kill(leader, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
#[test]
fn nonblocking_getfl_failure_reaps() {
    let h = OwnedDir::new();
    run_fault(&h.0, "getfl", "FAULT");
}
#[test]
fn nonblocking_setfl_failure_reaps() {
    let h = OwnedDir::new();
    run_fault(&h.0, "setfl", "FAULT");
}
#[test]
fn read_failure_reaps() {
    let h = OwnedDir::new();
    run_fault(&h.0, "read-error", "FAULTREAD");
}
#[test]
fn wait_failure_reaps() {
    let h = OwnedDir::new();
    run_fault(&h.0, "wait-error", "FAULTWAIT");
}
#[test]
fn interrupted_read_recovers() {
    let h = OwnedDir::new();
    run_fault_with(&h.0, "read-eintr", "FAULTREAD", true, true);
}
#[test]
fn interrupted_wait_recovers() {
    let h = OwnedDir::new();
    run_fault_with(&h.0, "wait-eintr", "FAULTWAIT", true, true);
}
#[test]
fn live_group_permission_error_is_not_success() {
    let h = OwnedDir::new();
    run_fault_with(&h.0, "kill-eperm", "FAULTKILL", false, false);
}
#[test]
fn group_inventory_failure_is_not_success() {
    let h = OwnedDir::new();
    for (mode, marker) in [
        ("list-error", "FAULTLIST"),
        ("list-truncated", "FAULTLIST"),
        ("info-error", "FAULTINFO"),
    ] {
        let trace = h.0.join("syscalls.trace");
        if trace.exists() {
            fs::remove_file(trace).unwrap();
        }
        run_fault_with(&h.0, mode, marker, true, false);
    }
}
