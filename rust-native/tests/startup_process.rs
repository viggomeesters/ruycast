//! Real owned headless/hidden protocol; no normal startup or actual permissions.
#![cfg(feature = "test-tools")]
use serde_json::{json, Value};
#[path = "support/child_output.rs"]
mod child_output;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
struct Host {
    child: Child,
    base: PathBuf,
    socket: PathBuf,
}
impl Host {
    fn cli_command(&self, args: &[&str]) -> Command {
        let binary =
            std::env::var_os("RUYCAST_TEST_CLI").expect("explicit current owned CLI required");
        assert!(Path::new(&binary).is_absolute());
        let mut command = Command::new(binary);
        command
            .args(args)
            .env_clear()
            .env("HOME", &self.base)
            .env("RUYCAST_HOME", self.base.join("p"))
            .env("RUYCAST_APP_BINARY", env!("CARGO_BIN_EXE_ruycast-native"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
    fn cli(&self, args: &[&str], expected: bool) -> Value {
        let output = wait_cli(self.cli_command(args).spawn().unwrap());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.success(), expected, "{value}");
        assert_eq!(value["ok"], expected, "{value}");
        value
    }
}
fn wait_cli(child: Child) -> Output {
    child_output::wait(child, Duration::from_secs(12))
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.base);
    }
}
fn rpc(socket: &Path, request: Value) -> Value {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    serde_json::to_writer(&mut stream, &request).unwrap();
    stream.write_all(b"\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
fn host(legacy: bool, mapped: bool) -> Host {
    host_with_commands(legacy, mapped, None)
}
fn host_with_commands(legacy: bool, mapped: bool, commands: Option<Value>) -> Host {
    host_with_mode(legacy, mapped, commands, None)
}
fn host_with_mode(
    legacy: bool,
    mapped: bool,
    commands: Option<Value>,
    keyboard: Option<&Path>,
) -> Host {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
            .expect("owned short fixture parent required"),
    );
    assert!(parent.is_absolute());
    let base = parent.join(format!(
        "ps-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
    fs::write(
        base.join(".ruycast-test-fixture"),
        b"own-disposable-fixture-v1",
    )
    .unwrap();
    let profile = base.join("p");
    fs::create_dir(&profile).unwrap();
    let apps = base.join("Applications");
    fs::create_dir(&apps).unwrap();
    let selected = if mapped {
        let path = base.join("cfg");
        fs::create_dir(&path).unwrap();
        fs::write(
            profile.join("config-location.json"),
            serde_json::to_vec(&json!({"directory":path})).unwrap(),
        )
        .unwrap();
        path
    } else {
        profile.clone()
    };
    let mut config: Value =
        serde_json::from_slice(include_bytes!("../../config/default.json")).unwrap();
    if let Some(commands) = commands {
        config["commands"] = commands;
    }
    fs::write(
        selected.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let binary = env!("CARGO_BIN_EXE_ruycast-native");
    let mut command = Command::new(binary);
    command
        .arg(if keyboard.is_some() {
            "--native-hidden"
        } else {
            "--headless-test"
        })
        .env_clear()
        .env("HOME", &base)
        .env(
            "RUYCAST_NATIVE_TEST_APP_ROOTS",
            serde_json::to_string(&vec![apps]).unwrap(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command.env(
        if legacy {
            "RUSTCAST_HOME"
        } else {
            "RUYCAST_HOME"
        },
        &profile,
    );
    if let Some(keyboard) = keyboard {
        fs::write(
            base.join(".ruycast-keyboard-fixture"),
            b"own-keyboard-fixture-v1",
        )
        .unwrap();
        command
            .env("DYLD_INSERT_LIBRARIES", keyboard)
            .env("RUYCAST_NATIVE_TEST_KEYBOARD", "1")
            .env("RUYCAST_KEYWORD_FIXTURE_HOME", &base)
            .env("RUYCAST_KEYBOARD_FIXTURE_HOME", &base)
            .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &base)
            .env("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD", "trace");
    }
    let child = command.spawn().unwrap();
    let h = Host {
        child,
        base,
        socket: profile.join("control.sock"),
    };
    let until = Instant::now() + Duration::from_secs(8);
    while !h.socket.exists() {
        assert!(Instant::now() < until, "owned host readiness timeout");
        std::thread::sleep(Duration::from_millis(10));
    }
    h
}
#[test]
fn analytics_same_command_tracks_actual_cli_launcher_and_owned_carbon_callback() {
    use sha2::{Digest, Sha256};
    let keyboard = PathBuf::from(
        std::env::var_os("RUYCAST_TEST_KEYBOARD_SDK")
            .expect("verified owned keyboard SDK required"),
    );
    assert!(keyboard.is_absolute());
    let expected =
        std::env::var("RUYCAST_TEST_KEYBOARD_SHA").expect("pinned owned keyboard SDK required");
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(&keyboard).unwrap())),
        expected
    );
    let h = host_with_mode(false, false, Some(json!([])), Some(&keyboard));
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let status = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
        if status["keyboardGeneration"].as_u64().is_some_and(|n| n > 0)
            && status["configurationUpdating"] != true
            && status["nativeUI"]["revision"]
                .as_u64()
                .is_some_and(|n| n > 0)
            && status["nativeUI"]["revision"] == status["nativeUI"]["appliedRevision"]
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "initial owned native source-map readiness timeout: {status}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let id = "own.analytics.same-command";
    let mut config: Value =
        serde_json::from_slice(&fs::read(h.base.join("p/config.json")).unwrap()).unwrap();
    config["commands"] = json!([{"id":id,"title":"Owned source map","kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","echo",h.base.join("source-map.marker"),"unused","owned map payload"],"aliases":["ownmap"],"hotkey":"cmd+a","timeout":5}]);
    fs::write(
        h.base.join("p/config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let reload = rpc(&h.socket, json!({"op":"reload"}));
    assert_eq!(reload["ok"], true, "{reload}");
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let status = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
        if status["keyboardGeneration"].as_u64().is_some_and(|n| n > 0)
            && status["configurationUpdating"] != true
            && status["nativeUI"]["revision"]
                .as_u64()
                .is_some_and(|n| n > 0)
            && status["nativeUI"]["revision"] == status["nativeUI"]["appliedRevision"]
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "owned native source-map readiness timeout: {status}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let document_after = |successes: u64| {
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            let document = rpc(&h.socket, json!({"op":"analytics"}))["result"].clone();
            if document["totalSuccesses"] == successes {
                return document;
            }
            assert!(
                Instant::now() < until,
                "owned source-map completion timeout: {document}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    h.cli(&["run", id], true);
    let first = document_after(1);
    let key = ruycast_native::analytics::command_key(id);
    assert_eq!(first["commands"][&key]["sources"], json!({"cli":1}));
    assert_eq!(
        rpc(&h.socket, json!({"op":"show","query":"ownmap"}))["ok"],
        true
    );
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let status = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
        if status["nativeUI"]["visible"] == true
            && status["nativeUI"]["selectedId"] == id
            && status["nativeUI"]["revision"] == status["nativeUI"]["appliedRevision"]
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "owned launcher selection timeout: {status}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        rpc(
            &h.socket,
            json!({"op":"native.test","action":"key-event","code":36,"flags":0,"chars":"\r"})
        )["ok"],
        true
    );
    let second = document_after(2);
    assert_eq!(
        second["commands"][&key]["sources"],
        json!({"cli":1,"launcher":1})
    );
    let press = rpc(
        &h.socket,
        json!({"op":"native.test","action":"keyboard-press","code":0,"modifiers":256}),
    );
    assert_eq!(press["ok"], true);
    assert_eq!(press["result"]["statuses"], json!([0]));
    let third = document_after(3);
    assert_eq!(
        third["commands"][&key]["sources"],
        json!({"cli":1,"launcher":1,"hotkey":1})
    );
    assert_eq!(
        (
            third["totalAttempts"].as_u64(),
            third["totalFailures"].as_u64()
        ),
        (Some(3), Some(0))
    );
    h.cli(&["run", id, "--dry-run"], true);
    let after_dry_run = document_after(3);
    assert_eq!(after_dry_run["totalAttempts"], 3);
    assert_eq!(
        after_dry_run["commands"][&key]["sources"],
        third["commands"][&key]["sources"]
    );
    assert_eq!(
        fs::read_to_string(h.base.join("source-map.marker")).unwrap(),
        "run\nrun\nrun\n"
    );
    assert_eq!(
        rpc(&h.socket, json!({"op":"native.test","action":"read"}))["result"]["windowShown"],
        false
    );
    println!(
        "{}",
        json!({"actualOwnedSources":third["commands"][&key]["sources"],"attempts":3,"dryRunExcluded":true,"windowShown":false})
    );
}
#[test]
fn analytics_cli_pause_privacy_confirmation_reset_and_inflight_epoch() {
    let fake = env!("CARGO_BIN_EXE_private-results-fixture");
    // The command only writes its explicit marker and synthetic stdout/stderr.
    // No shell, system command, personal profile, global clipboard or UI is used.
    let h = host_with_commands(false, false, Some(json!([])));
    let profile = h.base.join("p");
    let secret = "OWN_ANALYTICS_INPUT_QUERY_SENTINEL_9817";
    let commands: Vec<Value> = [("success", "echo"), ("failure", "exit7"), ("inflight", "wait")]
        .into_iter().map(|(id, mode)| json!({"id":format!("own.analytics.{id}"),"title":"Owned analytics fixture","kind":"script","target":fake,"arguments":["fake",mode,h.base.join(format!("{id}.marker")),"owned synthetic output","{input}"],"inputPrompt":"Owned input","timeout":8})).collect();
    let mut config: Value =
        serde_json::from_slice(&fs::read(profile.join("config.json")).unwrap()).unwrap();
    config["commands"] = json!(commands);
    fs::write(
        profile.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    assert_eq!(rpc(&h.socket, json!({"op":"reload"}))["ok"], true);
    assert_eq!(h.cli(&["analytics"], true)["result"]["totalAttempts"], 0);
    assert_eq!(
        rpc(&h.socket, json!({"op":"search","query":secret}))["ok"],
        true
    );
    h.cli(
        &[
            "run",
            "own.analytics.success",
            "--dry-run",
            "--input",
            secret,
        ],
        true,
    );
    assert!(!h.base.join("success.marker").exists());
    assert_eq!(
        h.cli(&["analytics", "summary"], true)["result"]["totalAttempts"],
        0
    );
    h.cli(&["run", "own.analytics.success", "--input", secret], true);
    h.cli(&["run", "own.analytics.failure", "--input", secret], false);
    let before_pause = h.cli(&["analytics"], true)["result"].clone();
    assert_eq!(
        (
            before_pause["totalAttempts"].as_u64(),
            before_pause["totalSuccesses"].as_u64(),
            before_pause["totalFailures"].as_u64()
        ),
        (Some(2), Some(1), Some(1))
    );
    h.cli(&["analytics", "pause"], true);
    assert_eq!(
        rpc(&h.socket, json!({"op":"analytics"}))["result"]["enabled"],
        false
    );
    let marker_before = fs::read(h.base.join("success.marker")).unwrap();
    h.cli(&["run", "own.analytics.success", "--input", secret], true);
    assert!(fs::read(h.base.join("success.marker")).unwrap().len() > marker_before.len());
    let paused = h.cli(&["analytics"], true)["result"].clone();
    for key in ["totalAttempts", "totalSuccesses", "totalFailures"] {
        assert_eq!(paused[key], before_pause[key]);
    }
    let text = fs::read_to_string(profile.join("analytics.json")).unwrap();
    assert_eq!(
        fs::metadata(profile.join("analytics.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    for private in [
        secret,
        "SYNTHETIC_PRIVATE_SECRET_9817",
        "own.analytics.success",
        "own.analytics.failure",
        h.base.to_str().unwrap(),
    ] {
        assert!(!text.contains(private), "persisted private field {private}");
    }
    let saved: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(saved["totalAttempts"], 2);
    let exported = h.cli(&["analytics", "export"], true)["result"].to_string();
    assert!(!exported.contains("commands"));
    assert!(!exported.contains(secret));
    assert!(!exported.contains(h.base.to_str().unwrap()));
    h.cli(&["analytics", "reset"], false);
    assert_eq!(h.cli(&["analytics"], true)["result"]["totalAttempts"], 2);
    h.cli(&["analytics", "resume"], true);
    assert_eq!(h.cli(&["analytics"], true)["result"]["enabled"], true);
    struct Pending(Option<Child>);
    impl Drop for Pending {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    let mut inflight = Pending(Some(
        h.cli_command(&["run", "own.analytics.inflight", "--input", secret])
            .spawn()
            .unwrap(),
    ));
    let until = Instant::now() + Duration::from_secs(3);
    while !h.base.join("inflight.marker").exists() {
        assert!(Instant::now() < until, "owned fake did not enter");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(h.cli(&["analytics"], true)["result"]["totalAttempts"], 3);
    let config_before = fs::read(profile.join("config.json")).unwrap();
    h.cli(&["analytics", "reset", "--confirm"], true);
    let output = wait_cli(inflight.0.take().unwrap());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let reset = h.cli(&["analytics"], true)["result"].clone();
    for key in [
        "totalAttempts",
        "totalSuccesses",
        "totalFailures",
        "priorSuccesses",
    ] {
        assert_eq!(reset[key], 0);
    }
    for key in ["commands", "timings", "days", "hours"] {
        assert!(
            reset[key].as_object().unwrap().is_empty(),
            "reset retained {key}"
        );
    }
    assert!(reset["events"].as_array().unwrap().is_empty());
    assert_eq!(
        fs::read(profile.join("config.json")).unwrap(),
        config_before
    );
}
#[test]
fn actual_status_has_owned_permission_booleans_without_conflating_candidate_readiness() {
    for legacy in [false, true] {
        let h = host(legacy, true);
        let first = rpc(&h.socket, json!({"op":"status"}));
        assert_eq!(first["ok"], true);
        assert_eq!(
            first["result"]["configPath"],
            json!(h.base.join("cfg/config.json"))
        );
        assert_eq!(first["result"]["accessibility"], false);
        assert_eq!(first["result"]["inputMonitoring"], false);
        assert_eq!(first["result"]["productionReady"], false);
        for accessibility in [false, true] {
            for monitoring in [false, true] {
                let reply = rpc(
                    &h.socket,
                    json!({"op":"permissions.fixture","accessibility":accessibility,"inputMonitoring":monitoring,"raycastRunning":false}),
                );
                assert_eq!(reply["ok"], true);
                let status = rpc(&h.socket, json!({"op":"status"}));
                assert_eq!(status["result"]["accessibility"], accessibility);
                assert_eq!(status["result"]["inputMonitoring"], monitoring);
                assert_eq!(status["result"]["health"], "healthy");
                assert_eq!(status["result"]["productionReady"], false);
            }
        }
        assert_eq!(
            rpc(
                &h.socket,
                json!({"op":"permissions.fixture","accessibility":"true","inputMonitoring":true,"raycastRunning":false})
            )["ok"],
            false
        );
    }
}
#[test]
fn version_and_invalid_arguments_do_not_resolve_or_create_a_profile() {
    let binary = env!("CARGO_BIN_EXE_ruycast-native");
    for (args, success) in [
        (vec!["--version"], true),
        (vec!["--not-a-command"], false),
        (vec!["--native-hidden"], false),
        (vec!["--headless-test"], false),
    ] {
        let out = Command::new(binary)
            .args(args)
            .env_clear()
            .output()
            .unwrap();
        assert_eq!(out.status.success(), success);
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["ok"], success);
    }
}

fn settings_tail_owned_host() -> Host {
    use sha2::{Digest, Sha256};
    let keyboard = PathBuf::from(std::env::var_os("RUYCAST_TEST_KEYBOARD_SDK").expect("verified owned keyboard SDK required"));
    assert!(keyboard.is_absolute());
    assert_eq!(format!("{:x}", Sha256::digest(fs::read(&keyboard).unwrap())), std::env::var("RUYCAST_TEST_KEYBOARD_SHA").unwrap());
    let h = host_with_mode(false, false, Some(json!([])), Some(&keyboard));
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let status = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
        if status["keyboardGeneration"].as_u64().is_some_and(|n| n > 0)
            && status["configurationUpdating"] != true
            && status["nativeUI"]["revision"].as_u64().is_some_and(|n| n > 0)
            && status["nativeUI"]["revision"] == status["nativeUI"]["appliedRevision"] {
            break;
        }
        assert!(Instant::now() < until, "own settings fixture readiness: {status}");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(rpc(&h.socket, json!({"op":"clipboard.pause","paused":true}))["ok"], true);
    let clips = (0..4).map(|i| json!({"id":format!("raycast-settings-{i}"),"text":format!("Owned history {i}"),"source":"fixture","date":ruycast_native::clipboard_history::now()})).collect::<Vec<_>>();
    assert_eq!(rpc(&h.socket, json!({"op":"clipboard.import","items":clips}))["ok"], true);
    assert_eq!(rpc(&h.socket, json!({"op":"clipboard.list"}))["result"].as_array().unwrap().len(), 4);
    h
}

#[test]
fn settings_rejected_registration_preserves_four_clips_bytes_and_old_callback() {
    let h = settings_tail_owned_host();
    let profile = h.base.join("p");
    let before = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
    let history = fs::read(profile.join("clipboard.json")).unwrap();
    let clips = rpc(&h.socket, json!({"op":"clipboard.list"}))["result"].clone();
    let mut candidate: Value = serde_json::from_slice(&fs::read(profile.join("config.json")).unwrap()).unwrap();
    candidate["launcherHotkey"] = json!("cmd+a");
    candidate["clipboard"]["limit"] = json!(1);
    candidate["theme"] = json!("light");
    fs::write(h.base.join("fail-code"), b"0").unwrap();
    fs::write(profile.join("config.json"), serde_json::to_vec(&candidate).unwrap()).unwrap();
    let reload = rpc(&h.socket, json!({"op":"reload"}));
    assert_eq!(reload["ok"], false, "{reload}");
    let after = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
    assert_eq!(after["configurationRevision"], before["configurationRevision"]);
    assert_eq!(after["keyboardGeneration"], before["keyboardGeneration"]);
    assert_eq!(after["theme"], before["theme"]);
    assert_eq!(rpc(&h.socket, json!({"op":"clipboard.list"}))["result"], clips);
    assert_eq!(fs::read(profile.join("clipboard.json")).unwrap(), history);
    let press = rpc(&h.socket, json!({"op":"native.test","action":"keyboard-press","code":49,"modifiers":6144}));
    assert_eq!(press["ok"], true, "{press}");
    assert_eq!(press["result"]["statuses"], json!([0]), "old binding must remain active: {press}");
}

#[test]
fn settings_clipboard_write_refusal_preserves_old_config_and_active_callback() {
    let h = settings_tail_owned_host();
    let profile = h.base.join("p");
    let before = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
    let clips = rpc(&h.socket, json!({"op":"clipboard.list"}))["result"].clone();
    fs::remove_file(profile.join("clipboard.json")).unwrap();
    fs::create_dir(profile.join("clipboard.json")).unwrap();
    let mut candidate: Value = serde_json::from_slice(&fs::read(profile.join("config.json")).unwrap()).unwrap();
    candidate["launcherHotkey"] = json!("cmd+a");
    candidate["clipboard"]["limit"] = json!(1);
    candidate["theme"] = json!("light");
    fs::write(profile.join("config.json"), serde_json::to_vec(&candidate).unwrap()).unwrap();
    let reload = rpc(&h.socket, json!({"op":"reload"}));
    let after = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
    let press = rpc(&h.socket, json!({"op":"native.test","action":"keyboard-press","code":49,"modifiers":6144}));
    assert_eq!(reload["ok"], false, "clipboard write refusal must reject config; reply={reload}; status={after}; old_callback={press}");
    assert_eq!(after["configurationRevision"], before["configurationRevision"]);
    assert_eq!(after["keyboardGeneration"], before["keyboardGeneration"]);
    assert_eq!(after["theme"], before["theme"]);
    assert_eq!(rpc(&h.socket, json!({"op":"clipboard.list"}))["result"], clips);
    assert!(profile.join("clipboard.json").is_dir());
    assert_eq!(press["ok"], true, "{press}");
    assert_eq!(press["result"]["statuses"], json!([0]), "old binding must remain active: {press}");
}

#[test]
fn settings_unchanged_reload_retries_repaired_clipboard_cleanup_immediately() {
    let h = settings_tail_owned_host();
    let profile = h.base.join("p");
    let assets = profile.join("clipboard-assets");
    assert!(!assets.exists());
    fs::write(&assets, b"owned unavailable directory").unwrap();
    let mut candidate: Value = serde_json::from_slice(&fs::read(profile.join("config.json")).unwrap()).unwrap();
    candidate["clipboard"]["limit"] = json!(1);
    candidate["clipboard"]["enabled"] = json!(false);
    fs::write(profile.join("config.json"), serde_json::to_vec(&candidate).unwrap()).unwrap();
    let first = rpc(&h.socket, json!({"op":"reload"}));
    assert_eq!(first["ok"], true, "cleanup failure must not roll back committed config: {first}");
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let status = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
        if status["clipboardCleanupError"].is_string() {
            assert_eq!(status["clipboardCount"], 1);
            assert_eq!(status["clipboardEnabled"], false);
            assert_eq!(status["clipboardPaused"], true);
            break;
        }
        assert!(Instant::now() < until, "cleanup failure was hidden: {status}");
        std::thread::sleep(Duration::from_millis(10));
    }
    fs::remove_file(&assets).unwrap();
    let bytes = fs::read(profile.join("config.json")).unwrap();
    let second = rpc(&h.socket, json!({"op":"reload"}));
    assert_eq!(second["ok"], true, "{second}");
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let status = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
        if status["clipboardCleanupError"].is_null() {
            assert_eq!(status["clipboardCount"], 1);
            assert_eq!(status["clipboardEnabled"], false);
            assert_eq!(status["clipboardPaused"], true);
            break;
        }
        assert!(Instant::now() < until, "explicit unchanged reload must retry repaired cleanup: {status}");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read(profile.join("config.json")).unwrap(), bytes);
}

#[test]
fn settings_rejected_image_retention_preserves_asset_then_disabled_paused_acceptance_prunes_it() {
    let h = settings_tail_owned_host();
    let profile = h.base.join("p");
    let assets = profile.join("clipboard-assets");
    fs::DirBuilder::new().mode(0o700).create(&assets).unwrap();
    let image = assets.join("11111111-1111-4111-8111-111111111111.png");
    let png = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6,
        0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 104, 112, 80, 248,
        15, 194, 12, 48, 6, 0, 64, 180, 7, 125, 193, 185, 247, 76, 0, 0, 0, 0, 73, 69, 78, 68, 174,
        66, 96, 130,
    ];
    fs::write(&image, png).unwrap();
    fs::set_permissions(&image, fs::Permissions::from_mode(0o600)).unwrap();
    let clips = json!([
        {"id":"raycast-settings-keep","text":"Owned keep","source":"fixture","date":ruycast_native::clipboard_history::now()},
        {"id":"raycast-settings-image","text":"Owned image","source":"fixture","date":ruycast_native::clipboard_history::now(),"contentType":"image","dataFile":"clipboard-assets/11111111-1111-4111-8111-111111111111.png"}
    ]);
    let history = serde_json::to_vec_pretty(&clips).unwrap();
    fs::write(profile.join("clipboard.json"), &history).unwrap();
    // Import deliberately rejects image assets. Explicit unchanged reload is
    // the existing owner rebase route for this wholly owned on-disk history.
    let refreshed = rpc(&h.socket, json!({"op":"reload"}));
    assert_eq!(refreshed["ok"], true, "{refreshed}");
    let loaded = rpc(&h.socket, json!({"op":"clipboard.list"}));
    assert_eq!(loaded["result"].as_array().unwrap().len(), 2);
    assert_eq!(loaded["result"][1]["dataFile"], clips[1]["dataFile"]);
    assert_eq!(fs::read(&image).unwrap(), png);
    let history = fs::read(profile.join("clipboard.json")).unwrap();
    let before = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
    let mut candidate: Value =
        serde_json::from_slice(&fs::read(profile.join("config.json")).unwrap()).unwrap();
    candidate["launcherHotkey"] = json!("cmd+a");
    candidate["clipboard"]["limit"] = json!(1);
    candidate["clipboard"]["enabled"] = json!(false);
    let trace = h.base.join("keyboard-trace.jsonl");
    let trace_before = fs::read(&trace).unwrap();
    fs::write(h.base.join("fail-code"), b"0").unwrap();
    fs::write(
        profile.join("config.json"),
        serde_json::to_vec(&candidate).unwrap(),
    )
    .unwrap();
    let rejected = rpc(&h.socket, json!({"op":"reload"}));
    assert_eq!(rejected["ok"], false, "{rejected}");
    let rpc_busy = rejected["error"] == "Configuration is being updated. Try again.";
    let deadline = Instant::now() + Duration::from_secs(3);
    let after = loop {
        let status = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
        let journal = fs::read(&trace).unwrap();
        assert!(journal.starts_with(&trace_before), "owned registration journal changed its prefix");
        let observed_register = std::str::from_utf8(&journal[trace_before.len()..])
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|entry| entry["phase"] == "register" && entry["a"] == 0 && entry["mainThread"] == true);
        if observed_register && status["configurationUpdating"] != true {
            assert_eq!(fs::read(h.base.join("fail-code")).unwrap(), b"0");
            eprintln!("owned registration code0 observed while fail-code0 armed; rejected RPC busy={rpc_busy}; writer settled");
            break status;
        }
        assert!(Instant::now() < deadline, "armed code0 registration failure did not settle: {status}; RPC={rejected}");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        after["configurationRevision"],
        before["configurationRevision"]
    );
    assert_eq!(after["keyboardGeneration"], before["keyboardGeneration"]);
    assert_eq!(after["clipboardEnabled"], before["clipboardEnabled"]);
    assert_eq!(after["clipboardPaused"], true);
    assert_eq!(
        rpc(&h.socket, json!({"op":"clipboard.list"}))["result"],
        loaded["result"]
    );
    assert_eq!(fs::read(profile.join("clipboard.json")).unwrap(), history);
    assert_eq!(fs::read(&image).unwrap(), png);
    assert_eq!(after["clipboardCount"], 2);
    fs::remove_file(h.base.join("fail-code")).unwrap();
    let accepted = rpc(&h.socket, json!({"op":"reload"}));
    assert_eq!(accepted["ok"], true, "{accepted}");
    let status = rpc(&h.socket, json!({"op":"status"}))["result"].clone();
    assert_eq!(status["clipboardEnabled"], false);
    assert_eq!(status["clipboardPaused"], true);
    assert_eq!(status["clipboardCount"], 1);
    assert!(status["clipboardCleanupError"].is_null(), "{status}");
    let kept = rpc(&h.socket, json!({"op":"clipboard.list"}));
    assert_eq!(kept["result"].as_array().unwrap().len(), 1);
    assert_eq!(kept["result"][0]["id"], "raycast-settings-keep");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(profile.join("clipboard.json")).unwrap())
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !image.exists(),
        "accepted retention must prune the owned image even while disabled and paused"
    );
}

