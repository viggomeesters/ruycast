//! Real script probe and shipping CLI in isolated process fixtures.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
struct OwnedDir(PathBuf);
impl OwnedDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("owned fixture parent"),
        );
        assert!(parent.is_absolute() && parent.is_dir());
        let path = parent.join(format!(
            "script-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for OwnedDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(program: &Path, arguments: &[&str], input: &Value) -> Value {
    let mut c = Command::new(program);
    c.args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "DYLD_INSERT_LIBRARIES",
        "RUYCAST_HOME",
        "RUSTCAST_HOME",
        "RUYCAST_CONFIG_DIR",
        "RUSTCAST_CONFIG_DIR",
    ] {
        c.env_remove(key);
    }
    let mut child = c.spawn().unwrap();
    let written = child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes());
    let output = child_output::wait(child, Duration::from_secs(5));
    written.expect("send fixture request");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn command(target: &str, args: Value) -> Value {
    json!({"id":"script.fixture","title":"Script fixture","kind":"script","target":target,"arguments":args})
}
fn probe(home: &Path, command: Value) -> Value {
    let output = run(
        Path::new(env!("CARGO_BIN_EXE_script-probe")),
        &[],
        &json!({"command":command,"home":home}),
    );
    assert_eq!(output["noChildren"], true);
    output["result"].clone()
}
#[test]
fn literal_arguments_and_cli_identical_response() {
    let home = OwnedDir::new();
    let c = command(
        "/usr/bin/printf",
        json!(["%s", "literal $(touch NEVER);\nΩ"]),
    );
    let expected = probe(&home.0, c.clone())["response"].clone();
    assert_eq!(expected["result"]["stdout"], c["arguments"][1]);
    let cli = std::env::var_os("RUYCAST_TEST_CLI").expect("explicit source-built CLI");
    assert_eq!(run(Path::new(&cli), &["__run-script"], &c), expected);
}
#[test]
fn invalid_timeout_kind_path_and_spawn_failure() {
    let home = OwnedDir::new();
    let c = command("/bin/sh", json!([]));
    for (key, value) in [
        ("timeout", json!(0)),
        ("timeout", json!(301)),
        ("kind", json!("url")),
        ("target", json!("relative")),
    ] {
        let mut changed = c.clone();
        changed[key] = value;
        assert_eq!(probe(&home.0, changed)["ok"], false);
    }
    assert_eq!(
        probe(
            &home.0,
            command(home.0.join("missing").to_str().unwrap(), json!([]))
        )["ok"],
        false
    );
}
#[test]
fn repeated_missing_executable_leaves_no_children() {
    let home = OwnedDir::new();
    for _ in 0..12 {
        assert_eq!(
            probe(
                &home.0,
                command(home.0.join("missing").to_str().unwrap(), json!([]))
            )["ok"],
            false
        );
    }
}
