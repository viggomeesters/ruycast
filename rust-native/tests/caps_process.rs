//! Original standalone Caps SDK tests; synthetic handles, never actual IOKit.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

fn invoke(input: Value, library: Option<&str>) -> Output {
    let probe = Path::new(env!("CARGO_BIN_EXE_caps-probe"));
    assert!(probe.is_absolute() && probe.is_file());
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("target"));
    assert!(parent.is_absolute());
    fs::create_dir_all(&parent).unwrap();
    let directory = parent.join(format!(
        "caps-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    struct Owned(PathBuf);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let owned = Owned(directory);
    let input_path = owned.0.join("input.json");
    fs::write(&input_path, serde_json::to_vec(&input).unwrap()).unwrap();
    let mut command = Command::new(probe);
    command
        .env_clear()
        .stdin(fs::File::open(input_path).unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(library) = library {
        let (variable, filename) = match library {
            "complete" => ("RUYCAST_TEST_CAPS_SHIM", "libcaps_fixture.dylib"),
            "missing" => ("RUYCAST_TEST_CAPS_MISSING", "libcaps_missing_set.dylib"),
            _ => panic!("Unknown fixture"),
        };
        let artifact = std::env::var_os(variable)
            .map(PathBuf::from)
            .unwrap_or_else(|| probe.parent().unwrap().join("examples").join(filename));
        assert!(
            artifact.is_absolute() && artifact.is_file(),
            "Build caps-fixture and caps-missing-set first: {artifact:?}"
        );
        command.env("DYLD_INSERT_LIBRARIES", artifact);
    }
    child_output::wait(command.spawn().unwrap(), Duration::from_secs(10))
}

fn run_case(
    operations: &[&str],
    original: bool,
    failures: u64,
    nested_at: Option<u32>,
) -> (Value, Vec<String>) {
    let output = invoke(
        json!({"operations":operations,"original":original,"failures":failures,"nestedAt":nested_at}),
        Some("complete"),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let trace = std::str::from_utf8(&output.stderr)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("caps-fixture:").map(str::to_owned))
        .collect();
    (serde_json::from_slice(&output.stdout).unwrap(), trace)
}

#[test]
fn roundtrip_both_original_latches_and_idempotent_refresh() {
    for original in [false, true] {
        let (data, trace) = run_case(
            &["activate", "activate", "clear", "retire", "retire"],
            original,
            0,
            None,
        );
        assert_eq!(
            trace,
            [
                "matching",
                "service",
                "task",
                "open",
                "release",
                "get",
                "set:false",
                "set:false",
                if original { "set:true" } else { "set:false" },
                "close"
            ]
        );
        assert_eq!(data["finalState"], u32::from(original));
        assert_eq!(data["recoveryPending"], false);
        assert_eq!(
            data["results"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["result"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["Ok(())"; 5]
        );
    }
}

#[test]
fn acquisition_failure_releases_service_and_never_uses_failed_output() {
    for (stage, expected, tail) in [
        (1, "Open(-1)", vec!["matching"]),
        (2, "Open(-2)", vec!["matching", "service"]),
        (
            4,
            "Open(-4)",
            vec!["matching", "service", "task", "open", "release"],
        ),
        (
            5,
            "Open(-5)",
            vec!["matching", "service", "task", "open", "release", "close"],
        ),
    ] {
        let (data, trace) = run_case(&["activate"], true, 1 << stage, None);
        assert_eq!(trace, tail);
        assert_eq!(data["results"][0]["result"], format!("Err({expected})"));
        assert_eq!(data["finalState"], 1);
    }
}

#[test]
fn failed_read_never_guesses_original_or_mutates() {
    let (data, trace) = run_case(&["activate"], true, 1 << 6, None);
    assert_eq!(
        trace,
        ["matching", "service", "task", "open", "release", "get", "close"]
    );
    assert_eq!(data["results"][0]["result"], "Err(Read(-6))");
    assert_eq!(data["finalState"], 1);
}

#[test]
fn failed_clear_restores_even_when_failed_call_changed_latch() {
    let (data, trace) = run_case(&["activate"], true, 1 << 7, None);
    assert_eq!(
        &trace[trace.len() - 3..],
        ["set:false", "set:true", "close"]
    );
    assert_eq!(data["results"][0]["result"], "Err(Clear(-7))");
    assert_eq!(data["finalState"], 1);
}

#[test]
fn restore_failure_survives_drop_blocks_reopen_and_recovers() {
    let (data, trace) = run_case(
        &[
            "activate", "drop", "new", "activate", "recover", "activate", "retire",
        ],
        true,
        1 << 8,
        None,
    );
    let rows = &data["results"];
    assert_eq!(rows[1]["recoveryPending"], true);
    assert_eq!(rows[1]["state"], 3);
    assert_eq!(rows[3]["result"], "Err(Open(-4))");
    assert_eq!(rows[4]["result"], "Ok(())");
    assert_eq!(rows[4]["recoveryPending"], false);
    assert_eq!(rows[4]["state"], 1);
    assert_eq!(&trace[7..10], ["set:true", "set:true", "close"]);
    assert_eq!(data["finalState"], 1);
    assert_eq!(data["recoveryPending"], false);
}

#[test]
fn repeated_failed_quarantine_restore_retains_authority() {
    let (data, trace) = run_case(
        &["activate", "drop", "recover", "recover"],
        true,
        (1 << 8) | (1 << 9),
        None,
    );
    assert_eq!(data["results"][2]["result"], "Err(Restore(-9))");
    assert_eq!(data["results"][2]["recoveryPending"], true);
    assert_eq!(
        &trace[trace.len() - 4..],
        ["set:true", "set:true", "set:true", "close"]
    );
    assert_eq!(data["recoveryPending"], false);
}

#[test]
fn close_error_consumes_handle_without_double_close() {
    let (data, trace) = run_case(&["activate", "retire", "retire"], true, 1 << 9, None);
    assert_eq!(data["results"][1]["result"], "Err(Close(-9))");
    assert_eq!(trace.iter().filter(|s| *s == "close").count(), 1);
    assert_eq!(data["finalState"], 1);
}

#[test]
fn nested_second_owner_cannot_acquire_during_setup_clear_restore_or_close() {
    for stage in 1..=9 {
        let (data, trace) = run_case(&["activate", "retire"], true, 0, Some(stage));
        assert_eq!(data["nested"], "Ok(Err(Open(-5)))");
        for call in ["matching", "open", "close"] {
            assert_eq!(trace.iter().filter(|s| s.as_str() == call).count(), 1);
        }
        assert_eq!(data["finalState"], 1);
    }
}

#[test]
fn incomplete_or_absent_fixture_fails_before_any_sdk_entry() {
    for library in [Some("missing"), None] {
        let output = invoke(json!({}), library);
        assert_eq!(output.status.code(), Some(2));
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        assert!(stderr.contains("Complete Caps SDK fixture unavailable."));
        assert!(!stderr.contains("caps-fixture:"));
    }
}
