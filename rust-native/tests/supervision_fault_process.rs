//! Real supervisor fault and pipe cleanup, with private hash-checked SDK copies.
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
fn copy_shim(home: &std::path::Path, probe: &std::path::Path) -> PathBuf {
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
    let copy = home.join("supervision-fault.dylib");
    fs::copy(&original, &copy).unwrap();
    assert_eq!(
        Sha256::digest(fs::read(&original).unwrap()),
        Sha256::digest(fs::read(&copy).unwrap())
    );
    copy
}
fn run(precommit: bool) {
    let home = runtime_host::OwnedDir::new();
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_supervision-probe"));
    let copy = copy_shim(&home.0, &probe);
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

fn cleanup_probe(mode: &str, panic: bool) -> (runtime_host::OwnedDir, Value) {
    let home = runtime_host::OwnedDir::new();
    fs::write(
        home.0.join(".ruycast-supervision-fixture"),
        "owned-supervision-helper-v1",
    )
    .unwrap();
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_supervision-probe"));
    let helper = env!("CARGO_BIN_EXE_supervision-io-fixture");
    let mut input = json!({"helper":helper,"command":{"id":"script.fixture","title":"Fixture","kind":"script","target":helper,"arguments":["success"],"timeout":2},"home":home.0});
    if panic {
        input["panicMs"] = json!(1000);
    }
    let mut cmd = Command::new(&probe);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "DYLD_INSERT_LIBRARIES",
        "RUYCAST_HOME",
        "RUSTCAST_HOME",
        "RUYCAST_CONFIG_DIR",
        "RUSTCAST_CONFIG_DIR",
        "RUYCAST_SUPERVISION_WAIT_MARKER",
        "RUYCAST_SUPERVISION_KILL_TRACE",
        "RUYCAST_SUPERVISION_HELPER_MODE",
        "RUYCAST_SUPERVISION_HELPER_HOME",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("RUYCAST_SUPERVISION_HELPER_MODE", mode)
        .env("RUYCAST_SUPERVISION_HELPER_HOME", &home.0);
    let shim = if panic {
        let copy = copy_shim(&home.0, &probe);
        cmd.env("DYLD_INSERT_LIBRARIES", &copy)
            .env("RUYCAST_SUPERVISION_KILL_TRACE", home.0.join("kill-trace"));
        Some(copy)
    } else {
        None
    };
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
    if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let folder = PathBuf::from(folder);
        fs::create_dir_all(&folder).unwrap();
        let shim_sha = shim.map(|path| format!("{:x}", Sha256::digest(fs::read(path).unwrap())));
        fs::write(folder.join(format!("supervision-cleanup-{mode}.json")),serde_json::to_vec_pretty(&json!({"input":input,"result":result,"committed":fs::read_to_string(home.0.join("helper-committed")).ok(),"cleaned":fs::read_to_string(home.0.join("helper-cleaned")).ok(),"killTracePresent":home.0.join("kill-trace").exists(),"shimSha256":shim_sha})).unwrap()).unwrap();
    }
    (home, result.get("result").unwrap().clone())
}
#[test]
fn parent_panic_closes_output_before_waiting_for_helper() {
    let (home, result) = cleanup_probe("parent-panic", true);
    assert_eq!(result["panicCaught"], true);
    assert_eq!(
        fs::read_to_string(home.0.join("helper-committed")).unwrap(),
        "committed"
    );
    assert_eq!(
        fs::read_to_string(home.0.join("helper-cleaned")).unwrap(),
        "closed"
    );
    assert!(
        !home.0.join("kill-trace").exists(),
        "parent killed helper after commit"
    );
}
#[test]
fn stderr_cannot_block_helper_cleanup() {
    let (_home, result) = cleanup_probe("stderr-flood", false);
    assert_eq!(result["response"]["ok"], true);
}

#[test]
fn cancel_deadline_and_panic_keep_independent_group_cleanup() {
    let home = runtime_host::OwnedDir::new();
    fs::write(
        home.0.join(".ruycast-supervision-fixture"),
        "owned-supervision-helper-v1",
    )
    .unwrap();
    let cli = std::env::var_os("RUYCAST_TEST_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_host::root().join(".build/rust/release/ruycast"));
    assert!(cli.is_absolute() && cli.is_file());
    let fixture = env!("CARGO_BIN_EXE_supervision-probe");
    let pid_file = home.0.join("script-pid");
    let sentinel = home.0.join("descendant");
    for option in ["cancelMs", "deadlineMs", "panicMs"] {
        let mut input = json!({"helper":cli,"command":{"id":"script.fixture","title":"Fixture","kind":"script","target":fixture,"arguments":["__supervision-tree",home.0],"timeout":2},"home":home.0});
        input[option] = json!(200);
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
            "RUYCAST_SUPERVISION_WAIT_MARKER",
            "RUYCAST_SUPERVISION_KILL_TRACE",
            "RUYCAST_SUPERVISION_HELPER_MODE",
            "RUYCAST_SUPERVISION_HELPER_HOME",
        ] {
            cmd.env_remove(key);
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
        if option == "panicMs" {
            assert_eq!(result["result"]["panicCaught"], true);
        } else {
            assert_eq!(result["result"]["response"]["ok"], false);
        }
        assert!(
            pid_file.exists(),
            "script did not start ({option}): {result}"
        );
        let pid: libc::pid_t = fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        assert!(pid > 0);
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        let error = std::io::Error::last_os_error().raw_os_error().unwrap();
        assert_eq!(error, libc::ESRCH);
        std::thread::sleep(Duration::from_millis(850));
        assert!(!sentinel.exists());
        if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
            let folder = PathBuf::from(folder);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join(format!("supervision-tree-{option}.json")),serde_json::to_vec_pretty(&json!({"input":input,"result":result,"leaderPid":pid,"killZeroErrno":error,"sentinelAbsentAfter850ms":!sentinel.exists(),"fixtureSha256":format!("{:x}",Sha256::digest(fs::read(fixture).unwrap()))})).unwrap()).unwrap();
        }
        fs::remove_file(&pid_file).unwrap();
        if sentinel.exists() {
            fs::remove_file(&sentinel).unwrap();
        }
    }
}
