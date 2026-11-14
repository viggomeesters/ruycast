//! Owned analytics runtime and source-built write shim.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Bounded second-instance probe shares owned host support"
)]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{Config, Host};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

fn host() -> Host {
    let mut h = Host::new(Config::current(), false);
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_ruycast-native"));
    let shim = std::env::var_os("RUYCAST_TEST_ANALYTICS_SHIM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            executable
                .parent()
                .unwrap()
                .join("examples/libanalytics_write_shim.dylib")
        });
    assert!(
        shim.is_absolute() && shim.is_file(),
        "Build analytics-write-shim first: {shim:?}"
    );
    let owned = h.base.join("analytics-write-shim.dylib");
    fs::copy(shim, &owned).unwrap();
    h.env
        .insert("DYLD_INSERT_LIBRARIES".into(), owned.into_os_string());
    let home = h.base.as_os_str().to_owned();
    h.env.insert("RUYCAST_ANALYTICS_FIXTURE_HOME".into(), home);
    h.start();
    h
}
#[test]
fn idle_cli_settings_flush_wakes_owner_and_preserves_existing_file() {
    let h = host();
    let started = Instant::now();
    let paused = h.rpc(&json!({"op":"analytics.settings","enabled":false}));
    assert_eq!(paused["ok"], true, "{paused}");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(paused["result"]["enabled"], false);
    let stored: Value =
        serde_json::from_slice(&fs::read(h.profile.join("analytics.json")).unwrap()).unwrap();
    assert_eq!(stored["enabled"], false);
    assert_eq!(h.cli(&["analytics", "resume"])["result"]["enabled"], true);
    assert_eq!(h.cli(&["analytics", "reset", "--confirm"])["ok"], true);
    let exported = h.cli(&["analytics", "export"])["result"].clone();
    assert!(exported.get("commands").is_none());
    assert_eq!(exported["attempts"], 0);
}
#[test]
fn raw_analytics_request_casts_match_foundation_without_config_coercion() {
    let h = host();
    for (value, expected) in [
        (json!(0), false),
        (json!(1), true),
        (json!(2), true),
        (json!(-1), true),
        (json!(1.5), true),
        (json!("1"), true),
        (Value::Null, true),
    ] {
        let result = h.rpc(&json!({"op":"analytics.settings","enabled":value}));
        assert_eq!(result["ok"], true);
        assert_eq!(result["result"]["enabled"], expected);
    }
    for value in [json!(true), json!(1.0), json!(1.5), json!("7")] {
        let result = h.rpc(&json!({"op":"analytics.settings","retentionDays":value}));
        assert_eq!(result["ok"], true);
        assert_eq!(result["result"]["retentionDays"], 1);
    }
    assert_eq!(
        h.rpc(&json!({"op":"analytics.settings","retentionDays":false}))["ok"],
        false
    );
    assert_eq!(
        h.rpc(&json!({"op":"analytics.reset","confirmed":0}))["ok"],
        false
    );
    assert_eq!(
        h.rpc(&json!({"op":"analytics.reset","confirmed":1}))["ok"],
        true
    );
}

