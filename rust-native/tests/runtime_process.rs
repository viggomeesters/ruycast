//! Fifteen original headless runtime cases on exclusive profiles and Unix sockets.
#![cfg(feature = "test-tools")]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host, OwnedDir};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::Shutdown,
    os::unix::{
        fs::{symlink, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn settings() -> Config {
    let settings = Config::current();
    runtime_receipt(&settings);
    settings
}
fn runtime_receipt(settings: &Config) {
    let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") else {
        return;
    };
    fs::create_dir_all(&directory).unwrap();
    let root = runtime_host::root();
    let sources = [
        "config/default.json",
        "rust-native/tests/runtime_process.rs",
        "rust-native/tests/support/runtime_host.rs",
        "rust-native/tests/support/status_rpc.rs",
        "rust-native/tests/support/child_output.rs",
        "rust-native/tests/fixtures/fcntl_sync_shim.rs",
        "rust-native/tests/fixtures/fcntl_abi.rs",
        "rust-native/src/main.rs",
        "rust-native/src/native_services.rs",
        "rust-native/src/runtime.rs",
        "rust-native/src/control.rs",
        "rust-native/src/configuration_jobs.rs",
        "rust-native/src/profile.rs",
        "rust-native/Cargo.toml",
        "rust-native/Cargo.lock",
        "rust/Cargo.toml",
        "rust/Cargo.lock",
        "rust/src/lib.rs",
        "rust/src/bin/ruycast.rs",
        "rust-toolchain.toml",
    ]
    .into_iter()
    .map(|p| (p, hash(&fs::read(root.join(p)).unwrap())))
    .collect::<BTreeMap<_, _>>();
    let sdk = settings
        .host
        .parent()
        .unwrap()
        .join("examples/libfcntl_sync_shim.dylib");
    let mut artifacts = BTreeMap::new();
    // Immutable content-addressed copies avoid retaining another large host/CLI
    // binary for every separately bounded case. Publish only complete bytes.
    for (name, path) in [
        ("host", &settings.host),
        ("cli", &settings.cli),
        ("writer-sdk", &sdk),
    ] {
        if name == "writer-sdk" && !path.is_file() {
            continue;
        }
        let (path, digest) = runtime_host::bind_artifact(path, name);
        artifacts.insert(path, digest);
    }
    fs::write(PathBuf::from(directory).join("runtime-reference-bindings.json"),serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":artifacts,"methods":15,"hostArguments":["--headless-test"],"scope":"Exclusive synthetic profiles, empty own Applications root, actual Rust CLI and optional same-profile Rust writer provider; no native-hidden, desktop input or applied app actions"})).unwrap()).unwrap();
}
fn host(start: bool) -> Host {
    Host::new(settings(), start)
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
fn acl_rows(path: &Path) -> Vec<String> {
    let output = runtime_host::run(
        Command::new("/bin/ls").args(["-lde"]).arg(path),
        Duration::from_secs(4),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .skip(1)
        .map(str::to_owned)
        .collect()
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
    let settings = settings();
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
fn headless_start_removes_profile_acl_through_legacy_alias() {
    let mut host = host(false);
    let alias = host.base.join("profile-alias");
    symlink(&host.profile, &alias).unwrap();
    fs::set_permissions(&host.profile, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&host.config, fs::Permissions::from_mode(0o600)).unwrap();
    let original = fs::read(&host.config).unwrap();
    let acl = "everyone allow read,execute,readattr,readextattr,readsecurity,file_inherit,directory_inherit";
    let installed = runtime_host::run(
        Command::new("/bin/chmod")
            .args(["+a", acl])
            .arg(&host.profile),
        Duration::from_secs(4),
    );
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert!(
        !acl_rows(&host.profile).is_empty(),
        "fixture must add inherited ACL"
    );
    host.env
        .insert("RUYCAST_HOME".into(), alias.as_os_str().to_owned());

    let deadline = Instant::now() + Duration::from_secs(6);
    host.start_entry_with_timeout(
        runtime_host::Entry::HeadlessTest,
        deadline.saturating_duration_since(Instant::now()),
    );
    let remaining = deadline.saturating_duration_since(Instant::now());
    assert!(!remaining.is_zero(), "headless status exceeded six seconds");
    let status_output =
        runtime_host::run(host.command(&host.settings.cli).arg("status"), remaining);
    assert_eq!(
        status_output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&status_output.stderr)
    );
    let response: Value = serde_json::from_slice(&status_output.stdout).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["result"]["pid"],
        host.process.as_ref().unwrap().id()
    );
    assert_eq!(response["result"]["headless"], true);
    assert_eq!(
        fs::metadata(&host.profile).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    assert!(
        acl_rows(&host.profile).is_empty(),
        "profile ACL must be removed"
    );
    assert!(fs::symlink_metadata(&alias)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&alias).unwrap(), host.profile);
    assert_eq!(fs::read(&host.config).unwrap(), original);
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
fn persisted_directory_is_shared_between_cli_and_owned_host_startup() {
    let mut host = host(false);
    let portable = host.base.join("Synced vault with spaces/.appdata/ruycast");
    fs::create_dir_all(&portable).unwrap();
    let original = fs::read(&host.config).unwrap();
    fs::write(portable.join("config.json"), &original).unwrap();
    fs::write(
        host.profile.join("config-location.json"),
        serde_json::to_vec(&json!({"directory":portable})).unwrap(),
    )
    .unwrap();
    let selected = portable.join("config.json");
    assert_eq!(host.cli(&["config", "path"])["result"], json!(selected));
    host.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(5));
    let status = host.cli(&["status"])["result"].clone();
    assert_eq!(status["configPath"], json!(selected));
    assert_eq!(status["profile"], json!(host.profile));
    assert_eq!(fs::read(&host.config).unwrap(), original);
    assert_eq!(fs::read(selected).unwrap(), original);
    // Match the original six-second teardown bound on this exact owned child.
    host.process.as_mut().unwrap().kill().unwrap();
    host.wait_exit(Duration::from_secs(6), true);
}

#[test]
fn owned_host_restart_uses_relocated_configuration() {
    let mut host = host(false);
    let portable = host.base.join("Synced vault with spaces/.appdata/ruycast");
    let original = fs::read(&host.config).unwrap();
    assert_eq!(
        host.cli(&["config", "directory", portable.to_str().unwrap()])["ok"],
        true
    );
    for _ in 0..2 {
        host.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(5));
        let response = host.cli(&["status"]);
        assert_eq!(response["ok"], true);
        assert_eq!(
            response["result"]["configPath"],
            json!(portable.join("config.json"))
        );
        assert_eq!(fs::read(&host.config).unwrap(), original);
        assert_eq!(fs::read(portable.join("config.json")).unwrap(), original);
        // Terminate only this unreaped owned child, as the original test did.
        assert_eq!(
            unsafe { libc::kill(host.process.as_ref().unwrap().id() as i32, libc::SIGTERM) },
            0
        );
        host.wait_exit(Duration::from_secs(6), false);
    }
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
    let mut host = host(false);
    let mut stale = ruycast_native::profile::ConfigStore::open(
        ruycast_native::profile::ProfilePaths::new(host.profile.clone(), host.base.clone(), None),
    )
    .unwrap();
    let original = fs::read(&host.config).unwrap();
    host.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(5));
    let portable = host.base.join("Synced vault with spaces/.appdata/ruycast");
    fs::create_dir_all(&portable).unwrap();
    fs::set_permissions(&portable, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(portable.join("config.json"), &original).unwrap();
    host.replace(
        &json!({"directory":portable}),
        Some(&host.profile.join("config-location.json")),
    );
    let mut changed = default_config();
    changed["clipboard"]["limit"] = json!(51);
    let error = match stale.save(&changed) {
        Ok(_) => panic!("stale selected-file owner must refuse the write"),
        Err(error) => error,
    };
    assert!(error.contains("Configuration changed"), "{error}");
    assert_eq!(stale.file, host.config);
    assert_eq!(stale.snapshot(), original);
    assert_eq!(fs::read(portable.join("config.json")).unwrap(), original);
    assert_eq!(fs::read(&host.config).unwrap(), original);
    // The Rust host intentionally adopts a valid selector. The frozen Swift
    // test's immediate CLI refusal/old host path are not timing-independent here.
    host.wait(|s| s["configPath"] == portable.join("config.json").to_str().unwrap());
    assert_eq!(fs::read(portable.join("config.json")).unwrap(), original);
    assert_eq!(
        host.cli(&["config", "set", "clipboard.limit", "51"])["ok"],
        true
    );
    let mut changed: Value =
        serde_json::from_slice(&fs::read(portable.join("config.json")).unwrap()).unwrap();
    assert_eq!(changed["clipboard"]["limit"], 51);
    assert_eq!(fs::read(&host.config).unwrap(), original);
    changed["theme"] = json!("light");
    host.replace(&changed, Some(&portable.join("config.json")));
    host.wait(|s| s["theme"] == "light");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&host.config).unwrap()).unwrap(),
        default_config()
    );
    assert_eq!(fs::read(&host.config).unwrap(), original);
    assert_eq!(
        unsafe { libc::kill(host.process.as_ref().unwrap().id() as i32, libc::SIGTERM) },
        0
    );
    host.wait_exit(Duration::from_secs(6), false);
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
    let mut host = host(false);
    host.start_entry_with_timeout(runtime_host::Entry::HeadlessTest, Duration::from_secs(5));
    let original = fs::read(&host.config).unwrap();
    let destination = host.base.join("Synced vault with spaces/.appdata/ruycast");
    let initial = host.cli(&["status"]);
    assert_eq!(initial["ok"], true);
    let pid = initial["result"]["pid"].clone();
    assert_eq!(pid, json!(host.process.as_ref().unwrap().id()));
    let result = host.cli(&["config", "directory", destination.to_str().unwrap()]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["result"]["originalRetained"], true);
    let status = host.cli(&["status"]);
    assert_eq!(status["ok"], true);
    assert_eq!(status["result"]["pid"], pid);
    assert_eq!(
        status["result"]["configPath"],
        json!(destination.join("config.json"))
    );
    assert_eq!(fs::read(&host.config).unwrap(), original);
    assert_eq!(
        host.cli(&["config", "set", "clipboard.limit", "51"])["ok"],
        true
    );
    let mut config: Value =
        serde_json::from_slice(&fs::read(destination.join("config.json")).unwrap()).unwrap();
    assert_eq!(config["clipboard"]["limit"], 51);
    assert_eq!(fs::read(&host.config).unwrap(), original);
    config["commands"].as_array_mut().unwrap().push(json!({"id":"fixture.relocated","title":"Relocated fixture","kind":"url","target":"https://example.com"}));
    let candidate = destination.join("replacement.json");
    fs::write(&candidate, serde_json::to_vec(&config).unwrap()).unwrap();
    fs::rename(candidate, destination.join("config.json")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("New location was not observed within original six-second polling budget");
        let output = runtime_host::run(
            host.command(&host.settings.cli).arg("commands"),
            remaining.min(Duration::from_secs(10)),
        );
        assert_eq!(output.status.code(), Some(0));
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["ok"], true);
        if response["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["id"] == "fixture.relocated")
        {
            assert!(Instant::now() <= deadline);
            break;
        }
        thread::sleep(
            Duration::from_millis(60).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    assert!(!destination.join("control.sock").exists());
    assert!(host.profile.join("control.sock").exists());
    assert_eq!(fs::read(&host.config).unwrap(), original);
    assert_eq!(
        unsafe { libc::kill(host.process.as_ref().unwrap().id() as i32, libc::SIGTERM) },
        0
    );
    host.wait_exit(Duration::from_secs(6), false);
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

fn rejected_socket_start(host: &Host) -> std::process::Output {
    runtime_host::run(
        host.command(&host.settings.host).arg("--headless-test"),
        Duration::from_secs(4),
    )
}

fn request_open_stream(stream: &mut UnixStream, request: &Value) -> Value {
    stream
        .write_all(&serde_json::to_vec(request).unwrap())
        .unwrap();
    stream.write_all(b"\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn start_socket_recovery(host: &mut Host) {
    host.start();
    let status = host.rpc_timeout(&json!({"op": "status"}), Duration::from_secs(2));
    assert_eq!(status["ok"], true);
    assert_eq!(status["result"]["pid"], host.process.as_ref().unwrap().id());
    assert_eq!(status["result"]["headless"], true);
    let profile = PathBuf::from(status["result"]["profile"].as_str().unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(profile, host.profile.canonicalize().unwrap());
}

#[test]
fn socket_recovery_regular_endpoint_is_preserved_then_retry_recovers() {
    let mut host = host(false);
    let payload = b"An unrelated file must survive failed server startup.";
    fs::write(host.socket(), payload).unwrap();
    let rejected = rejected_socket_start(&host);
    assert!(!rejected.status.success());
    assert_eq!(fs::read(host.socket()).unwrap(), payload);
    fs::remove_file(host.socket()).unwrap();
    start_socket_recovery(&mut host);
    host.stop(true);
    assert!(!host.socket().exists());
}

#[test]
fn socket_recovery_linked_endpoint_and_target_are_preserved() {
    let mut host = host(false);
    let target = host.base.join("endpoint-target");
    fs::write(&target, b"preserved target").unwrap();
    symlink(&target, host.socket()).unwrap();
    let rejected = rejected_socket_start(&host);
    assert!(!rejected.status.success());
    assert!(fs::symlink_metadata(host.socket())
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&target).unwrap(), b"preserved target");
    fs::remove_file(host.socket()).unwrap();
    start_socket_recovery(&mut host);
    host.stop(true);
}

#[test]
fn socket_recovery_live_endpoint_without_instance_lock_is_preserved() {
    let mut host = host(false);
    let listener = UnixListener::bind(host.socket()).unwrap();
    let before = fs::metadata(host.socket()).unwrap();
    let rejected = rejected_socket_start(&host);
    assert!(!rejected.status.success());
    let after = fs::metadata(host.socket()).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    drop(listener);
    start_socket_recovery(&mut host);
    host.stop(true);
}

#[test]
fn socket_recovery_stale_endpoint_recovers_and_graceful_quit_removes_it() {
    let mut host = host(false);
    drop(UnixListener::bind(host.socket()).unwrap());
    start_socket_recovery(&mut host);
    host.stop(true);
    assert!(!host.socket().exists());
}

#[test]
fn socket_recovery_graceful_stop_preserves_replacement_regular_file() {
    let mut host = host(false);
    start_socket_recovery(&mut host);
    let mut stream = host.connect(Duration::from_secs(2));
    fs::remove_file(host.socket()).unwrap();
    let replacement = b"replacement while host is alive";
    fs::write(host.socket(), replacement).unwrap();
    assert_eq!(
        request_open_stream(&mut stream, &json!({"op":"quit"}))["ok"],
        true
    );
    let output = host.wait_exit(Duration::from_secs(3), false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(host.socket()).unwrap(), replacement);
}

#[test]
fn socket_recovery_graceful_stop_preserves_replacement_socket() {
    let mut host = host(false);
    start_socket_recovery(&mut host);
    let mut stream = host.connect(Duration::from_secs(2));
    fs::remove_file(host.socket()).unwrap();
    let replacement = UnixListener::bind(host.socket()).unwrap();
    let before = fs::metadata(host.socket()).unwrap();
    assert_eq!(
        request_open_stream(&mut stream, &json!({"op":"quit"}))["ok"],
        true
    );
    let output = host.wait_exit(Duration::from_secs(3), false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = fs::metadata(host.socket()).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    assert!(runtime_host::status_rpc::connect(
        &host.socket(),
        Instant::now() + Duration::from_secs(3)
    )
    .is_ok());
    drop(replacement);
    start_socket_recovery(&mut host);
    host.stop(true);
}

#[test]
fn socket_recovery_linked_instance_lock_is_preserved_then_retry_recovers() {
    let mut host = host(false);
    let target = host.base.join("lock-target");
    fs::write(&target, b"preserved lock target").unwrap();
    let lock = host.profile.join("instance.lock");
    symlink(&target, &lock).unwrap();
    let rejected = rejected_socket_start(&host);
    assert!(!rejected.status.success());
    assert!(fs::symlink_metadata(&lock)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&target).unwrap(), b"preserved lock target");
    fs::remove_file(&lock).unwrap();
    start_socket_recovery(&mut host);
    host.stop(true);
}

#[test]
fn socket_recovery_fifo_instance_lock_is_rejected_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    let mut host = host(false);
    let lock = host.profile.join("instance.lock");
    let name = std::ffi::CString::new(lock.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let rejected = rejected_socket_start(&host);
    assert!(!rejected.status.success());
    assert!(fs::symlink_metadata(&lock).unwrap().file_type().is_fifo());
    fs::remove_file(&lock).unwrap();
    start_socket_recovery(&mut host);
    host.stop(true);
}
