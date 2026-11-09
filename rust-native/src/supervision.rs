//! Independent Rust CLI supervision. The stdin writer is the host lifeline;
//! no owner PID polling or custom inherited descriptors are needed.
use rustcast_core::Command;
#[cfg(any(test, feature = "test-tools"))]
use serde_json::json;
use serde_json::Value;
use std::{
    fs,
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::MetadataExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command as Process, Stdio},
    thread,
    time::{Duration, Instant},
};

const HEADER_LIMIT: usize = 8 * 1024 * 1024;
const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
const STARTUP: Duration = Duration::from_secs(3);
const CANCEL: &str = "Script cancelled; execution may have already occurred.";
const UNKNOWN: &str =
    "Script runner stopped unexpectedly; check the command's outcome before retrying.";

/// Plain trusted transport outcome, intentionally not serialized as a provider
/// response. The CLI receives response only; native policy also reads capture.
#[derive(Debug)]
pub struct Outcome {
    pub response: Value,
    pub capture: Option<rustcast_core::script::Capture>,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Ready {
    script_supervisor: u64,
    ready: bool,
}
fn validate_ready(bytes: &[u8]) -> Result<(), String> {
    let ready: Ready = serde_json::from_slice(bytes)
        .map_err(|_| "The script runner is incompatible. Check the app installation.")?;
    if ready.script_supervisor != rustcast_core::script::SUPERVISOR_VERSION || !ready.ready {
        return Err("The script runner is incompatible. Check the app installation.".into());
    }
    Ok(())
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reply {
    ok: bool,
    script_supervisor: u64,
    response: Value,
    // Required field, nullable only when no Completion was produced.
    #[serde(deserialize_with = "required_capture")]
    capture: Option<rustcast_core::script::Capture>,
}
fn required_capture<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<rustcast_core::script::Capture>, D::Error> {
    serde::Deserialize::deserialize(d)
}
fn decode_reply(bytes: &[u8]) -> Result<Outcome, String> {
    let reply: Reply = serde_json::from_slice(bytes).map_err(|_| UNKNOWN)?;
    if !reply.ok || reply.script_supervisor != rustcast_core::script::SUPERVISOR_VERSION {
        return Err(UNKNOWN.into());
    }
    validate_response(&reply.response)?;
    if reply.capture.is_none()
        && (reply.response["ok"] == true || reply.response.get("exitCode").is_some())
    {
        return Err(UNKNOWN.into());
    }
    Ok(Outcome {
        response: reply.response,
        capture: reply.capture,
    })
}

fn trusted(path: PathBuf) -> Result<PathBuf, String> {
    let info = fs::symlink_metadata(&path)
        .map_err(|_| "The script runner is missing. Check the app installation.")?;
    if !info.is_file()
        || ![0, unsafe { libc::getuid() }].contains(&info.uid())
        || info.mode() & 0o6022 != 0
        || info.mode() & 0o111 == 0
    {
        return Err("The script runner is unavailable. Check the app installation.".into());
    }
    Ok(path)
}
pub fn discover() -> Result<PathBuf, String> {
    discover_from(
        &std::env::current_exe()
            .map_err(|e| e.to_string())?
            .canonicalize()
            .map_err(|e| e.to_string())?,
    )
}
fn discover_from(executable: &Path) -> Result<PathBuf, String> {
    let parent = executable.parent().ok_or("Missing application folder")?;
    if parent.file_name().is_some_and(|s| s == "MacOS")
        && parent
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|s| s == "Contents")
    {
        return trusted(parent.parent().unwrap().join("Resources/ruycast"));
    }
    for ancestor in executable.ancestors().skip(1) {
        if ancestor.join("rust/Cargo.toml").is_file()
            && ancestor.join("rust-native/Cargo.toml").is_file()
        {
            return trusted(ancestor.join(".build/rust/release/ruycast"));
        }
    }
    Err("The script runner is missing. Check the app installation.".into())
}

struct Helper {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: Option<ChildStdout>,
    may_commit: bool,
}
impl Helper {
    fn cancel(&mut self) {
        self.input.take();
    }
    fn wait(&mut self) -> Result<std::process::ExitStatus, String> {
        self.input.take();
        self.output.take();
        let child = self.child.as_mut().ok_or(UNKNOWN)?;
        child.stderr.take();
        match child.wait() {
            Ok(status) => {
                self.child.take();
                Ok(status)
            }
            Err(error) => {
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.child.take();
                }
                Err(UNKNOWN.into())
            }
        }
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        self.input.take();
        self.output.take();
        if let Some(mut child) = self.child.take() {
            child.stdout.take();
            child.stderr.take();
            // Before possible newline commit, the helper has no executable
            // command. Afterwards only its independent runner may clean up.
            if !self.may_commit {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let peek = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        child.id() as libc::id_t,
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                };
                if peek == 0 {
                    let _ = child.kill();
                } else if io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
                    return;
                }
            }
            for _ in 0..3 {
                match child.wait() {
                    Ok(_) => break,
                    Err(e) if e.raw_os_error() == Some(libc::ECHILD) => break,
                    Err(_) => thread::yield_now(),
                }
            }
        }
    }
}
fn nonblocking(fd: i32) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("Could not prepare the script runner.".into());
    }
    Ok(())
}
fn line(
    helper: &mut Helper,
    limit: usize,
    deadline: Option<Instant>,
    cancel: &mut dyn FnMut() -> bool,
    final_line: bool,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 16384];
    loop {
        if cancel() || deadline.is_some_and(|d| Instant::now() >= d) {
            helper.cancel();
            if !helper.may_commit {
                return Err(CANCEL.into());
            }
        }
        let result = helper.output.as_mut().ok_or(UNKNOWN)?.read(&mut buffer);
        match result {
            Ok(0) => {
                if final_line
                    && bytes.last() == Some(&b'\n')
                    && bytes.iter().filter(|b| **b == b'\n').count() == 1
                {
                    return Ok(bytes);
                }
                return Err(UNKNOWN.into());
            }
            Ok(n) => {
                if bytes.len() + n > limit {
                    return Err("Script runner response exceeds the supported size; check its outcome before retrying.".into());
                }
                bytes.extend_from_slice(&buffer[..n]);
                if !final_line && bytes.contains(&b'\n') {
                    if bytes.last() != Some(&b'\n')
                        || bytes.iter().filter(|b| **b == b'\n').count() != 1
                    {
                        return Err(UNKNOWN.into());
                    }
                    return Ok(bytes);
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                let wait_deadline = if helper.input.is_some() { deadline } else { None };
                rustcast_core::script::wait_pipe_readiness(
                    &mut [libc::pollfd {
                        fd: helper.output.as_ref().ok_or(UNKNOWN)?.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    }],
                    wait_deadline,
                )
                .map_err(|_| UNKNOWN.to_string())?;
            }
            Err(_) => return Err(UNKNOWN.into()),
        }
    }
}

