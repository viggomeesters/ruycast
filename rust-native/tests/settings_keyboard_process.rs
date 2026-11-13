//! General Settings recorder through the complete owned keyboard SDK.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_keyboard.rs"]
mod native_keyboard;
use native_keyboard::{actions, actions::native_bridge::Bridge, emit, host_for_role, trace};
use serde_json::{json, Value};
use std::{
    thread,
    time::{Duration, Instant},
};

fn host() -> Bridge {
    host_for_role(&[], true, true, true, None, "GeneralKeyboard")
}
fn probe(host: &Bridge, action: &str, fields: Value) -> Value {
    let mut request = fields.as_object().unwrap().clone();
    request.insert("op".into(), json!("native.test"));
    request.insert("surface".into(), json!("settings"));
    request.insert("action".into(), json!(action));
    let value = host.rpc(&Value::Object(request));
    assert_eq!(value["ok"], true, "{value}");
    value["result"].clone()
}
fn wait(host: &Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        let remaining = until.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "Settings recorder did not reach its expected state"
        );
        let response = host.rpc_timeout(
            &json!({"op":"native.test","surface":"settings","action":"settings-read"}),
            remaining.min(Duration::from_secs(3)),
        );
        assert_eq!(response["ok"], true, "{response}");
        let value = response["result"].clone();
        if predicate(&value) {
            return value;
        }
        thread::sleep(
            Duration::from_millis(10).min(until.saturating_duration_since(Instant::now())),
        );
    }
}
fn general(host: &Bridge) {
    host.rpc(&json!({"op":"show","mode":"settings"}));
    host.settled(|_| true);
    probe(host, "library-tab", json!({"tab":"general"}));
    wait(host, |v| v["visible"] == true);
}
#[test]
fn general_recorder_uses_shared_lease_and_cannot_resume_peer_recorder() {
    let h = host();
    h.rpc(&json!({"op":"show","query":"finder"}));
    h.settled(|_| true);
    actions::hotkey(&h);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["recordingShortcut"],
        true
    );
    general(&h);
    probe(&h, "settings-record", json!({"index":0}));
    wait(&h, |v| v["recording"][0] == true);
    assert_eq!(emit(&h, 49, 6144, json!({})), json!([-9874]));
    probe(&h, "key-event", json!({"code":53,"flags":0,"chars":""}));
    wait(&h, |v| {
        v["recording"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x == false)
    });
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["recordingShortcut"],
        true
    );
    assert_eq!(emit(&h, 49, 6144, json!({})), json!([-9874]));
    actions::probe(&h, "cancel-editor");
    h.settled(|s| s["recordingShortcut"] == false);
    assert_eq!(emit(&h, 49, 6144, json!({})), json!([0]));
}
#[test]
fn general_recorded_hotkey_replaces_physical_plan_through_config_staging() {
    let h = host();
    general(&h);
    let before = h.rpc(&json!({"op":"status"}))["result"]["keyboardGeneration"]
        .as_u64()
        .unwrap();
    probe(&h, "settings-record", json!({"index":0}));
    wait(&h, |v| v["recording"][0] == true);
    probe(
        &h,
        "key-event",
        json!({"code":0,"flags":actions::CMD|actions::OPT,"chars":"a"}),
    );
    wait(&h, |v| {
        v["surface"]["configuration"]["launcherHotkey"] == "alt+cmd+a"
    });
    h.settled(|s| {
        s["keyboardGeneration"].as_u64().unwrap() > before && s["recordingShortcut"] == false
    });
    assert_eq!(emit(&h, 49, 6144, json!({})), json!([-9874]));
    assert_eq!(emit(&h, 0, 2304, json!({})), json!([0]));
    h.settled(|s| s["nativeUI"]["visible"] == true);
    let events = trace(&h);
    assert!(events.iter().any(|v| v["phase"] == "unregister"));
    assert!(events
        .iter()
        .any(|v| v["phase"] == "register" && v["a"] == 0 && v["b"] == 2304));
}
