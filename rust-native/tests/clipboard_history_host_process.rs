//! Actual hidden history host with a private complete Rust SDK and named boards.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_bridge.rs"]
mod native_bridge;
use native_bridge::{runtime_host, Bridge};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

// Match unittest sequencing while each host keeps its private SDK and board.
static HOST_SCENARIOS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn host(clipboard: Value) -> Bridge {
    host_options(clipboard, 760, "light")
}
fn host_options(clipboard: Value, width: u64, theme: &str) -> Bridge {
    let mut h = Bridge::new(runtime_host::Config::current(), false);
    let mut config = runtime_host::default_config();
    config["theme"] = json!(theme);
    for (key, value) in clipboard.as_object().unwrap() {
        config["clipboard"][key] = value.clone();
    }
    h.replace(&config, None);
    let artifact = PathBuf::from(
        std::env::var_os("RUYCAST_TEST_HISTORY_SDK")
            .expect("Set the actual complete Rust clipboard history SDK"),
    );
    assert!(artifact.is_absolute() && artifact.is_file());
    let private = h.base.join("clipboard-history-effects.dylib");
    fs::copy(&artifact, &private).unwrap();
    assert_eq!(
        Sha256::digest(fs::read(&artifact).unwrap()),
        Sha256::digest(fs::read(&private).unwrap())
    );
    for (key, value) in [
        ("DYLD_INSERT_LIBRARIES", private.clone().into_os_string()),
        (
            "RUYCAST_WORKSPACE_FIXTURE_HOME",
            h.base.clone().into_os_string(),
        ),
        ("RUYCAST_NATIVE_TEST_CLIPBOARD", "named".into()),
        ("RUYCAST_NATIVE_TEST_SNIPPET", "1".into()),
        ("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD", "1".into()),
        ("RUYCAST_NATIVE_TEST_WIDTH", width.to_string().into()),
    ] {
        h.env.insert(key.into(), value);
    }
    binding(&h, &artifact, &private);
    h.start();
    h.settled(|_| true);
    h
}
fn fixture(h: &Bridge, text: &str) {
    fixture_payload(h, json!({"Text":text}));
}
fn fixture_payload(h: &Bridge, payload: Value) {
    fixture_fields(h, payload, json!({}));
}
fn fixture_fields(h: &Bridge, payload: Value, mut fields: Value) {
    fields["payload"] = payload;
    fields["defer"] = json!(true);
    let value = h.probe("clipboard-fixture", fields);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(value["namedBoard"], true);
}
// Exact bytes from the original synthetic 2×2 RGBA PNG helper.
fn png() -> &'static [u8] {
    &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6,
        0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 104, 112, 80, 248,
        15, 194, 12, 48, 6, 0, 64, 180, 7, 125, 193, 185, 247, 76, 0, 0, 0, 0, 73, 69, 78, 68, 174,
        66, 96, 130,
    ]
}
fn items(h: &Bridge) -> Value {
    h.rpc(&json!({"op":"clipboard.list"}))["result"].clone()
}

#[test]
fn test_pause_disable_and_source_exclusion() {
    let _serial = HOST_SCENARIOS.lock().unwrap();
    // Keep all three hosts alive until the scenario ends, as unittest cleanup did.
    let mut hosts = Vec::new();
    for config in [
        json!({"enabled":false}),
        json!({"excludedApps":["fixture"]}),
    ] {
        let h = host(config);
        fixture(&h, "not captured");
        assert_eq!(items(&h), json!([]));
        hosts.push(h);
    }
    let h = host(json!({}));
    assert_eq!(
        h.rpc(&json!({"op":"clipboard.pause","paused":true}))["ok"],
        true
    );
    fixture(&h, "paused");
    assert_eq!(items(&h), json!([]));
    assert_eq!(
        h.rpc(&json!({"op":"clipboard.pause","paused":false}))["ok"],
        true
    );
    fixture(&h, "resumed");
    h.wait(|s| s["clipboardCount"] == 1);
    hosts.push(h);
}

#[test]
fn test_atomic_import_dedup_and_external_edit_capture_base() {
    let _serial = HOST_SCENARIOS.lock().unwrap();
    let h = host(json!({}));
    fixture(&h, "first");
    h.wait(|s| s["clipboardCount"] == 1);
    let current = items(&h);
    let mut external = current.as_array().unwrap().clone();
    external.push(json!({"id":"external", "text":"External owned edit", "source":"fixture", "date":current[0]["date"]}));
    let file = h.profile.join("clipboard.json");
    fs::write(&file, serde_json::to_vec(&external).unwrap()).unwrap();
    fixture(&h, "second");
    h.wait(|s| s["clipboardCount"] == 3);
    assert!(items(&h)
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "external"));
    let before = fs::read(&file).unwrap();
    let bad = json!([
        {"id":"raycast-valid", "text":"valid", "source":"fixture", "date":current[0]["date"]},
        {"id":"invalid", "text":"bad", "source":"fixture", "date":current[0]["date"]}
    ]);
    assert_eq!(
        h.rpc(&json!({"op":"clipboard.import", "items":bad}))["ok"],
        false
    );
    assert_eq!(fs::read(&file).unwrap(), before);
}