pub(crate) fn run(
    path: &Path,
    command: &Command,
    home: &Path,
    deadline: Option<Instant>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Outcome, String> {
    if cancelled() || deadline.is_some_and(|d| Instant::now() >= d) {
        return Err(CANCEL.into());
    }
    if !home.is_absolute() || home.as_os_str().as_bytes().contains(&0) {
        return Err("The script home folder must be an absolute path.".into());
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Header<'a> {
        command: &'a Command,
        home_bytes: &'a [u8],
    }
    let header = serde_json::to_vec(&Header {
        command,
        home_bytes: home.as_os_str().as_bytes(),
    })
    .map_err(|e| e.to_string())?;
    if header.len() >= HEADER_LIMIT {
        return Err("Script runner input exceeds the supported size.".into());
    }
    let startup = Instant::now() + STARTUP;
    let startup = deadline.map_or(startup, |d| d.min(startup));
    let mut launcher = Process::new(path);
    launcher
        .arg("__run-script-supervised")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    // Same validated std fork/exec error-pipe path as the shared script runner.
    unsafe {
        launcher.pre_exec(|| Ok(()));
    }
    let child = launcher
        .spawn()
        .map_err(|_| "The script runner could not start. Check the app installation.")?;
    let mut helper = Helper {
        child: Some(child),
        input: None,
        output: None,
        may_commit: false,
    };
    helper.input = helper.child.as_mut().unwrap().stdin.take();
    helper.output = helper.child.as_mut().unwrap().stdout.take();
    nonblocking(
        helper
            .input
            .as_ref()
            .ok_or("Script runner input unavailable")?
            .as_raw_fd(),
    )?;
    nonblocking(
        helper
            .output
            .as_ref()
            .ok_or("Script runner output unavailable")?
            .as_raw_fd(),
    )?;
    let ready = line(&mut helper, 4096, Some(startup), &mut cancelled, false)?;
    validate_ready(&ready)?;
    let mut sent = 0;
    while sent < header.len() {
        if cancelled() || Instant::now() >= startup {
            return Err(CANCEL.into());
        }
        match helper.input.as_mut().unwrap().write(&header[sent..]) {
            Ok(0) => return Err(UNKNOWN.into()),
            Ok(n) => sent += n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                rustcast_core::script::wait_pipe_readiness(
                    &mut [libc::pollfd {
                        fd: helper.input.as_ref().ok_or(UNKNOWN)?.as_raw_fd(),
                        events: libc::POLLOUT,
                        revents: 0,
                    }],
                    Some(startup),
                )
                .map_err(|_| UNKNOWN.to_string())?;
            }
            Err(_) => return Err(UNKNOWN.into()),
        }
    }
    // Conservative BEFORE attempting the delimiter: even an uncertain write
    // must never trigger direct helper termination after possible execution.
    helper.may_commit = true;
    loop {
        if cancelled() || Instant::now() >= startup {
            return Err(CANCEL.into());
        }
        match helper.input.as_mut().unwrap().write(b"\n") {
            Ok(1) => break,
            Ok(_) => return Err(UNKNOWN.into()),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                rustcast_core::script::wait_pipe_readiness(
                    &mut [libc::pollfd {
                        fd: helper.input.as_ref().ok_or(UNKNOWN)?.as_raw_fd(),
                        events: libc::POLLOUT,
                        revents: 0,
                    }],
                    Some(startup),
                )
                .map_err(|_| UNKNOWN.to_string())?;
            }
            Err(_) => return Err(UNKNOWN.into()),
        }
    }
    let response = line(&mut helper, RESPONSE_LIMIT, deadline, &mut cancelled, true)?;
    let status = helper.wait()?;
    if !status.success() {
        return Err(UNKNOWN.into());
    }
    decode_reply(&response)
}

