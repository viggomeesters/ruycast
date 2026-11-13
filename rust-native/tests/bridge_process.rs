//! Native bridge and normal-host refusal cases on prohibited/offscreen profiles.
#![cfg(feature = "test-tools")]
#[path = "support/native_bridge.rs"]
mod native_bridge;
use native_bridge::{normal_host, runtime_host, Bridge};
use runtime_host::{default_config, Config};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn receipt(settings: &Config) {
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        fs::create_dir_all(&directory).unwrap();
        let root = runtime_host::root();
        let sources = [
            "config/default.json",
            "rust-native/tests/bridge_process.rs",
            "rust-native/tests/support/native_bridge.rs",
            "rust-native/tests/support/runtime_host.rs",
            "rust-native/tests/support/status_rpc.rs",
            "rust-native/tests/support/child_output.rs",
            "rust-native/src/main.rs",
            "rust-native/src/native.rs",
            "rust-native/src/native_model.rs",
            "rust-native/src/native_surface.rs",
            "rust-native/src/native_services.rs",
            "rust-native/src/connected.rs",
            "rust-native/src/control.rs",
            "rust-native/src/runtime.rs",
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
        let normal = normal_host();
        let artifacts = [
            ("host", &settings.host),
            ("cli", &settings.cli),
            ("normal-host", &normal),
        ]
        .into_iter()
        .map(|(label, path)| runtime_host::bind_artifact(path, label))
        .collect::<BTreeMap<_, _>>();
        fs::write(PathBuf::from(directory).join("bridge-reference-bindings.json"),serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":artifacts,"suiteMethods":13,"hostArguments":["--native-hidden","--headless-test"],"normalHostPurpose":"normal-feature refusal only; matrix retains verified normalHostInputs release-role binding","scope":"Owned prohibited/offscreen AppKit profiles and panel renders; deliberately visible own fixture remains offscreen and never key/main; no desktop input, global registration or applied real opens"})).unwrap()).unwrap();
    }
}
fn bridge(start: bool) -> Bridge {
    let settings = Config::current();
    receipt(&settings);
    Bridge::new(settings, start)
}
fn host() -> Bridge {
    let host = bridge(true);
    host.settled(|_| true);
    host
}
fn probe(host: &Bridge, action: &str) -> Value {
    host.probe(action, json!({}))
}
fn capture(host: &Bridge, name: &str) -> Value {
    host.capture(name)
}
#[test]
fn real_main_loop_normal_quit_without_kill_fallback() {
    let mut host = host();
    assert_eq!(
        host.rpc(&json!({"op":"status"}))["result"]["headless"],
        false
    );
    assert_eq!(probe(&host, "read")["windowShown"], false);
    assert_eq!(host.rpc(&json!({"op":"quit"}))["ok"], true);
    let output = host.wait_exit(Duration::from_secs(4), false);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"]["state"],
        "stopped"
    );
    assert!(!host.socket().exists());
}
#[test]
fn nested_native_run_loop_defers_source_reentry_without_losing_delivery() {
    let host = host();
    assert!(
        probe(&host, "source-reentry")["deferredDeliveries"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(host.rpc(&json!({"op":"status"}))["ok"], true);
    assert_eq!(probe(&host, "read")["windowShown"], false);
}
#[test]
fn failed_worker_start_releases_loop_and_profile() {
    let mut host = bridge(false);
    fs::write(&host.config, "broken").unwrap();
    let output = runtime_host::run(
        host.command(&host.settings.host).arg("--native-hidden"),
        Duration::from_secs(4),
    );
    assert!(!output.status.success());
    assert!(!host.socket().exists());
    fs::write(&host.config, serde_json::to_vec(&default_config()).unwrap()).unwrap();
    host.start();
    host.settled(|_| true);
}
#[test]
fn normal_binary_refuses_hidden_test_runtime() {
    let host = bridge(false);
    let normal = normal_host();
    let output = runtime_host::run(
        host.command(&normal).arg("--native-hidden"),
        Duration::from_secs(4),
    );
    assert!(!output.status.success());
    assert!(!host.socket().exists());
}
#[test]
fn normal_binary_refuses_headless_fixture_flag() {
    let host = bridge(false);
    let normal = normal_host();
    let output = runtime_host::run(
        host.command(&normal).arg("--headless-test"),
        Duration::from_secs(4),
    );
    assert!(!output.status.success());
    assert!(!host.socket().exists());
}
#[test]
fn show_search_and_local_native_query_share_the_owner() {
    let host = host();
    assert_eq!(host.rpc(&json!({"op":"show","query":"finder"}))["ok"], true);
    host.settled(|s| s["nativeUI"]["visible"] == true);
    let view = probe(&host, "read");
    assert_eq!(view["windowShown"], false);
    assert_eq!(view["input"], "finder");
    assert_eq!(view["modelQuery"], "finder");
    host.probe("query", json!({"query":"safari"}));
    host.settled(|s| s["nativeUI"]["selectedId"] == "app.safari");
    assert_eq!(probe(&host, "read")["modelQuery"], "safari");
    let cli = host.cli(&["search", "safari"]);
    assert_eq!(cli["ok"], true);
    assert_eq!(cli["result"][0]["id"], "app.safari");
    host.probe("key", json!({"key":"escape"}));
    host.settled(|s| s["nativeUI"]["visible"] == false);
}
#[test]
fn pending_native_query_is_not_clobbered_by_older_snapshot() {
    let host = host();
    host.rpc(&json!({"op":"show","query":"finder"}));
    host.settled(|s| s["nativeUI"]["visible"] == true);
    let reply = host.probe("stale-query", json!({"query":"safari"}));
    assert_eq!(reply["input"], "safari");
    host.settled(|s| s["nativeUI"]["selectedId"] == "app.safari");
    assert_eq!(probe(&host, "read")["input"], "safari");
}
#[test]
fn native_composition_survives_pending_snapshot_and_return() {
    let host = host();
    host.rpc(&json!({"op":"show","query":"finder"}));
    host.settled(|s| s["nativeUI"]["visible"] == true);
    probe(&host, "ime");
    host.settled(|_| true);
    assert_eq!(probe(&host, "read")["windowShown"], false);
}
struct BusyWorker {
    result: mpsc::Receiver<std::io::Result<Value>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl BusyWorker {
    fn start(socket: PathBuf) -> Self {
        let (tx, result) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = runtime_host::status_rpc::request(
                &socket,
                &json!({"op":"native.delay"}),
                Instant::now() + Duration::from_secs(3),
            );
            let _ = tx.send(result);
        });
        Self {
            result,
            worker: Some(worker),
        }
    }
    fn finish(mut self) -> Value {
        let result = self.result.recv_timeout(Duration::from_secs(2)).unwrap();
        self.worker.take().unwrap().join().unwrap();
        result.unwrap()
    }
}
impl Drop for BusyWorker {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            if self.result.recv_timeout(Duration::from_secs(2)).is_ok() {
                let _ = worker.join();
            }
        }
    }
}
#[test]
fn escape_hides_during_busy_owner_and_supersedes_older_show() {
    let host = host();
    host.rpc(&json!({"op":"show","query":"finder"}));
    host.settled(|s| s["nativeUI"]["visible"] == true);
    probe(&host, "schedule-escape");
    let worker = BusyWorker::start(host.socket());
    thread::sleep(Duration::from_millis(30));
    let shown = host.rpc(&json!({"op":"show"}));
    let result = worker.finish();
    assert_eq!(result["result"]["localCloseDuringDelay"], true);
    assert_eq!(shown["ok"], false, "{shown}");
    assert!(shown["error"].as_str().unwrap().contains("superseded"));
    host.settled(|s| s["nativeUI"]["visible"] == false);
    assert_eq!(host.rpc(&json!({"op":"show"}))["ok"], true);
    host.settled(|s| s["nativeUI"]["visible"] == true);
}
#[test]
fn loaded_config_updates_one_live_native_catalog() {
    let host = host();
    host.rpc(&json!({"op":"show","query":"finder"}));
    host.settled(|s| s["nativeUI"]["visible"] == true);
    let mut config = default_config();
    config["commands"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["id"] != "app.finder");
    host.replace(&config, None);
    let status = host.settled(|s| s["commands"] == config["commands"].as_array().unwrap().len());
    assert_ne!(status["nativeUI"]["selectedId"], "app.finder");
    assert_eq!(probe(&host, "read")["windowShown"], false);
    fs::write(&host.config, "broken PRIVATE-SENTINEL").unwrap();
    host.settled(|s| !s["configError"].is_null());
    assert!(!probe(&host, "read")
        .to_string()
        .contains("PRIVATE-SENTINEL"));
    assert_eq!(
        host.rpc(&json!({"op":"status"}))["result"]["commands"],
        config["commands"].as_array().unwrap().len()
    );
    capture(&host, "invalid-config");
}
#[test]
fn watched_theme_reaches_actual_native_appearance() {
    let host = host();
    host.rpc(&json!({"op":"show"}));
    host.settled(|s| s["nativeUI"]["visible"] == true);
    let mut config = default_config();
    assert!(probe(&host, "read")["explicitAppearance"].is_null());
    for theme in [
        Some("dark"),
        Some("light"),
        Some("graphite"),
        Some("system"),
        None,
    ] {
        let revision = host.rpc(&json!({"op":"status"}))["result"]["configurationRevision"]
            .as_u64()
            .unwrap();
        match theme {
            Some(theme) => {
                config["theme"] = json!(theme);
            }
            None => {
                config.as_object_mut().unwrap().remove("theme");
            }
        }
        host.replace(&config, None);
        host.settled(|s| {
            s["configurationRevision"].as_u64().unwrap() > revision
                && s["theme"] == theme.unwrap_or("system")
        });
        let appearance = probe(&host, "read");
        if matches!(theme, Some("system") | None) {
            assert!(appearance["explicitAppearance"].is_null());
        } else {
            assert_eq!(
                appearance["appearance"]
                    .as_str()
                    .unwrap()
                    .contains("DarkAqua"),
                matches!(theme, Some("dark" | "graphite"))
            );
        }
        capture(&host, &format!("theme-{}", theme.unwrap_or("unset")));
    }
}
#[test]
fn rapid_arrows_and_click_on_other_row_keep_intended_identity() {
    let host = host();
    let mut config = default_config();
    config["commands"]=Value::Array((0..21).map(|i|json!({"id":format!("fixture.{i}"),"title":format!("Synthetic note {i}"),"kind":"url","target":format!("https://example.invalid/{i}"),"aliases":[format!("note{i}")],"arguments":[]})).collect());
    host.replace(&config, None);
    host.settled(|s| s["commands"] == 21);
    host.rpc(&json!({"op":"show","query":""}));
    host.settled(|s| s["nativeUI"]["visible"] == true);
    let expected = host.cli(&["search", "note"])["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].clone())
        .collect::<Vec<_>>();
    host.probe("burst", json!({"query":"note","count":5}));
    host.settled(|s| s["nativeUI"]["selectedId"] == expected[5]);
    assert_eq!(probe(&host, "read")["selectedRow"], 5);
    host.probe("click", json!({"row":1}));
    host.settled(|s| s["nativeUI"]["selectedId"] == expected[1]);
    assert_eq!(probe(&host, "read")["selectedRow"], 1);
    assert_eq!(probe(&host, "read")["windowShown"], false);
    host.wait(|s| s["openRequestsRunning"] == 0);
    host.settled(|_| true);
    assert!(probe(&host, "read")
        .to_string()
        .contains("fixture boundary is not installed"));
    capture(&host, "missing-native-boundary");
}
#[test]
fn offscreen_visibility_policy_records_actual_transition_with_passive_reads() {
    let host = host();
    let before = probe(&host, "read");
    assert_eq!(before["windowShown"], false);
    probe(&host, "fixture-force-visible");
    let visible = probe(&host, "read");
    assert_eq!(visible["windowShown"], true);
    assert_eq!(visible["windowKey"], false);
    assert_eq!(visible["windowMain"], false);
    assert_eq!(
        visible["hiddenVisibilityRepairs"],
        before["hiddenVisibilityRepairs"]
    );
    assert_eq!(probe(&host, "read")["windowShown"], true);
    assert_eq!(host.rpc(&json!({"op":"show","query":"notes"}))["ok"], true);
    host.settled(|_| true);
    let after = probe(&host, "read");
    for field in ["windowShown", "windowKey", "windowMain"] {
        assert_eq!(after[field], false);
    }
    assert_eq!(
        after["hiddenVisibilityRepairs"].as_u64().unwrap(),
        before["hiddenVisibilityRepairs"].as_u64().unwrap() + 1
    );
    let repair = &after["lastHiddenVisibilityRepair"];
    assert_eq!(repair["visibleBefore"], true);
    assert_eq!(repair["keyBefore"], false);
    assert_eq!(repair["mainBefore"], false);
    assert!(repair["x"].as_f64().unwrap() < -10000.0);
    assert!(repair["y"].as_f64().unwrap() < -10000.0);
    assert_eq!(repair["phase"], "apply-view");
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let folder = PathBuf::from(directory).parent().unwrap().join("rich");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("hidden-visibility-policy.json"),
            serde_json::to_vec_pretty(&json!({"before":before,"visible":visible,"after":after}))
                .unwrap(),
        )
        .unwrap();
    }
}
