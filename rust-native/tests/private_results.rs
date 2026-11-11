#![cfg(feature = "test-tools")]
//! Real Rust fake -> protocol3 helper -> serialized host -> socket/CLI response.
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Host {
    home: PathBuf,
    child: Child,
    owns_home: bool,
}
impl Host {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let parent = PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
            .expect("explicit owned short fixture parent required"));
        assert!(parent.is_absolute());
        let home = parent.join(format!("ruy-pr-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        assert!(!home.exists());
        fs::create_dir_all(home.join("p")).unwrap();
        fs::create_dir(home.join("Applications")).unwrap();
        fs::write(
            home.join(".ruycast-test-fixture"),
            b"own-disposable-fixture-v1",
        )
        .unwrap();
        let fake = env!("CARGO_BIN_EXE_private-results-fixture");
        let mut commands = vec![];
        for (id, mode, payload, provider, input, enabled, timeout) in [
            (
                "provider.ok",
                "write",
                r#"{"ok":true,"extra":7}"#,
                true,
                false,
                true,
                2.,
            ),
            (
                "provider.choice",
                "write",
                r#"{"ok":true,"detail":"owned report","choices":{"title":"Owned","placeholder":"Find","items":[{"id":"one","title":"One","subtitle":"","value":"literal"}]}}"#,
                true,
                false,
                true,
                2.,
            ),
            (
                "provider.invalid",
                "write",
                "not-json",
                true,
                false,
                true,
                2.,
            ),
            (
                "provider.failed",
                "exit7",
                r#"{"ok":true}"#,
                true,
                false,
                true,
                2.,
            ),
            (
                "provider.log-failure",
                "exit23",
                r#"{"ok":false,"code":"provider_failed","error":"Status writer rejected the note","diagnostics":{"provider":"task-status.sh","providerExitCode":"23","providerStderr":"token=sample-secret-value"}}"#,
                true,
                false,
                true,
                2.,
            ),
            (
                "provider.log-input",
                "exit23",
                r#"{"ok":false,"code":"provider_failed","error":"Rejected literal input Ω","diagnostics":{"providerStderr":"literal input Ω token=sample-secret-value"}}"#,
                true,
                true,
                true,
                2.,
            ),
            ("provider.input", "echo", "unused", true, true, true, 2.),
            (
                "provider.disabled",
                "write",
                r#"{"ok":true}"#,
                true,
                false,
                false,
                2.,
            ),
            (
                "capture.ok",
                "write",
                r#"{"ok":true,"owned":"decoded"}"#,
                false,
                false,
                true,
                2.,
            ),
            (
                "capture.app-failure",
                "write",
                r#"{"ok":false,"error":"owned application failure"}"#,
                false,
                false,
                true,
                2.,
            ),
            (
                "capture.daily",
                "write",
                r#"{"ok":true,"daily_error":"owned daily failure"}"#,
                false,
                false,
                true,
                2.,
            ),
            (
                "capture.invalid",
                "write",
                "not-json",
                false,
                false,
                true,
                2.,
            ),
            (
                "capture.quick-note",
                "write",
                r#"{"ok":true,"note_path":"/owned/never-open &?#.md"}"#,
                false,
                false,
                true,
                2.,
            ),
            (
                "capture.timeout",
                "wait",
                r#"{"ok":true}"#,
                false,
                false,
                true,
                0.15,
            ),
            (
                "capture.disconnect",
                "wait",
                r#"{"ok":true}"#,
                false,
                false,
                true,
                3.,
            ),
        ] {
            let mut arguments = vec![
                "fake".to_owned(),
                mode.to_owned(),
                home.join(format!("{id}.marker")).display().to_string(),
                payload.to_owned(),
            ];
            if provider {
                arguments.push("synthetic/personal_bridge.py".into());
            }
            if input {
                arguments.push("{input}".into());
            }
            commands.push(json!({"id":id,"title":id,"kind":"script","target":fake,"arguments":arguments,"aliases":if enabled{vec![]}else{vec!["café"]},"inputPrompt":if input{Some("Message")}else{None},"enabled":enabled,"timeout":timeout}));
        }
        let config = json!({"version":1,"launcherHotkey":"ctrl+alt+space","clipboardHotkey":"ctrl+alt+v","clipboard":{"enabled":false,"limit":200,"maxTextBytes":100000,"retentionDays":30,"excludedApps":[]},"commands":commands});
        let mut config = config;
        config["commands"].as_array_mut().unwrap().push(json!({"id":"owned.history","title":"Owned History","kind":"builtin","target":"utility-text-history"}));
        fs::write(home.join("p/config.json"), config.to_string()).unwrap();
        let child = Command::new(fake)
            .arg("host")
            .arg(&home)
            .env("HOME", &home)
            .env("RUYCAST_HOME", home.join("p"))
            .env(
                "RUYCAST_NATIVE_TEST_APP_ROOTS",
                json!([home.join("Applications")]).to_string(),
            )
            .env_remove("RUYCAST_CONFIG_DIR")
            .env_remove("RUSTCAST_CONFIG_DIR")
            .env_remove("DYLD_INSERT_LIBRARIES")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut host = Self { home, child, owns_home: true };
        let end = Instant::now() + Duration::from_secs(6);
        loop {
            if host.home.join("p/control.sock").exists()
                && host.rpc(json!({"op":"status"}))["ok"] == true
            {
                break;
            }
            assert!(
                host.child.try_wait().unwrap().is_none(),
                "owned host exited"
            );
            assert!(Instant::now() < end, "owned host readiness timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        host
    }
    fn rpc(&self, value: Value) -> Value {
        let mut stream = UnixStream::connect(self.home.join("p/control.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(6)))
            .unwrap();
        writeln!(stream, "{value}").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn runs(&self, id: &str) -> usize {
        fs::read_to_string(self.home.join(format!("{id}.marker")))
            .unwrap_or_default()
            .lines()
            .count()
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            if let Ok(mut stream) = UnixStream::connect(self.home.join("p/control.sock")) {
                let _ = writeln!(stream, "{}", json!({"op":"quit"}));
            }
            let end = Instant::now() + Duration::from_secs(4);
            while self.child.try_wait().ok().flatten().is_none() && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if self.owns_home { fs::remove_dir_all(&self.home).unwrap(); }
    }
}
#[test]
fn synchronous_management_failure_log_keeps_message_and_trace() { synchronous_failure_log("builtin.dashboard"); }
#[test]
fn synchronous_history_failure_log_keeps_message_and_trace() { synchronous_failure_log("owned.history"); }
fn synchronous_failure_log(id: &str) {
    let host = Host::new();
    let cli = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".build/rust/release/ruycast");
    let call = |args: &[&str]| {
        let output = Command::new(&cli).args(args).env_clear().env("HOME", &host.home).env("RUYCAST_HOME", host.home.join("p")).output().unwrap();
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let response = call(&["run", id]);
    assert_eq!(response["ok"], false);
    assert!(response["error"].as_str().unwrap().contains("unavailable"));
    let logs = call(&["logs", "--command", id, "--errors"]);
    assert_eq!(logs["result"][0]["message"], response["error"]);
    assert_eq!(logs["result"][0]["code"], "unavailable");
    assert!(response["diagnosticID"].is_string());
    assert_eq!(logs["result"][0]["traceID"], response["diagnosticID"]);
}
#[test]
fn provider_failure_keeps_sanitized_code_message_exit_and_queryable_trace() {
    let host = Host::new();
    let cli = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".build/rust/release/ruycast");
    let call = |args: &[&str]| {
        let output = Command::new(&cli).args(args).env_clear()
            .env("HOME", &host.home).env("RUYCAST_HOME", host.home.join("p"))
            .output().unwrap();
        (output.status, serde_json::from_slice::<Value>(&output.stdout).unwrap())
    };
    let (status, response) = call(&["run", "provider.log-failure"]);
    assert!(!status.success());
    assert_eq!(response["ok"], false);
    assert_eq!(response["exitCode"], 23);
    assert_eq!(response["error"], "Status writer rejected the note");
    let (status, logs) = call(&["logs", "--command", "provider.log-failure", "--errors"]);
    assert!(status.success(), "{logs}");
    let entry = &logs["result"][0];
    assert_eq!(entry["commandID"], "provider.log-failure");
    assert_eq!(entry["exitStatus"], 23);
    assert_eq!(entry["code"], "provider_failed");
    assert!(entry["message"].as_str().unwrap().contains("Status writer rejected"));
    assert_eq!(response["diagnosticID"], entry["traceID"]);
    assert!(!logs.to_string().contains("sample-secret-value"));
    assert!(!fs::read_to_string(host.home.join("p/command-log.json")).unwrap().contains("SYNTHETIC_PRIVATE_SECRET_9817"));
    assert_eq!(host.runs("provider.log-failure"), 1);
    let (status, response) = call(&["run", "provider.log-input", "--input", r#"{"input":"literal input Ω"}"#]);
    assert!(!status.success());
    assert_eq!(response["exitCode"], 23);
    let (status, logs) = call(&["logs", "--command", "provider.log-input", "--errors"]);
    assert!(status.success());
    assert_eq!(logs["result"][0]["message"], "Rejected [redacted]");
    assert_eq!(response["diagnosticID"], logs["result"][0]["traceID"]);
    assert!(!logs.to_string().contains("literal input Ω"));
    assert!(!logs.to_string().contains("sample-secret-value"));
    assert_eq!(host.runs("provider.log-input"), 1);
    let (status, response) = call(&["run", "provider.ok"]);
    assert!(status.success());
    assert_eq!(response["extra"], 7);
    let (status, logs) = call(&["logs", "--command", "provider.ok"]);
    assert!(status.success());
    assert_eq!(logs["result"][0]["code"], "success");
    assert!(logs["result"][0].get("message").is_none());
    let persisted = fs::read_to_string(host.home.join("p/command-log.json")).unwrap();
    assert!(!persisted.contains("literal input Ω"));
    assert!(!persisted.contains("sample-secret-value"));
}
#[test]
fn private_capture_real_host_cli_input_failure_privacy_and_no_retry() {
    let mut host = Host::new();
    for id in [
        "provider.ok",
        "provider.choice",
        "capture.ok",
        "capture.quick-note",
    ] {
        let r = host.rpc(json!({"op":"execute","id":id}));
        assert_eq!(r["ok"], true, "{id}: {r}");
        assert_eq!(host.runs(id), 1);
        if id == "provider.ok" {
            assert_eq!(r["extra"], 7);
        }
        if id == "capture.ok" {
            assert_eq!(r["result"]["owned"], "decoded");
        }
        if id == "provider.choice" {
            assert_eq!(r["detail"], "owned report");
            assert!(r["choices"].is_object());
        }
    }
    for id in [
        "provider.invalid",
        "provider.failed",
        "capture.app-failure",
        "capture.daily",
        "capture.invalid",
    ] {
        let r = host.rpc(json!({"op":"execute","id":id}));
        assert_eq!(r["ok"], false, "{id}: {r}");
        assert_eq!(host.runs(id), 1);
        if id == "provider.failed" {
            assert_eq!(r["exitCode"], 7);
        }
        if ["provider.invalid", "capture.daily", "capture.invalid"].contains(&id) {
            assert_eq!(r["actionCompleted"], true);
        }
    }
    for input in ["", "   ", "\n\t", "\u{2007}"] {
        let r = host.rpc(json!({"op":"execute","id":"provider.input","values":{"input":input}}));
        assert_eq!(r["ok"], false);
        assert_eq!(host.runs("provider.input"), 0);
    }
    for id in ["provider.disabled", "cafe\u{301}"] {
        let r = host.rpc(json!({"op":"execute","id":id}));
        assert_eq!(r["ok"], false);
        assert_eq!(host.runs("provider.disabled"), 0);
    }
    let r = host.rpc(json!({"op":"execute","id":"capture.ok","dryRun":true}));
    assert_eq!(r["ok"], true);
    assert_eq!(host.runs("capture.ok"), 1);
    let cli = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".build/rust/release/ruycast");
    // The initial private adapter and literal choice/reload follow-ups traverse
    // the real supervisor, then the same private Owner used by native runtime.
    let cycle:rustcast_core::Command=serde_json::from_value(json!({"id":"provider.cycle","title":"Owned cycle","kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","cycle",host.home.join("cycle.marker"),"unused","personal_bridge.py","{input}"],"timeout":2})).unwrap();
    let run_cycle = |prepared: &rustcast_core::Command| {
        let actual = ruycast_native::supervision::probe(
            &cli,
            prepared,
            &host.home,
            Some(Instant::now() + Duration::from_secs(2)),
            || false,
        )
        .unwrap();
        let capture: rustcast_core::script::Capture =
            serde_json::from_value(actual["capture"].clone()).unwrap();
        ruycast_native::script_result::interpret_command(
            &cycle,
            actual["response"].clone(),
            true,
            ruycast_native::script_result::Audience::Native,
            Some(&capture),
        )
    };
    let mut owner = ruycast_native::script_session::Owner::default();
    let file = host.home.join("p/config.json");
    let initial = ruycast_native::execution::prepare(&cycle, &json!({})).unwrap();
    owner
        .present(&cycle, &file, run_cycle(&initial), None)
        .unwrap();
    let token = owner.token().unwrap();
    assert_eq!(
        owner.surface().unwrap().detail.as_deref(),
        Some("Owned cycle report")
    );
    let stale = owner.prepare(token, Some("one"), &cycle, &file).unwrap();
    owner.query(token, "changed").unwrap();
    assert!(owner
        .admit(stale, &cycle, &file, |_, _, _| panic!(
            "stale selection executed"
        ))
        .is_err());
    owner.query(token, "").unwrap();
    let mut disabled = cycle.clone();
    disabled.enabled = Some(false);
    assert!(owner.prepare(token, Some("one"), &disabled, &file).is_err());
    let reload = owner.prepare(token, None, &cycle, &file).unwrap();
    let mut completed = None;
    let origin = owner
        .admit(reload, &cycle, &file, |prepared, _, _| {
            completed = Some(run_cycle(prepared));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        owner
            .complete(&origin, completed.take().unwrap(), None)
            .unwrap(),
        ruycast_native::script_session::Completion::Replaced
    );
    let refreshed = owner.token().unwrap();
    assert_ne!(token, refreshed);
    let chosen = owner
        .prepare(refreshed, Some("one"), &cycle, &file)
        .unwrap();
    let origin = owner
        .admit(chosen, &cycle, &file, |prepared, _, _| {
            let result = run_cycle(prepared);
            assert_eq!(result.response["received"], "literal Ω\n$(no-shell)");
            completed = Some(result);
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        owner
            .complete(&origin, completed.take().unwrap(), None)
            .unwrap(),
        ruycast_native::script_session::Completion::Closed { .. }
    ));
    assert!(owner.token().is_none());
    assert_eq!(
        fs::read_to_string(host.home.join("cycle.marker"))
            .unwrap()
            .lines()
            .count(),
        3
    );
    let secret = "SYNTHETIC_PRIVATE_SECRET_9817";
    let input = json!({"ok":true,"private":secret}).to_string();
    let run = Command::new(cli)
        .args([
            "run",
            "provider.input",
            "--input",
            &json!({"input":input}).to_string(),
        ])
        .env("HOME", &host.home)
        .env("RUYCAST_HOME", host.home.join("p"))
        .env_remove("RUYCAST_CONFIG_DIR")
        .env_remove("RUSTCAST_CONFIG_DIR")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stdout)
    );
    let reply: Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(reply["private"], secret);
    assert_eq!(host.runs("provider.input"), 1);
    let timeout = host.rpc(json!({"op":"execute","id":"capture.timeout"}));
    assert_eq!(timeout["ok"], false);
    assert_eq!(host.runs("capture.timeout"), 1);
    let mut disconnected = UnixStream::connect(host.home.join("p/control.sock")).unwrap();
    writeln!(
        disconnected,
        "{}",
        json!({"op":"execute","id":"capture.disconnect"})
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while host.runs("capture.disconnect") == 0 {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(disconnected);
    let end = Instant::now() + Duration::from_secs(3);
    while host.rpc(json!({"op":"status"}))["result"]["liveExecutionWorkers"]
        .as_u64()
        .expect("live worker status")
        > 0
    {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(host.runs("capture.timeout"), 1);
    assert_eq!(host.runs("capture.disconnect"), 1);
    for operation in ["logs", "analytics", "analytics.export"] {
        let reply = host.rpc(json!({"op":operation}));
        assert!(
            !reply.to_string().contains(secret),
            "secret leaked in {operation}"
        );
        assert!(!reply.to_string().contains("never-open"));
    }
    assert_eq!(host.rpc(json!({"op":"quit"}))["ok"], true);
    let end = Instant::now() + Duration::from_secs(3);
    while host.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut child_status = 0;
    assert_eq!(
        unsafe { libc::waitpid(-1, &mut child_status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
    for entry in fs::read_dir(host.home.join("p")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            let bytes = fs::read(entry.path()).unwrap();
            assert!(
                !String::from_utf8_lossy(&bytes).contains(secret),
                "secret leaked to {:?}",
                entry.file_name()
            );
        }
    }
    println!(
        "owned-fixture={} all-marker-counts-once; no private provider or GUI opener invoked",
        host.home.display()
    );
}

#[test]
fn script_result_atomic_replace_reports_the_complete_owned_reload_response() {
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("explicit owned parent"),
    );
    let home = parent.join(format!("ruy-sr-{}", std::process::id()));
    fs::create_dir(&home).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(home.clone());
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(home.join("p")).unwrap();
    fs::set_permissions(home.join("p"), fs::Permissions::from_mode(0o700)).unwrap();
    // Match json.dumps' ASCII escaping and separator spaces. Value's sorted
    // object-key order remains a documented difference from Python insertion order.
    let python_config_bytes = |config: &Value| {
        let text = serde_json::to_string(config).unwrap();
        let mut out = String::new();
        let mut quoted = false;
        let mut escaped = false;
        for ch in text.chars() {
            if ch.is_ascii() {
                out.push(ch);
            } else {
                let mut units = [0; 2];
                for unit in ch.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            if !quoted && matches!(ch, ',' | ':') {
                out.push(' ');
            }
            if escaped {
                escaped = false;
            } else if quoted && ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = !quoted;
            }
        }
        let bytes = out.into_bytes();
        assert_eq!(
            &serde_json::from_slice::<Value>(&bytes).unwrap(),
            config,
            "fixture JSON roundtrip"
        );
        bytes
    };
    let mut config: Value = serde_json::from_slice(
        &fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/default.json")).unwrap(),
    )
    .unwrap();
    let file = home.join("p/config.json");
    // Python Host.__init__ writes DEFAULT; Execution.host atomically replaces
    // that file with the script configuration before starting the same host.
    fs::write(&file, python_config_bytes(&config)).unwrap();
    fs::write(
        home.join(".ruycast-test-fixture"),
        b"own-disposable-fixture-v1",
    )
    .unwrap();
    fs::create_dir(home.join("Applications")).unwrap();
    let fixture = env!("CARGO_BIN_EXE_private-results-fixture");
    let marker = home.join("script.fixture.marker");
    let mut command = json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":fixture,"arguments":["fake","write",marker,"fixture result\n"],"timeout":3});
    config["commands"]
        .as_array_mut()
        .unwrap()
        .push(command.clone());
    let candidate = file.with_extension("candidate");
    fs::write(&candidate, python_config_bytes(&config)).unwrap();
    fs::rename(&candidate, &file).unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_ruycast-native"));
    let mut launch = Command::new(&binary);
    launch.arg("--headless-test");
    // Same finite sanitization as the unchanged Python Host, without replacing
    // its inherited ordinary environment by env_clear.
    for name in [
        "RUYCAST_NATIVE_TEST_CONFIG_ADOPTION_BARRIER",
        "RUYCAST_NATIVE_TEST_KEYBOARD",
        "RUYCAST_KEYBOARD_FIXTURE_HOME",
        "RUYCAST_NATIVE_TEST_SNIPPET",
        "RUYCAST_HOME",
        "RUSTCAST_HOME",
        "RUYCAST_CONFIG_DIR",
        "RUSTCAST_CONFIG_DIR",
        "DYLD_INSERT_LIBRARIES",
        "RUYCAST_NATIVE_TEST_APP_ROOTS",
        "RUYCAST_NATIVE_TEST_APP_BARRIER",
        "RUYCAST_WORKSPACE_FIXTURE_HOME",
        "RUYCAST_NATIVE_TEST_DISPATCH_MS",
        "RUYCAST_NATIVE_TEST_CLIPBOARD",
        "RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT",
        "RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD",
        "RUYCAST_NATIVE_TEST_REQUIRE_HIDE",
        "RUYCAST_NATIVE_TEST_TEMPLATE_BARRIER",
        "RUYCAST_NATIVE_TEST_OPEN_DEADLINE_MS",
    ] {
        launch.env_remove(name);
    }
    let child = launch
        .env("HOME", &home)
        .env("RUYCAST_HOME", home.join("p"))
        .env("RUYCAST_APP_BINARY", &binary)
        .env(
            "RUYCAST_NATIVE_TEST_APP_ROOTS",
            json!([home.join("Applications")]).to_string(),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut host = Host {
        home: home.clone(),
        child,
        owns_home: false,
    };
    let until = Instant::now() + Duration::from_secs(6);
    let first_ready = loop {
        let status = (|| -> std::io::Result<Value> {
            let mut stream = UnixStream::connect(home.join("p/control.sock"))?;
            stream.set_read_timeout(Some(Duration::from_millis(200)))?;
            stream.set_write_timeout(Some(Duration::from_millis(200)))?;
            writeln!(stream, "{}", json!({"op":"status"}))?;
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line)?;
            serde_json::from_str(&line).map_err(std::io::Error::other)
        })();
        if let Ok(status) = status {
            if status["ok"] == true {
                break status;
            }
        }
        assert!(
            host.child.try_wait().unwrap().is_none(),
            "owned real headless host exited"
        );
        assert!(Instant::now() < until, "owned headless readiness timeout");
        std::thread::sleep(Duration::from_millis(20));
    };
    let payload = r#"{"ok": true, "choices": {"title": "Project Koppelen", "placeholder": "Zoek project…", "items": [{"id": "one", "title": "Café Project", "subtitle": "EX / PLAN", "value": "PRIVATE literal $(value) Ω", "extra": "unchanged"}]}, "detail": "PRIVATE report Ω", "extra": {"data": "PRIVATE extra"}}"#;
    command["arguments"] = json!(["fake", "write", marker, format!("{payload}\n")]);
    let mut replacement: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    replacement["commands"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["id"] != "script.fixture");
    replacement["commands"]
        .as_array_mut()
        .unwrap()
        .push(command);
    let candidate = file.with_extension("candidate");
    fs::write(&candidate, python_config_bytes(&replacement)).unwrap();
    fs::rename(candidate, &file).unwrap();
    // One request, no retry, waiting barrier, wrapper or altered reload result.
    let response = host.rpc(json!({"op":"reload"}));
    println!("owned_first_ready_response={first_ready}");
    println!("owned_reload_response={response}");
    assert_eq!(
        response["ok"], true,
        "complete owned reload response: {response}"
    );
    let response = host.rpc(json!({"op":"execute","id":"runfixture"}));
    let mut expected: Value = serde_json::from_str(payload).unwrap();
    expected["exitCode"] = json!(0);
    assert_eq!(
        response, expected,
        "original ordinary script result contract"
    );
    assert_eq!(
        host.rpc(json!({"op":"usage"}))["result"]["script.fixture"],
        1
    );
    for name in ["command-log.json", "usage.json"] {
        assert!(!fs::read_to_string(home.join("p").join(name))
            .unwrap()
            .contains("PRIVATE"));
    }
}

#[test]
fn public_performance_cli_and_rpc_preserve_real_search_reset_schema_and_privacy() {
    use std::os::unix::fs::PermissionsExt;
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("explicit owned parent"),
    );
    assert!(parent.is_absolute());
    let home = parent.join(format!("ruy-perf-{}", std::process::id()));
    fs::create_dir(&home).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(home.clone());
    fs::create_dir(home.join("p")).unwrap();
    fs::set_permissions(home.join("p"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(home.join("Applications")).unwrap();
    fs::write(
        home.join(".ruycast-test-fixture"),
        b"own-disposable-fixture-v1",
    )
    .unwrap();
    let mut config: Value =
        serde_json::from_slice(include_bytes!("../../config/default.json")).unwrap();
    config["clipboard"]["enabled"] = json!(false);
    config["commands"] = json!([{"id":"owned.performance.command","title":"OWNED_PERFORMANCE_PRIVATE_QUERY","kind":"url","target":"https://example.invalid/OWNED_PERFORMANCE_PRIVATE_URL"}]);
    fs::write(
        home.join("p/config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_ruycast-native"));
    let cli = std::env::var_os("RUYCAST_TEST_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join(".build/rust/release/ruycast")
        });
    assert!(binary.is_absolute() && cli.is_absolute());
    let child = Command::new(&binary)
        .arg("--headless-test")
        .env_clear()
        .env("HOME", &home)
        .env("TMPDIR", &home)
        .env("RUYCAST_HOME", home.join("p"))
        .env(
            "RUYCAST_NATIVE_TEST_APP_ROOTS",
            json!([home.join("Applications")]).to_string(),
        )
        .env("RUYCAST_NATIVE_TEST_SYSTEM_SEARCH", "owned")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut host = Host { home, child, owns_home: false };
    let deadline = Instant::now() + Duration::from_secs(6);
    while !host.home.join("p/control.sock").exists() {
        assert!(
            host.child.try_wait().unwrap().is_none(),
            "owned performance host exited before readiness"
        );
        assert!(
            Instant::now() < deadline,
            "owned performance socket readiness timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(host.rpc(json!({"op":"status"}))["ok"], true);
    let run_cli = |args: &[&str]| {
        let result = Command::new(&cli)
            .args(args)
            .env_clear()
            .env("HOME", &host.home)
            .env("TMPDIR", &host.home)
            .env("RUYCAST_HOME", host.home.join("p"))
            .env("RUYCAST_APP_BINARY", &binary)
            .output()
            .unwrap();
        assert!(
            result.stderr.is_empty(),
            "owned performance CLI stderr must be empty"
        );
        let value: Value = serde_json::from_slice(&result.stdout).unwrap();
        (result.status.success(), value)
    };
    // Capture both original operations and both real CLI exits before asserting,
    // so the old candidate's failure cannot hide the second missing operation.
    let summary = host.rpc(json!({"op":"performance"}));
    let reset = host.rpc(json!({"op":"performance.reset"}));
    let (cli_summary_ok, cli_summary) = run_cli(&["performance", "summary"]);
    let (cli_reset_ok, cli_reset) = run_cli(&["performance", "reset", "--confirm"]);
    println!(
        "owned_public_performance_initial={}",
        json!({"rpcSummary":summary,"rpcReset":reset,"cliSummarySuccess":cli_summary_ok,"cliSummary":cli_summary,"cliResetSuccess":cli_reset_ok,"cliReset":cli_reset})
    );
    let schema = |reply: &Value| {
        assert_eq!(reply["ok"], true, "public performance response: {reply}");
        let result = &reply["result"];
        assert_eq!(result["schema"], 2);
        assert_eq!(result["sampleLimitPerStage"], 256);
        assert_eq!(result["scope"], "software stages; field reconciliation is not result drawing or physical presentation; host.drawCallback may be absent for layer-backed display; hotkey timing begins at handler delivery");
        let stages = result["stages"].as_object().unwrap();
        assert_eq!(stages.len(), 6);
        for name in [
            "search.model",
            "input.model",
            "input.reconcile",
            "open.ready",
            "host.drawCallback",
            "hotkey.handler",
        ] {
            let metric = &stages[name];
            assert!(metric["count"].is_u64() && metric["cancelled"].is_u64());
            assert!(metric["retainedSamples"].as_u64().unwrap() <= 256);
        }
    };
    schema(&summary);
    schema(&reset);
    schema(&cli_summary);
    schema(&cli_reset);
    assert!(cli_summary_ok && cli_reset_ok);
    for metric in cli_reset["result"]["stages"].as_object().unwrap().values() {
        assert_eq!(metric["count"], 0);
        assert_eq!(metric["cancelled"], 0);
        assert_eq!(metric["retainedSamples"], 0);
        assert!(
            metric["p50MS"].is_null() && metric["p95MS"].is_null() && metric["maxMS"].is_null()
        );
    }
    assert_eq!(cli_reset["result"]["activeIntervals"], 0);
    let (searched, rows) = run_cli(&["search", "OWNED_PERFORMANCE_PRIVATE_QUERY"]);
    assert!(searched && rows["ok"] == true);
    assert!(rows["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["id"] == "owned.performance.command"));
    let (success, measured) = run_cli(&["performance", "summary"]);
    assert!(success);
    schema(&measured);
    let search = &measured["result"]["stages"]["search.model"];
    assert!(
        search["count"].as_u64().unwrap() > 0,
        "real public search must reach the production search-model timer"
    );
    assert!(
        search["p50MS"].is_number() && search["p95MS"].is_number() && search["maxMS"].is_number()
    );
    let trace_bytes = serde_json::to_string(&measured).unwrap();
    for private in [
        "OWNED_PERFORMANCE_PRIVATE_QUERY",
        "OWNED_PERFORMANCE_PRIVATE_URL",
        "owned.performance.command",
        "example.invalid",
    ] {
        assert!(
            !trace_bytes.contains(private),
            "performance must contain only fixed stages and numeric measurements"
        );
    }
    let (reset_success, cleared) = run_cli(&["performance", "reset", "--confirm"]);
    assert!(reset_success);
    schema(&cleared);
    assert_eq!(cleared["result"]["stages"]["search.model"]["count"], 0);
    assert!(cleared["result"]["stages"]["search.model"]["p50MS"].is_null());
    assert_eq!(host.rpc(json!({"op":"quit"}))["ok"], true);
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(exit) = host.child.try_wait().unwrap() {
            assert!(exit.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "owned performance host did not quit normally"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