#[path = "support/child_output.rs"]
mod child_output;
struct Release(PathBuf);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::write(&self.0, b"");
    }
}
fn entered(h: &Host) {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        if h.base.join("analytics-write-entered").exists() {
            return;
        }
        assert!(
            Instant::now() < end,
            "No actual analytics publication hold observed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn pending_setting(
    h: &Host,
    timeout: Duration,
) -> std::sync::mpsc::Receiver<std::io::Result<Value>> {
    let socket = h.socket();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = runtime_host::status_rpc::request(
            &socket,
            &json!({"op":"analytics.settings","enabled":false}),
            Instant::now() + timeout,
        );
        let _ = tx.send(result);
    });
    rx
}
#[test]
fn held_io_keeps_owner_responsive_and_reports_failed_flush() {
    let h = host();
    fs::write(h.base.join("hold-analytics-write"), b"").unwrap();
    let fault = h.base.join("fail-analytics-write");
    fs::write(&fault, b"").unwrap();
    let release = Release(h.base.join("analytics-write-release"));
    let pending = pending_setting(&h, Duration::from_secs(6));
    entered(&h);
    assert!(matches!(
        pending.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert_eq!(h.rpc(&json!({"op":"status"}))["ok"], true);
    assert_eq!(
        h.rpc(&json!({"op":"analytics"}))["result"]["enabled"],
        false
    );
    drop(release);
    let result = pending
        .recv_timeout(Duration::from_secs(4))
        .unwrap()
        .unwrap();
    assert_eq!(result["ok"], false);
    assert!(result["storageError"]
        .as_str()
        .unwrap()
        .contains("could not be saved"));
    assert!(h.base.join("analytics-write-failed").exists());
    fs::remove_file(fault).unwrap();
    assert_eq!(
        h.rpc(&json!({"op":"analytics.settings","enabled":true}))["ok"],
        true
    );
    let stored: Value =
        serde_json::from_slice(&fs::read(h.profile.join("analytics.json")).unwrap()).unwrap();
    assert_eq!(stored["enabled"], true);
}
#[test]
fn shutdown_does_not_confirm_a_flush_that_is_still_held() {
    let mut h = host();
    fs::write(h.base.join("hold-analytics-write"), b"").unwrap();
    fs::write(h.base.join("fail-analytics-write"), b"").unwrap();
    let release = Release(h.base.join("analytics-write-release"));
    let pending = pending_setting(&h, Duration::from_secs(9));
    entered(&h);
    assert_eq!(h.rpc(&json!({"op":"quit"}))["ok"], true);
    let result = pending
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["applied"], true);
    assert_eq!(result["persistence"], "unknown");
    assert!(h.process.as_mut().unwrap().try_wait().unwrap().is_none());
    let child = h
        .command(&h.settings.host)
        .arg("--headless-test")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let second = child_output::wait(child, Duration::from_secs(3));
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("already running"));
    drop(release);
    let output = h.wait_exit(Duration::from_secs(5), false);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(h.base.join("analytics-write-failed").exists());
}

#[test]
fn full_flush_queue_does_not_hide_already_applied_setting() {
    use std::io::Write;
    let h = host();
    fs::write(h.base.join("hold-analytics-write"), b"").unwrap();
    let release = Release(h.base.join("analytics-write-release"));
    let first = pending_setting(&h, Duration::from_secs(9));
    entered(&h);
    for retention in 2..17 {
        let mut stream = h.connect(Duration::from_secs(3));
        serde_json::to_writer(
            &mut stream,
            &json!({"op":"analytics.settings","retentionDays":retention}),
        )
        .unwrap();
        stream.write_all(b"\n").unwrap();
        let end = Instant::now() + Duration::from_secs(2);
        loop {
            if h.rpc(&json!({"op":"analytics"}))["result"]["retentionDays"] == retention {
                break;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(stream);
    }
    let result = h.rpc(&json!({"op":"analytics.settings","retentionDays":20}));
    assert_eq!(result["ok"], false);
    assert_eq!(result["applied"], true);
    assert_eq!(result["persistence"], "unknown");
    assert!(result["reason"].as_str().unwrap().contains("busy"));
    assert_eq!(
        h.rpc(&json!({"op":"analytics"}))["result"]["retentionDays"],
        20
    );
    drop(release);
    assert_eq!(
        first.recv_timeout(Duration::from_secs(4)).unwrap().unwrap()["ok"],
        true
    );
    assert_eq!(h.rpc(&json!({"op":"analytics.settings"}))["ok"], true);
    let stored: Value =
        serde_json::from_slice(&fs::read(h.profile.join("analytics.json")).unwrap()).unwrap();
    assert_eq!(stored["retentionDays"], 20);
}
