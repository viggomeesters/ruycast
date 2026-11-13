//! Inline editing behavior in an owned hidden native host.
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
fn alias_enter_and_unchanged_focus_loss_do_not_leak_form_or_rewrite() {
    let h = host();
    show(&h, "finder");
    let v = begin(&h, "app.finder", false);
    assert_eq!(v["formVisible"], false);
    assert_eq!(p(&h, "inline-control-read")["hitEditor"], true);
    assert_eq!(p(&h, "inline-control-read")["underlyingHidden"], true);
    assert_eq!(
        p(&h, "read")["accessibilityText"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| **s == json!("Aliases for Finder"))
            .count(),
        1
    );
    settings(&h, "alias-type", json!({"text":"inlinefinder"}));
    capture(&h, "inline-alias");
    press(&h, 36, 0, "");
    done(&h);
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["inlinefinder"])
    );
    begin(&h, "app.finder", false);
    let before = fs::read(&h.config).unwrap();
    p(&h, "inline-focus-search");
    done(&h);
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn alias_focus_loss_commits_and_error_keeps_repairable_draft() {
    let h = host();
    show(&h, "finder");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"focusfinder"}));
    p(&h, "inline-focus-search");
    done(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["focusfinder"]));
    begin(&h, "app.finder", false);
    let before = fs::read(&h.config).unwrap();
    settings(&h, "alias-type", json!({"text":"safari"}));
    p(&h, "inline-focus-search");
    view(&h, duplicate);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    settings(&h, "alias-type", json!({"text":"repaired"}));
    press(&h, 36, 0, "");
    done(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["repaired"]));
}
#[test]
fn switch_commits_a_before_opening_b_and_validation_failure_keeps_a() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"savedfirst"}));
    begin(&h, "app.safari", false);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["savedfirst"]));
    assert_eq!(p(&h, "read")["aliasValue"], "safari");
    settings(&h, "alias-type", json!({"text":"savedfirst"}));
    let token = p(&h, "read")["editorToken"].clone();
    let before = fs::read(&h.config).unwrap();
    settings(&h, "inline-alias", json!({"id":"app.finder"}));
    view(&h, duplicate);
    assert_eq!(p(&h, "read")["editorToken"], token);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    settings(&h, "alias-type", json!({"text":"newsafari"}));
    begin(&h, "app.finder", false);
    assert_eq!(command(&h, "app.safari")["aliases"], json!(["newsafari"]));
}
#[test]
fn checkbox_transition_commits_alias_or_retains_error() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"beforetoggle"}));
    settings(&h, "library-enable", json!({"id":"app.safari"}));
    done(&h);
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["beforetoggle"])
    );
    assert_eq!(command(&h, "app.safari")["enabled"], false);
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"safari"}));
    let before = fs::read(&h.config).unwrap();
    settings(&h, "library-enable", json!({"id":"app.safari"}));
    view(&h, duplicate);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(p(&h, "read")["inlineVisible"], true);
}
#[test]
fn inline_hotkey_records_immediately_clears_and_escape_does_not_write() {
    let h = host();
    show(&h, "finder");
    begin(&h, "app.finder", true);
    let before = fs::read(&h.config).unwrap();
    press(&h, 53, 0, "");
    done(&h);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    begin(&h, "app.finder", true);
    capture(&h, "inline-hotkey");
    press(&h, 13, CMD, "w");
    done(&h);
    assert_eq!(command(&h, "app.finder")["hotkey"], "cmd+w");
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["managementUI"]["visible"],
        true
    );
    begin(&h, "app.finder", true);
    press(&h, 51, 0, "");
    done(&h);
    assert!(command(&h, "app.finder").get("hotkey").is_none());
}