#[test]
fn settings_original_sided_filter_preserves_both_routes_repeat_release_and_modifier_boundaries() {
    use ruycast_native::{
        keyboard_state::{Binding, CommandFilter, Event},
        recorder::{COMMAND, LEFT_COMMAND, OPTION, RIGHT_COMMAND, SHIFT},
    };
    let mut filter = CommandFilter::default();
    filter.set_bindings(
        [
            ("right-cmd+k", "right"),
            ("left-cmd+k", "left"),
            ("shift+right-cmd+k", "shifted"),
        ]
        .into_iter()
        .map(|(key, action)| Binding::parse(key, action.into()).unwrap())
        .collect(),
    );
    let right = COMMAND | RIGHT_COMMAND;
    let left = COMMAND | LEFT_COMMAND;
    assert_eq!(
        filter
            .handle(Event::Down, 40, right, false)
            .action
            .as_deref(),
        Some("right")
    );
    assert!(filter.handle(Event::Down, 40, right, true).action.is_none());
    assert!(filter.handle(Event::Up, 40, 0, false).consumed);
    assert_eq!(
        filter
            .handle(Event::Down, 40, left, false)
            .action
            .as_deref(),
        Some("left")
    );
    assert!(filter.handle(Event::Up, 40, left, false).consumed);
    for (key, flags) in [
        (40, COMMAND),
        (40, right | LEFT_COMMAND),
        (41, right),
        (40, right | OPTION),
    ] {
        assert!(!filter.handle(Event::Down, key, flags, false).consumed);
    }
    assert_eq!(
        filter
            .handle(Event::Down, 40, right | SHIFT, false)
            .action
            .as_deref(),
        Some("shifted")
    );
    filter.reset();
    assert!(!filter.handle(Event::Up, 40, 0, false).consumed);
    filter.set_bindings(vec![]);
    assert!(!filter.handle(Event::Down, 40, right, false).consumed);
}

