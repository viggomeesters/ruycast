//! Manager retirement through the owned four-selector workspace fixture.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

struct Owned(PathBuf);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn probe(scenario: &str, shim: bool) -> (Value, Vec<Value>) {
    let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    assert!(parent.is_absolute());
    fs::create_dir_all(&parent).unwrap();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = parent.join(format!(
        "manager-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let owned = Owned(dir);
    fs::write(
        owned.0.join(".ruycast-test-fixture"),
        b"own-disposable-fixture-v1",
    )
    .unwrap();
    let input = owned.0.join("input.json");
    fs::write(
        &input,
        serde_json::to_vec(&json!({"scenario":scenario})).unwrap(),
    )
    .unwrap();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_manager-probe"));
    assert!(executable.is_absolute() && executable.is_file());
    let mut command = Command::new(&executable);
    command
        .env_clear()
        .env("HOME", &owned.0)
        .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &owned.0)
        .stdin(fs::File::open(input).unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if shim {
        let source = std::env::var_os("RUYCAST_TEST_WORKSPACE_SHIM")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                executable
                    .parent()
                    .unwrap()
                    .join("examples/libworkspace_fixture.dylib")
            });
        assert!(
            source.is_absolute() && source.is_file(),
            "Build workspace-fixture example first"
        );
        let fixture = owned.0.join("workspace.dylib");
        fs::copy(source, &fixture).unwrap();
        command.env("DYLD_INSERT_LIBRARIES", fixture);
    }
    let output = child_output::wait(command.spawn().unwrap(), Duration::from_secs(10));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let trace_path = owned.0.join("workspace-trace.jsonl");
    if !shim {
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("boundary"));
        assert!(!trace_path.exists());
        return (result, Vec::new());
    }
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(result["ok"], true);
    assert_eq!(result["ownApplicationActive"], false);
    assert_eq!(result["ownWindowCount"], 0);
    let trace: Vec<Value> = fs::read_to_string(trace_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(trace[0]["selectors"], 4);
    if let Some(evidence) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let directory = PathBuf::from(evidence).join("manager");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(format!("{scenario}.json")),
            serde_json::to_vec_pretty(&json!({"result":result,"trace":trace})).unwrap(),
        )
        .unwrap();
    }
    (result, trace)
}
fn no_open(trace: &[Value]) {
    assert!(!trace
        .iter()
        .any(|x| x["phase"].as_str().unwrap().starts_with("open-")));
}

#[test]
fn sdk_lookup_retirement_returns_before_release_and_revokes_open() {
    let (data, trace) = probe("retire-lookup", true);
    let result = &data["result"];
    let outcome = &result["deliveries"][0]["outcome"];
    assert_eq!(outcome["committed"], false);
    assert_eq!(outcome["response"]["ok"], false);
    assert_eq!(result["beforeRelease"], 1);
    assert_eq!(result["replacementBeforeRelease"], 1);
    assert_eq!(result["releasedOnlyAfterShutdown"], true);
    assert_eq!(result["afterRelease"], 0);
    assert_eq!(result["secondDelivery"], json!([]));
    assert_eq!(data["beforeCommitCalls"], 0);
    no_open(&trace);
}
#[test]
fn committed_blocked_default_open_returns_unknown_without_waiting() {
    let (data, trace) = probe("retire-sync", true);
    let result = &data["result"];
    let outcome = &result["deliveries"][0]["outcome"];
    assert_eq!(outcome["committed"], true);
    assert_eq!(outcome["kind"], "workspace");
    assert_eq!(outcome["captureAvailable"], false);
    assert_eq!(outcome["response"]["code"], "open-outcome-unknown");
    assert_eq!(outcome["response"]["actionCompleted"], true);
    assert_eq!(result["beforeRelease"], 1);
    assert_eq!(result["replacementBeforeRelease"], 1);
    assert_eq!(result["afterRelease"], 0);
    assert_eq!(result["secondDelivery"], json!([]));
    assert_eq!(
        trace
            .iter()
            .map(|x| x["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["installed", "open-default"]
    );
}
#[test]
fn owner_hook_permission_is_not_sdk_commit() {
    let (data, trace) = probe("retire-hook", true);
    let result = &data["result"];
    assert_eq!(data["beforeCommitCalls"], 1);
    assert_eq!(result["deliveries"][0]["outcome"]["committed"], false);
    assert_eq!(result["beforeRelease"], 1);
    assert_eq!(result["afterRelease"], 0);
    assert_eq!(result["secondDelivery"], json!([]));
    no_open(&trace);
}
#[test]
fn process_wide_cli_reservation_survives_manager_replacement() {
    let (data, trace) = probe("capacity", true);
    let result = &data["result"];
    assert_eq!(result["cliRejected"], true);
    assert_eq!(result["nativeReserved"], true);
    assert_eq!(result["nativeRejected"], true);
    assert_eq!(result["beforeRelease"], result["limit"]);
    assert!(result["afterShutdown"].as_u64().unwrap() >= 1);
    assert_eq!(result["afterRelease"], 0);
    assert_eq!(result["oldDeliveries"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["deliveries"].as_array().unwrap().len() as u64,
        result["limit"].as_u64().unwrap() - 1
    );
    assert_eq!(data["beforeCommitCalls"], 0);
    no_open(&trace);
}
#[test]
fn ready_success_wins_shutdown_and_delivers_exactly_once() {
    let (data, _) = probe("ready", true);
    let result = &data["result"];
    assert_eq!(result["deliveries"][0]["context"], 7);
    let outcome = &result["deliveries"][0]["outcome"];
    assert_eq!(outcome["kind"], "workspace");
    assert_eq!(outcome["committed"], true);
    assert_eq!(
        outcome["response"],
        json!({"ok":true,"result":{"command":"fixture","state":"opened"}})
    );
    assert_eq!(outcome["captureAvailable"], false);
    assert_eq!(result["secondDelivery"], json!([]));
    assert_eq!(result["liveWorkers"], 0);
    assert_eq!(data["pendingCallbacks"], 0);
}
#[test]
fn fixture_without_interposition_fails_before_any_sdk_access() {
    probe("ready", false);
}
