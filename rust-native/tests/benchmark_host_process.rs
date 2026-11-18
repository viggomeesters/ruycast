#![cfg(feature = "test-tools")]
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("owned test fixture parent"),
        );
        assert!(parent.is_absolute() && parent.is_dir());
        let directory = parent.join(format!(
            "benchmark-driver-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        Self(directory)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_benchmark-host-fixture"))
}

struct DriverOutput {
    success: bool,
    stderr: String,
}

fn run(mode: &str, active: &str, idle: &str, timeout: &str) -> (DriverOutput, Value, Fixture) {
    let fixture = Fixture::new();
    let report = fixture.path().join("result.json");
    let copies = ["host", "cli", "probe"].map(|role| {
        let path = fixture.path().join(format!("fixture-{role}-{mode}"));
        fs::copy(fixture_bin(), &path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    });
    let arguments = vec![
        "benchmark-host",
        "--host",
        copies[0].to_str().unwrap(),
        "--cli",
        copies[1].to_str().unwrap(),
        "--probe",
        copies[2].to_str().unwrap(),
        "--out",
        report.to_str().unwrap(),
        "--cold-starts",
        "1",
        "--samples",
        "1",
        "--active-seconds",
        active,
        "--idle-seconds",
        idle,
        "--operation-timeout",
        timeout,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let specification: rustcast_core::Command = serde_json::from_value(serde_json::json!({
        "id":"owned.benchmark-driver","title":"Owned benchmark driver","kind":"script",
        "target":env!("CARGO_BIN_EXE_ruycast-build"),"arguments":arguments,"timeout":30.0
    }))
    .unwrap();
    let completion =
        rustcast_core::script::run(&specification, fixture.path(), None, || false).unwrap();
    let success =
        matches!(completion.end, rustcast_core::script::End::Exited(status) if status.success());
    let output = DriverOutput {
        success,
        stderr: completion.stderr,
    };
    assert!(report.is_file(), "{}", output.stderr);
    let value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    (output, value, fixture)
}

fn events(fixture: &Fixture) -> Vec<String> {
    fs::read_to_string(fixture.path().join("benchmark-events.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn shutdown_requires_acknowledged_quit_and_distinguishes_signals() {
    let (success, report, fixture) = run("ok", "0.05", "0.05", "2");
    assert!(success.success, "{}", success.stderr);
    assert_eq!(report["ok"], true);
    assert_eq!(report["idleEndpoint"]["statusVerified"], true);
    assert_eq!(report["idleEndpoint"]["searchVerified"], true);
    assert_eq!(report["finalShutdown"]["graceful"], true);
    assert_eq!(report["finalShutdown"]["forcedCleanup"], false);
    let success_events = events(&fixture);
    assert_eq!(
        success_events
            .iter()
            .filter(|event| *event == "quit")
            .count(),
        2
    );
    assert!(!success_events.iter().any(|event| event == "terminate"));

    let (terminated, report, fixture) = run("quit_stays_alive", "0", "0", "2");
    assert!(!terminated.success);
    assert_eq!(report["ok"], false);
    assert_eq!(report["cleanup"]["forcedCleanup"], true);
    assert_eq!(report["cleanup"]["exitCode"], 0);
    assert!(events(&fixture).iter().any(|event| event == "terminate"));

    let (killed, report, _) = run("quit_ignores_terminate", "0", "0", "2");
    assert!(!killed.success);
    assert_eq!(report["ok"], false);
    assert_eq!(report["cleanup"]["method"], "kill");
    assert_eq!(report["cleanup"]["exitCode"], -libc::SIGKILL);
}

#[test]
fn child_timeouts_write_failure_receipts() {
    for (mode, phase) in [
        ("init_hang", "init-config"),
        ("cli_hang", "cli-status"),
        ("probe_hang", "active"),
    ] {
        let (output, report, _) = run(mode, "0.05", "0", "2");
        assert!(!output.success, "{mode}");
        assert_eq!(report["ok"], false, "{mode}");
        assert_eq!(report["phase"], phase, "{mode}");
        assert_eq!(report["errorType"], "TimeoutExpired", "{mode}");
        if mode == "probe_hang" {
            assert_eq!(report["active"]["complete"], false);
        }
    }
}

#[test]
fn idle_status_hang_and_drip_cannot_pass_on_healthy_counters() {
    for mode in ["idle_hang", "idle_drip"] {
        let (output, report, fixture) = run(mode, "0.05", "0.2", "2");
        assert!(!output.success, "{mode}");
        assert_eq!(report["ok"], false, "{mode}");
        assert_eq!(report["phase"], "idle-endpoint", "{mode}");
        assert!(events(&fixture)
            .iter()
            .any(|event| event == "idle-status-hang"));
        assert_eq!(report["idle"]["complete"], true);
        assert!(report["idle"]["samples"].as_array().unwrap().len() >= 2);
        if mode == "idle_drip" {
            assert_eq!(report["errorType"], "TimeoutError");
        }
    }
}

#[test]
fn active_socket_failure_keeps_completed_requests_and_partial_samples() {
    let (output, report, _) = run("clipboard_hang", "0.05", "0", "2");
    assert!(!output.success);
    assert_eq!(report["ok"], false);
    assert_eq!(report["phase"], "active");
    assert_eq!(report["active"]["complete"], false);
    assert!(!report["active"]["samples"].as_array().unwrap().is_empty());
    assert_eq!(report["active"]["requests"], 1);
}
