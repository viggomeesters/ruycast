//! Actual hidden AppKit keyword-choice host with owned keyboard and clipboard SDKs.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_keyboard.rs"]
mod native_keyboard;
use native_keyboard::{
    actions::native_bridge::{runtime_host::default_config, Bridge},
    host,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn choice_host() -> Bridge {
    let mut h = host(&[], false, true, true, None);
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_ruycast-native"));
    let artifact = std::env::var_os("RUYCAST_TEST_CHOICE_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            executable
                .parent()
                .unwrap()
                .join("examples/libchoice_fixture.dylib")
        });
    assert!(
        artifact.is_absolute() && artifact.is_file(),
        "Build choice-fixture first: {artifact:?}"
    );
    let guard = h.base.join("choice-guard.dylib");
    fs::copy(&artifact, &guard).unwrap();
    assert_eq!(
        Sha256::digest(fs::read(&artifact).unwrap()),
        Sha256::digest(fs::read(&guard).unwrap())
    );
    if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let out = PathBuf::from(folder);
        fs::create_dir_all(&out).unwrap();
        let copy = out.join("native-choice-guard.dylib");
        fs::copy(&guard, &copy).unwrap();
        let root = native_keyboard::actions::native_bridge::runtime_host::root();
        let names = [
            "rust-native/tests/fixtures/choice_board_sdk.rs",
            "rust-native/tests/fixtures/choice_monitor_sdk.rs",
            "rust-native/tests/fixtures/choice_fixture.rs",
            "rust-native/Cargo.toml",
            "rust-native/Cargo.lock",
            "rust/Cargo.toml",
            "rust-toolchain.toml",
        ];
        let sources = names
            .into_iter()
            .map(|name| {
                (
                    name.to_owned(),
                    format!("{:x}", Sha256::digest(fs::read(root.join(name)).unwrap())),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let name = copy
            .strip_prefix(&root)
            .unwrap_or(&copy)
            .to_string_lossy()
            .into_owned();
        fs::write(out.join("native-choice-bindings.json"),serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":{name:format!("{:x}",Sha256::digest(fs::read(&copy).unwrap()))}})).unwrap()).unwrap();
    }
    let keyboard = h
        .env
        .get(std::ffi::OsStr::new("DYLD_INSERT_LIBRARIES"))
        .unwrap()
        .clone();
    let paths = std::env::split_paths(&keyboard).chain(std::iter::once(guard));
    h.env.insert(
        "DYLD_INSERT_LIBRARIES".into(),
        std::env::join_paths(paths).unwrap(),
    );
    let mut config = default_config();
    config["commands"] = json!([{ "id":"own.snippet", "title":"Own snippet", "kind":"snippet", "target":"Hello {field name=\"name\" default=\"World\"}{field name=\"kind\" options=\"!,?\" default=\"!\"}", "keyword":"abc" }]);
    config["snippetExpansionEnabled"] = json!(true);
    h.replace(&config, None);
    h.start();
    h.settled(|s| s["snippetExpansion"] == "On");
    kw(&h, "text", json!({"value":"abc"}));
    h
}
fn kw(h: &Bridge, control: &str, fields: Value) -> Value {
    let mut fields = fields.as_object().unwrap().clone();
    fields.insert("control".into(), json!(control));
    h.probe("keyboard-keyword", Value::Object(fields))
}
fn typed(h: &Bridge, text: &str) -> Vec<Value> {
    text.chars()
        .map(|c| kw(h, "event", json!({"text":c.to_string()}))["consumed"].clone())
        .collect()
}
fn choice(h: &Bridge, action: &str, fields: Value) -> Value {
    let mut fields = fields.as_object().unwrap().clone();
    fields.insert("choiceAction".into(), json!(action));
    kw(h, "choice", Value::Object(fields))
}
fn wait_choice(h: &Bridge, phase: &str) -> Value {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        let value = choice(h, "read-request", json!({}));
        if value["phase"] == phase {
            return value;
        }
        assert!(Instant::now() < end, "{value}");
        thread::sleep(Duration::from_millis(10));
    }
}
fn visible() -> Bridge {
    let h = choice_host();
    assert_eq!(
        typed(&h, "abc"),
        vec![json!(false), json!(false), json!(false)]
    );
    let v = wait_choice(&h, "Visible");
    assert_eq!(v["anchor"], true);
    assert_eq!(v["window"], true);
    assert_eq!(kw(&h, "state", json!({}))["posts"], 0);
    choice(&h, "set", json!({"field":0,"value":"World"}));
    h
}
fn wait_output(h: &Bridge) -> Value {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        let state = kw(h, "state", json!({}));
        if state["posts"].as_u64().unwrap() > 0 {
            return state;
        }
        assert!(Instant::now() < end, "{state}");
        thread::sleep(Duration::from_millis(10));
    }
}
fn inserted(state: &Value) -> String {
    let units: Vec<u16> = state["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["down"] == true && e["key"] == 0)
        .flat_map(|e| {
            e["units"]
                .as_array()
                .unwrap()
                .iter()
                .map(|u| u.as_u64().unwrap() as u16)
        })
        .collect();
    String::from_utf16(&units).unwrap()
}
fn activate(h: &Bridge, values: &[&str]) {
    h.probe(
        "keyboard-hid",
        json!({"control":"activation-burst","values":values}),
    );
}
#[test]
fn live_trigger_uses_original_anchor_and_single_use_replacement() {
    let h = visible();
    choice(&h, "set", json!({"field":0,"value":"Rust"}));
    choice(&h, "insert", json!({}));
    wait_choice(&h, "None");
    let output = wait_output(&h);
    assert_eq!(inserted(&output), "Hello Rust!");
    assert_eq!(
        output["output"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["key"] == 51 && v["down"] == true)
            .count(),
        3
    );
    let before = output["posts"].clone();
    assert_eq!(choice(&h, "insert", json!({}))["phase"], "None");
    assert_eq!(kw(&h, "state", json!({}))["posts"], before);
}
#[test]
fn replacement_foreign_and_unknown_activations_cancel() {
    for actor in ["replacement", "foreign", "missing", "malformed"] {
        let h = visible();
        activate(&h, &[actor]);
        wait_choice(&h, "None");
        assert_eq!(kw(&h, "state", json!({}))["posts"], 0);
    }
}
#[test]
fn own_activation_keeps_form_but_away_and_back_stays_cancelled() {
    let h = visible();
    activate(&h, &["own"]);
    assert_eq!(choice(&h, "read-request", json!({}))["phase"], "Visible");
    activate(&h, &["foreign", "own"]);
    wait_choice(&h, "None");
    assert_eq!(kw(&h, "state", json!({}))["posts"], 0);
}

#[test]
fn extra_input_during_pending_capture_cancels_without_reuse() {
    let h = choice_host();
    assert_eq!(
        kw(&h, "event-burst", json!({"text":"abcX"}))["consumed"],
        json!([false, false, false, false])
    );
    wait_choice(&h, "None");
    assert_eq!(kw(&h, "state", json!({}))["posts"], 0);
}
#[test]
fn changed_anchor_and_secure_policy_fail_closed() {
    for field in [json!({"value":"changed"}), json!({"value":"abc","at":1})] {
        let h = choice_host();
        kw(&h, "text", field);
        typed(&h, "abc");
        wait_choice(&h, "None");
        assert_eq!(kw(&h, "state", json!({}))["posts"], 0);
    }
    let h = visible();
    kw(&h, "environment", json!({"flags":4096}));
    choice(&h, "insert", json!({}));
    wait_choice(&h, "None");
    assert_eq!(kw(&h, "state", json!({}))["posts"], 0);
}
#[test]
fn global_external_click_cancels_without_activating_original() {
    let h = visible();
    assert_eq!(
        kw(&h, "choice-monitors", json!({})),
        json!({"created":1,"removed":0,"delivered":0,"active":1})
    );
    choice(&h, "click", json!({}));
    wait_choice(&h, "None");
    assert_eq!(kw(&h, "state", json!({}))["posts"], 0);
    assert_eq!(
        kw(&h, "choice-monitors", json!({})),
        json!({"created":1,"removed":1,"delivered":1,"active":0})
    );
    assert_eq!(kw(&h, "state", json!({}))["activationRequests"], 0);
}
#[test]
fn original_anchor_is_rechecked_after_confirmation() {
    let h = visible();
    kw(&h, "text", json!({"value":"different"}));
    choice(&h, "insert", json!({}));
    wait_choice(&h, "None");
    assert_eq!(kw(&h, "state", json!({}))["posts"], 0);
}
