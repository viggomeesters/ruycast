//! Inline editor transition behavior in an owned hidden native host.
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
fn worker_rejection_acknowledges_and_keeps_the_native_draft_typable() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"retained"}));
    let token = p(&h, "read")["editorToken"].clone();
    let before = fs::read(&h.config).unwrap();
    p(&h, "inline-stale-switch");
    view(&h, |v| text_contains(v, "Selection changed"));
    assert_eq!(p(&h, "read")["editorToken"], token);
    settings(&h, "alias-type", json!({"text":"x"}));
    assert!(p(&h, "read")["aliasValue"].as_str().unwrap().contains('x'));
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn cancelled_parent_filters_an_already_published_next_editor() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"published"}));
    begin(&h, "app.safari", false);
    settings(&h, "alias-type", json!({"text":"mustnotpublish"}));
    let before = fs::read(&h.config).unwrap();
    p(&h, "cancel-inline-origin");
    done(&h);
    p(&h, "save-editor");
    settled(&h, |_| true);
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn navigation_commits_before_leaving_and_validation_error_keeps_context() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"beforenav"}));
    settings(&h, "library-tab", json!({"tab":"snippets"}));
    done(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["beforenav"]));
    assert_eq!(p(&h, "library-read")["tab"], "snippets");
    settings(&h, "library-tab", json!({"tab":"commands"}));
    settled(&h, |_| true);
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"safari"}));
    let before = fs::read(&h.config).unwrap();
    settings(&h, "library-tab", json!({"tab":"quicklinks"}));
    view(&h, duplicate);
    assert_eq!(p(&h, "library-read")["tab"], "commands");
    assert_eq!(fs::read(&h.config).unwrap(), before);
    settings(&h, "alias-type", json!({"text":"repairednav"}));
    settings(&h, "library-filter", json!({"filter":"disabled"}));
    done(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["repairednav"]));
    assert_eq!(p(&h, "library-read")["filter"], "disabled");
}
#[test]
fn add_and_title_edit_commit_the_previous_inline_draft() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"beforecreate"}));
    settings(&h, "library-add", json!({"kind":"snippet"}));
    view(&h, |v| {
        v["formVisible"] == true
            && v["screenText"]
                .as_array()
                .unwrap()
                .contains(&json!("New Snippet"))
    });
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["beforecreate"])
    );
    p(&h, "cancel-editor");
    done(&h);
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"beforeedit"}));
    let rows = p(&h, "library-read")["rows"].as_array().unwrap().clone();
    let index = rows.iter().position(|r| r["id"] == "app.safari").unwrap();
    settings(&h, "click", json!({"row":index}));
    view(&h, |v| {
        v["formVisible"] == true && v["screenText"].as_array().unwrap().contains(&json!("Name"))
    });
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["beforeedit"]));
    assert_eq!(p(&h, "command-read")["value"]["id"], "app.safari");
}
#[test]
fn native_tracking_defers_autosave_until_binding_action_has_run() {
    let h = host();
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"beforetracking"}));
    settings(&h, "inline-tracking-switch", json!({"id":"app.safari"}));
    view(&h, |v| {
        v["inlineVisible"] == true && v["aliasValue"] == "safari"
    });
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["beforetracking"])
    );
    settings(&h, "alias-type", json!({"text":"safaridraft"}));
    assert_eq!(p(&h, "inline-control-read")["hitEditor"], true);
}