#[cfg(feature = "test-tools")]
pub fn probe(
    path: &Path,
    command: &Command,
    home: &Path,
    deadline: Option<Instant>,
    cancelled: impl FnMut() -> bool,
) -> Result<Value, String> {
    let outcome = run(path, command, home, deadline, cancelled)?;
    Ok(json!({"response":outcome.response,"capture":outcome.capture}))
}

fn validate_response(value: &Value) -> Result<(), String> {
    let out = |v: &Value| {
        v.as_str()
            .is_some_and(|s| s.len() <= rustcast_core::script::OUTPUT_LIMIT * 3)
    };
    if value["ok"] == true {
        if value.as_object().is_none_or(|m| m.len() != 2)
            || value["result"].as_object().is_none_or(|m| m.len() != 3)
            || value["result"]["exitCode"] != 0
            || !out(&value["result"]["stdout"])
            || !out(&value["result"]["stderr"])
        {
            return Err(UNKNOWN.into());
        }
    } else if value["ok"] == false {
        let error = value["error"]
            .as_str()
            .filter(|s| s.len() <= 4096)
            .ok_or(UNKNOWN)?;
        if error.is_empty() {
            return Err(UNKNOWN.into());
        }
        if let Some(exit) = value.get("exitCode") {
            if !exit
                .as_i64()
                .is_some_and(|n| i32::try_from(n).is_ok() && n != 0)
                || !out(&value["stdout"])
                || !out(&value["stderr"])
                || value.as_object().is_none_or(|m| m.len() != 5)
            {
                return Err(UNKNOWN.into());
            }
        } else if value.as_object().is_none_or(|m| m.len() != 2) {
            return Err(UNKNOWN.into());
        }
    } else {
        return Err(UNKNOWN.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn ready_frame_rejects_old_duplicate_unknown_and_mistyped_fields() {
        assert!(validate_ready(br#"{"scriptSupervisor":3,"ready":true}"#).is_ok());
        for raw in [
            br#"{"scriptSupervisor":1,"ready":true}"#.as_slice(),
            br#"{"scriptSupervisor":3,"ready":false}"#,
            br#"{"scriptSupervisor":3,"ready":"true"}"#,
            br#"{"scriptSupervisor":3,"ready":true,"extra":1}"#,
            br#"{"scriptSupervisor":1,"scriptSupervisor":3,"ready":true}"#,
        ] {
            assert!(validate_ready(raw).is_err());
        }
    }
    #[test]
    fn version_three_reply_requires_known_capture_for_completed_processes() {
        let raw = json!({"ok":true,"scriptSupervisor":rustcast_core::script::SUPERVISOR_VERSION,
            "response":{"ok":true,"result":{"exitCode":0,"stdout":"","stderr":""}},
            "capture":rustcast_core::script::Capture::empty()});
        assert!(decode_reply(&serde_json::to_vec(&raw).unwrap()).is_ok());
        let mut missing = raw.clone();
        missing.as_object_mut().unwrap().remove("capture");
        assert!(decode_reply(&serde_json::to_vec(&missing).unwrap()).is_err());
        for value in [
            Value::Null,
            json!({}),
            json!({"stdout":{"truncated":false,"validUtf8":true},"stderr":{"truncated":false,"eof":true,"validUtf8":true}}),
            json!({"stdout":{"truncated":false,"eof":true,"validUtf8":true},"stderr":{"truncated":false}}),
            json!({"stdout":{"truncated":"false","eof":true,"validUtf8":true},"stderr":{"truncated":false,"eof":true,"validUtf8":true}}),
            json!({"stdout":{"truncated":false,"eof":true,"validUtf8":true,"unknown":false},"stderr":{"truncated":false,"eof":true,"validUtf8":true}}),
        ] {
            let mut bad = raw.clone();
            bad["capture"] = value;
            assert!(decode_reply(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
        let mut bad = raw.clone();
        bad["scriptSupervisor"] = json!(1);
        assert!(decode_reply(&serde_json::to_vec(&bad).unwrap()).is_err());
        bad = raw.clone();
        bad["extra"] = json!(true);
        assert!(decode_reply(&serde_json::to_vec(&bad).unwrap()).is_err());
        let bytes = serde_json::to_string(&raw).unwrap();
        let duplicate = bytes.replacen("{", "{\"capture\":null,", 1);
        assert!(decode_reply(duplicate.as_bytes()).is_err());
        let failure = json!({"ok":true,"scriptSupervisor":3,"response":{"ok":false,"error":"start failed"},"capture":null});
        assert!(decode_reply(&serde_json::to_vec(&failure).unwrap()).is_ok());
        let mut bad = failure;
        bad["response"] = json!({"ok":false,"error":"exit7","exitCode":7,"stdout":"","stderr":""});
        assert!(decode_reply(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    #[test]
    fn discovery_is_bundle_or_actual_checkout_only_and_rejects_unsafe_files() {
        let root = std::env::temp_dir().join(format!(
            "ruycast-runner-discovery-{}-{}",
            std::process::id(),
            crate::activity::now().to_bits()
        ));
        struct Temp(PathBuf);
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Temp(root.clone());
        for folder in ["rust", "rust-native", ".build/rust/release"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::write(root.join("rust/Cargo.toml"), b"fixture").unwrap();
        fs::write(root.join("rust-native/Cargo.toml"), b"fixture").unwrap();
        let runner = root.join(".build/rust/release/ruycast");
        fs::write(&runner, b"fixture").unwrap();
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            discover_from(&root.join("rust-native/target/debug/host")).unwrap(),
            runner
        );
        assert!(discover_from(Path::new("/not-a-checkout/host")).is_err());
        assert!(discover_from(&root.join("Fixture.app/Contents/MacOS/host")).is_err()); // no checkout fallback
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(discover_from(&root.join("rust-native/target/debug/host")).is_err());
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o4700)).unwrap();
        assert!(discover_from(&root.join("rust-native/target/debug/host")).is_err());
        fs::remove_file(&runner).unwrap();
        std::os::unix::fs::symlink("/usr/bin/true", &runner).unwrap();
        assert!(discover_from(&root.join("rust-native/target/debug/host")).is_err());
    }
    #[test]
    fn response_shape_is_bounded_and_cannot_claim_false_success() {
        assert!(validate_response(
            &json!({"ok":true,"result":{"exitCode":0,"stdout":"Ω","stderr":""}})
        )
        .is_ok());
        assert!(validate_response(
            &json!({"ok":false,"error":"failure","exitCode":-1,"stdout":"","stderr":""})
        )
        .is_ok());
        for invalid in [
            json!({"ok":true}),
            json!({"ok":true,"result":{"exitCode":1,"stdout":"","stderr":""}}),
            json!({"ok":false,"error":"failure","exitCode":0,"stdout":"","stderr":""}),
            json!({"ok":false,"error":""}),
            json!({"ok":true,"result":{"exitCode":0,"stdout":"x".repeat(196609),"stderr":""}}),
        ] {
            assert!(validate_response(&invalid).is_err());
        }
    }
}
