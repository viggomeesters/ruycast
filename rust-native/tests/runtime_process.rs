//! Fifteen original headless runtime cases on exclusive profiles and Unix sockets.
#![cfg(feature = "test-tools")]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host, OwnedDir};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::Shutdown,
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::PathBuf,
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
fn host(start: bool) -> Host {
    Host::new(Config::current(), start)
}
fn wait_marker(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "Actual directory sync not reached"
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn writer_host(delayed: bool, mutation: bool) -> (Host, PathBuf) {
    let mut host = host(false);
    let marker = host.base.join("writer-entered");
    let sdk = host
        .settings
        .host
        .parent()
        .unwrap()
        .join("examples/libfcntl_sync_shim.dylib");
    assert!(
        sdk.is_file(),
        "Build the same-profile fcntl-sync-shim example first"
    );
    for (k, v) in [
        ("DYLD_INSERT_LIBRARIES", sdk.into_os_string()),
        (
            "RUYCAST_TEST_FSYNC_DIRECTORY",
            host.profile.as_os_str().to_owned(),
        ),
        ("RUYCAST_TEST_FSYNC_MARKER", marker.as_os_str().to_owned()),
    ] {
        host.env.insert(k.into(), v);
    }
    if delayed {
        host.env
            .insert("RUYCAST_TEST_FSYNC_DELAY_MS".into(), "750".into());
        host.env
            .insert("RUYCAST_TEST_FSYNC_DELAY_ONLY".into(), "1".into());
    }
    if mutation {
        host.env
            .insert("RUYCAST_TEST_FSYNC_MUTATE".into(), "selector".into());
        host.env.insert(
            "RUYCAST_TEST_SELECTOR_JSON".into(),
            json!({"directory":host.profile}).to_string().into(),
        );
    }
    host.start();
    assert!(!marker.exists());
    (host, marker)
}
struct RpcWorker {
    result: mpsc::Receiver<std::io::Result<Value>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl RpcWorker {
    fn start(socket: PathBuf, request: Value) -> Self {
        let (tx, result) = mpsc::channel();
        let worker = thread::spawn(move || {
            let value = runtime_host::status_rpc::request(
                &socket,
                &request,
                Instant::now() + Duration::from_secs(3),
            );
            let _ = tx.send(value);
        });
        Self {
            result,
            worker: Some(worker),
        }
    }
    fn finish(mut self) -> Value {
        let result = self.result.recv_timeout(Duration::from_secs(4)).unwrap();
        self.worker.take().unwrap().join().unwrap();
        result.unwrap()
    }
}
impl Drop for RpcWorker {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            if self.result.recv_timeout(Duration::from_secs(4)).is_ok() {
                let _ = worker.join();
            }
        }
    }
}
fn read_reply(stream: UnixStream) -> Value {
    let mut reader = BufReader::new(stream.take(1024 * 1024 + 1));
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line).unwrap();
    assert!(
        line.len() <= 1024 * 1024 && line.last() == Some(&b'\n'),
        "Incomplete or oversized own host response"
    );
    serde_json::from_slice(&line).unwrap()
}
#[test]
fn held_configuration_write_keeps_status_search_responsive_and_second_writer_bounded() {
    let (host, marker) = writer_host(true, false);
    let worker = RpcWorker::start(
        host.socket(),
        json!({"op":"config.directory","directory":host.base.join("portable")}),
    );
    wait_marker(&marker);
    let at = Instant::now();
    let status = host.rpc(&json!({"op":"status"}))["result"].clone();
    assert_eq!(status["configurationUpdating"], true);
    assert_eq!(
        host.rpc(&json!({"op":"search","query":"finder"}))["ok"],
        true
    );
    let rejected = host.rpc(&json!({"op":"config.directory","directory":host.base.join("second")}));
    assert_eq!(rejected["ok"], false);
    assert!(rejected["error"].as_str().unwrap().contains("updated"));
    assert!(at.elapsed() < Duration::from_millis(500));
    let result = worker.finish();
    assert_eq!(result["ok"], true);
    assert_eq!(result["configurationWrite"]["published"], true);
    assert_eq!(result["configurationWrite"]["outcomeUncertain"], false);
    assert!(!host.base.join("second/config.json").exists());
    host.wait(|s| s["configurationUpdating"] == false);
}
#[test]
fn quit_while_filesystem_write_is_entered_returns_conservative_configuration_truth() {
    let (mut host, marker) = writer_host(true, false);
    let mut pending = host.connect(Duration::from_secs(3));
    writeln!(
        pending,
        "{}",
        json!({"op":"config.directory","directory":host.base.join("portable")})
    )
    .unwrap();
    wait_marker(&marker);
    let at = Instant::now();
    assert_eq!(host.rpc(&json!({"op":"quit"}))["ok"], true);
    let response = read_reply(pending);
    assert!(at.elapsed() < Duration::from_millis(500));
    assert_eq!(response["ok"], false);
    assert_eq!(
        response["configurationWrite"],
        json!({"published":false,"outcomeUncertain":true})
    );
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("may have been saved"));
    let output = host.wait_exit(Duration::from_secs(4), false);
    assert!(output.status.success());
    assert!(host.base.join("portable/config.json").exists());
    assert!(host.profile.join("config-location.json").exists());
}
#[test]
fn postpublication_selector_conflict_reports_saved_outcome_and_preserves_both_copies() {
    let (host, marker) = writer_host(false, true);
    let before = fs::read(&host.config).unwrap();
    let response =
        host.rpc(&json!({"op":"config.directory","directory":host.base.join("portable")}));
    assert!(marker.exists());
    assert_eq!(response["ok"], false);
    assert_eq!(
        response["configurationWrite"],
        json!({"published":true,"outcomeUncertain":false})
    );
    assert_eq!(response["configurationReconciliationRequired"], true);
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("may have been saved"));
    assert_eq!(fs::read(&host.config).unwrap(), before);
    assert_eq!(
        fs::read(host.base.join("portable/config.json")).unwrap(),
        before
    );
    host.wait(|s| s["configurationUpdating"] == false);
}
#[test]
fn empty_or_missing_explicit_profile_never_modifies_cwd() {
    let owned = OwnedDir::new();
    fs::set_permissions(&owned.0, fs::Permissions::from_mode(0o755)).unwrap();
    let settings = Config::current();
    for name in [None, Some("RUYCAST_HOME"), Some("RUSTCAST_HOME")] {
        let mut command = Command::new(&settings.host);
        command
            .arg("--headless-test")
            .current_dir(&owned.0)
            .env_clear()
            .env("HOME", &owned.0)
            .env("TMPDIR", &owned.0);
        if let Some(name) = name {
            command.env(name, "");
        }
        let output = runtime_host::run(&mut command, Duration::from_secs(3));
        assert!(!output.status.success());
        assert_eq!(
            fs::metadata(&owned.0).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(fs::read_dir(&owned.0).unwrap().next().is_none());
    }
}
#[test]
fn legacy_cli_status_search_reload_and_config_edit() {
    let host = host(true);
    let status = host.cli(&["status"])["result"].clone();
    assert_eq!(status["implementation"], "rust");
    assert_eq!(status["productionReady"], false);
    assert_eq!(status["headless"], true);
    assert!(matches!(
        status["health"].as_str(),
        Some("healthy" | "needs-attention")
    ));
    assert_eq!(
        host.rpc(&json!({"op":"search","query":"finder"}))["result"][0]["id"],
        "app.finder"
    );
    assert_eq!(
        host.cli(&["config", "set", "launcherHotkey", "\"cmd+space\""])["ok"],
        true
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&host.config).unwrap()).unwrap()
            ["launcherHotkey"],
        "cmd+space"
    );
    assert_eq!(host.cli(&["reload"])["ok"], true);
    assert_eq!(
        host.rpc(&json!({"op":"execute","id":"finder","dryRun":true}))["dryRun"],
        true
    );
    assert_eq!(
        host.rpc(&json!({"op":"execute","id":"finder"}))["ok"],
        false
    );
}
#[test]
fn raw_nullable_or_invalid_command_arrays_preserve_last_good() {
    let host = host(true);
    let value = default_config();
    for (field, invalid) in [
        ("aliases", Value::Null),
        ("arguments", Value::Null),
        ("aliases", json!("bad")),
        ("arguments", json!([null])),
        ("aliases", json!({})),
    ] {
        let mut bad = value.clone();
        bad["commands"][0][field] = invalid;
        host.replace(&bad, None);
        assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], false);
        assert_eq!(
            host.rpc(&json!({"op":"status"}))["result"]["commands"],
            value["commands"].as_array().unwrap().len()
        );
        let cli = host.cli_output(&["config", "validate", host.config.to_str().unwrap()]);
        assert!(!cli.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&cli.stdout).unwrap()["ok"],
            false
        );
        host.replace(&value, None);
        assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    }
    assert_eq!(
        host.cli(&["config", "validate", host.config.to_str().unwrap()])["ok"],
        true
    );
}
#[test]
fn atomic_replace_invalid_last_good_and_recovery() {
    let host = host(true);
    let mut changed = default_config();
    changed["commands"].as_array_mut().unwrap().push(json!({"id":"fixture.extra","title":"Extra","kind":"url","target":"https://example.com","aliases":[]}));
    host.replace(&changed, None);
    host.wait(|s| s["commands"] == changed["commands"].as_array().unwrap().len());
    host.replace_bytes(b"broken PRIVATE-SENTINEL", None);
    let status = host.wait(|s| !s["configError"].is_null());
    assert_eq!(
        status["commands"],
        changed["commands"].as_array().unwrap().len()
    );
    assert!(!status.to_string().contains("PRIVATE-SENTINEL"));
    assert_eq!(fs::read(&host.config).unwrap(), b"broken PRIVATE-SENTINEL");
    host.replace(&default_config(), None);
    host.wait(|s| {
        s["configError"].is_null()
            && s["commands"] == default_config()["commands"].as_array().unwrap().len()
    });
}
#[test]
fn in_place_edit_is_seen_by_five_second_fallback() {
    let host = host(true);
    let mut value = default_config();
    value["theme"] = json!("dark");
    fs::write(&host.config, serde_json::to_vec(&value).unwrap()).unwrap();
    host.wait(|s| s["theme"] == "dark");
}
#[test]
fn external_selector_rebinds_observation_to_portable_directory() {
    let host = host(true);
    let portable = host.base.join("portable");
    fs::create_dir(&portable).unwrap();
    fs::set_permissions(&portable, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        portable.join("config.json"),
        serde_json::to_vec(&default_config()).unwrap(),
    )
    .unwrap();
    host.replace(
        &json!({"directory":portable}),
        Some(&host.profile.join("config-location.json")),
    );
    host.wait(|s| s["configPath"] == portable.join("config.json").to_str().unwrap());
    let mut changed = default_config();
    changed["theme"] = json!("light");
    host.replace(&changed, Some(&portable.join("config.json")));
    host.wait(|s| s["theme"] == "light");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&host.config).unwrap()).unwrap(),
        default_config()
    );
}
#[test]
fn selector_size_bound_and_recovery_keep_valid_state() {
    let host = host(true);
    host.replace_bytes(
        &vec![b' '; 1_000_001],
        Some(&host.profile.join("config-location.json")),
    );
    let status = host.wait(|s| !s["configError"].is_null());
    assert_eq!(
        status["commands"],
        default_config()["commands"].as_array().unwrap().len()
    );
    host.replace(
        &json!({"directory":host.profile}),
        Some(&host.profile.join("config-location.json")),
    );
    host.wait(|s| s["configError"].is_null());
}
#[test]
fn relocate_keeps_original_then_cli_edits_new_target() {
    let host = host(true);
    let original = fs::read(&host.config).unwrap();
    let destination = host.base.join("portable");
    let result = host.rpc(&json!({"op":"config.directory","directory":destination}));
    assert_eq!(result["ok"], true);
    assert_eq!(result["result"]["originalRetained"], true);
    assert_eq!(fs::read(&host.config).unwrap(), original);
    assert_eq!(
        host.cli(&["config", "set", "clipboard.limit", "51"])["ok"],
        true
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(destination.join("config.json")).unwrap())
            .unwrap()["clipboard"]["limit"],
        51
    );
    assert_eq!(fs::read(&host.config).unwrap(), original);
}
#[test]
fn slow_incomplete_request_has_absolute_read_deadline() {
    let host = host(true);
    let stream = host.connect(Duration::from_secs(5));
    let mut sender = stream.try_clone().unwrap();
    sender
        .set_write_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let (stop_tx, stop_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let at = Instant::now();
    let worker = thread::spawn(move || {
        for _ in 0..20 {
            if sender.write_all(b" ").is_err() {
                break;
            }
            if !matches!(
                stop_rx.recv_timeout(Duration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                break;
            }
        }
        let _ = done_tx.send(());
    });
    let shutdown = stream.try_clone().unwrap();
    assert_eq!(read_reply(stream)["ok"], false);
    assert!(at.elapsed() < Duration::from_secs(5));
    let _ = shutdown.shutdown(Shutdown::Both);
    let _ = stop_tx.send(());
    done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
    assert_eq!(host.rpc(&json!({"op":"status"}))["ok"], true);
}
#[test]
fn second_instance_refused_and_crashed_socket_recovers() {
    let mut host = host(true);
    let second = runtime_host::run(
        host.command(&host.settings.host).arg("--headless-test"),
        Duration::from_secs(3),
    );
    assert!(!second.status.success());
    assert_eq!(host.rpc(&json!({"op":"status"}))["ok"], true);
    host.stop(false);
    assert!(host.socket().exists());
    host.start();
    assert_eq!(
        fs::metadata(&host.profile).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(host.socket()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    host.stop(true);
    assert!(!host.socket().exists());
}
#[test]
fn invalid_initial_config_releases_instance_and_socket() {
    let mut host = host(false);
    fs::write(&host.config, b"invalid").unwrap();
    let run = runtime_host::run(
        host.command(&host.settings.host).arg("--headless-test"),
        Duration::from_secs(3),
    );
    assert!(!run.status.success());
    assert!(!host.socket().exists());
    fs::write(&host.config, serde_json::to_vec(&default_config()).unwrap()).unwrap();
    host.start();
}
#[test]
fn quit_acknowledges_then_exits_normally_without_kill_fallback() {
    let mut host = host(true);
    assert_eq!(
        host.rpc(&json!({"op":"quit"})),
        json!({"ok":true,"result":{"state":"stopping"}})
    );
    let out = host.wait_exit(Duration::from_secs(3), false);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        json!({"ok":true,"result":{"state":"stopped"}})
    );
    assert!(!host.socket().exists());
}