#[test]
fn settings_disabled_dryrun_and_discovered_identity_cannot_bypass_saved_policy() {
    let mut h = host_with_commands(false, false, Some(json!([])));
    assert_eq!(rpc(&h.socket, json!({"op":"quit"}))["ok"], true);
    let deadline=Instant::now()+Duration::from_secs(4);
    while h.child.try_wait().unwrap().is_none() {
        assert!(Instant::now()<deadline,"owned headless host did not quit");
        std::thread::sleep(Duration::from_millis(10));
    }
    h.child.wait().unwrap();
    assert!(!h.socket.exists(),"owned socket must retire before restart");
    let contents=h.base.join("Applications/Owned Settings.app/Contents");
    fs::create_dir_all(&contents).unwrap();
    fs::write(contents.join("Info.plist"),br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>dev.fixture.settings-policy</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleDisplayName</key><string>Owned Settings Discovery</string></dict></plist>"#).unwrap();
    h.child=Command::new(env!("CARGO_BIN_EXE_ruycast-native"))
        .arg("--headless-test").env_clear().env("HOME",&h.base)
        .env("RUYCAST_HOME",h.base.join("p"))
        .env("RUYCAST_NATIVE_TEST_APP_ROOTS",serde_json::to_string(&vec![h.base.join("Applications")]).unwrap())
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit()).spawn().unwrap();
    let deadline=Instant::now()+Duration::from_secs(8);
    while !h.socket.exists() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    let discovered="discovered.dev.fixture.settings-policy";
    loop {
        let rows=h.cli(&["search","Owned Settings Discovery"],true);
        if rows["result"].as_array().unwrap().iter().any(|c|c["id"]==discovered){break;}
        assert!(Instant::now()<deadline,"real owned app was not discovered: {rows}");
        std::thread::sleep(Duration::from_millis(10));
    }
    let profile=h.base.join("p");
    let mut candidate:Value=serde_json::from_slice(&fs::read(profile.join("config.json")).unwrap()).unwrap();
    candidate["commands"]=json!([{"id":"own.settings.disabled-app","title":"Owned Settings Saved","kind":"app","target":"dev.fixture.settings-policy","aliases":["myfinder","files-home"],"enabled":false}]);
    fs::write(profile.join("config.json"),serde_json::to_vec(&candidate).unwrap()).unwrap();
    let reload=rpc(&h.socket,json!({"op":"reload"}));assert_eq!(reload["ok"],true,"{reload}");
    let before=fs::read(profile.join("config.json")).unwrap();
    let disabled=h.cli(&["run","own.settings.disabled-app","--dry-run"],false);
    assert!(disabled["error"].as_str().unwrap().to_lowercase().contains("disabled"),"{disabled}");
    let unavailable=h.cli(&["run",discovered,"--dry-run"],false);
    assert_eq!(unavailable["ok"],false);
    for query in ["myfinder","Owned Settings Discovery"] {
        let rows=h.cli(&["search",query],true);
        assert!(!rows["result"].as_array().unwrap().iter().any(|c|c["target"]=="dev.fixture.settings-policy"),"{rows}");
    }
    assert_eq!(fs::read(profile.join("config.json")).unwrap(),before);
    assert_eq!(h.cli(&["config","get"],true)["result"]["commands"][0]["aliases"],json!(["myfinder","files-home"]));
}

