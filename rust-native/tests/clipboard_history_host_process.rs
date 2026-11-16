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
    let value = h.probe(
        "clipboard-fixture",
        json!({"payload":{"Text":text}, "defer":true}),
    );
    thread::sleep(Duration::from_millis(500));
    assert_eq!(value["namedBoard"], true);
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
