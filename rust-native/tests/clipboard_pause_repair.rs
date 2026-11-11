//! Real headless host with a pre-main abort guard on the general pasteboard.
//! Disposable profiles only; no AppKit activation, keys, Keychain or user data.
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{symlink, DirBuilderExt, MetadataExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn rpc(socket: &Path, request: Value) -> Value {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    serde_json::to_writer(&mut stream, &request).unwrap();
    stream.write_all(b"\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
struct Host {
    base: PathBuf,
    child: Option<Child>,
}
impl Host {
    fn new(kind: &str) -> Self {
        let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
                fs::create_dir_all(&target).unwrap();
                assert!(fs::symlink_metadata(&target).unwrap().is_dir(), "Fixture target must not be a symlink");
                assert_eq!(fs::canonicalize(&target).unwrap(), target, "Fixture target must not traverse symlinks");
                let parent = target.join("pause-fixtures");
                match fs::DirBuilder::new().mode(0o700).create(&parent) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("Cannot create own fixture parent: {error}"),
                }
                parent
            });
        let metadata = fs::symlink_metadata(&parent).expect("Fixture parent must exist");
        assert!(parent.is_absolute() && metadata.is_dir(), "Fixture parent must be an absolute directory, not a symlink");
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() }, "Fixture parent must be owned by this user");
        assert_eq!(metadata.mode() & 0o077, 0, "Fixture parent must be private");
        assert_eq!(fs::canonicalize(&parent).unwrap(), parent, "Fixture parent must not traverse symlinks");
        let base = parent.join(format!("rcpr-{}-{kind}", std::process::id()));
        assert!(base.join("p/control.sock").as_os_str().len() <= 104, "Set RUYCAST_NATIVE_TEST_FIXTURE_DIR to a shorter own private directory");
        assert!(!base.exists());
        fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(base.join("p"))
            .unwrap();
        fs::create_dir(base.join("Applications")).unwrap();
        fs::write(
            base.join(".ruycast-test-fixture"),
            b"own-disposable-fixture-v1",
        )
        .unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/default.json"),
            base.join("p/config.json"),
        )
        .unwrap();
        Self { base, child: None }
    }
    fn socket(&self) -> PathBuf {
        self.base.join("p/control.sock")
    }
    fn start(&mut self, shims: &str, fault: bool) {
        self.start_with(shims, fault, false);
    }
    fn start_with(&mut self, shims: &str, fault: bool, native: bool) {
        assert!(self.child.is_none());
        let binary = env!("CARGO_BIN_EXE_ruycast-native");
        let mut command = Command::new(binary);
        command
            .arg(if native {
                "--native-hidden"
            } else {
                "--headless-test"
            })
            .env_clear()
            .env("HOME", &self.base)
            .env("RUYCAST_HOME", self.base.join("p"))
            .env("RUYCAST_APP_BINARY", binary)
            .env("DYLD_INSERT_LIBRARIES", shims)
            .env(
                "RUYCAST_NATIVE_TEST_APP_ROOTS",
                serde_json::to_string(&vec![self.base.join("Applications")]).unwrap(),
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if native {
            command
                .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &self.base)
                .env("RUYCAST_NATIVE_TEST_CLIPBOARD", "named")
                .env("RUYCAST_NATIVE_TEST_SNIPPET", "1")
                .env("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD", "1");
        }
        if fault {
            command.env("RUYCAST_TEST_FSYNC_DIRECTORY", self.base.join("p"));
        }
        self.child = Some(command.spawn().unwrap());
        let end = Instant::now() + Duration::from_secs(8);
        while !self.socket().exists() {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "host exited before readiness"
            );
            assert!(Instant::now() < end, "host readiness timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(self.status()["ok"], true);
    }
    fn status(&self) -> Value {
        rpc(&self.socket(), json!({"op":"status"}))
    }
    fn blocked(&self) {
        let status = self.status();
        assert_eq!(status["result"]["clipboardPaused"], true, "{status}");
        assert_eq!(status["result"]["clipboardCaptureActive"], false);
        assert_eq!(status["result"]["health"], "needs-attention");
        assert!(status["result"]["clipboardPauseError"]
            .as_str()
            .unwrap()
            .contains("paused"));
        assert_eq!(
            rpc(&self.socket(), json!({"op":"clipboard.list"}))["ok"],
            true
        );
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            assert_eq!(rpc(&self.socket(), json!({"op":"quit"}))["ok"], true);
            let end = Instant::now() + Duration::from_secs(4);
            while child.try_wait().unwrap().is_none() {
                if Instant::now() >= end {
                    let _ = child.kill();
                    panic!("host shutdown timeout");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(child.wait().unwrap().success());
            assert!(!self.socket().exists());
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if fs::read(self.base.join(".ruycast-test-fixture"))
            .ok()
            .as_deref()
            == Some(b"own-disposable-fixture-v1")
        {
            fs::remove_dir_all(&self.base).unwrap();
        }
    }
}
fn compile(root: &Path, source: &str, output: &str, frameworks: bool) -> PathBuf {
    let output = root.join("target").join(output);
    let mut command = Command::new("cc");
    command.args(["-Wall", "-Wextra", "-Werror", "-dynamiclib"]);
    if frameworks {
        command.args([
            "-fobjc-arc",
            "-fblocks",
            "-framework",
            "AppKit",
            "-framework",
            "ApplicationServices",
        ]);
    }
    let result = command
        .arg(root.join("tests").join(source))
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    output
}
#[test]
fn real_host_preserves_invalid_pause_and_requires_confirmed_explicit_resume() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let guard = compile(
        root,
        "snippet_board_shim.m",
        "pause-repair-board-guard.dylib",
        true,
    );
    let fixture = Host::new("shim-build");
    let fault = ruycast_native::test_fixtures::fcntl(&fixture.base.join("fixture-build.log"));
    let guard = guard.to_str().unwrap();
    let fault_shims = format!("{guard}:{}", fault.display());
    let mut outcomes = Vec::new();
    for (kind, bytes) in [
        ("invalid", b"not-a-bool".as_slice()),
        ("shape", b"{\"paused\":false}".as_slice()),
        ("null", b"null".as_slice()),
        ("symlink", b"false".as_slice()),
        ("directory", b"".as_slice()),
    ] {
        let mut h = Host::new(kind);
        let pause = h.base.join("p/paused.json");
        let target = h.base.join("owned-target");
        match kind {
            "symlink" => {
                fs::write(&target, bytes).unwrap();
                symlink(&target, &pause).unwrap();
            }
            "directory" => fs::create_dir(&pause).unwrap(),
            _ => fs::write(&pause, bytes).unwrap(),
        }
        h.start(guard, false);
        h.blocked();
        std::thread::sleep(Duration::from_millis(250));
        h.blocked();
        h.stop();
        h.start(guard, false);
        h.blocked();
        if kind == "symlink" || kind == "directory" {
            assert_eq!(
                rpc(&h.socket(), json!({"op":"clipboard.pause","paused":false}))["ok"],
                false
            );
            h.blocked();
            if kind == "symlink" {
                assert_eq!(fs::read(&target).unwrap(), bytes);
                assert!(fs::symlink_metadata(&pause)
                    .unwrap()
                    .file_type()
                    .is_symlink());
                fs::remove_file(&pause).unwrap();
            } else {
                assert!(pause.is_dir());
                fs::remove_dir(&pause).unwrap();
            }
        } else {
            assert_eq!(fs::read(&pause).unwrap(), bytes);
            assert_eq!(
                rpc(&h.socket(), json!({"op":"clipboard.clear"}))["ok"],
                true
            );
            h.blocked();
        }
        let response = rpc(&h.socket(), json!({"op":"clipboard.pause","paused":false}));
        assert_eq!(response["ok"], true, "{response}");
        assert_eq!(response["result"]["paused"], false);
        assert!(h.status()["result"]["clipboardPauseError"].is_null());
        assert_eq!(h.status()["result"]["health"], "healthy");
        assert_eq!(fs::read(&pause).unwrap(), b"false");
        assert_eq!(
            fs::metadata(&pause).unwrap().permissions().mode() & 0o777,
            0o600
        );
        h.stop();
        h.start(guard, false);
        assert_eq!(h.status()["result"]["clipboardPaused"], false);
        h.stop();
        outcomes.push(json!({"case":kind,"startup":true,"preserved":true,"reopen":true,"explicitResume":true}));
    }
    let mut h = Host::new("uncertain");
    fs::write(h.base.join("p/paused.json"), b"broken").unwrap();
    h.start(&fault_shims, true);
    h.blocked();
    let mut uncertain_responses = Vec::new();
    for _ in 0..2 {
        let response = rpc(&h.socket(), json!({"op":"clipboard.pause","paused":false}));
        assert_eq!(response["ok"], true, "{response}");
        assert_eq!(response["result"]["paused"], true);
        assert!(response["durabilityWarning"].is_string());
        uncertain_responses.push(response);
        h.blocked();
        assert_eq!(fs::read(h.base.join("p/paused.json")).unwrap(), b"false");
    }
    h.stop();
    // Retest an explicit, now-confirmed retry rather than silently enabling capture.
    fs::write(h.base.join("p/paused.json"), b"broken").unwrap();
    h.start(guard, false);
    h.blocked();
    assert_eq!(
        rpc(&h.socket(), json!({"op":"clipboard.pause","paused":false}))["result"]["paused"],
        false
    );
    assert!(h.status()["result"]["clipboardPauseError"].is_null());
    h.stop();
    let mut missing = Host::new("missing");
    missing.start(guard, false);
    assert_eq!(missing.status()["result"]["clipboardPaused"], false);
    assert!(!missing.base.join("p/paused.json").exists());
    missing.stop();
    let output = root.join("target/clipboard-pause-repair");
    fs::create_dir_all(&output).unwrap();
    let native_shim = compile(
        root,
        "clipboard_history_shim.m",
        "pause-repair-native.dylib",
        true,
    );
    let mut native = Host::new("native");
    fs::write(native.base.join("p/paused.json"), b"broken").unwrap();
    native.start_with(native_shim.to_str().unwrap(), false, true);
    native.blocked();
    assert_eq!(
        rpc(&native.socket(), json!({"op":"show","mode":"clipboard"}))["ok"],
        true
    );
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        let status = native.status();
        let s = &status["result"]["nativeUI"];
        if s["visible"] == true
            && s["revision"].as_u64().is_some_and(|v| v > 0)
            && s["revision"] == s["appliedRevision"]
        {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    let capture = rpc(
        &native.socket(),
        json!({"op":"native.test","action":"screenshot"}),
    );
    assert_eq!(capture["ok"], true, "{capture}");
    let view = &capture["result"];
    for field in ["windowShown", "windowKey", "windowMain"] {
        assert_eq!(view[field], false);
    }
    assert!(
        view["screenText"].as_array().unwrap().iter().any(|v| v
            .as_str()
            .is_some_and(|s| s.contains("Clipboard capture is paused:"))),
        "{view}"
    );
    fs::write(
        output.join("native-error.json"),
        serde_json::to_vec_pretty(view).unwrap(),
    )
    .unwrap();
    fs::copy(
        native.base.join("p/native-probe.png"),
        output.join("native-error.png"),
    )
    .unwrap();
    let deferred = rpc(
        &native.socket(),
        json!({"op":"native.test","action":"clipboard-fixture","payload":{"Text":"Owned paused fixture"},"defer":true}),
    );
    assert_eq!(deferred["ok"], true, "{deferred}");
    std::thread::sleep(Duration::from_millis(500));
    native.blocked();
    assert_eq!(native.status()["result"]["clipboardCount"], 0);
    assert_eq!(
        fs::read(native.base.join("p/paused.json")).unwrap(),
        b"broken"
    );
    assert_eq!(
        rpc(
            &native.socket(),
            json!({"op":"clipboard.pause","paused":false})
        )["result"]["paused"],
        false
    );
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        let capture = rpc(
            &native.socket(),
            json!({"op":"native.test","action":"screenshot"}),
        );
        let view = &capture["result"];
        if !view["screenText"].as_array().unwrap().iter().any(|v| {
            v.as_str()
                .is_some_and(|s| s.contains("Clipboard capture is paused:"))
        }) {
            fs::write(
                output.join("native-recovered.json"),
                serde_json::to_vec_pretty(view).unwrap(),
            )
            .unwrap();
            fs::copy(
                native.base.join("p/native-probe.png"),
                output.join("native-recovered.png"),
            )
            .unwrap();
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    native.stop();
    fs::write(output.join("result.json"),serde_json::to_vec_pretty(&json!({"ok":true,"cases":outcomes,"publishedUncertainRemainsPaused":true,"uncertainResponses":uncertain_responses,"missingDefaultsFalse":true,"generalPasteboardAbortGuard":true,"physicalEffects":false})).unwrap()).unwrap();
}
