//! Actual hidden history host with a private complete Rust SDK and named boards.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_bridge.rs"]
mod native_bridge;
use native_bridge::{runtime_host, Bridge};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, thread, time::Duration};

// Match unittest sequencing while each host keeps its private SDK and board.
static HOST_SCENARIOS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn host(clipboard: Value) -> Bridge {
    let mut h = Bridge::new(runtime_host::Config::current(), false);
    let mut config = runtime_host::default_config();
    config["theme"] = json!("light");
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
        ("DYLD_INSERT_LIBRARIES", private.into_os_string()),
        (
            "RUYCAST_WORKSPACE_FIXTURE_HOME",
            h.base.clone().into_os_string(),
        ),
        ("RUYCAST_NATIVE_TEST_CLIPBOARD", "named".into()),
        ("RUYCAST_NATIVE_TEST_SNIPPET", "1".into()),
        ("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD", "1".into()),
        ("RUYCAST_NATIVE_TEST_WIDTH", "760".into()),
    ] {
        h.env.insert(key.into(), value);
    }
    h.start();
    h.settled(|_| true);
    h
}
fn fixture(h: &Bridge, text: &str) {
    fixture_payload(h, json!({"Text":text}));
}
fn fixture_payload(h: &Bridge, payload: Value) {
    let value = h.probe(
        "clipboard-fixture",
        json!({"payload":payload, "defer":true}),
    );
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
