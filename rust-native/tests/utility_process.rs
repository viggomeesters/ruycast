//! Actual host/CLI/native responder routes with fully owned Security/AppKit
//! substitutes. No personal profile, Keychain, pasteboard, app focus or input.
#![cfg(feature = "test-tools")]
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::{Duration, Instant},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn build_fixtures() -> &'static (PathBuf, PathBuf) {
    static BUILT: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    BUILT.get_or_init(|| {
        let out = root().join("target/utility-evidence");
        fs::create_dir_all(&out).unwrap();
        let security = out.join("utility-security.dylib");
        let result = Command::new("rustc")
            .args(["--edition=2021", "--crate-type=cdylib", "-Dwarnings"])
            .arg(root().join("tests/fixtures/utility_security_fixture.rs"))
            .arg("-o")
            .arg(&security)
            .output()
            .unwrap();
        fs::write(out.join("security-build.log"), &result.stderr).unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let effects = out.join("utility-effects.dylib");
        let result = Command::new("cc")
            .args([
                "-Wall",
                "-Wextra",
                "-Werror",
                "-fobjc-arc",
                "-fblocks",
                "-dynamiclib",
                "-framework",
                "AppKit",
                "-framework",
                "ApplicationServices",
            ])
            .arg(root().join("tests/clipboard_history_shim.m"))
            .arg("-o")
            .arg(&effects)
            .output()
            .unwrap();
        fs::write(out.join("effects-build.log"), &result.stderr).unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        (security, effects)
    })
}
fn rpc(socket: &Path, value: Value) -> Result<Value, String> {
    let mut stream = UnixStream::connect(socket).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    serde_json::to_writer(&mut stream, &value).unwrap();
    stream.write_all(b"\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|e| format!("{e}: {line}"))
}
struct Host {
    child: Child,
    base: PathBuf,
    profile: PathBuf,
    out: PathBuf,
    security: bool,
}
impl Host {
    fn new(
        name: &str,
        gui: bool,
        width: usize,
        dark: bool,
        security: bool,
        short_deadline: bool,
    ) -> Self {
        Self::configured(name, gui, width, dark, security, short_deadline, |_| {})
    }
    fn configured(
        name: &str,
        gui: bool,
        width: usize,
        dark: bool,
        security: bool,
        short_deadline: bool,
        configure: impl FnOnce(&mut Value),
    ) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
            .map(PathBuf::from)
            .expect("Set RUYCAST_NATIVE_TEST_FIXTURE_DIR to an owned absolute fixture directory");
        assert!(parent.is_absolute());
        let (sec, effects) = build_fixtures();
        fs::create_dir_all(&parent).unwrap();
        let suffix = format!(
            "{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let base = parent.join(&suffix);
        fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
        let profile = base.join("p");
        fs::create_dir(&profile).unwrap();
        let apps = base.join("Applications");
        fs::create_dir(&apps).unwrap();
        fs::write(
            base.join(".ruycast-test-fixture"),
            b"own-disposable-fixture-v1",
        )
        .unwrap();
        let mut config: Value = serde_json::from_slice(
            &fs::read(root().parent().unwrap().join("config/default.json")).unwrap(),
        )
        .unwrap();
        config["theme"] = json!(if dark { "dark" } else { "light" });
        for (name, target, title) in [
            ("set", "utility-set-text", "Set Text"),
            ("paste", "utility-paste-text", "Paste Text"),
            ("texts", "utility-text-history", "Text History"),
            ("password", "utility-password", "Generate Password"),
            ("username", "utility-username", "Generate Username"),
            ("passwords", "utility-password-history", "Password History"),
            ("usernames", "utility-username-history", "Username History"),
        ] {
            let mut command = json!({"id":format!("utility.fixture.{name}"),"title":title,"kind":"builtin","target":target,"aliases":[format!("u{name}")]});
            if name == "set" {
                command["inputPrompt"] = json!("Text");
            }
            config["commands"].as_array_mut().unwrap().push(command);
        }
        configure(&mut config);
        fs::write(
            profile.join("config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let binary = env!("CARGO_BIN_EXE_ruycast-native");
        let libraries = if security {
            format!("{}:{}", effects.display(), sec.display())
        } else {
            effects.display().to_string()
        };
        let out = root().join("target/utility-evidence").join(suffix);
        fs::create_dir(&out).unwrap();
        let mut source_hashes = serde_json::Map::new();
        for relative in [
            "src/utilities.rs",
            "src/utility_storage.rs",
            "src/utility_jobs.rs",
            "src/runtime_utilities.rs",
            "src/runtime.rs",
            "src/actions.rs",
            "src/connected.rs",
            "src/native.rs",
            "src/native_model.rs",
            "src/native_editor.rs",
            "src/native_surface.rs",
            "src/main_services.rs",
                    "src/control.rs",
            "src/management_commands.rs",
            "src/paste.rs",
            "src/snippet_clipboard.rs",
            "src/native_paste.rs",
            "tests/utility_process.rs",
            "tests/fixtures/utility_security_fixture.rs",
            "Cargo.toml",
            "Cargo.lock",
        ] {
            source_hashes.insert(
                relative.into(),
                json!(format!(
                    "{:x}",
                    Sha256::digest(fs::read(root().join(relative)).unwrap())
                )),
            );
        }
        fs::write(root().join("target/utility-evidence/last-run.json"),json!({"processId":std::process::id(),"nativeBinary":binary,"nativeBinarySHA256":format!("{:x}",Sha256::digest(fs::read(binary).unwrap())),"securitySHA256":format!("{:x}",Sha256::digest(fs::read(sec).unwrap())),"effectsSHA256":format!("{:x}",Sha256::digest(fs::read(effects).unwrap())),"sources":source_hashes}).to_string()).unwrap();
        let mut command = Command::new(binary);
        command
            .arg(if gui {
                "--native-hidden"
            } else {
                "--headless-test"
            })
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/opt/homebrew/bin")
            .env("HOME", &base)
            .env("RUYCAST_HOME", &profile)
            .env("RUYCAST_APP_BINARY", binary)
            .env(
                "RUYCAST_NATIVE_TEST_APP_ROOTS",
                serde_json::to_string(&[apps]).unwrap(),
            )
            .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &base)
            .env("RUYCAST_UTILITY_FIXTURE_HOME", &base)
            .env("RUYCAST_NATIVE_TEST_SNIPPET", "1")
            .env("RUYCAST_NATIVE_TEST_CLIPBOARD", "named")
            .env("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD", "1")
            .env("RUYCAST_NATIVE_TEST_WIDTH", width.to_string())
            .env("DYLD_INSERT_LIBRARIES", libraries)
            .stdout(Stdio::from(
                fs::File::create(out.join("stdout.log")).unwrap(),
            ))
            .stderr(Stdio::from(
                fs::File::create(out.join("stderr.log")).unwrap(),
            ));
        if short_deadline {
            command.env("RUYCAST_NATIVE_TEST_DISPATCH_MS", "350");
        }
        let child = command.spawn().unwrap();
        let mut host = Self {
            child,
            base,
            profile,
            out,
            security,
        };
        let end = Instant::now() + Duration::from_secs(8);
        loop {
            if host.try_rpc(json!({"op":"status"})).is_ok() {
                break;
            }
            assert!(
                host.child.try_wait().unwrap().is_none(),
                "{}",
                fs::read_to_string(host.out.join("stderr.log")).unwrap()
            );
            assert!(
                Instant::now() < end,
                "Host readiness expired: {}",
                host.out.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        if gui {
            host.settled();
        }
        host
    }
    fn socket(&self) -> PathBuf {
        self.profile.join("control.sock")
    }
    fn try_rpc(&self, value: Value) -> Result<Value, String> {
        rpc(&self.socket(), value)
    }
    fn rpc(&self, value: Value) -> Value {
        self.try_rpc(value).unwrap()
    }
    fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let end = Instant::now() + Duration::from_secs(6);
        loop {
            let status = self.rpc(json!({"op":"status"}));
            if predicate(&status["result"]) {
                return status["result"].clone();
            }
            assert!(Instant::now() < end, "Status did not settle: {status}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn settled(&self) -> Value {
        self.wait(|s| {
            s["configurationUpdating"] != true
                && s["nativeUI"]["revision"]
                    .as_u64()
                    .is_some_and(|v| v > 0 && Some(v) == s["nativeUI"]["appliedRevision"].as_u64())
                && s["nativeUI"]["utilityBusy"] != true
        })
    }
    fn probe(&self, action: &str, fields: Value) -> Value {
        let mut request = fields;
        request["op"] = json!("native.test");
        request["action"] = json!(action);
        let response = self.rpc(request);
        assert_eq!(response["ok"], true, "{response}");
        response["result"].clone()
    }
    fn show(&self, mode: &str, query: &str) {
        let response = self.rpc(json!({"op":"show","mode":mode,"query":query}));
        assert_eq!(response["ok"], true, "{response}");
        self.settled();
    }
    fn execute(&self, id: &str, input: Option<&str>) -> Value {
        self.rpc(json!({"op":"execute","id":id,"values":input.map(|v|json!({"input":v})).unwrap_or_else(||json!({}))}))
    }
    fn search(&self, mode: &str, query: &str) -> Value {
        let response = self.rpc(json!({"op":"search","mode":mode,"query":query}));
        assert_eq!(response["ok"], true, "{response}");
        response["result"].clone()
    }
    fn key(&self, code: u16, chars: &str, flags: u64) {
        self.probe(
            "key-event",
            json!({"code":code,"chars":chars,"flags":flags}),
        );
    }
    fn capture(&self, name: &str) -> Value {
        let value = self.probe("screenshot", json!({}));
        for key in ["windowShown", "windowKey", "windowMain"] {
            assert_eq!(value[key], false, "{value}");
        }
        fs::copy(
            self.profile.join("native-probe.png"),
            self.out.join(format!("{name}.png")),
        )
        .unwrap();
        fs::write(
            self.out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
        value
    }
    fn cli(&self, args: &[&str]) -> Value {
        let cli = root().parent().unwrap().join("rust/target/debug/ruycast");
        let output = Command::new(cli)
            .args(args)
            .env_clear()
            .env("HOME", &self.base)
            .env("RUYCAST_HOME", &self.profile)
            .env("RUYCAST_APP_BINARY", env!("CARGO_BIN_EXE_ruycast-native"))
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            output.status.success(),
            "{value} {}",
            String::from_utf8_lossy(&output.stderr)
        );
        value
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.try_rpc(json!({"op":"quit"}));
        let end = Instant::now() + Duration::from_secs(4);
        loop {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            if Instant::now() >= end {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for name in [
            "utility-security-events",
            "utility-keychain.bin",
            "workspace-trace.jsonl",
            "snippet-status.json",
        ] {
            if self.base.join(name).exists() {
                let _ = fs::copy(self.base.join(name), self.out.join(name));
            }
        }
        for name in [
            "utilities.json",
            "command-log.json",
            "analytics.json",
            "usage.json",
        ] {
            if self.profile.join(name).exists() {
                let _ = fs::copy(self.profile.join(name), self.out.join(name));
            }
        }
        let _=fs::write(self.out.join("ownership.json"),json!({"securityTable":self.security,"personalData":false,"generalBoard":false,"physicalInput":false}).to_string());
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn cli_text_histories_import_and_real_storage_reopen() {
    let host = Host::new("cli", false, 760, false, true, false);
    assert_ne!(host.execute("utility.fixture.paste", None)["ok"], true);
    assert_eq!(
        host.cli(&["run", "uset", "--input", "first\nline"])["ok"],
        true
    );
    assert_eq!(
        host.execute("utility.fixture.set", Some("second"))["ok"],
        true
    );
    assert_eq!(
        host.execute("utility.fixture.set", Some("first\nline"))["ok"],
        true
    );
    let rows = host.cli(&["search", "", "--mode", "text-history"])["result"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(rows[0]["title"], "first line");
    assert_eq!(
        host.execute(rows[1]["id"].as_str().unwrap(), None)["ok"],
        true
    );
    let disk: Value =
        serde_json::from_slice(&fs::read(host.profile.join("utilities.json")).unwrap()).unwrap();
    assert_eq!(disk["savedText"], "second");
    assert_eq!(
        fs::metadata(host.profile.join("utilities.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let file = host.base.join("import.json");
    fs::write(&file,json!({"savedText":"must not replace","textHistory":[{"value":"older import","created":"2026-10-25T00:00:00Z"}],"usernameHistory":[]}).to_string()).unwrap();
    assert_eq!(
        host.cli(&["utilities", "import", &format!("@{}", file.display())])["result"]["added"],
        1
    );
    assert_eq!(
        host.cli(&["utilities", "import", &format!("@{}", file.display())])["result"]["added"],
        0
    );
    assert_eq!(
        host.search("utility-text-history", "older")
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let before = fs::read(host.profile.join("utilities.json")).unwrap();
    let reopened = Host::new("retained", false, 760, false, true, false);
    fs::write(reopened.profile.join("utilities.json"), &before).unwrap();
    assert_eq!(
        reopened
            .search("utility-text-history", "")
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_ne!(
        host.rpc(json!({"op":"utilities.import","profile":{"passwordHistory":[]}}))["ok"],
        true
    );
    assert_eq!(
        fs::read(host.profile.join("utilities.json")).unwrap(),
        before
    );
    fs::write(
        host.profile.join("utilities.json"),
        b"malformed owned profile",
    )
    .unwrap();
    assert_ne!(
        host.execute("utility.fixture.set", Some("must preserve"))["ok"],
        true
    );
    assert_eq!(
        fs::read(host.profile.join("utilities.json")).unwrap(),
        b"malformed owned profile"
    );
    let oversized = host.base.join("oversized-import.json");
    fs::write(&oversized, vec![b' '; 1_048_577]).unwrap();
    let cli = root().parent().unwrap().join("rust/target/debug/ruycast");
    let output = Command::new(cli)
        .args(["utilities", "import", &format!("@{}", oversized.display())])
        .env("RUYCAST_HOME", &host.profile)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 MiB"));
}
#[test]
fn native_copy_is_concealed_and_metadata_logs_do_not_expose_password() {
    let host = Host::new("secret", true, 760, true, true, false);
    let old = "old-owned-password";
    fs::write(
        host.base.join("utility-keychain.bin"),
        json!([{"value":old,"created":"2000-01-01T00:00:00Z"}]).to_string(),
    )
    .unwrap();
    assert_eq!(host.cli(&["run", "upassword"])["ok"], true);
    let board = host.probe("utility-board", json!({}));
    assert_eq!(board["privateNamedBoard"], true);
    assert_eq!(board["concealed"], true);
    assert_eq!(board["textLength"], 32);
    assert_eq!(
        board["textSHA256"],
        format!("{:x}", Sha256::digest(b"AAECAwQFBgcICQoLDA0ODxAREhMUFRYX"))
    );
    let rows = host.search("password-history", "");
    assert_eq!(rows.as_array().unwrap().len(), 2);
    let metadata = serde_json::to_string(&rows).unwrap();
    assert!(!metadata.contains(old));
    assert!(!metadata.contains("AAECAwQFBgcICQoLDA0ODxAREhMUFRYX"));
    host.show("utility-password-history", "");
    let screen = host.capture("password-history");
    assert!(screen["accessibilityText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str() == Some("Search password history…")));
    assert!(!screen.to_string().contains(old));
    assert_eq!(
        host.execute(rows[1]["id"].as_str().unwrap(), None)["ok"],
        true
    );
    assert_eq!(
        host.probe("utility-board", json!({}))["textSHA256"],
        format!("{:x}", Sha256::digest(old.as_bytes()))
    );
    assert_eq!(host.execute("utility.fixture.username", None)["ok"], true);
    assert_eq!(host.probe("utility-board", json!({}))["concealed"], false);
    assert_eq!(
        host.search("username-history", "amber")[0]["title"],
        "amber-atlas-0203"
    );
    for op in ["logs", "analytics.export", "usage"] {
        let value = host.rpc(json!({"op":op})).to_string();
        assert!(!value.contains(old));
        assert!(!value.contains("AAECAwQFBgcICQoLDA0ODxAREhMUFRYX"));
    }
    let state: Value =
        serde_json::from_slice(&fs::read(host.base.join("snippet-status.json")).unwrap()).unwrap();
    assert!(state["ownChangeCount"].is_number());
    let events = fs::read_to_string(host.base.join("utility-security-events")).unwrap();
    assert!(!events.contains("invalid-query"));
}
#[test]
fn native_multiline_capture_return_shift_return_and_paste_use_real_responders() {
    let host = Host::new("capture", true, 600, false, true, false);
    host.show("commands", "uset");
    host.key(36, "\r", 0);
    host.wait(|s| s["nativeUI"]["editorOpen"] == true);
    host.settled();
    assert_eq!(host.probe("read", json!({}))["runInputInline"], true);
    host.key(36, "\r", 1 << 20);
    host.settled();
    let view = host.probe("read", json!({}));
    assert_eq!(view["formVisible"], true, "{view}");
    assert_eq!(view["runInputInline"], false);
    host.probe("run-input-type", json!({"text":"first"}));
    host.key(36, "\r", 1 << 17);
    host.probe("run-input-type", json!({"text":"second"}));
    let text = host.probe("run-input-read", json!({}));
    assert_eq!(text["text"], "first\nsecond");
    host.capture("set-text-capture");
    host.key(36, "\r", 0);
    host.wait(|s| s["nativeUI"]["editorOpen"] != true && s["snippetRequestsRunning"] == 0);
    let disk: Value =
        serde_json::from_slice(&fs::read(host.profile.join("utilities.json")).unwrap()).unwrap();
    assert_eq!(disk["savedText"], "first\nsecond");
    host.show("commands", "upaste");
    assert_eq!(host.cli(&["run", "upaste"])["ok"], true);
    let trace = fs::read_to_string(host.base.join("workspace-trace.jsonl")).unwrap();
    assert!(trace.contains("snippet-post"));
    assert!(trace.contains("first\\nsecond") || trace.contains("first\nsecond"));
    assert!(!trace.contains("unexpected-general"));
}

#[test]
fn history_surfaces_keep_latest_query_and_render_empty_busy_error_light_dark() {
    for (width, dark) in [(600, false), (760, true)] {
        let host = Host::new(
            if dark {
                "surface-dark"
            } else {
                "surface-light"
            },
            true,
            width,
            dark,
            true,
            false,
        );
        host.show("utility-text-history", "");
        host.capture("empty-text-history");
        assert_eq!(
            host.execute(
                "utility.fixture.set",
                Some("Owned multiline history Å\nSecond line")
            )["ok"],
            true
        );
        host.show("utility-text-history", "");
        host.capture("text-history");
        assert_eq!(host.execute("utility.fixture.username", None)["ok"], true);
        host.show("utility-username-history", "");
        host.capture("username-history");
        fs::write(host.base.join("utility-hold-read"), b"own hold").unwrap();
        assert_eq!(
            host.rpc(json!({"op":"show","mode":"utility-password-history"}))["ok"],
            true
        );
        host.wait(|s| {
            s["nativeUI"]["utilityBusy"] == true
                && s["nativeUI"]["revision"] == s["nativeUI"]["appliedRevision"]
        });
        let busy = host.capture("busy-password-history");
        assert!(busy["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str() == Some("Loading…")));
        assert!(!busy["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str() == Some("No history found")));
        host.probe("query", json!({"query":"latest no-match"}));
        fs::write(host.base.join("utility-release"), b"release").unwrap();
        host.settled();
        let view = host.probe("read", json!({}));
        assert_eq!(view["input"], "latest no-match");
        assert_eq!(view["rows"], 0);
        host.capture("latest-empty-password-history");
        fs::write(
            host.profile.join("utilities.json"),
            b"owned invalid profile",
        )
        .unwrap();
        host.show("utility-text-history", "");
        let error = host.capture("invalid-text-history");
        assert!(!error["screenText"].as_array().unwrap().iter().any(|s| s.as_str()==Some("No history found")));
        assert!(error["screenText"].as_array().unwrap().iter().any(|s| s
            .as_str()
            .is_some_and(|s| s.contains("restore a valid copy"))));
    }
}
#[test]
fn invalid_keychain_and_missing_complete_fixture_are_preserved() {
    let host = Host::new("denied", false, 760, false, true, false);
    let original = b"invalid owned Keychain entry";
    fs::write(host.base.join("utility-keychain.bin"), original).unwrap();
    assert_ne!(host.execute("utility.fixture.password", None)["ok"], true);
    assert_eq!(
        fs::read(host.base.join("utility-keychain.bin")).unwrap(),
        original
    );
    fs::write(host.base.join("utility-denied"), b"owned denial").unwrap();
    assert_ne!(
        host.rpc(json!({"op":"search","mode":"password-history","query":""}))["ok"],
        true
    );
    let unavailable = Host::new("no-security", false, 760, false, false, false);
    let result = unavailable.execute("utility.fixture.password", None);
    assert_ne!(result["ok"], true);
    assert!(result["error"].as_str().unwrap().contains("secure random"));
    assert!(!unavailable.base.join("utility-keychain.bin").exists());
}
#[test]
fn keychain_delay_does_not_block_status_and_cancelled_copy_never_posts() {
    for same_show in [false, true] {
        let host = Host::new("cancel", true, 760, false, true, !same_show);
        fs::write(host.base.join("utility-hold-read"), b"hold").unwrap();
        host.show("commands", "upassword");
        let socket = host.socket();
        let run = std::thread::spawn(move || {
            rpc(
                &socket,
                json!({"op":"execute","id":"utility.fixture.password","values":{}}),
            )
            .unwrap()
        });
        let end = Instant::now() + Duration::from_secs(3);
        while !host.base.join("utility-entered-read").exists() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        let started = Instant::now();
        assert_eq!(host.rpc(json!({"op":"status"}))["ok"], true);
        assert!(started.elapsed() < Duration::from_millis(250));
        if same_show {
            host.show("commands", "upassword");
        }
        let result = run.join().unwrap();
        assert_ne!(result["ok"], true, "{result}");
        fs::write(host.base.join("utility-release"), b"release").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!host.base.join("utility-keychain.bin").exists());
        let trace = fs::read_to_string(host.base.join("workspace-trace.jsonl")).unwrap_or_default();
        assert!(!trace.contains("snippet-write"));
        assert!(!trace.contains("snippet-post"));
    }
}

#[test]
fn held_keychain_write_reports_unknown_and_cannot_start_late_add_or_copy() {
    for existing in [false, true] {
        let host = Host::new("held-write", true, 760, false, true, true);
        if existing { fs::write(host.base.join("utility-keychain.bin"), b"[]").unwrap(); }
        fs::write(host.base.join("utility-hold-write"), b"hold").unwrap();
        host.show("commands", "upassword");
        let socket = host.socket();
        let run = std::thread::spawn(move || rpc(&socket, json!({"op":"execute","id":"utility.fixture.password","values":{}})).unwrap());
        let end = Instant::now() + Duration::from_secs(3);
        while !host.base.join("utility-entered-write").exists() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        let response = run.join().unwrap();
        assert_ne!(response["ok"], true, "{response}");
        assert_eq!(response["utilitiesStorageOutcome"], "unknown", "{response}");
        assert_eq!(response["utilitiesReconciliationRequired"], true);
        assert_eq!(response["actionCompleted"], true);
        fs::write(host.base.join("utility-release"), b"release").unwrap();
        std::thread::sleep(Duration::from_millis(150));
        if existing {
            let rows: Value = serde_json::from_slice(&fs::read(host.base.join("utility-keychain.bin")).unwrap()).unwrap();
            assert_eq!(rows.as_array().unwrap().len(), 1);
        } else { assert!(!host.base.join("utility-keychain.bin").exists()); }
        let trace = fs::read_to_string(host.base.join("workspace-trace.jsonl")).unwrap_or_default();
        assert!(!trace.contains("snippet-write"));
        assert!(!trace.contains("snippet-post"));
    }
}

fn wait_owned_file(path: &Path) {
    let end = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(Instant::now() < end, "Missing owned witness {}", path.display());
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn required_utilities(config: &mut Value) {
    for command in config["commands"].as_array_mut().unwrap() {
        if command["id"].as_str().is_some_and(|s| s.starts_with("utility.fixture.")) {
            command["inputPrompt"] = json!("Owned required");
        }
    }
    config["commands"].as_array_mut().unwrap().push(json!({"id":"utility.fixture.disabled","title":"Disabled Utility","kind":"builtin","target":"utility-set-text","inputPrompt":"Owned required","aliases":["udisabled"],"enabled":false}));
}
#[test]
fn repair_same_show_and_distinct_query_preserve_new_loading_while_old_read_is_held() {
    for same in [true, false] {
        let host = Host::new(if same {"repair-same"} else {"repair-distinct"}, true, 760, true, true, false);
        fs::write(host.base.join("utility-hold-read"), b"hold").unwrap();
        assert_eq!(host.rpc(json!({"op":"show","mode":"utility-password-history","query":""}))["ok"],true);
        wait_owned_file(&host.base.join("utility-entered-read"));
        host.wait(|s|s["nativeUI"]["utilityBusy"]==true);
        let query = if same {""} else {"new distinct query"};
        assert_eq!(host.rpc(json!({"op":"show","mode":"utility-password-history","query":query}))["ok"],true);
        // Observe multiple real owner polls while the first uninterruptible SDK read remains held.
        let until=Instant::now()+Duration::from_millis(220);
        let mut observations=vec![];
        while Instant::now()<until {
            let status=host.rpc(json!({"op":"status"}));
            assert_eq!(status["result"]["nativeUI"]["utilityBusy"],true,"{status}");
            observations.push(status);
            std::thread::sleep(Duration::from_millis(15));
        }
        let screen=host.capture("replacement-still-loading");
        assert!(screen["screenText"].as_array().unwrap().iter().any(|s|s=="Loading…"));
        assert!(!screen.to_string().contains("expired or was cancelled"));
        assert_eq!(host.probe("read",json!({}))["input"],query);
        fs::write(host.out.join("held-observations.json"),serde_json::to_vec_pretty(&observations).unwrap()).unwrap();
        fs::write(host.base.join("utility-release"),b"release").unwrap();
        host.settled();
        host.capture("replacement-completed");
    }
}
#[test]
fn repair_required_input_validation_precedes_all_utility_admission_and_preserves_dryrun_disabled() {
    let host=Host::configured("repair-validation",true,760,false,true,false,required_utilities);
    let original=json!({"savedText":"Owned original","textHistory":[],"usernameHistory":[]}).to_string();
    fs::write(host.profile.join("utilities.json"),&original).unwrap();
    let before_log=host.rpc(json!({"op":"logs"}));
    let invalid=[
        (json!({}),"Enter Owned required."),
        (json!({"input":""}),"Enter Owned required."),
        (json!({"input":" \t\r\n"}),"Enter Owned required."),
        (json!({"input":"\u{2003}"}),"Enter Owned required."),
        (json!({"input":"\u{0085}\u{00a0}\u{2009}\u{2028}\u{2029}\u{3000}"}),"Enter Owned required."),
        (Value::Null,"Input values must be strings."),
        (json!({"input":null}),"Input values must be strings."),
        (json!({"input":42}),"Input values must be strings."),
        (json!([]),"Input values must be strings."),
        (json!({"input":"valid","extra":{}}),"Input values must be strings."),
    ];
    let mut evidence=vec![];
    for name in ["set","paste","password","username","texts","passwords","usernames"] {
        for (values,error) in &invalid {
            let response=host.rpc(json!({"op":"execute","id":format!("utility.fixture.{name}"),"values":values}));
            assert_eq!(response["ok"],false,"{response}");
            assert_eq!(response["error"],*error,"{response}");
            evidence.push(json!({"command":name,"values":values,"response":response}));
        }
    }
    let absent=host.rpc(json!({"op":"execute","id":"utility.fixture.set"}));
    assert_eq!(absent["error"],"Enter Owned required.");
    let disabled=host.rpc(json!({"op":"execute","id":"utility.fixture.disabled","values":{"input":"valid"}}));
    assert_eq!(disabled["error"],"Command is disabled.");
    let dry=host.rpc(json!({"op":"execute","id":"utility.fixture.set","values":null,"dryRun":true}));
    assert_eq!(dry["ok"],true);assert_eq!(dry["dryRun"],true);
    assert_eq!(host.rpc(json!({"op":"execute","id":"utility.fixture.disabled","dryRun":true}))["error"],"Command is disabled.");
    assert_eq!(fs::read_to_string(host.profile.join("utilities.json")).unwrap(),original);
    assert!(!host.base.join("utility-keychain.bin").exists());
    assert!(!host.base.join("utility-security-events").exists());
    assert_eq!(host.rpc(json!({"op":"logs"})),before_log);
    let valid=host.rpc(json!({"op":"execute","id":"utility.fixture.set","values":{"input":"  Owned valid Ω  ","extra":"accepted string"}}));
    assert_eq!(valid["ok"],true,"{valid}");
    let profile:Value=serde_json::from_slice(&fs::read(host.profile.join("utilities.json")).unwrap()).unwrap();
    assert_eq!(profile["savedText"],"  Owned valid Ω  ");
    let zero_width=host.execute("utility.fixture.set",Some("\u{200b}"));
    assert_eq!(zero_width["error"],"Enter Owned required.","{zero_width}");
    for name in ["texts","passwords","usernames"] {
        let response=host.execute(&format!("utility.fixture.{name}"),Some("valid"));
        assert_eq!(response["ok"],true,"{response}");host.settled();
    }
    fs::write(host.out.join("input-validation.json"),serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
}
#[test]
fn repair_native_prompted_history_uses_inline_and_expanded_return_and_cancellation() {
    let host=Host::configured("repair-native-prompt",true,600,false,true,false,required_utilities);
    for expanded in [false,true] {
        host.show("commands","utexts");host.key(36,"\r",0);
        host.wait(|s|s["nativeUI"]["editorOpen"]==true);
        assert_eq!(host.probe("read",json!({}))["runInputInline"],true);
        if expanded {host.key(36,"\r",1<<20);host.settled();}
        let type_action=if expanded {"run-input-type"} else {"run-inline-type"};
        host.probe(type_action,json!({"text":"\u{2003}"}));host.key(36,"\r",if expanded {1<<20} else {0});
        host.wait(|s|s["nativeUI"]["editorOpen"]==true);
        let until=Instant::now()+Duration::from_millis(80);while Instant::now()<until {std::thread::sleep(Duration::from_millis(5));}
        let rejected=host.capture(if expanded {"expanded-required"} else {"inline-required"});
        assert!(rejected.to_string().contains("Enter Owned required."),"{rejected}");
        assert!(host.rpc(json!({"op":"status"}))["result"]["nativeUI"]["utilityMode"].is_null());
        host.probe(type_action,json!({"text":"valid"}));host.key(36,"\r",if expanded {1<<20} else {0});
        host.wait(|s|s["nativeUI"]["utilityMode"]=="utility-text-history"&&s["nativeUI"]["editorOpen"]==false);
        host.settled();host.capture(if expanded {"expanded-opened-history"} else {"inline-opened-history"});
    }
    for expanded in [false,true] {
        host.show("commands","utexts");host.key(36,"\r",0);host.wait(|s|s["nativeUI"]["editorOpen"]==true);
        if expanded {host.key(36,"\r",1<<20);host.settled();}
        host.probe(if expanded {"run-input-type"} else {"run-inline-type"},json!({"text":"valid stale submission"}));
        host.probe("schedule-input-submit",json!({}));
        let socket=host.socket();
        let hold=std::thread::spawn(move||rpc(&socket,json!({"op":"native.delay","executionBarrier":true,"holdOwner":true})).unwrap());
        wait_owned_file(&host.profile.join("owner-delay-started"));
        wait_owned_file(&host.profile.join("input-submit-fired"));
        fs::write(host.profile.join("owner-delay-release"),b"release after actual cancellation").unwrap();
        assert_eq!(hold.join().unwrap()["ok"],true);
        host.wait(|s|s["nativeUI"]["editorOpen"]==false);
        std::thread::sleep(Duration::from_millis(60));
        assert!(host.rpc(json!({"op":"status"}))["result"]["nativeUI"]["utilityMode"].is_null());
        fs::remove_file(host.profile.join("input-submit-fired")).unwrap();
        fs::remove_file(host.profile.join("owner-delay-started")).unwrap();
        fs::remove_file(host.profile.join("owner-delay-release")).unwrap();
        fs::remove_file(host.profile.join("owner-delay-finished")).unwrap();
    }
    host.show("commands","udisabled valid");
    assert_ne!(host.rpc(json!({"op":"status"}))["result"]["nativeUI"]["editorOpen"],true);
    assert!(!host.profile.join("utilities.json").exists());
    assert!(!host.base.join("utility-keychain.bin").exists());
}

#[test]
fn quick_capture_ids_use_native_return_shift_return_ime_and_single_submission_in_both_entries() {
    let parent = PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").unwrap());
    let markers = parent.join(format!("q{:x}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&markers).unwrap();
    for (index, id) in ["capture.quick-note", "capture.quick-task"].into_iter().enumerate() {
        for alias_entry in [false, true] {
            let marker = markers.join(format!("{index}-{alias_entry}"));
            let host = Host::configured("qc", true, 760, false, false, false, |config| {
                // This is an ordinary script target override, as supported by the
                // original ID-based editor classification. It touches only marker.
                config["commands"] = json!([{"id":id,"title":"Owned quick capture","kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","wait",marker,"{\"ok\":true}","{input}"],"inputPrompt":"Owned input","aliases":["qcapture"],"timeout":8}]);
            });
            host.show("commands", if alias_entry {"qcapture First line"} else {"qcapture"});
            if !alias_entry {
                host.key(36, "\r", 0);
            }
            host.wait(|status| status["nativeUI"]["editorOpen"] == true);
            host.settled();
            assert_eq!(host.probe("read", json!({}))["runInputInline"], true);
            host.key(36, "\r", 1 << 20);
            host.settled();
            let view = host.probe("read", json!({}));
            assert_eq!(view["formVisible"], true, "{view}");
            assert_eq!(view["runInputInline"], false);
            if !alias_entry {
                host.probe("run-input-type", json!({"text":"First line"}));
            }
            host.key(36, "\r", 1 << 17);
            host.probe("run-input-type", json!({"text":"Second line Ω"}));
            let input = host.probe("run-input-read", json!({}));
            assert_eq!(input["text"], "First line\nSecond line Ω");
            assert_eq!(input["focused"], true);
            host.probe("editor-mark", json!({}));
            host.key(36, "\r", 0);
            assert!(!marker.exists(), "IME confirmation must not execute {id}");
            assert_eq!(host.probe("run-input-read", json!({}))["pending"], false);
            host.probe("editor-unmark", json!({}));
            host.key(36, "\r", if alias_entry { 1 << 20 } else { 0 });
            wait_owned_file(&marker);
            // The already existing fake holds its one actual execution for four
            // seconds. A second genuine Return must not admit a second job.
            host.key(36, "\r", 0);
            host.wait(|status| status["nativeUI"]["editorOpen"] != true);
            assert_eq!(fs::read_to_string(&marker).unwrap(), "run\n");
            let capture = host.capture(&format!("capture-{index}-{alias_entry}"));
            assert_eq!(capture["windowShown"], false);
            let log = host.rpc(json!({"op":"logs"}));
            assert!(!log.to_string().contains("First line"));
            assert!(!log.to_string().contains("Second line Ω"));
        }
    }
    let ordinary_marker = markers.join("ordinary");
    let host = Host::configured("qordinary", true, 760, false, false, false, |config| {
        config["commands"] = json!([{"id":"ordinary.input","title":"Owned ordinary input","kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","write",ordinary_marker,"{\"ok\":true}","{input}"],"inputPrompt":"Owned input","aliases":["qordinary"]}]);
    });
    host.show("commands", "qordinary");
    host.key(36, "\r", 0);
    host.wait(|status| status["nativeUI"]["editorOpen"] == true);
    host.key(36, "\r", 1 << 20);
    host.settled();
    host.probe("run-input-type", json!({"text":"Ordinary"}));
    host.key(36, "\r", 0);
    host.probe("run-input-type", json!({"text":"multiline"}));
    assert_eq!(host.probe("run-input-read", json!({}))["text"], "Ordinary\nmultiline");
    assert!(!ordinary_marker.exists());
    host.probe("cancel-editor", json!({}));
    assert!(!ordinary_marker.exists());
    drop(host);
    fs::remove_dir_all(markers).unwrap();
}
