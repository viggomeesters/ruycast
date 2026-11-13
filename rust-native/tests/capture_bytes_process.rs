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
    run_env(program, arguments, input, &[], true, Duration::from_secs(7))
}
fn run_env(
    program: &Path,
    arguments: &[&str],
    input: &Value,
    environment: &[(&str, &str)],
    require_success: bool,
    timeout: Duration,
) -> Value {
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
        "RUYCAST_TEST_OLD_SUPERVISOR_VERSION",
        "RUYCAST_TEST_OLD_SUPERVISOR_MARKER",
    ] {
        c.env_remove(key);
    }
    c.envs(environment.iter().copied());
    let mut child = c.spawn().unwrap();
    let written = child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes());
    let output = child_output::wait(child, timeout);
    written.expect("send fixture request");
    if require_success {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
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
#[test]
fn spawn_error_has_explicit_unknown_capture_and_legacy_response_is_unchanged() {
    let h = OwnedDir::new();
    let cli = PathBuf::from(std::env::var_os("RUYCAST_TEST_CLI").unwrap());
    let probe_bin = Path::new(env!("CARGO_BIN_EXE_supervision-probe"));
    let target = env!("CARGO_BIN_EXE_script-probe");
    let mut command = json!({"id":"script.fixture","title":"Fixture","kind":"script","target":h.0.join("missing-executable"),"arguments":[],"timeout":2});
    let result = run(
        probe_bin,
        &[],
        &json!({"helper":cli,"home":h.0,"command":command}),
    );
    assert_eq!(result["noChildren"], true);
    assert!(result["result"]["capture"].is_null());
    assert_eq!(result["result"]["response"]["ok"], false);
    for args in [
        json!(["capture-fixture", "bytes", "stdout", "65537"]),
        json!(["capture-fixture", "invalid"]),
        json!(["capture-fixture", "exit7"]),
    ] {
        command["target"] = json!(target);
        command["arguments"] = args;
        let expected = run_env(
            &cli,
            &["__run-script"],
            &command,
            &[],
            false,
            Duration::from_secs(4),
        );
        let actual = run(
            probe_bin,
            &[],
            &json!({"helper":cli,"home":h.0,"command":command}),
        );
        assert_eq!(actual["noChildren"], true);
        assert_eq!(actual["result"]["response"], expected);
    }
}
#[test]
fn old_versions_are_refused_before_command_sideeffect() {
    let h = OwnedDir::new();
    let executable = env!("CARGO_BIN_EXE_script-probe");
    for version in ["1", "2"] {
        let marker = h.0.join(format!("old-helper-command-{version}"));
        let command = json!({"id":"script.fixture","title":"Fixture","kind":"script","target":executable,"arguments":["capture-fixture","invalid"],"timeout":2});
        let marker_text = marker.to_str().unwrap();
        let result = run_env(
            Path::new(env!("CARGO_BIN_EXE_supervision-probe")),
            &[],
            &json!({"helper":executable,"command":command,"home":h.0}),
            &[
                ("RUYCAST_TEST_OLD_SUPERVISOR_VERSION", version),
                ("RUYCAST_TEST_OLD_SUPERVISOR_MARKER", marker_text),
            ],
            true,
            Duration::from_secs(7),
        );
        assert_eq!(result["noChildren"], true);
        assert!(result["result"]["error"]
            .as_str()
            .unwrap()
            .contains("incompatible"));
        assert!(!marker.exists());
    }
}
struct Holder {
    ready: PathBuf,
    gate: PathBuf,
    done: PathBuf,
    released: bool,
}
impl Holder {
    fn new(base: &Path, held: &str) -> Self {
        Self {
            ready: base.join(format!("ready-{held}")),
            gate: base.join(format!("release-{held}")),
            done: base.join(format!("done-{held}")),
            released: false,
        }
    }
    fn release(&mut self) {
        fs::write(&self.gate, "release").unwrap();
        if self.ready.exists() {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !self.done.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                self.done.exists(),
                "owned detached holder did not self-close"
            );
        }
        self.released = true;
    }
}
impl Drop for Holder {
    fn drop(&mut self) {
        if !self.released {
            let _ = fs::write(&self.gate, "release");
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while self.ready.exists() && !self.done.exists() && std::time::Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
#[test]
fn ready_detached_pipe_holders_do_not_create_false_eof() {
    let h = OwnedDir::new();
    for held in ["stdout", "stderr", "both"] {
        let mut holder = Holder::new(&h.0, held);
        let v = probe(&h.0, json!(["capture-holder", "parent", held, h.0]));
        assert!(holder.ready.exists());
        assert!(!holder.done.exists());
        for stream in ["stdout", "stderr"] {
            assert_eq!(
                v["capture"][stream],
                json!({"truncated":false,"validUtf8":true,"eof":held!="both" && held!=stream})
            );
        }
        let provider: Value =
            serde_json::from_str(v["response"]["result"]["stdout"].as_str().unwrap()).unwrap();
        assert_eq!(provider["capture"]["stdout"]["eof"], true);
        holder.release();
    }
}
