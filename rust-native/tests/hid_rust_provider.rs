//! Focused black-box checks of the actual loaded Rust provider and native owner.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::Duration,
};
fn library(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.parent()
                .unwrap()
                .join(".build/full-rust-release/native")
        });
    let path = target.join("release/examples").join(name);
    assert!(path.is_absolute() && path.is_file());
    path
}
fn run(library_name: Option<&str>, input: Option<Value>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hid-probe"));
    command
        .env_clear()
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(name) = library_name {
        command.env("DYLD_INSERT_LIBRARIES", library(name));
    }
    let mut child = command.spawn().unwrap();
    let written = input.map(|value| serde_json::to_writer(child.stdin.take().unwrap(), &value));
    let output = child_output::wait(child, Duration::from_secs(5));
    assert!(
        written.is_none_or(|result| result.is_ok()),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn probe(operations: Value) -> Value {
    let output = run(
        Some("libhid_fixture.dylib"),
        Some(json!({"operations":operations,"original":true,"env":0})),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(data["cleanup"]["allocated"], 0);
    assert_eq!(data["cleanup"]["caps"], 0);
    data
}
#[test]
fn physical_flags_and_balanced_cleanup() {
    let flags = (1u64 << 63) | (1 << 16) | 0x10;
    let data = probe(
        json!([{"op":"plan","hyper":true},{"op":"refresh"},{"op":"input","usage":57,"down":true,"device":0,"page":7,"time":0},{"op":"event","type":10,"key":0,"flags":flags},{"op":"input","usage":57,"down":false,"device":0,"page":7,"time":0},{"op":"stop"}]),
    );
    let rows = &data["rows"];
    let hyper: u64 = (17..=20).map(|bit| 1u64 << bit).sum();
    assert_eq!(rows[1]["status"], "On");
    assert_eq!(rows[2]["state"]["lastDown"], true);
    assert_eq!(rows[2]["state"]["lastFlags"], hyper);
    assert_eq!(
        rows[3]["result"],
        json!({"consumed":false,"flags":(1u64<<63)|0x10|hyper})
    );
    assert_eq!(rows[4]["state"]["lastFlags"], 0);
    assert_eq!(rows[5]["state"]["caps"], 0);
}
#[test]
fn setup_failure_restores_and_retries() {
    let data = probe(
        json!([{"op":"plan","hyper":true},{"op":"failure","name":"manager_open"},{"op":"refresh"},{"op":"failure","name":""},{"op":"refresh"}]),
    );
    assert_ne!(data["rows"][2]["status"], "On");
    assert_eq!(data["rows"][2]["state"]["caps"], 1);
    assert_eq!(data["rows"][2]["state"]["allocated"], 0);
    assert_eq!(data["rows"][4]["status"], "On");
}
#[test]
fn absent_and_missing_post_refuse_before_any_sdk_entry() {
    for name in [None, Some("libhid_missing_post.dylib")] {
        let output = run(name, None);
        assert_eq!(output.status.code(), Some(2));
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("Complete HID SDK fixture unavailable."),
            "{error}"
        );
        assert!(!error.contains("hid-fixture:"));
        assert!(!error.contains("caps-fixture:"));
    }
}