#[test]
fn test_rich_copy_routes_and_invalid_data_do_not_clear_destination() {
    let _serial = HOST_SCENARIOS.lock().unwrap();
    let h = host(json!({}));
    fixture_payload(
        &h,
        json!({"Files":["file:///synthetic/a.txt", "file:///synthetic/b.txt"]}),
    );
    h.wait(|s| s["clipboardCount"] == 1);
    let files = items(&h);
    assert_eq!(
        h.cli(&["clipboard", "copy", files[0]["id"].as_str().unwrap()])["ok"],
        true
    );
    fixture_payload(&h, json!({"Image":png()}));
    h.wait(|s| s["clipboardCount"] == 2);
    let image = items(&h)[0].clone();
    assert_eq!(
        h.cli(&["clipboard", "copy", image["id"].as_str().unwrap()])["ok"],
        true
    );
    let trace = h.base.join("workspace-trace.jsonl");
    let before = fs::read_to_string(&trace)
        .unwrap()
        .matches("snippet-clear")
        .count();
    fs::write(
        h.profile.join(image["dataFile"].as_str().unwrap()),
        b"invalid image",
    )
    .unwrap();
    let result = h.rpc(&json!({"op":"clipboard.copy", "id":image["id"]}));
    assert_eq!(result["ok"], false);
    assert_eq!(
        fs::read_to_string(&trace)
            .unwrap()
            .matches("snippet-clear")
            .count(),
        before
    );
}

#[test]
fn test_capture_text_files_image_and_private_own_activation_filters() {
    let _serial = HOST_SCENARIOS.lock().unwrap();
    let mut h = host(json!({}));
    fixture(&h, "Synthetic Å\nSecond line");
    h.wait(|s| s["clipboardCount"] == 1);
    assert_eq!(items(&h)[0]["text"], "Synthetic Å\nSecond line");
    for kind in [
        "org.nspasteboard.ConcealedType",
        "org.nspasteboard.TransientType",
        "org.nspasteboard.AutoGeneratedType",
    ] {
        fixture_fields(
            &h,
            json!({"Text":format!("private {kind}")}),
            json!({"private":kind}),
        );
        assert_eq!(items(&h).as_array().unwrap().len(), 1);
    }
    fixture_fields(&h, json!({"Text":"own"}), json!({"own":true}));
    assert_eq!(items(&h).as_array().unwrap().len(), 1);
    fixture_fields(
        &h,
        json!({"Text":"uncertain source"}),
        json!({"activation":true}),
    );
    assert_eq!(items(&h).as_array().unwrap().len(), 1);
    fixture_payload(
        &h,
        json!({"Files":["file:///synthetic/missing%20file.txt"]}),
    );
    h.wait(|s| s["clipboardCount"] == 2);
    assert_eq!(
        items(&h)[0]["fileURLs"],
        json!(["file:///synthetic/missing%20file.txt"])
    );
    fixture_payload(&h, json!({"Image":png()}));
    h.wait(|s| s["clipboardCount"] == 3);
    let image = items(&h)[0].clone();
    assert_eq!(image["contentType"], "image");
    assert!(
        fs::read(h.profile.join(image["dataFile"].as_str().unwrap()))
            .unwrap()
            .starts_with(b"\x89PNG")
    );
    assert_eq!(h.rpc(&json!({"op":"quit"}))["ok"], true);
    let deadline = Instant::now() + Duration::from_secs(4);
    let output = h.wait_exit(Duration::from_secs(4), false);
    assert!(
        Instant::now() < deadline,
        "Quit and output drains exceeded original four-second deadline"
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"]["state"],
        "stopped"
    );
    assert!(!h.profile.join("control.sock").exists());
}

