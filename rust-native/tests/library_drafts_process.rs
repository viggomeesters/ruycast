//! Concurrent palette and Library drafts in an owned hidden native host.
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
    let mut request = fields.as_object().unwrap().clone();
    request.insert("op".into(), json!("native.test"));
    request.insert("surface".into(), json!("settings"));
    request.insert("action".into(), json!(action));
    let response = h.rpc(&Value::Object(request));
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
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        assert!(
            Instant::now() < deadline,
            "Settings did not reach expected state"
        );
        let v = p(h, "read");
        if pred(&v) {
            return v;
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
fn edit(h: &Bridge) {
    settings(
        h,
        "key-event",
        json!({"code":40,"flags":CMD,"chars":"k","repeat":false}),
    );
    let v = view(h, |v| v["actionsVisible"] == true);
    let index = v["actionLabels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|x| x.as_str().unwrap().starts_with("Edit"))
        .unwrap();
    settings(h, "action-button", json!({"index":index}));
    view(h, |v| v["formVisible"] == true && text_contains(v, "Name"));
}
fn field(h: &Bridge, key: &str, text: &str) {
    settings(h, "command-input", json!({"key":key,"text":text}));
}
fn saved(h: &Bridge) {
    view(h, |v| v["formVisible"] == false);
}
#[test]
fn different_window_drafts_merge_and_keep_each_others_focus() {
    let h = host();
    actions::show(&h, "finder");
    alias(&h);
    h.probe("alias-input", json!({"text":"myfinder"}));
    let token = actions::probe(&h, "read")["editorToken"].clone();
    show(&h, "safari");
    edit(&h);
    field(&h, "title", "Safari renamed");
    p(&h, "save-editor");
    saved(&h);
    h.settled(|_| true);
    assert_eq!(actions::probe(&h, "read")["editorToken"], token);
    assert_eq!(actions::probe(&h, "read")["aliasValue"], "myfinder");
    actions::probe(&h, "save-editor");
    actions::saved(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["myfinder"]));
    assert_eq!(command(&h, "app.safari")["title"], "Safari renamed");
}
#[test]
fn same_field_conflict_retains_second_draft() {
    let h = host();
    actions::show(&h, "finder");
    actions::action(&h, "Edit");
    actions::view(&h, |v| v["formVisible"] == true && text_contains(v, "Name"));
    h.probe(
        "command-input",
        json!({"key":"title","text":"First window"}),
    );
    show(&h, "finder");
    edit(&h);
    field(&h, "title", "Second window");
    actions::probe(&h, "save-editor");
    actions::saved(&h);
    settled(&h, |_| true);
    let before = fs::read(&h.config).unwrap();
    p(&h, "save-editor");
    view(&h, |v| text_contains(v, "changed elsewhere"));
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(p(&h, "command-read")["value"]["title"], "Second window");
}
#[test]
fn cmd_w_closes_only_settings_and_discards_only_its_draft() {
    let h = host();
    actions::show(&h, "finder");
    alias(&h);
    h.probe("alias-input", json!({"text":"keepmine"}));
    let token = actions::probe(&h, "read")["editorToken"].clone();
    show(&h, "safari");
    edit(&h);
    field(&h, "title", "Discard");
    let before = fs::read(&h.config).unwrap();
    settings(
        &h,
        "key-event",
        json!({"code":13,"flags":CMD,"chars":"w","repeat":false}),
    );
    settled(&h, |s| {
        s["managementUI"]["visible"] == false && s["nativeUI"]["visible"] == true
    });
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(actions::probe(&h, "read")["editorToken"], token);
    assert_eq!(actions::probe(&h, "read")["aliasValue"], "keepmine");
    show(&h, "safari");
    assert_eq!(p(&h, "read")["formVisible"], false);
}
#[test]
fn repeated_show_preserves_actual_alias_and_name_editor_focus() {
    let h = host();
    show(&h, "finder");
    settings(
        &h,
        "key-event",
        json!({"code":43,"flags":CMD|OPT,"chars":",","repeat":false}),
    );
    view(&h, |v| {
        v["formVisible"] == true && text_contains(v, "Alias")
    });
    settings(&h, "alias-input", json!({"text":"my"}));
    let token = p(&h, "read")["editorToken"].clone();
    assert_eq!(h.rpc(&json!({"op":"show","mode":"settings"}))["ok"], true);
    settled(&h, |_| true);
    assert_eq!(p(&h, "read")["editorToken"], token);
    settings(&h, "alias-type", json!({"text":"x"}));
    assert!(p(&h, "read")["aliasValue"].as_str().unwrap().contains('x'));
    p(&h, "cancel-editor");
    saved(&h);
    edit(&h);
    assert_eq!(h.rpc(&json!({"op":"show","mode":"settings"}))["ok"], true);
    settled(&h, |_| true);
    settings(&h, "command-type", json!({"text":"Renamed"}));
    assert!(p(&h, "command-read")["value"]["title"]
        .as_str()
        .unwrap()
        .contains("Renamed"));
}
#[test]
fn cmd_w_is_recorded_before_window_close_and_repeated_show_keeps_recording() {
    let h = host();
    show(&h, "finder");
    edit(&h);
    let before = fs::read(&h.config).unwrap();
    p(&h, "record");
    view(&h, |v| v["recording"] == true);
    assert_eq!(h.rpc(&json!({"op":"show","mode":"settings"}))["ok"], true);
    settled(&h, |_| true);
    assert_eq!(p(&h, "read")["recording"], true);
    settings(
        &h,
        "key-event",
        json!({"code":13,"flags":CMD,"chars":"w","repeat":false}),
    );
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["managementUI"]["visible"],
        true
    );
    assert_eq!(p(&h, "read")["recording"], false);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(p(&h, "command-read")["value"]["hotkey"], "cmd+w");
    p(&h, "save-editor");
    saved(&h);
    assert_eq!(command(&h, "app.finder")["hotkey"], "cmd+w");
}
