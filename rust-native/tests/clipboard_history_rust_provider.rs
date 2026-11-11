//! Loaded complete SDK owns both framework General substitution and history board.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::Value;
use std::{
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn run(tag: &str, marker: bool, library: bool, guard: bool) -> (Value, Vec<Value>) {
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_HISTORY_SDK_TEST_PARENT").expect("Own absolute parent"),
    );
    assert!(parent.is_absolute() && parent.is_dir());
    let home = parent.join(format!("{}-{tag}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&home)
        .unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(home.clone());
    if marker {
        std::fs::write(
            home.join(".ruycast-test-fixture"),
            "own-disposable-fixture-v1",
        )
        .unwrap();
    }
    let target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("Own target"));
    assert!(target.is_absolute());
    let mut command = Command::new(target.join("release/examples/clipboard_history_sdk_probe"));
    command
        .env_clear()
        .env("HOME", &home)
        .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &home)
        .env("RUYCAST_NATIVE_TEST_SNIPPET", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if guard {
        command.env("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD", "1");
    }
    if library {
        command.env(
            "DYLD_INSERT_LIBRARIES",
            target.join("release/examples/libclipboard_history_fixture.dylib"),
        );
    }
    let output = child_output::wait(command.spawn().unwrap(), Duration::from_secs(10));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = serde_json::from_slice(&output.stdout).unwrap();
    let trace = std::fs::read_to_string(home.join("workspace-trace.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (result, trace)
}
#[test]
fn framework_text_initialization_and_history_board_stay_private_and_distinct() {
    for (tag, guard) in [("normal-guard", true), ("normal-no-guard", false)] {
        let (v, trace) = run(tag, true, true, guard);
        assert_eq!(v["ready"], true);
        assert_eq!(v["stableFramework"], true);
        assert_eq!(v["stableHistory"], true);
        assert_eq!(v["distinct"], true);
        assert!(v["historyName"]
            .as_str()
            .unwrap()
            .starts_with("ruycast-clipboard-history-fixture-"));
        assert_ne!(v["frameworkName"], "NSGeneralPboard");
        assert_eq!(v["frameworkText"], "Own framework");
        assert_eq!(v["historyText"], "Own history Å");
        assert_eq!(v["active"], false);
        assert_eq!(v["windows"], 0);
        assert!(!trace
            .iter()
            .any(|row| row["phase"] == "unexpected-general-read"
                || row["phase"] == "unexpected-general-pasteboard"));
        assert!(trace.iter().any(|row| row["phase"] == "installed"));
        assert!(trace.iter().any(|row| row["phase"] == "snippet-installed"));
    }
}
#[test]
fn missing_marker_and_absent_complete_sdk_refuse_before_framework_queries() {
    for (tag, marker, library) in [("no-marker", false, true), ("no-library", true, false)] {
        let (v, trace) = run(tag, marker, library, true);
        assert_eq!(v["ready"], false);
        assert!(!trace.iter().any(|row| row["phase"] == "clipboard-read"));
    }
}
