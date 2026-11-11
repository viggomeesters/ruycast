//! Actual native workspace owner with the loaded synthetic Rust interposer.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn request(target: &str) -> Value {
    json!({"request":{"kind":"app","target":target,"commandId":"fixture","title":"Fixture"}})
}
fn run(tag: &str, packet: Value, marker: bool, library: bool) -> (Value, Vec<Value>) {
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_WORKSPACE_TEST_PARENT").expect("Own absolute test parent"),
    );
    assert!(parent.is_absolute() && parent.is_dir());
    let home = parent.join(format!("{}-{tag}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&home)
        .unwrap();
    std::fs::write(
        home.join(".ruycast-test-fixture"),
        if marker {
            "own-disposable-fixture-v1"
        } else {
            "wrong"
        },
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_workspace-probe"));
    command
        .env_clear()
        .env("HOME", &home)
        .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if library {
        let target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("Own Cargo target"));
        assert!(target.is_absolute());
        let artifact = target.join("release/examples/libworkspace_fixture.dylib");
        assert!(artifact.is_file());
        command.env("DYLD_INSERT_LIBRARIES", artifact);
    }
    let mut child = command.spawn().unwrap();
    let written = serde_json::to_writer(child.stdin.take().unwrap(), &packet);
    let output = child_output::wait(child, Duration::from_secs(12));
    assert!(
        written.is_ok() && output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["ownApplicationActive"], false);
    assert_eq!(value["ownWindowCount"], 0);
    let trace = std::fs::read_to_string(home.join("workspace-trace.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    std::fs::remove_dir_all(home).unwrap();
    (value, trace)
}
#[test]
fn native_worker_callback_uses_exact_synthetic_route() {
    let (value, trace) = run(
        "normal",
        json!({"cases":[request("dev.fixture.success")]}),
        true,
        true,
    );
    let case = &value["cases"][0];
    assert_eq!(
        case["response"],
        json!({"ok":true,"result":{"command":"fixture","state":"opened"}})
    );
    assert_eq!(case["beforeCommitCalls"], 1);
    assert_eq!(value["afterRelease"], 0);
    assert_eq!(
        trace
            .iter()
            .map(|row| row["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["installed", "resolve", "open-app", "callback"]
    );
    assert_eq!(trace[0]["selectors"], 4);
    assert!(trace[1..].iter().all(|row| row["mainThread"] == false));
}
#[test]
fn missing_target_invalid_location_and_callback_error_never_fall_back() {
    let (value, trace) = run(
        "faults",
        json!({"cases":[request("dev.fixture.missing"),request("dev.fixture.invalid-location"),request("dev.fixture.error")]}),
        true,
        true,
    );
    for case in &value["cases"].as_array().unwrap()[..2] {
        assert_eq!(case["committed"], false);
        assert_eq!(case["beforeCommitCalls"], 0);
        assert_eq!(case["response"]["ok"], false);
    }
    assert_eq!(
        value["cases"][2]["response"],
        json!({"ok":false,"error":"Fixture native error."})
    );
    assert_eq!(
        trace
            .iter()
            .filter(|row| row["phase"] == "open-app")
            .count(),
        1
    );
    assert_eq!(value["afterRelease"], 0);
}
#[test]
fn held_callback_ownership_releases_and_invalid_boundaries_refuse() {
    let mut hold = request("dev.fixture.hold");
    hold["timeoutMs"] = json!(10);
    let (value, _) = run(
        "held",
        json!({"cases":[hold],"releaseCallbacks":true,"settleMs":10}),
        true,
        true,
    );
    assert_eq!(value["beforeRelease"], 1);
    assert_eq!(value["afterRelease"], 0);
    assert_eq!(
        value["cases"][0]["response"]["code"],
        "open-outcome-unknown"
    );
    for (tag, marker, library) in [("bad-marker", false, true), ("absent", true, false)] {
        let (value, trace) = run(
            tag,
            json!({"cases":[request("dev.fixture.success")]}),
            marker,
            library,
        );
        assert_eq!(value["cases"][0]["committed"], false);
        assert!(trace.is_empty());
    }
}
