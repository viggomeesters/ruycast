//! Inline composition and selection behavior in an owned hidden native host.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::{native_bridge::Bridge, *};
use serde_json::{json, Value};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn settings(h: &Bridge, action: &str, fields: Value) -> Value {
    let mut req = fields.as_object().unwrap().clone();
    req.insert("op".into(), json!("native.test"));
    req.insert("surface".into(), json!("settings"));
    req.insert("action".into(), json!(action));
    let response = h.rpc(&Value::Object(req));
    assert_eq!(response["ok"], true, "{response}");
    response["result"].clone()
}
fn p(h: &Bridge, action: &str) -> Value {
    settings(h, action, json!({}))
}
fn settled(h: &Bridge, pred: impl Fn(&Value) -> bool) {
    h.wait(|s| {
        s["configurationUpdating"] != true
            && s["managementUI"]["revision"].as_u64().unwrap_or(0) > 0
            && s["managementUI"]["revision"] == s["managementUI"]["appliedRevision"]
            && pred(s)
    });
}
fn view(h: &Bridge, pred: impl Fn(&Value) -> bool) -> Value {
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        assert!(
            Instant::now() < until,
            "Settings did not reach expected state"
        );
        let value = p(h, "read");
        if pred(&value) {
            return value;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn show(h: &Bridge, query: &str) {
    assert_eq!(
        h.rpc(&json!({"op":"show","mode":"settings","query":query}))["ok"],
        true
    );
    settled(h, |s| s["managementUI"]["visible"] == true);
    view(h, |v| v["input"] == query);
}
fn capture(h: &Bridge, name: &str) {
    let value = p(h, "screenshot");
    let image = h.profile.join("native-settings-probe.png");
    assert!(fs::read(&image).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    if let Some(evidence) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let directory = std::path::PathBuf::from(evidence)
            .parent()
            .unwrap()
            .join("library");
        fs::create_dir_all(&directory).unwrap();
        fs::copy(image, directory.join(format!("{name}.png"))).unwrap();
        fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}
fn begin(h: &Bridge, id: &str, hotkey: bool) -> Value {
    let before = p(h, "read")["editorToken"].clone();
    settings(
        h,
        if hotkey {
            "inline-hotkey"
        } else {
            "inline-alias"
        },
        json!({"id":id}),
    );
    view(h, |v| {
        v["inlineVisible"] == true
            && v["editorId"] == id
            && v["editorToken"] != before
            && (!hotkey || v["recording"] == true)
    })
}
fn press(h: &Bridge, code: u32) {
    settings(
        h,
        "key-event",
        json!({"code":code,"flags":0,"chars":"","repeat":false}),
    );
}
#[test]
fn reload_preserves_actual_typing_and_composition() {
    let h = host();
    show(&h, "finder");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"mydraft"}));
    let token = p(&h, "read")["editorToken"].clone();
    let mut cfg = config(&h);
    cfg["clipboard"]["limit"] = json!(84);
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    settled(&h, |_| true);
    assert_eq!(p(&h, "read")["editorToken"], token);
    settings(&h, "alias-type", json!({"text":"x"}));
    assert!(p(&h, "read")["aliasValue"].as_str().unwrap().contains('x'));
    assert_eq!(
        settings(&h, "inline-mark", json!({"text":"日本"}))["marked"],
        true
    );
    cfg["commands"][0]["title"] = json!("Finder renamed");
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    settled(&h, |_| true);
    assert_eq!(p(&h, "inline-marked-read")["marked"], true);
    p(&h, "inline-unmark");
    press(&h, 53);
}
#[test]
fn removed_anchor_falls_back_without_losing_marked_text_or_persisting() {
    let h = host();
    show(&h, "finder");
    begin(&h, "app.finder", false);
    let token = p(&h, "read")["editorToken"].clone();
    assert_eq!(
        settings(&h, "inline-mark", json!({"text":"日本"}))["marked"],
        true
    );
    let mut cfg = config(&h);
    cfg["commands"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["id"] != "app.finder");
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    settled(&h, |_| true);
    let before = fs::read(&h.config).unwrap();
    let v = p(&h, "inline-marked-read");
    assert_eq!(v["marked"], true);
    assert_eq!(v["formVisible"], true);
    assert_eq!(p(&h, "inline-control-read")["hitEditor"], true);
    assert_eq!(v["editorToken"], token);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    capture(&h, "inline-anchor-fallback");
    p(&h, "inline-unmark");
    press(&h, 53);
}
#[test]
fn scroll_far_unselected_binding_selects_target_and_preserves_composition() {
    let h = host();
    let commands=(0..200).map(|i|json!({"id":format!("url.{i:03}"),"title":format!("Item {i:03}"),"kind":"url","target":format!("https://example.org/{i}"),"aliases":[format!("alias{i}")]})).collect::<Vec<_>>();
    let mut cfg = actions::native_bridge::runtime_host::default_config();
    cfg["commands"] = json!(commands);
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    settled(&h, |_| true);
    show(&h, "");
    begin(&h, "url.150", false);
    settled(&h, |s| s["managementUI"]["selectedId"] == "url.150");
    assert_eq!(
        settings(&h, "inline-mark", json!({"text":"日本"}))["marked"],
        true
    );
    settings(&h, "inline-scroll", json!({"y":0.0}));
    assert_eq!(p(&h, "inline-marked-read")["marked"], true);
    settings(&h, "inline-scroll", json!({"y":150.0*39.0}));
    assert_eq!(p(&h, "inline-marked-read")["marked"], true);
    capture(&h, "inline-scrolled");
    p(&h, "inline-unmark");
    press(&h, 53);
}
#[test]
fn marked_alias_refuses_binding_and_checkbox_transition() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    let token = p(&h, "read")["editorToken"].clone();
    assert_eq!(
        settings(&h, "inline-mark", json!({"text":"日本"}))["marked"],
        true
    );
    let before = fs::read(&h.config).unwrap();
    settings(&h, "library-enable", json!({"id":"app.safari"}));
    settings(&h, "inline-alias", json!({"id":"app.safari"}));
    settled(&h, |_| true);
    assert_eq!(p(&h, "read")["editorToken"], token);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(p(&h, "inline-marked-read")["marked"], true);
    p(&h, "inline-unmark");
    press(&h, 53);
}