#[test]
fn test_cli_import_export_search_copy_paste_delete_clear() {
    let _serial = HOST_SCENARIOS.lock().unwrap();
    let h = host(json!({}));
    let values = json!([{"id":"raycast-clip", "text":"Synthetic CLI Å", "date":chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()).format("%Y-%m-%dT%H:%M:%SZ").to_string(), "source":"fixture"}]);
    let file = h.base.join("import.json");
    fs::write(&file, serde_json::to_vec(&values).unwrap()).unwrap();
    assert_eq!(
        h.cli(&["clipboard", "import", &format!("@{}", file.display())])["result"]["accepted"],
        1
    );
    assert_eq!(
        h.cli(&["clipboard", "list", "cli å"])["result"][0]["id"],
        "raycast-clip"
    );
    assert_eq!(
        h.cli(&["clipboard", "export"])["result"],
        h.cli(&["clipboard", "list"])["result"]
    );
    assert_eq!(
        h.cli(&["clipboard", "copy", "raycast-clip"])["result"]["state"],
        "copied"
    );
    thread::sleep(Duration::from_millis(500));
    assert_eq!(
        items(&h)
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("raycast-clip")]
    );
    assert_eq!(h.rpc(&json!({"op":"show", "mode":"clipboard"}))["ok"], true);
    h.settled(|s| s["nativeUI"]["visible"] == true);
    assert_eq!(
        h.cli(&["clipboard", "paste", "raycast-clip"])["result"]["state"],
        "copied"
    );
    h.wait(|s| s["snippetRequestsRunning"] == 0);
    let trace = fs::read_to_string(h.base.join("workspace-trace.jsonl")).unwrap();
    assert!(trace.contains("snippet-post"));
    assert!(!trace.contains("unexpected-general"));
    assert_eq!(
        h.cli(&["clipboard", "delete", "raycast-clip"])["result"]["count"],
        0
    );
    assert_eq!(h.cli(&["clipboard", "clear"])["result"]["count"], 0);
}

fn settings_probe(h: &Bridge, action: &str, mut fields: Value) -> Value {
    fields["op"] = json!("native.test");
    fields["surface"] = json!("settings");
    fields["action"] = json!(action);
    let response = h.rpc(&fields);
    assert_eq!(response["ok"], true, "{response}");
    response["result"].clone()
}
#[test]
fn test_native_preferences_save_is_revision_guarded_and_validated() {
    let _serial = HOST_SCENARIOS.lock().unwrap();
    let h = host(json!({}));
    assert_eq!(h.rpc(&json!({"op":"show", "mode":"settings"}))["ok"], true);
    h.wait(|s| {
        s["managementUI"]["visible"] == true
            && s["managementUI"]["revision"] == s["managementUI"]["appliedRevision"]
    });
    settings_probe(&h, "clipboard-preferences", json!({}));
    h.wait(|s| s["managementUI"]["tab"] == "clipboard");
    thread::sleep(Duration::from_millis(50));
    let view = settings_probe(&h, "screenshot", json!({}));
    let output = runtime_host::root().join("rust-native/target/clipboard-evidence");
    fs::create_dir_all(&output).unwrap();
    fs::copy(
        h.profile.join("native-settings-probe.png"),
        output.join("preferences-light.png"),
    )
    .unwrap();
    fs::write(
        output.join("preferences-light.json"),
        serde_json::to_vec_pretty(&view).unwrap(),
    )
    .unwrap();
    let before: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    settings_probe(&h, "clipboard-preferences-save", json!({"limit":0}));
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&h.config).unwrap()).unwrap(),
        before
    );
    settings_probe(
        &h,
        "clipboard-preferences-save",
        json!({"limit":12, "retentionDays":7, "excludedApps":["dev.fixture.private"]}),
    );
    h.wait(|s| {
        s["configurationRevision"].as_u64().unwrap() > 0 && s["configurationUpdating"] == false
    });
    let actual: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    assert_eq!(actual["clipboard"]["limit"], 12);
    assert_eq!(actual["clipboard"]["retentionDays"], 7);
    assert_eq!(
        actual["clipboard"]["excludedApps"],
        json!(["dev.fixture.private"])
    );
}

