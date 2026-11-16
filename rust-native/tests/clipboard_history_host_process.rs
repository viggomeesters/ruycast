//! Actual hidden history host with a private complete Rust SDK and named boards.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_bridge.rs"]
mod native_bridge;
use native_bridge::{runtime_host, Bridge};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, thread, time::Duration};

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