#[test]
fn settings_reenable_conflict_then_alias_preservation_and_retention_fourteen_reload() {
    let h=settings_tail_owned_host();
    let profile=h.base.join("p");
    let mut candidate:Value=serde_json::from_slice(&fs::read(profile.join("config.json")).unwrap()).unwrap();
    candidate["commands"]=json!([
        {"id":"own.settings.enable","title":"Owned re-enable","kind":"script","target":"/usr/bin/true","arguments":[],"aliases":["myfinder","files-home"],"hotkey":"alt+cmd+g","enabled":false},
        {"id":"own.settings.conflict","title":"Owned conflict","kind":"script","target":"/usr/bin/true","arguments":[],"aliases":[],"hotkey":"alt+cmd+g"}
    ]);
    fs::write(profile.join("config.json"),serde_json::to_vec(&candidate).unwrap()).unwrap();
    let reload=rpc(&h.socket,json!({"op":"reload"}));assert_eq!(reload["ok"],true,"{reload}");
    assert_eq!(rpc(&h.socket,json!({"op":"show","mode":"settings","query":"myfinder"}))["ok"],true);
    let deadline=Instant::now()+Duration::from_secs(5);
    loop {
        let status=rpc(&h.socket,json!({"op":"status"}))["result"].clone();
        if status["configurationUpdating"]!=true && status["managementUI"]["selectedId"]=="own.settings.enable" && status["managementUI"]["revision"]==status["managementUI"]["appliedRevision"] {break;}
        assert!(Instant::now()<deadline,"own management alias selection not ready: {status}");
        std::thread::sleep(Duration::from_millis(10));
    }
    let before=fs::read(profile.join("config.json")).unwrap();
    let status=rpc(&h.socket,json!({"op":"status"}))["result"].clone();
    let enabled=rpc(&h.socket,json!({"op":"native.test","surface":"settings","action":"library-enable","id":"own.settings.enable"}));assert_eq!(enabled["ok"],true,"{enabled}");
    let deadline=Instant::now()+Duration::from_secs(5);
    loop {
        let view=rpc(&h.socket,json!({"op":"native.test","surface":"settings","action":"read"}));assert_eq!(view["ok"],true,"{view}");
        if view["result"]["screenText"].as_array().unwrap().iter().any(|s|s.as_str().is_some_and(|s|s.to_lowercase().contains("duplicate hotkey"))){break;}
        assert!(Instant::now()<deadline,"conflicting enable did not show its real error: {view}");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read(profile.join("config.json")).unwrap(),before);
    let rejected=rpc(&h.socket,json!({"op":"status"}))["result"].clone();
    assert_eq!(rejected["configurationRevision"],status["configurationRevision"]);
    assert_eq!(rejected["keyboardGeneration"],status["keyboardGeneration"]);
    let mut commands=candidate["commands"].clone();commands[1].as_object_mut().unwrap().remove("hotkey");
    h.cli(&["config","set","commands",&serde_json::to_string(&commands).unwrap()],true);
    let deadline=Instant::now()+Duration::from_secs(5);
    loop {
        let status=rpc(&h.socket,json!({"op":"status"}))["result"].clone();
        if status["configurationUpdating"]!=true && status["managementUI"]["revision"]==status["managementUI"]["appliedRevision"] {break;}
        assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));
    }
    let enabled=rpc(&h.socket,json!({"op":"native.test","surface":"settings","action":"library-enable","id":"own.settings.enable"}));assert_eq!(enabled["ok"],true,"{enabled}");
    let deadline=Instant::now()+Duration::from_secs(5);
    loop {
        let now:Value=serde_json::from_slice(&fs::read(profile.join("config.json")).unwrap()).unwrap();
        let status=rpc(&h.socket,json!({"op":"status"}))["result"].clone();
        if now["commands"][0]["enabled"]==true && status["configurationUpdating"]==false
            && status["managementUI"]["revision"]==status["managementUI"]["appliedRevision"] {break;}
        assert!(Instant::now()<deadline,"repaired enable did not persist: {now}");std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(h.cli(&["search","myfinder"],true)["result"][0]["id"],"own.settings.enable");
    let config=h.cli(&["config","get"],true)["result"].clone();
    assert_eq!(config["commands"][0]["aliases"],json!(["myfinder","files-home"]));
    assert_eq!(config["commands"][0]["target"],"/usr/bin/true");
    h.cli(&["config","set","clipboard.retentionDays","14"],true);
    let mut expected=config;expected["clipboard"]["retentionDays"]=json!(14);
    assert_eq!(h.cli(&["config","get"],true)["result"],expected);
    let reload=rpc(&h.socket,json!({"op":"reload"}));assert_eq!(reload["ok"],true,"{reload}");
    assert_eq!(rpc(&h.socket,json!({"op":"status"}))["result"]["clipboardPaused"],true);
    assert_eq!(h.cli(&["config","get"],true)["result"]["clipboard"]["retentionDays"],14);
    let view=rpc(&h.socket,json!({"op":"native.test","surface":"settings","action":"read"}));assert_eq!(view["ok"],true,"{view}");
    assert_eq!(view["result"]["windowShown"],false);assert_eq!(view["result"]["windowKey"],false);assert_eq!(view["result"]["windowMain"],false);
}


