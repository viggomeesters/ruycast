//! Library checkbox behavior in an owned hidden native host.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::{native_bridge::Bridge, *};
use serde_json::{json, Value};
use std::{
    fs,
    os::fd::AsRawFd,
    thread,
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
fn enabled(h: &Bridge, id: &str) -> Value {
    command(h, id)["enabled"].clone()
}
fn set(h: &Bridge, action: &str, id: &str) -> Value {
    settings(h, action, json!({"id":id}))
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
fn inline_enabled_checkbox_targets_unselected_row_and_updates_palette() {
    let h = host();
    show(&h, "");
    let selected = p(&h, "library-read")["selectedRow"].clone();
    let control = set(&h, "library-enabled-read", "app.safari");
    assert_eq!(control["checked"], true);
    assert_eq!(control["hitCheckbox"], true);
    assert_eq!(control["label"], "Enable Safari");
    set(&h, "library-enable", "app.safari");
    settled(&h, |_| {
        !h.cli(&["search", "safari"])["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "app.safari")
    });
    assert_eq!(enabled(&h, "app.safari"), false);
    assert_eq!(
        set(&h, "library-enabled-read", "app.safari")["checked"],
        false
    );
    assert_eq!(p(&h, "library-read")["selectedRow"], selected);
    assert_eq!(p(&h, "read")["formVisible"], false);
    assert_eq!(p(&h, "read")["actionsVisible"], false);
    set(&h, "library-enable", "app.safari");
    settled(&h, |_| enabled(&h, "app.safari") == true);
    capture(&h, "inline-enabled");
}
#[test]
fn first_checkbox_click_after_same_results_query_and_reopen_uses_current_binding() {
    let h = host();
    show(&h, "");
    set(&h, "library-enable", "app.safari");
    settled(&h, |_| enabled(&h, "app.safari") == false);
    p(&h, "window-close");
    settled(&h, |s| s["managementUI"]["visible"] == false);
    assert_eq!(h.rpc(&json!({"op":"show","mode":"settings"}))["ok"], true);
    settled(&h, |s| s["managementUI"]["visible"] == true);
    set(&h, "library-enable", "app.safari");
    settled(&h, |_| enabled(&h, "app.safari") == true);
}
#[test]
fn old_checkbox_cannot_write_after_query_reload_close_or_editor_open() {
    let h = host();
    show(&h, "finder");
    set(&h, "retain-library-enabled", "app.finder");
    settings(&h, "query", json!({"query":"safari"}));
    settled(&h, |_| true);
    let before = fs::read(&h.config).unwrap();
    p(&h, "old-library-enable");
    settled(&h, |_| true);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    show(&h, "finder");
    set(&h, "retain-library-enabled", "app.finder");
    let mut cfg = config(&h);
    cfg["commands"][0]["title"] = json!("Renamed");
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    settled(&h, |_| true);
    let before = fs::read(&h.config).unwrap();
    p(&h, "old-library-enable");
    assert_eq!(fs::read(&h.config).unwrap(), before);
    show(&h, "finder");
    set(&h, "retain-library-enabled", "app.finder");
    p(&h, "window-close");
    settled(&h, |s| s["managementUI"]["visible"] == false);
    show(&h, "finder");
    let before = fs::read(&h.config).unwrap();
    p(&h, "old-library-enable");
    assert_eq!(fs::read(&h.config).unwrap(), before);
    set(&h, "retain-library-enabled", "app.finder");
    settings(
        &h,
        "key-event",
        json!({"code":40,"flags":CMD,"chars":"k","repeat":false}),
    );
    let v = view(&h, |v| v["actionsVisible"] == true);
    let index = v["actionLabels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|v| v.as_str().unwrap().starts_with("Edit"))
        .unwrap();
    settings(&h, "action-button", json!({"index":index}));
    view(&h, |v| v["formVisible"] == true && text_contains(v, "Name"));
    let before = fs::read(&h.config).unwrap();
    p(&h, "old-library-enable");
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(p(&h, "read")["formVisible"], true);
}
#[test]
fn inline_checkbox_failure_keeps_published_value_and_allows_retry() {
    let h = host();
    show(&h, "finder");
    let before = fs::read(&h.config).unwrap();
    {
        let lock = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(h.profile.join("config.lock"))
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        set(&h, "library-enable", "app.finder");
        view(&h, |v| {
            v["screenText"].as_array().unwrap().iter().any(|s| {
                let s = s.as_str().unwrap().to_lowercase();
                s.contains("lock") || s.contains("busy") || s.contains("being saved")
            })
        });
        assert_eq!(fs::read(&h.config).unwrap(), before);
        assert_eq!(
            p(&h, "library-read")["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == "app.finder")
                .unwrap()["enabled"],
            true
        );
        assert_eq!(
            set(&h, "library-enabled-read", "app.finder")["checked"],
            true
        );
    }
    set(&h, "library-enable", "app.finder");
    settled(&h, |_| enabled(&h, "app.finder") == false);
}
#[test]
fn inline_enable_does_not_discard_a_palette_draft() {
    let h = host();
    actions::show(&h, "finder");
    alias(&h);
    h.probe("alias-input", json!({"text":"keepmine"}));
    let token = actions::probe(&h, "read")["editorToken"].clone();
    show(&h, "safari");
    set(&h, "library-enable", "app.safari");
    settled(&h, |_| enabled(&h, "app.safari") == false);
    assert_eq!(actions::probe(&h, "read")["editorToken"], token);
    assert_eq!(actions::probe(&h, "read")["aliasValue"], "keepmine");
    actions::probe(&h, "save-editor");
    saved(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["keepmine"]));
}
