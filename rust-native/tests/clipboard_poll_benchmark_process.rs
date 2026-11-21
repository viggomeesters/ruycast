#![cfg(feature = "test-tools")]

use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};

fn run(args: &[&str], config: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruycast-native"));
    command.args(args).env_clear();
    if let Some(config) = config {
        command.env("RUYCAST_CONFIG_DIR", config);
    }
    command.output().unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid benchmark JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn named_poll_benchmark_reports_stable_current_rust_capture_work() {
    let unchanged = run(
        &["--benchmark-clipboard-poll", "9", "unchanged", "current"],
        None,
    );
    assert!(
        unchanged.status.success(),
        "{}",
        String::from_utf8_lossy(&unchanged.stderr)
    );
    let unchanged = json(&unchanged);
    assert_eq!(unchanged["mode"], "unchanged");
    assert_eq!(unchanged["ticks"], 9);
    assert_eq!(unchanged["applicationReads"], 0);
    assert_eq!(unchanged["distinctForegroundPIDs"], 0);
    assert_eq!(unchanged["retainedClips"], 1);
    assert!(unchanged["elapsedSeconds"].as_f64().unwrap().is_finite());
    assert!(unchanged["cpuSeconds"].as_f64().unwrap().is_finite());

    let active = run(
        &["--benchmark-clipboard-poll", "9", "active", "current"],
        None,
    );
    assert!(
        active.status.success(),
        "{}",
        String::from_utf8_lossy(&active.stderr)
    );
    let active = json(&active);
    assert_eq!(active["mode"], "changed-every-fourth-poll");
    assert_eq!(active["ticks"], 9);
    assert_eq!(active["applicationReads"], 6);
    assert_eq!(active["distinctForegroundPIDs"], 1);
    assert_eq!(active["retainedClips"], 4);
    assert!(active["elapsedSeconds"].as_f64().unwrap().is_finite());
    assert!(active["cpuSeconds"].as_f64().unwrap().is_finite());
    assert!(active["scope"]
        .as_str()
        .unwrap()
        .contains("named synthetic pasteboard"));

    let workspace = run(&["--benchmark-clipboard-poll", "2", "unchanged"], None);
    assert!(
        workspace.status.success(),
        "{}",
        String::from_utf8_lossy(&workspace.stderr)
    );
    let workspace = json(&workspace);
    assert_eq!(workspace["mode"], "unchanged");
    assert_eq!(workspace["ticks"], 2);
}

#[test]
fn configured_profile_is_rejected_before_any_benchmark_io() {
    let config =
        std::env::temp_dir().join(format!("ruycast-benchmark-config-{}", std::process::id()));
    assert!(!config.exists());
    let output = run(&["--benchmark-clipboard-poll", "9"], Some(&config));
    assert!(!output.status.success());
    assert_eq!(
        json(&output)["error"],
        "Unset RUYCAST_CONFIG_DIR before running the isolated polling fixture."
    );
    assert!(!config.exists());
}

#[test]
fn poll_benchmark_rejects_ticks_outside_the_documented_bounds() {
    for ticks in ["0", "100001"] {
        let output = run(&["--benchmark-clipboard-poll", ticks], None);
        assert!(!output.status.success());
        assert_eq!(json(&output)["error"], "Expected 1–100000 polls");
    }
}