// These two original Python-discovery conjunctions need the explicit late
// artifact-bound fixture route; ordinary public cargo tests must not invent it.
mod benchmark_cli_contracts {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, io::Read, os::unix::fs::{MetadataExt, OpenOptionsExt}};

    fn sha(path: &Path) -> String {
        let mut file = fs::OpenOptions::new().read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC).open(path).unwrap();
        let before = file.metadata().unwrap();
        assert!(before.is_file() && before.len() <= 128 * 1024 * 1024);
        let mut digest = Sha256::new();let mut total = 0u64;let mut buffer = [0u8;65536];
        loop {
            let count = file.read(&mut buffer).unwrap();if count == 0 {break;}
            total += count as u64;assert!(total <= 128 * 1024 * 1024);
            digest.update(&buffer[..count]);
        }
        let after = file.metadata().unwrap();let current = fs::symlink_metadata(path).unwrap();
        let identity = |m: &fs::Metadata| (m.dev(),m.ino(),m.len(),m.mode(),m.uid(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec());
        assert_eq!(identity(&before),identity(&after));assert_eq!(identity(&before),identity(&current));
        format!("{:x}",digest.finalize())
    }
    fn tool(key: &str) -> PathBuf {
        let path = PathBuf::from(std::env::var_os(key).expect("explicit independently qualified artifact required"));
        assert!(path.is_absolute());assert_eq!(path.canonicalize().unwrap(),path);
        let expected = std::env::var(format!("{key}_SHA256")).expect("independent artifact SHA required");
        assert_eq!(sha(&path),expected);path
    }
    fn private_file(path: &Path, bytes: &[u8]) {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path).unwrap();
        file.write_all(bytes).unwrap();
    }
    fn benchmark_case(changed: bool) {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
        let parent = PathBuf::from(std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").expect("owned short private parent required"));
        assert!(parent.is_absolute());assert_eq!(parent.canonicalize().unwrap(),parent);
        let metadata = fs::symlink_metadata(&parent).unwrap();
        assert!(metadata.is_dir() && metadata.uid() == unsafe {libc::geteuid()} && metadata.mode() & 0o077 == 0);
        let cli = tool("RUYCAST_CLI_PARITY_BINARY");
        let host = tool("RUYCAST_CLI_PARITY_NATIVE");
        let python = tool("RUYCAST_BENCHMARK_PYTHON");
        let python_alias = python.parent().unwrap().join("python3");
        assert_eq!(python_alias.canonicalize().unwrap(),python,"legacy fixture shebang must use the same qualified interpreter");
        let adapter = PathBuf::from(env!("CARGO_BIN_EXE_private-results-fixture")).canonicalize().unwrap();
        let driver = repository.join("scripts/benchmark-host.py");
        let original = repository.join("Tests/test_cli.py");
        let before = BTreeMap::from([
            (cli.clone(),sha(&cli)),(host.clone(),sha(&host)),(python.clone(),sha(&python)),
            (adapter.clone(),sha(&adapter)),(driver.clone(),sha(&driver)),(original.clone(),sha(&original)),
        ]);
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let case = parent.join(format!("bc{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&case).unwrap();
        assert!(case.join("rc-h-0123456789/control.sock").as_os_str().as_encoded_bytes().len() <= 104);
        private_file(&case.join(".ruycast-test-fixture"),b"own-disposable-fixture-v1");
        let external = case.join("external-config");fs::DirBuilder::new().mode(0o700).create(&external).unwrap();
        let mut authored: Value = serde_json::from_str(include_str!("../../config/default.json")).unwrap();
        authored["commands"] = json!([{"id":"external.marker","title":"External must stay unchanged","kind":"url","target":"https://example.com/external"}]);
        let authored_bytes = serde_json::to_vec_pretty(&authored).unwrap();
        let external_file = external.join("config.json");private_file(&external_file,&authored_bytes);
        let selected_cli = if changed {
            let own = case.join("changing-cli");
            // Verbatim program from test_cli.py::test_host_benchmark_rejects_changed_cli.
            let bytes = b"#!/usr/bin/env python3\nfrom pathlib import Path\np=Path(__file__)\np.write_text(p.read_text()+\"# changed\\n\")\nprint('{\"ok\":true}')\n";
            private_file(&own,bytes);fs::set_permissions(&own,fs::Permissions::from_mode(0o700)).unwrap();own
        } else {cli.clone()};
        let selected_before = sha(&selected_cli);
        let report = case.join("result.json");
        let path = format!("{}:/usr/bin:/bin",python.parent().unwrap().display());
        let mut arguments = vec!["-i".to_owned(),format!("PATH={path}"),format!("HOME={}",case.display()),
            format!("TMPDIR={}",case.display()),format!("RUYCAST_NATIVE_TEST_FIXTURE_DIR={}",case.display()),
            format!("RUYCAST_CONFIG_DIR={}",external.display()),format!("RUYCAST_CLI_PARITY_BINARY={}",cli.display()),
            format!("RUYCAST_CLI_PARITY_BINARY_SHA256={}",before[&cli]),format!("RUYCAST_CLI_PARITY_NATIVE={}",host.display()),
            format!("RUYCAST_CLI_PARITY_NATIVE_SHA256={}",before[&host]),python.to_string_lossy().into_owned(),driver.to_string_lossy().into_owned()];
        for (flag,value) in [("--host",adapter.as_path()),("--cli",selected_cli.as_path()),("--probe",Path::new("/usr/bin/true")),("--out",report.as_path())] {
            arguments.extend([flag.into(),value.to_string_lossy().into_owned()]);
        }
        arguments.extend(["--cold-starts","1","--samples","1","--active-seconds","0","--idle-seconds","0"].map(str::to_owned));
        let specification: rustcast_core::Command = serde_json::from_value(json!({
            "id":"owned.benchmark-contract","title":"Owned benchmark contract","kind":"script","target":"/usr/bin/env","arguments":arguments,"timeout":30.0
        })).unwrap();
        private_file(&case.join("before.json"),&serde_json::to_vec_pretty(&before).unwrap());
        // Reuse the existing bounded script supervisor: owned PG/EOF custody,
        // output caps, 30s timeout and final group cleanup, no new runner.
        let completion = rustcast_core::script::run(&specification,&case,None,||false).unwrap();
        private_file(&case.join("stdout.log"),completion.stdout.as_bytes());
        private_file(&case.join("stderr.log"),completion.stderr.as_bytes());
        private_file(&case.join("status.json"),&serde_json::to_vec_pretty(&json!({"end":format!("{:?}",completion.end),"capture":completion.capture})).unwrap());
        assert!(!completion.capture.stdout.truncated && !completion.capture.stderr.truncated);
        assert!(completion.capture.stdout.eof && completion.capture.stderr.eof);
        assert!(completion.capture.stdout.valid_utf8 && completion.capture.stderr.valid_utf8);
        let status = match completion.end {rustcast_core::script::End::Exited(status)=>status,other=>panic!("original30s boundary failed: {other:?}")};
        let value: Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
        assert_eq!(fs::read(&external_file).unwrap(),authored_bytes);
        assert_eq!(value["hostSHA256"],before[&adapter]);
        assert_eq!(value["cliSHA256"],selected_before);
        for (path,expected) in &before {assert_eq!(sha(path),*expected,"bound source/artifact changed: {}",path.display());}
        if changed {
            assert!(!status.success());assert_eq!(value["ok"],false);assert_eq!(value["phase"],"cli-status");
            assert_eq!(value["errorType"],"RuntimeError");
            assert!(value["error"].as_str().unwrap().contains("CLI binary changed during measurement"));
            assert!(completion.stderr.contains("CLI binary changed during measurement"));
            assert_eq!(value["cleanup"]["graceful"],true);assert_eq!(value["cleanup"]["forcedCleanup"],false);
            assert_ne!(sha(&selected_cli),selected_before);
        } else {
            assert!(status.success(),"{}",completion.stderr);assert_eq!(value["ok"],true);
            assert_eq!(value["fixtureConfigurationIsolated"],true);assert_eq!(value["idleEndpoint"]["searchVerified"],true);
            assert_eq!(value["idleEndpoint"]["commandID"],"fixture.42");
            assert_eq!(value["coldShutdowns"][0]["graceful"],true);
            assert_eq!(value["coldShutdowns"][0]["forcedCleanup"],false);
            assert_eq!(value["warmCLI"]["status"]["samples"],1);
            assert_eq!(value["warmCLI"]["search"]["samples"],1);
            assert_eq!(value["finalShutdown"]["graceful"],true);assert_eq!(value["finalShutdown"]["forcedCleanup"],false);
            assert_eq!(value["cliSHA256"],before[&cli]);
        }
        private_file(&case.join("after.json"),&serde_json::to_vec_pretty(&before.keys().map(|path|(path.clone(),sha(path))).collect::<BTreeMap<_,_>>()).unwrap());
        println!("owned benchmark contract evidence: {}",case.display());
        // Preserve this exclusively created case for Root's primary-result
        // readback. Root removes it after retaining the evidence, never /tmp.
    }
    #[test]
    #[ignore = "Explicit late benchmark contract with qualified current Rust artifacts and owned fixture parent"]
    fn real_rust_benchmark_ignores_external_configuration_with_500_commands() {benchmark_case(false)}
    #[test]
    #[ignore = "Explicit late benchmark contract with qualified current Rust artifacts and owned fixture parent"]
    fn real_rust_benchmark_rejects_changed_cli_and_quits_gracefully() {benchmark_case(true)}
}
