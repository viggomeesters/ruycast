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
fn stream_probe(home: &Path, mode: &str, options: Value) -> Value {
    let target = env!("CARGO_BIN_EXE_private-results-fixture");
    let mut c = command(target, json!(["script-streams", mode]));
    if mode == "flood" {
        c["timeout"] = json!(0.25);
    }
    let mut request = json!({"home":home,"command":c});
    request
        .as_object_mut()
        .unwrap()
        .extend(options.as_object().unwrap().clone());
    let output = run(Path::new(env!("CARGO_BIN_EXE_script-probe")), &[], &request);
    assert_eq!(output["noChildren"], true);
    output["result"].clone()
}
#[test]
fn nonzero_and_signal_response() {
    let h = OwnedDir::new();
    let v = stream_probe(&h.0, "exit7", json!({}));
    let r = &v["response"];
    assert_eq!(r["ok"], false);
    assert_eq!(r["exitCode"], 7);
    assert_eq!(r["stdout"], "output");
    assert_eq!(r["stderr"], "error");
    let v = stream_probe(&h.0, "signal", json!({}));
    assert_eq!(v["response"]["exitCode"], -1);
}
#[test]
fn both_output_streams_bounded_and_drained() {
    let h = OwnedDir::new();
    let v = stream_probe(&h.0, "bounded", json!({}));
    assert_eq!(v["response"]["ok"], true);
    assert_eq!(v["stdoutBytes"], 65536);
    assert_eq!(v["stderrBytes"], 65536);
}
#[test]
fn continuous_flood_cannot_starve_cancel_or_timeout() {
    let h = OwnedDir::new();
    for (options, end) in [
        (json!({}), "timeout"),
        (json!({"cancelMs":150}), "cancelled"),
    ] {
        let v = stream_probe(&h.0, "flood", options);
        assert_eq!(v["end"], end);
        assert_eq!(v["stdoutBytes"], 65536);
        assert_eq!(v["stderrBytes"], 65536);
    }
}
#[test]
fn home_expansion_and_null_stdin() {
    use std::os::unix::fs::PermissionsExt;
    let h = OwnedDir::new();
    let fixture = h.0.join("fixture");
    fs::copy(env!("CARGO_BIN_EXE_private-results-fixture"), &fixture).unwrap();
    fs::set_permissions(&fixture, fs::Permissions::from_mode(0o700)).unwrap();
    let c = command("~/fixture", json!(["script-streams", "stdin-eof"]));
    assert_eq!(probe(&h.0, c)["response"]["result"]["stdout"], "1");
}
#[test]
fn cancel_and_deadline_before_spawn() {
    let h = OwnedDir::new();
    let sentinel = h.0.join("should-not-run");
    let c = command("/usr/bin/touch", json!([sentinel]));
    for (option, end) in [("cancelMs", "cancelled"), ("deadlineMs", "deadline")] {
        let mut req = json!({"home":h.0,"command":c});
        req[option] = json!(0);
        let v = run(Path::new(env!("CARGO_BIN_EXE_script-probe")), &[], &req);
        assert_eq!(v["noChildren"], true);
        assert_eq!(v["result"]["end"], end);
        assert!(!sentinel.exists());
    }
}
fn tree_case(mode: &str) {
    let h = OwnedDir::new();
    let sentinel = h.0.join("descendant-ran");
    let pid = h.0.join("leader.pid");
    let mut c = command(
        env!("CARGO_BIN_EXE_script-probe"),
        json!(["script-tree", "leader", pid, sentinel]),
    );
    let mut request = json!({"home":h.0,"command":c});
    match mode {
        "timeout" => {
            c = request["command"].clone();
            c["timeout"] = json!(0.2);
            request["command"] = c;
        }
        "cancelled" => request["cancelMs"] = json!(200),
        "deadline" => request["deadlineMs"] = json!(200),
        "panic" => request["panicMs"] = json!(200),
        _ => unreachable!(),
    };
    let output = run(Path::new(env!("CARGO_BIN_EXE_script-probe")), &[], &request);
    assert_eq!(output["noChildren"], true);
    let r = &output["result"];
    if mode == "panic" {
        assert_eq!(r["panicCaught"], true);
    } else {
        assert_eq!(r["end"], mode);
        if mode == "timeout" {
            assert!(r["response"]["error"]
                .as_str()
                .unwrap()
                .contains("timed out"));
        }
        if mode == "cancelled" {
            assert!(r["response"]["error"]
                .as_str()
                .unwrap()
                .contains("may have already occurred"));
        }
    }
    assert!(pid.exists(), "fixture never started");
    let leader = fs::read_to_string(pid).unwrap().parse::<i32>().unwrap();
    assert_eq!(unsafe { libc::kill(leader, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    std::thread::sleep(Duration::from_millis(850));
    assert!(!sentinel.exists(), "descendant survived");
}
#[test]
fn timeout_cleans_group() {
    tree_case("timeout");
}
#[test]
fn cancellation_cleans_group() {
    tree_case("cancelled");
}
#[test]
fn absolute_deadline_cleans_group() {
    tree_case("deadline");
}
#[test]
fn callback_panic_cleans_group() {
    tree_case("panic");
}
#[test]
fn early_success_closes_inherited_pipes() {
    let h = OwnedDir::new();
    let sentinel = h.0.join("pipe-descendant");
    let c = command(
        env!("CARGO_BIN_EXE_script-probe"),
        json!(["script-tree", "early", h.0.join("unused-pid"), sentinel]),
    );
    let result = probe(&h.0, c);
    assert_eq!(result["response"]["result"]["stdout"], "done\n");
    std::thread::sleep(Duration::from_millis(850));
    assert!(!sentinel.exists());
}
