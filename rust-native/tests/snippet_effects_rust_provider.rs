//! Loaded Rust effects component with real named-board preparation and no desktop input.
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
fn run(
    tag: &str,
    flags: &[&str],
    marker: bool,
    enabled: bool,
    library: bool,
) -> (Value, Vec<Value>) {
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_SNIPPET_EFFECTS_TEST_PARENT").expect("Own absolute parent"),
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
    for flag in flags {
        std::fs::write(home.join(flag), "Own fault").unwrap();
    }
    let target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("Own target"));
    assert!(target.is_absolute());
    let mut command = Command::new(target.join("release/examples/snippet_effects_probe"));
    command
        .env_clear()
        .env("HOME", &home)
        .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if enabled {
        command.env("RUYCAST_NATIVE_TEST_SNIPPET", "1");
    }
    if library {
        command.env(
            "DYLD_INSERT_LIBRARIES",
            target.join("release/examples/libsnippet_effects_fixture.dylib"),
        );
    }
    let output = child_output::wait(command.spawn().unwrap(), Duration::from_secs(10));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let final_trace: Vec<Value> = std::fs::read_to_string(home.join("workspace-trace.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let trace = value["operationTrace"]
        .as_array()
        .cloned()
        .unwrap_or(final_trace);
    (value, trace)
}
fn events(trace: &[Value], phase: &str) -> Vec<Value> {
    trace
        .iter()
        .filter(|v| v["phase"] == phase)
        .cloned()
        .collect()
}
#[test]
fn real_prepared_named_board_and_synthetic_app_events_preserve_owned_values() {
    let (v, trace) = run("normal", &[], true, true, true);
    assert_eq!(v["ready"], true);
    assert_eq!(v["trusted"], true);
    assert_eq!(v["written"], true);
    assert_eq!(v["activated"], true);
    assert_eq!(v["initialPID"], 4260);
    assert_eq!(v["sameIdentity"], true);
    assert_eq!(v["sameAfterRelaunch"], false);
    assert_eq!(v["externalPID"], 4261);
    assert_eq!(v["active"], false);
    assert_eq!(v["windows"], 0);
    let writes = events(&trace, "snippet-write");
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0]["text"], "A😀end");
    assert_eq!(writes[0]["html"], "<div>A😀end</div>");
    assert_eq!(writes[0]["items"], 1);
    let posts = events(&trace, "snippet-post");
    assert_eq!(posts.len(), 2);
    assert_eq!(v["created"], 2);
    for (i, post) in posts.iter().enumerate() {
        assert_eq!(post["key"], 9);
        assert_eq!(post["down"], i == 0);
        assert_eq!(post["flags"], 1 << 20);
        assert_eq!(post["tag"], 0x52554859504552u64);
        assert_eq!(post["pid"], 4260);
        assert_eq!(post["mainThread"], true);
    }
    let installed = trace
        .iter()
        .position(|r| r["phase"] == "installed")
        .unwrap();
    let effects = trace
        .iter()
        .position(|r| r["phase"] == "snippet-installed")
        .unwrap();
    assert!(installed < effects);
}
#[test]
fn staging_and_publish_failures_keep_the_original_fault_boundaries() {
    for fault in [
        "fail-string",
        "fail-html",
        "fail-write",
        "fail-event",
        "activation-noop",
    ] {
        let (v, trace) = run(fault, &[fault], true, true, true);
        assert_eq!(v["created"], 0);
        assert!(events(&trace, "snippet-post").is_empty());
        if fault.starts_with("fail-") && fault != "fail-event" {
            assert_eq!(v["written"], false);
        }
        if ["fail-string", "fail-html"].contains(&fault) {
            assert!(events(&trace, "snippet-clear").is_empty());
            assert_eq!(events(&trace, "snippet-item-failed").len(), 1);
        }
        if fault == "fail-write" {
            assert_eq!(events(&trace, "snippet-clear").len(), 1);
            assert_eq!(events(&trace, "snippet-write-failed").len(), 1);
        }
        if fault == "activation-noop" {
            assert_eq!(v["activated"], false);
        }
    }
}
#[test]
fn incomplete_authority_and_denied_trust_refuse_before_effects() {
    for (tag, marker, enabled, library) in [
        ("no-marker", false, true, true),
        ("disabled", true, false, true),
        ("no-sdk", true, true, false),
    ] {
        let (v, trace) = run(tag, &[], marker, enabled, library);
        assert_eq!(v["ready"], false);
        assert!(events(&trace, "snippet-post").is_empty());
        assert!(events(&trace, "snippet-clear").is_empty());
    }
    let (v, trace) = run("denied", &["deny-ax"], true, true, true);
    assert_eq!(v["trusted"], false);
    assert!(events(&trace, "snippet-clear").is_empty());
    assert!(events(&trace, "snippet-post").is_empty());
}
