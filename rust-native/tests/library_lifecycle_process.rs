//! Library lifecycle behavior in an owned hidden native host.
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
#[test]
fn toolbar_add_is_private_until_save_then_shared_with_palette() {
    let h = host();
    show(&h, "");
    let before = fs::read(&h.config).unwrap();
    settings(&h, "library-add", json!({"kind":"snippet"}));
    view(&h, |v| v["formVisible"] == true);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    field(&h, "title", "Created in Settings");
    field(&h, "target", "Hello");
    field(&h, "aliases", "newsettings");
    let id = p(&h, "command-read")["value"]["id"].clone();
    capture(&h, "new-snippet");
    p(&h, "save-editor");
    saved(&h);
    assert_eq!(h.cli(&["search", "newsettings"])["result"][0]["id"], id);
}
#[test]
fn invalid_config_keeps_both_last_good_views_and_refuses_draft_save() {
    let h = host();
    actions::show(&h, "finder");
    show(&h, "finder");
    edit(&h);
    field(&h, "title", "Keep draft");
    fs::write(&h.config, "broken PRIVATE-SENTINEL").unwrap();
    settled(&h, |s| !s["configError"].is_null());
    let before = fs::read(&h.config).unwrap();
    p(&h, "save-editor");
    view(&h, |v| {
        v["formVisible"] == true && v["screenText"].as_array().unwrap().len() > 4
    });
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(p(&h, "command-read")["value"]["title"], "Keep draft");
    assert!(!p(&h, "read").to_string().contains("PRIVATE-SENTINEL"));
    assert!(actions::probe(&h, "read")["rows"].as_u64().unwrap() > 0);
}
#[test]
fn theme_and_application_resign_update_both_windows() {
    let h = host();
    actions::show(&h, "finder");
    show(&h, "finder");
    for theme in ["light", "dark"] {
        let mut cfg = config(&h);
        cfg["theme"] = json!(theme);
        h.replace(&cfg, None);
        settled(&h, |s| s["theme"] == theme);
        h.settled(|_| true);
        assert_eq!(
            p(&h, "read")["appearance"]
                .as_str()
                .unwrap()
                .contains("DarkAqua"),
            theme == "dark"
        );
        assert_eq!(
            actions::probe(&h, "read")["appearance"]
                .as_str()
                .unwrap()
                .contains("DarkAqua"),
            theme == "dark"
        );
        capture(&h, &format!("library-{theme}"));
    }
    hotkey(&h);
    settings(
        &h,
        "key-event",
        json!({"code":43,"flags":CMD|SHIFT,"chars":",","repeat":false}),
    );
    view(&h, |v| v["formVisible"] == true && v["recording"] == true);
    p(&h, "resign-app");
    assert_eq!(p(&h, "read")["recording"], false);
    assert_eq!(actions::probe(&h, "read")["recording"], false);
    assert_eq!(actions::probe(&h, "read")["formVisible"], true);
    assert_eq!(p(&h, "read")["formVisible"], true);
}
#[test]
fn hidden_settings_callbacks_do_not_change_navigation_or_cancel_palette() {
    let h = host();
    actions::show(&h, "finder");
    alias(&h);
    let token = actions::probe(&h, "read")["editorToken"].clone();
    show(&h, "safari");
    p(&h, "window-close");
    settled(&h, |s| s["managementUI"]["visible"] == false);
    let before = p(&h, "library-read");
    settings(&h, "library-tab", json!({"tab":"snippets"}));
    settings(&h, "library-filter", json!({"filter":"disabled"}));
    settled(&h, |_| true);
    let after = p(&h, "library-read");
    assert_eq!(after["tab"], before["tab"]);
    assert_eq!(after["filter"], before["filter"]);
    assert_eq!(after["visible"], false);
    assert_eq!(actions::probe(&h, "read")["editorToken"], token);
}
#[test]
fn normal_quit_with_both_windows_and_drafts_releases_socket() {
    let mut h = host();
    actions::show(&h, "finder");
    alias(&h);
    show(&h, "safari");
    edit(&h);
    let before = fs::read(&h.config).unwrap();
    assert_eq!(h.rpc(&json!({"op":"quit"}))["ok"], true);
    let out = h.wait_exit(Duration::from_secs(4), false);
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!h.profile.join("control.sock").exists());
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
