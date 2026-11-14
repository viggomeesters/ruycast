//! Actual child helpers with malformed protocol, bounded by the real supervisor.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Owned fixtures share bounded child output support"
)]
#[path = "support/child_output.rs"]
mod child_output;
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn probe(mode: &str, deadline: Option<u64>) -> Value {
    let home = runtime_host::OwnedDir::new();
    let helper = env!("CARGO_BIN_EXE_supervision-io-fixture");
    let sentinel = home.0.join("must-not-launch");
    let mut input = json!({"helper":helper,"command":{"id":"script.fixture","title":"Fixture","kind":"script","target":"/usr/bin/touch","arguments":[sentinel],"timeout":2},"home":home.0});
    if let Some(deadline) = deadline {
        input["deadlineMs"] = json!(deadline);
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_supervision-probe"));
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "DYLD_INSERT_LIBRARIES",
        "RUYCAST_HOME",
        "RUSTCAST_HOME",
        "RUYCAST_CONFIG_DIR",
        "RUSTCAST_CONFIG_DIR",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("RUYCAST_SUPERVISION_HELPER_MODE", mode);
    let mut child = cmd.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let bytes = serde_json::to_vec(&input).unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let out = child_output::wait(child, Duration::from_secs(7));
    writer.join().unwrap().unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["noChildren"], true);
    assert!(!sentinel.exists());
    if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let folder = PathBuf::from(folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(format!("supervision-protocol-{mode}.json")),serde_json::to_vec_pretty(&json!({"input":input,"mode":mode,"result":result,"fixtureSha256":format!("{:x}",Sha256::digest(fs::read(helper).unwrap()))})).unwrap()).unwrap();
    }
    result.get("result").unwrap().clone()
}
#[test]
fn bad_ready_or_no_ready_never_receives_a_command() {
    for mode in ["old-ready", "old-version", "no-ready"] {
        assert!(probe(mode, Some(100)).get("error").is_some());
    }
}
#[test]
fn partial_malformed_and_wrong_exit_response_are_unknown() {
    for mode in ["partial", "malformed", "missing-capture", "wrong-exit"] {
        assert!(probe(mode, None).get("error").is_some());
    }
}
#[test]
fn oversized_final_response_closes_output_and_reaps_helper() {
    assert!(probe("oversized-final", None).get("error").is_some());
}
