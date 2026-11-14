//! Real supervisor refusal before launch, with only disposable sentinel paths.
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
    io::{self, Read, Write},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
fn cli() -> PathBuf {
    let path = std::env::var_os("RUYCAST_TEST_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_host::root().join(".build/rust/release/ruycast"));
    assert!(
        path.is_absolute() && path.is_file(),
        "Build the real Rust CLI first"
    );
    path
}
fn command(marker: &Path) -> Value {
    json!({"id":"script.fixture","title":"Fixture","kind":"script","target":"/usr/bin/touch","arguments":[marker],"timeout":2})
}
fn spawn(program: &Path, args: &[&str]) -> Child {
    let mut cmd = Command::new(program);
    cmd.args(args)
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
        cmd.env_remove(key);
    }
    cmd.spawn().unwrap()
}
// Own the child until the bounded output helper takes over, including assertion unwind.
struct PendingChild(Option<Child>);
impl Drop for PendingChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn ready_line(child: &mut Child) -> io::Result<Vec<u8>> {
    let stdout = child.stdout.as_mut().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut data = vec![];
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let mut fd = libc::pollfd {
            fd: stdout.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = remaining.as_millis().min(i32::MAX as u128) as i32;
        let n = unsafe { libc::poll(&mut fd, 1, timeout) };
        if n < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if n == 0 {
            break;
        }
        let mut byte = [0];
        if stdout.read(&mut byte)? == 0 {
            return Ok(data);
        }
        data.push(byte[0]);
        if byte[0] == b'\n' {
            return Ok(data);
        }
        if data.len() > 1_048_576 {
            return Err(io::Error::other("Own helper ready line exceeds limit"));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "Own helper protocol line timed out",
    ))
}
fn evidence(name: &str, input: &[u8], result: &Value) {
    if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let folder = PathBuf::from(folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(format!("supervision-{name}.json")),serde_json::to_vec_pretty(&json!({"inputBytes":input.len(),"inputSha256":format!("{:x}",Sha256::digest(input)),"result":result})).unwrap()).unwrap();
    }
}
fn finish(mut child: Child, input: Vec<u8>, timeout: Duration) -> Value {
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let out = child_output::wait(child, timeout);
    let written = writer.join().unwrap();
    // communicate() in the original driver also tolerates a peer closing stdin early.
    if let Err(error) = written {
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn direct(name: &str, payload: Vec<u8>) -> Value {
    let mut child = PendingChild(Some(spawn(&cli(), &["__run-script-supervised"])));
    let ready: Value =
        serde_json::from_slice(&ready_line(child.0.as_mut().unwrap()).unwrap()).unwrap();
    assert_eq!(ready, json!({"scriptSupervisor":3,"ready":true}));
    let input = payload.clone();
    let envelope = finish(child.0.take().unwrap(), payload, Duration::from_secs(6));
    assert_eq!(envelope["scriptSupervisor"], 3);
    assert_eq!(envelope["ok"], true);
    evidence(name, &input, &envelope);
    envelope.get("response").unwrap().clone()
}
#[test]
fn failed_helper_start_has_no_transient_child() {
    let home = runtime_host::OwnedDir::new();
    let sentinel = home.0.join("must-not-launch");
    for index in 0..8 {
        let request =
            json!({"helper":home.0.join("missing"),"command":command(&sentinel),"home":home.0});
        let bytes = serde_json::to_vec(&request).unwrap();
        let result = finish(
            spawn(Path::new(env!("CARGO_BIN_EXE_supervision-probe")), &[]),
            bytes.clone(),
            Duration::from_secs(7),
        );
        assert_eq!(result["noChildren"], true);
        assert!(result["result"].get("error").is_some());
        assert!(!sentinel.exists());
        evidence(&format!("missing-helper-{index}"), &bytes, &result);
    }
}
#[test]
fn partial_header_eof_and_invalid_header_cannot_launch() {
    let home = runtime_host::OwnedDir::new();
    let marker = home.0.join("must-not-launch");
    let header = serde_json::to_vec(
        &json!({"command":command(&marker),"homeBytes":home.0.as_os_str().as_encoded_bytes()}),
    )
    .unwrap();
    let mut extra = header.clone();
    extra.extend_from_slice(b"\nextra");
    for (index, payload) in [
        header.clone(),
        header[..30].to_vec(),
        b"{invalid}\n".to_vec(),
        extra,
    ]
    .into_iter()
    .enumerate()
    {
        let result = direct(&format!("partial-header-{index}"), payload);
        assert_eq!(result["ok"], false);
        assert!(!marker.exists());
    }
}
#[test]
fn oversized_header_and_invalid_home_fail_before_launch() {
    let mut payload = vec![b'x'; 8 * 1024 * 1024 + 1];
    payload.push(b'\n');
    assert_eq!(direct("oversized-header", payload)["ok"], false);
    let home = runtime_host::OwnedDir::new();
    let marker = home.0.join("must-not-launch");
    for (index, raw_home) in [vec![], b"relative".to_vec(), b"/bad\0home".to_vec()]
        .into_iter()
        .enumerate()
    {
        let mut payload =
            serde_json::to_vec(&json!({"command":command(&marker),"homeBytes":raw_home})).unwrap();
        payload.push(b'\n');
        let result = direct(&format!("invalid-home-{index}"), payload);
        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("home"));
        assert!(!marker.exists());
    }
}

fn finish_live(mut child: Child, timeout: Duration) {
    // Closing stdin only after the response proves live-header handling, not EOF parsing.
    drop(child.stdin.take());
    let out = child_output::wait(child, timeout);
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
}
#[test]
fn large_encoded_header_preserves_payload_or_reports_os_limit() {
    let home = runtime_host::OwnedDir::new();
    let c = json!({"id":"script.fixture","title":"Fixture","kind":"script","target":"/usr/bin/printf","arguments":["%s","\u{1}".repeat(400000)],"timeout":2});
    let request = json!({"helper":cli(),"command":c,"home":home.0});
    let input = serde_json::to_vec(&request).unwrap();
    let result = finish(
        spawn(Path::new(env!("CARGO_BIN_EXE_supervision-probe")), &[]),
        input.clone(),
        Duration::from_secs(7),
    );
    assert_eq!(result["noChildren"], true);
    let response = result["result"].get("response").unwrap();
    if response.get("ok").unwrap().as_bool().unwrap() {
        assert_eq!(response["result"]["stdout"], "\u{1}".repeat(65536));
    } else {
        assert!(response["error"]
            .as_str()
            .unwrap()
            .contains("Argument list too long"));
    }
    evidence("large-encoded-header", &input, &result);
}
#[test]
fn non_utf8_home_is_expanded_as_raw_path_bytes() {
    use std::os::unix::ffi::OsStringExt;
    let home = runtime_host::OwnedDir::new();
    let mut raw_home = home.0.as_os_str().as_encoded_bytes().to_vec();
    raw_home.extend_from_slice(b"/raw-\xff");
    let mut raw_target = raw_home.clone();
    raw_target.extend_from_slice(b"/fixture");
    let expected_errno = match Command::new(std::ffi::OsString::from_vec(raw_target)).output() {
        Err(error) => error.raw_os_error().unwrap(),
        Ok(_) => panic!("Nonexistent own fixture unexpectedly executed"),
    };
    let mut c = command(&home.0.join("unused"));
    c["target"] = json!("~/fixture");
    c["arguments"] = json!([]);
    let header = json!({"command":c,"homeBytes":raw_home});
    let mut payload = serde_json::to_vec(&header).unwrap();
    payload.push(b'\n');
    let mut child = PendingChild(Some(spawn(&cli(), &["__run-script-supervised"])));
    let _: Value = serde_json::from_slice(&ready_line(child.0.as_mut().unwrap()).unwrap()).unwrap();
    child
        .0
        .as_mut()
        .unwrap()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&payload)
        .unwrap();
    child
        .0
        .as_mut()
        .unwrap()
        .stdin
        .as_mut()
        .unwrap()
        .flush()
        .unwrap();
    let result: Value =
        serde_json::from_slice(&ready_line(child.0.as_mut().unwrap()).unwrap()).unwrap();
    finish_live(child.0.take().unwrap(), Duration::from_secs(3));
    let response = result.get("response").unwrap();
    assert_eq!(response["ok"], false);
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains(&format!("os error {expected_errno}")));
    evidence("raw-home", &payload, &result);
}
#[test]
fn uncommitted_header_has_independent_startup_deadline() {
    let mut child = PendingChild(Some(spawn(&cli(), &["__run-script-supervised"])));
    let _: Value = serde_json::from_slice(&ready_line(child.0.as_mut().unwrap()).unwrap()).unwrap();
    child
        .0
        .as_mut()
        .unwrap()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{")
        .unwrap();
    child
        .0
        .as_mut()
        .unwrap()
        .stdin
        .as_mut()
        .unwrap()
        .flush()
        .unwrap();
    let result: Value =
        serde_json::from_slice(&ready_line(child.0.as_mut().unwrap()).unwrap()).unwrap();
    assert_eq!(result["response"]["ok"], false);
    assert!(result["response"]["error"]
        .as_str()
        .unwrap()
        .contains("timed out"));
    finish_live(child.0.take().unwrap(), Duration::from_secs(2));
    evidence("uncommitted-header", b"{", &result);
}