const SOURCES: &[&str] = &[
    "rust-native/tests/clipboard_history_host_process.rs",
    "rust-native/tests/support/runtime_host.rs",
    "rust-native/tests/support/native_bridge.rs",
    "rust-native/tests/support/status_rpc.rs",
    "rust-native/tests/support/child_output.rs",
    "rust-native/tests/fixtures/workspace_sdk.rs",
    "rust-native/tests/fixtures/snippet_effects_sdk.rs",
    "rust-native/tests/fixtures/clipboard_history_sdk.rs",
    "rust-native/tests/fixtures/clipboard_history_fixture.rs",
    "rust-native/Cargo.toml",
    "rust-native/Cargo.lock",
    "rust/Cargo.toml",
    "rust/Cargo.lock",
    "rust-toolchain.toml",
    "rust-native/src/runtime.rs",
    "config/default.json",
];
fn binding(h: &Bridge, original: &std::path::Path, private: &std::path::Path) {
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        let sources = SOURCES
            .iter()
            .map(|name| {
                (
                    *name,
                    format!(
                        "{:x}",
                        Sha256::digest(fs::read(runtime_host::root().join(name)).unwrap())
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let loaded = format!("{:x}", Sha256::digest(fs::read(private).unwrap()));
        let mut artifacts = BTreeMap::new();
        let mut roles = BTreeMap::new();
        for (role, path) in [
            ("host", h.settings.host.as_path()),
            ("cli", h.settings.cli.as_path()),
            ("sdk", original),
        ] {
            let (path, sha) = runtime_host::bind_artifact(path, role);
            if role == "sdk" {
                assert_eq!(sha, loaded);
            }
            roles.insert(role, path.clone());
            artifacts.insert(path, sha);
        }
        fs::write(directory.join("clipboard-history-bindings.json"), serde_json::to_vec_pretty(&json!({
            "schema":"ruycast.clipboard-history-bindings.v1", "sources":sources, "artifacts":artifacts,
            "artifactRoles":roles, "sdkCopyHashChecked":true,
            "retiredPythonSource":{"path":"rust-native/tests/clipboard_history_process.py", "sha256":"72798c56f799ebc73893de1e53345ead4d2bc82125e1a76be47bd0b529752066", "role":"historical-original-case-source-only"},
            "scope":"Owned hidden/offscreen host and named boards; no general clipboard, desktop capture, activation or physical input"
        })).unwrap()).unwrap();
    }
}
fn capture(h: &Bridge, name: &str) -> Value {
    h.settled(|_| true);
    let value = h.probe("screenshot", json!({}));
    for field in ["windowShown", "windowKey", "windowMain"] {
        assert_eq!(value[field], false);
    }
    let output = runtime_host::root().join("rust-native/target/clipboard-evidence");
    fs::create_dir_all(&output).unwrap();
    fs::copy(
        h.profile.join("native-probe.png"),
        output.join(format!("{name}.png")),
    )
    .unwrap();
    fs::write(
        output.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
    value
}
struct PreviewHold(PathBuf);
impl Drop for PreviewHold {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
#[test]
fn test_native_text_image_empty_error_busy_light_dark_narrow_wide() {
    let _serial = HOST_SCENARIOS.lock().unwrap();
    let mut hosts = Vec::new();
    for (width, theme) in [(600, "light"), (760, "dark")] {
        let h = host_options(json!({}), width, theme);
        assert_eq!(h.rpc(&json!({"op":"show", "mode":"clipboard"}))["ok"], true);
        h.settled(|s| s["nativeUI"]["visible"] == true);
        capture(&h, &format!("empty-{width}-{theme}"));
        fixture(&h, "Synthetic clipboard text\nOne deliberate second line.");
        h.wait(|s| s["clipboardCount"] == 1);
        let text = capture(&h, &format!("text-{width}-{theme}"));
        let screen = text["screenText"].as_array().unwrap();
        assert!(screen.contains(&json!(
            "Synthetic clipboard text\nOne deliberate second line."
        )));
        assert!(!screen.contains(&json!("Clipboard is empty")));
        assert!(screen.contains(&json!("Search clipboard")));
        let hold = PreviewHold(h.profile.join("clipboard-preview-hold"));
        fs::write(&hold.0, "own fixture").unwrap();
        fixture_payload(&h, json!({"Image":png()}));
        h.wait(|s| s["clipboardCount"] == 2);
        h.probe("query", json!({"query":"Image"}));
        h.settled(|_| true);
        let end = Instant::now() + Duration::from_secs(3);
        let entered = h.profile.join("clipboard-preview-entered");
        while !entered.exists() && Instant::now() < end {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(entered.exists());
        let busy = capture(&h, &format!("busy-{width}-{theme}"));
        assert!(busy["screenText"]
            .as_array()
            .unwrap()
            .contains(&json!("Loading preview…")));
        fs::remove_file(&hold.0).unwrap();
        thread::sleep(Duration::from_millis(150));
        capture(&h, &format!("image-{width}-{theme}"));
        let image = items(&h)[0].clone();
        fs::remove_file(h.profile.join(image["dataFile"].as_str().unwrap())).unwrap();
        h.probe("query", json!({"query":"no-match"}));
        h.settled(|_| true);
        h.probe("query", json!({"query":"Image"}));
        h.settled(|_| true);
        thread::sleep(Duration::from_millis(150));
        let error = capture(&h, &format!("error-{width}-{theme}"));
        assert!(error["screenText"].as_array().unwrap().iter().any(|t| t
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("unavailable")));
        hosts.push(h);
    }
}
