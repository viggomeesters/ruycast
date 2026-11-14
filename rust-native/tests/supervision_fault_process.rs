//! Actual parent waitpid fault injection, with a private hash-checked SDK copy.
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
fn run(precommit: bool) {
    let home = runtime_host::OwnedDir::new();
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_supervision-probe"));
    let original = std::env::var_os("RUYCAST_TEST_SUPERVISION_SHIM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            probe
                .parent()
                .unwrap()
                .join("examples/libsupervision_fault_shim.dylib")
        });
    assert!(
        original.is_absolute() && original.is_file(),
        "Build supervision-fault-shim first"
    );
    let copy = home.0.join("supervision-fault.dylib");
    fs::copy(&original, &copy).unwrap();
    assert_eq!(
        Sha256::digest(fs::read(&original).unwrap()),
        Sha256::digest(fs::read(&copy).unwrap())
    );
    let fixture = env!("CARGO_BIN_EXE_supervision-io-fixture");
    let helper = if precommit {
        PathBuf::from(fixture)
    } else {
        let cli = std::env::var_os("RUYCAST_TEST_CLI")
            .map(PathBuf::from)
            .unwrap_or_else(|| runtime_host::root().join(".build/rust/release/ruycast"));
        assert!(cli.is_absolute() && cli.is_file());
        cli
    };
    let input = json!({"helper":helper,"command":{"id":"script.fixture","title":"Fixture","kind":"script","target":fixture,"arguments":["success"],"timeout":2},"home":home.0});
    let mut cmd = Command::new(probe);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "DYLD_INSERT_LIBRARIES",
        "RUYCAST_HOME",
        "RUSTCAST_HOME",
        "RUYCAST_CONFIG_DIR",
        "RUSTCAST_CONFIG_DIR",
        "RUYCAST_SUPERVISION_KILL_TRACE",
        "RUYCAST_SUPERVISION_HELPER_MODE",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("DYLD_INSERT_LIBRARIES", &copy).env(
        "RUYCAST_SUPERVISION_WAIT_MARKER",
        home.0.join("wait-injected"),
    );
    if precommit {
        cmd.env("RUYCAST_SUPERVISION_HELPER_MODE", "old-ready-wait");
    }
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
    assert!(result["result"].get("error").is_some());
    assert_eq!(
        fs::read_to_string(home.0.join("wait-injected")).unwrap(),
        "injected"
    );
    if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let folder = PathBuf::from(folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(format!("supervision-wait-fault-{}.json",if precommit {"precommit"} else {"committed"})),serde_json::to_vec_pretty(&json!({"input":input,"result":result,"marker":"injected","shimSha256":format!("{:x}",Sha256::digest(fs::read(&copy).unwrap()))})).unwrap()).unwrap();
    }
}
#[test]
fn real_parent_wait_error_retries_reap_without_child_leak() {
    run(false);
}
#[test]
fn precommit_cleanup_wait_error_is_retried() {
    run(true);
}
