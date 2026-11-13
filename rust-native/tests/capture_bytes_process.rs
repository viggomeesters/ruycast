//! Trusted byte capture through the actual supervisor and CLI.
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
    let output = child_output::wait(child, Duration::from_secs(7));
    written.expect("send fixture request");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn probe(home: &Path, args: Value) -> Value {
    let cli =
        PathBuf::from(std::env::var_os("RUYCAST_TEST_CLI").expect("explicit source-built CLI"));
    let command = json!({"id":"script.fixture","title":"Fixture","kind":"script","target":env!("CARGO_BIN_EXE_script-probe"),"arguments":args,"timeout":2});
    let v = run(
        Path::new(env!("CARGO_BIN_EXE_supervision-probe")),
        &[],
        &json!({"helper":cli,"command":command,"home":home}),
    );
    assert_eq!(v["noChildren"], true);
    v["result"].clone()
}
#[test]
fn exact_limit_and_overflow_are_distinct_per_stream() {
    let h = OwnedDir::new();
    for own in ["stdout", "stderr"] {
        for size in [65536, 65537] {
            let other = if own == "stdout" { "stderr" } else { "stdout" };
            let v = probe(
                &h.0,
                json!(["capture-fixture", "bytes", own, size.to_string()]),
            );
            assert_eq!(
                v["capture"][own],
                json!({"truncated":size>65536,"eof":true,"validUtf8":true})
            );
            assert_eq!(
                v["capture"][other],
                json!({"truncated":false,"eof":true,"validUtf8":true})
            );
            assert_eq!(v["response"]["result"][own].as_str().unwrap().len(), 65536);
        }
    }
}
#[test]
fn utf8_validity_describes_retained_raw_prefix_not_full_original_stream() {
    let h = OwnedDir::new();
    for (mode, truncated, valid) in [
        ("omega", false, true),
        ("split", true, false),
        ("invalid", false, false),
    ] {
        let v = probe(&h.0, json!(["capture-fixture", mode]));
        assert_eq!(
            v["capture"]["stdout"],
            json!({"truncated":truncated,"eof":true,"validUtf8":valid})
        );
        assert_eq!(
            v["capture"]["stderr"],
            json!({"truncated":false,"eof":true,"validUtf8":true})
        );
    }
}
#[test]
fn stderr_overflow_and_invalid_utf8_are_independent_of_valid_stdout() {
    let h = OwnedDir::new();
    let v = probe(&h.0, json!(["capture-fixture", "stderr-invalid"]));
    assert_eq!(
        v["capture"]["stdout"],
        json!({"truncated":false,"eof":true,"validUtf8":true})
    );
    assert_eq!(
        v["capture"]["stderr"],
        json!({"truncated":true,"eof":true,"validUtf8":false})
    );
}
#[test]
fn valid_json_prefix_with_discarded_tail_and_spoof_fields_is_still_incomplete() {
    let h = OwnedDir::new();
    let v = probe(&h.0, json!(["capture-fixture", "spoof-prefix"]));
    let prefix: Value =
        serde_json::from_str(v["response"]["result"]["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(prefix["ok"], true);
    assert_eq!(prefix["capture"]["stdout"]["truncated"], false);
    assert_eq!(
        v["capture"]["stdout"],
        json!({"truncated":true,"eof":true,"validUtf8":true})
    );
}
#[test]
fn lossy_json_cannot_claim_its_original_bytes_are_valid() {
    let h = OwnedDir::new();
    let v = probe(&h.0, json!(["capture-fixture", "spoof-invalid"]));
    let native: Value =
        serde_json::from_str(v["response"]["result"]["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(native["detail"], "�");
    assert_eq!(native["capture"]["validUtf8"], true);
    assert_eq!(v["capture"]["stdout"]["validUtf8"], false);
}
