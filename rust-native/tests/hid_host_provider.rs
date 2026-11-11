//! Bounded children keep actual native owner probes on their process main thread.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn run(mode: &str, library: &str) {
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_HID_HOST_TEST_PARENT").expect("Own absolute test parent"),
    );
    assert!(parent.is_absolute() && parent.is_dir());
    let directory = parent.join(format!("{}-{mode}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    if mode != "missing-marker" {
        std::fs::write(
            directory.join(".ruycast-keyboard-fixture"),
            "own-keyboard-fixture-v1",
        )
        .unwrap();
    }
    if mode == "failure" {
        std::fs::write(directory.join("hid-fail-manager_open"), "own fault").unwrap();
    }
    let target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("Own build target"));
    assert!(target.is_absolute());
    let examples = target.join("release/examples");
    let child = Command::new(examples.join("hid-host-probe"))
        .arg(mode)
        .env_clear()
        .env("RUYCAST_KEYBOARD_FIXTURE_HOME", &directory)
        .env("DYLD_INSERT_LIBRARIES", examples.join(library))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let result = child_output::wait(child, Duration::from_secs(8));
    let trace = std::fs::read_to_string(directory.join("keyboard-trace.jsonl")).unwrap_or_default();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    if mode.starts_with("missing-") {
        assert!(trace.is_empty(), "Loader must refuse before target/install");
    } else {
        assert!(trace.lines().all(
            |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["mainThread"] == true
        ));
    }
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn shared_host_callbacks_and_caps_restore() {
    run("normal", "libhid_host.dylib");
}
#[test]
fn host_setup_failure_restores_shared_latch() {
    run("failure", "libhid_host.dylib");
}
#[test]
fn incomplete_host_tables_refuse_before_sdk_entry() {
    run("missing-post", "libhid_host_missing_post.dylib");
    run("missing-parameter", "libhid_host_missing_parameter.dylib");
    run("missing-marker", "libhid_host.dylib");
}
