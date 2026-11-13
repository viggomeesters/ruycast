//! Real offscreen native activation with the owned synthetic Workspace SDK.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::{
    native_bridge::{
        runtime_host::{default_config, Config},
        Bridge,
    },
    *,
};
use serde_json::{json, Value};
use std::{
    fs,
    time::{Duration, Instant},
};

fn host(command: Value) -> Bridge {
    host_with_apps(command, false, false, None)
}
fn host_with_picker(command: Value, picker: bool) -> Bridge {
    host_with_apps(command, picker, false, None)
}
fn host_with_apps(
    command: Value,
    picker: bool,
    discovered: bool,
    clipboard: Option<&str>,
) -> Bridge {
    let settings = Config::current();
    let shim = std::env::var_os("RUYCAST_TEST_WORKSPACE_SHIM")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            settings
                .host
                .parent()
                .unwrap()
                .join("examples/libworkspace_fixture.dylib")
        });
    assert!(shim.is_absolute() && shim.is_file());
    let mut h = Bridge::new(settings, false);
    if picker {
        let contents = h.applications.join("Picker.app/Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("Info.plist"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.fixture.success</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleName</key><string>Fixture Selected Browser</string></dict></plist>").unwrap();
    }
    if discovered {
        let contents = h.applications.join("Fixture.app/Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("Info.plist"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.fixture.discovered</string><key>CFBundleName</key><string>Discovered Native Fixture</string></dict></plist>").unwrap();
    }
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(command);
    h.replace(&cfg, None);
    h.env
        .insert("RUYCAST_NATIVE_TEST_WIDTH".into(), "760".into());
    h.env
        .insert("DYLD_INSERT_LIBRARIES".into(), shim.into_os_string());
    let fixture_home = h.base.as_os_str().to_owned();
    h.env
        .insert("RUYCAST_WORKSPACE_FIXTURE_HOME".into(), fixture_home);
    h.env
        .insert("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD".into(), "trace".into());
    h.env
        .insert("RUYCAST_NATIVE_TEST_REQUIRE_HIDE".into(), "1".into());
    if let Some(value) = clipboard {
        h.env
            .insert("RUYCAST_NATIVE_TEST_CLIPBOARD".into(), "named".into());
        h.env
            .insert("RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT".into(), value.into());
    }
    h.start();
    h.settled(|_| true);
    if picker || discovered {
        h.wait(|s| s["appDiscoveryPending"] == false);
    }
    h.env.remove(std::ffi::OsStr::new("DYLD_INSERT_LIBRARIES"));
    h.env
        .remove(std::ffi::OsStr::new("RUYCAST_WORKSPACE_FIXTURE_HOME"));
    h
}
#[test]
fn native_picker_save_then_run_uses_selected_bundle_with_exact_hide() {
    let h = host_with_picker(command("url", "https://example.invalid/picker"), true);
    show(&h, "nativeopen");
    action(&h, "Edit");
    view(&h, |v| v["formVisible"] == true);
    h.settled(|_| true);
    h.probe("app-picker", json!({"command":"open"}));
    h.probe("app-picker", json!({"command":"click","index":1}));
    probe(&h, "save-editor");
    saved(&h);
    let cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    assert_eq!(
        cfg["commands"].as_array().unwrap().last().unwrap()["applicationID"],
        "dev.fixture.success"
    );
    activate(&h);
    let trace = rows(&h.base.join("workspace-trace.jsonl"));
    let opens = trace
        .iter()
        .filter(|r| r["phase"].as_str().unwrap().starts_with("open-"))
        .collect::<Vec<_>>();
    assert_eq!(
        opens.iter().map(|r| r["phase"].clone()).collect::<Vec<_>>(),
        vec![json!("open-with-targets"), json!("open-with")]
    );
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
    assert_eq!(
        trace
            .iter()
            .find(|r| r["phase"] == "open-with-targets")
            .unwrap()["urls"],
        json!(["https://example.invalid/picker"])
    );
    assert!(trace
        .iter()
        .any(|r| r["phase"] == "resolve" && r["bundleID"] == "dev.fixture.success"));
}
fn command(kind: &str, target: &str) -> Value {
    json!({"id":"fixture.open","title":"Native Open Fixture","kind":kind,"target":target,"aliases":["nativeopen"]})
}
fn rows(path: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn activate(h: &Bridge) {
    show(h, "nativeopen");
    key(h, 36, 0, "\r", false);
    done(h);
}
struct Release(std::path::PathBuf);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::write(&self.0, "release");
    }
}
#[test]
fn new_show_and_query_revoke_blocked_old_lookup() {
    for action_name in ["show", "query"] {
        let h = host(command("app", "dev.fixture.hold-resolve"));
        let gate = h.base.join("workspace-release");
        let _release = Release(gate.clone());
        show(&h, "nativeopen");
        key(&h, 36, 0, "\r", false);
        let marker = h.base.join("resolve-entered");
        let end = Instant::now() + Duration::from_secs(4);
        while !marker.exists() {
            assert!(Instant::now() < end, "own fixture did not enter resolve");
            std::thread::sleep(Duration::from_millis(10));
        }
        if action_name == "show" {
            show(&h, "nativeopen");
        } else {
            h.probe("query", json!({"query":"new-intent"}));
        }
        fs::write(gate, "release").unwrap();
        h.wait(|s| s["openRequestsRunning"] == 0 && s["nativeUI"]["executionRunning"] != true);
        assert!(rows(&h.base.join("workspace-trace.jsonl"))
            .iter()
            .all(|r| !r["phase"].as_str().unwrap().starts_with("open-")));
        assert!(rows(&h.profile.join("native-hide.jsonl")).is_empty());
        assert!(h.cli(&["usage"])["result"].get("fixture.open").is_none());
    }
}
fn done(h: &Bridge) {
    let marker = h.base.join("open-started");
    let end = Instant::now() + Duration::from_secs(4);
    while !marker.exists() {
        assert!(
            Instant::now() < end,
            "own fixture did not enter open-started"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    h.wait(|s| s["openRequestsRunning"] == 0 && s["nativeUI"]["executionRunning"] != true);
}
#[test]
fn discovered_app_is_not_required_to_be_saved_for_usage() {
    let h = host_with_apps(command("app", "dev.fixture.inline"), false, true, None);
    show(&h, "Discovered Native Fixture");
    key(&h, 36, 0, "\r", false);
    done(&h);
    assert_eq!(
        h.cli(&["usage"])["result"]["discovered.dev.fixture.discovered"],
        1
    );
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
}
#[test]
fn url_clipboard_and_literal_path_share_native_admission() {
    let h = host_with_apps(
        command("url", "https://example.invalid/{clipboard}"),
        false,
        false,
        Some("synthetic-Ω"),
    );
    activate(&h);
    let trace = rows(&h.base.join("workspace-trace.jsonl"));
    assert_eq!(
        trace
            .iter()
            .find(|r| r["phase"] == "clipboard-read")
            .unwrap()["mainThread"],
        true
    );
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
    fs::remove_file(h.base.join("open-started")).unwrap();
    let path = h.base.join("own literal Ω");
    fs::write(&path, "own").unwrap();
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    *cfg["commands"].as_array_mut().unwrap().last_mut().unwrap() =
        command("path", path.to_str().unwrap());
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    activate(&h);
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 2);
    assert_eq!(h.cli(&["usage"])["result"]["fixture.open"], 2);
}
#[test]
fn new_quicklink_prefill_uses_real_named_main_service_without_execution() {
    for (text, expected, name) in [
        (
            "  https://example.invalid/new  ",
            "https://example.invalid/new",
            "value",
        ),
        ("   ", "https://", "empty"),
    ] {
        let h = host_with_apps(
            command("app", "dev.fixture.inline"),
            false,
            false,
            Some(text),
        );
        show(&h, "nativeopen");
        let before = fs::read(&h.config).unwrap();
        action(&h, "New Quicklink");
        view(&h, |v| v["formVisible"] == true);
        let value = h.probe("command-read", json!({}))["value"].clone();
        assert!(value["id"].as_str().unwrap().starts_with("url."));
        assert_eq!(value["target"], expected);
        assert_eq!(
            fs::read_to_string(h.base.join("creation-prefill-read")).unwrap(),
            "own-named-main-thread\n"
        );
        assert_eq!(fs::read(&h.config).unwrap(), before);
        assert!(rows(&h.base.join("workspace-trace.jsonl"))
            .iter()
            .all(|r| !r["phase"].as_str().unwrap().starts_with("open-")));
        assert!(rows(&h.profile.join("native-hide.jsonl")).is_empty());
        assert_eq!(h.cli(&["usage"])["result"], json!({}));
        let screenshot = h.probe("screenshot", json!({}));
        if let Some(out) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
            let path = std::path::PathBuf::from(out);
            let directory = path.parent().unwrap().join("native-open");
            fs::create_dir_all(&directory).unwrap();
            fs::copy(
                h.profile.join("native-probe.png"),
                directory.join(format!("creation-prefill-{name}.png")),
            )
            .unwrap();
            fs::write(
                directory.join(format!("creation-prefill-{name}.json")),
                serde_json::to_vec_pretty(&screenshot).unwrap(),
            )
            .unwrap();
        }
        probe(&h, "cancel-editor");
        saved(&h);
    }
}
#[test]
fn existing_prompt_input_is_validated_and_executes_without_config_write() {
    let mut cmd = command("app", "dev.fixture.inline");
    cmd["inputPrompt"] = json!("Label");
    let h = host(cmd);
    let before = fs::read(&h.config).unwrap();
    show(&h, "nativeopen");
    key(&h, 36, 0, "\r", false);
    h.settled(|_| true);
    assert_eq!(probe(&h, "read")["runInputInline"], true);
    key(&h, 36, CMD, "\r", false);
    h.settled(|_| true);
    h.probe("run-input-type", json!({"text":"own prompt value"}));
    key(&h, 36, CMD, "\r", false);
    done(&h);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
}
#[test]
fn open_with_uses_exact_hide_before_its_native_callback() {
    let mut cmd = command("url", "https://example.invalid/own");
    cmd["applicationID"] = json!("dev.fixture.inline");
    let h = host(cmd);
    activate(&h);
    assert!(rows(&h.base.join("workspace-trace.jsonl"))
        .iter()
        .any(|r| r["phase"] == "open-with"));
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
}
#[test]
fn saved_app_hides_exact_panel_before_sdk_and_records_success() {
    let h = host(command("app", "dev.fixture.inline"));
    activate(&h);
    let hides = rows(&h.profile.join("native-hide.jsonl"));
    assert_eq!(hides.len(), 1);
    assert_eq!(hides[0]["mainThread"], true);
    assert_eq!(hides[0]["afterVisible"], false);
    assert_eq!(probe(&h, "read")["windowShown"], false);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        false
    );
    assert_eq!(h.cli(&["usage"])["result"]["fixture.open"], 1);
    assert!(rows(&h.base.join("workspace-trace.jsonl"))
        .iter()
        .filter(|r| r["phase"].as_str().unwrap().starts_with("open-"))
        .all(|r| r["mainThread"] == false));
}
