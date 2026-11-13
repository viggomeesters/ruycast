//! Inline focus and undo behavior in an owned hidden native host.
#![cfg(feature = "test-tools")]
#![allow(dead_code)]
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
fn done(h: &Bridge) {
    settled(h, |s| s["managementUI"]["editorOpen"] == false);
}
fn press(h: &Bridge, code: u32, flags: u64, chars: &str) {
    settings(
        h,
        "key-event",
        json!({"code":code,"flags":flags,"chars":if (code==36||code==76)&&chars.is_empty(){"\r"} else {chars},"repeat":false}),
    );
}
fn duplicate(v: &Value) -> bool {
    v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().to_lowercase().contains("duplicate"))
}
#[test]
fn repeated_focus_return_coalesces_one_autosave_and_stays_typable() {
    let h = host();
    show(&h, "finder");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"coalesced"}));
    let before = fs::read(&h.config).unwrap();
    p(&h, "inline-repeat-focus");
    settled(&h, |_| true);
    let control = p(&h, "inline-control-read");
    assert_eq!(control["autosaveSlots"], 1);
    assert_eq!(control["hitEditor"], true);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    settings(&h, "alias-type", json!({"text":"x"}));
    p(&h, "inline-focus-search");
    done(&h);
    assert!(command(&h, "app.finder")["aliases"][0]
        .as_str()
        .unwrap()
        .contains('x'));
}
#[test]
fn group_collapse_commits_the_inline_alias_and_search_still_finds_it() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"groupfinder"}));
    settings(&h, "click", json!({"row":0}));
    done(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["groupfinder"]));
    assert!(!p(&h, "library-read")["rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["id"] == "app.finder"));
    settings(&h, "query", json!({"query":"groupfinder"}));
    settled(&h, |s| s["managementUI"]["selectedId"] == "app.finder");
}
#[test]
fn query_typing_saves_alias_without_clobbering_query_and_retains_errors() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"beforequery"}));
    settings(&h, "inline-focus-and-query", json!({"query":"safari"}));
    done(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["beforequery"]));
    assert_eq!(p(&h, "read")["input"], "safari");
    assert_eq!(p(&h, "read")["modelQuery"], "safari");
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"safari"}));
    let before = fs::read(&h.config).unwrap();
    settings(&h, "inline-focus-and-query", json!({"query":"safari"}));
    view(&h, |v| v["formVisible"] == true && duplicate(v));
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(p(&h, "read")["input"], "safari");
    settings(&h, "alias-type", json!({"text":"queryrepair"}));
    press(&h, 36, 0, "");
    done(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["queryrepair"]));
    assert_eq!(p(&h, "read")["modelQuery"], "safari");
}
#[test]
fn alias_undo_history_is_local_to_the_current_bound_draft() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"firstprivate"}));
    assert_eq!(p(&h, "alias-undo-state")["canUndo"], true);
    begin(&h, "app.safari", false);
    assert_eq!(p(&h, "alias-undo-state")["canUndo"], false);
    assert_eq!(p(&h, "read")["aliasValue"], "safari");
    settings(&h, "alias-type", json!({"text":"secondprivate"}));
    press(&h, 6, CMD, "z");
    assert_eq!(p(&h, "read")["aliasValue"], "safari");
}
#[test]
fn close_and_app_resign_never_autosave_dirty_alias() {
    let h = host();
    show(&h, "finder");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"discard"}));
    let before = fs::read(&h.config).unwrap();
    p(&h, "window-close");
    settled(&h, |s| s["managementUI"]["visible"] == false);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    show(&h, "finder");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"hold"}));
    p(&h, "resign-app");
    p(&h, "inline-focus-search");
    settled(&h, |_| true);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["managementUI"]["editorOpen"],
        true
    );
}
