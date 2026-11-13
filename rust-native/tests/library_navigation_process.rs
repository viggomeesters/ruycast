//! Library navigation behavior in an owned hidden native host.
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
fn fixture(h: &Bridge, commands: Value) {
    let mut config = actions::native_bridge::runtime_host::default_config();
    config["commands"] = commands;
    h.replace(&config, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    settled(h, |_| true);
}
fn rows(h: &Bridge) -> Vec<Value> {
    p(h, "library-read")["rows"].as_array().unwrap().clone()
}
fn nav(h: &Bridge, action: &str, fields: Value) -> Value {
    settings(h, action, fields);
    settled(h, |_| true);
    p(h, "library-read")
}
fn key_settings(h: &Bridge, code: u32, flags: u64, chars: &str) {
    settings(
        h,
        "key-event",
        json!({"code":code,"flags":flags,"chars":chars,"repeat":false}),
    );
}
#[test]
fn cli_settings_is_separate_and_reusable_without_showing_a_window() {
    let h = host();
    assert_eq!(h.cli(&["show", "settings"])["ok"], true);
    settled(&h, |s| s["managementUI"]["visible"] == true);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        false
    );
    assert_eq!(p(&h, "read")["windowShown"], false);
    assert!(p(&h, "read")["screenText"]
        .as_array()
        .unwrap()
        .contains(&json!("Commands")));
    capture(&h, "settings-library");
    p(&h, "window-close");
    settled(&h, |s| s["managementUI"]["visible"] == false);
    show(&h, "finder");
    assert_eq!(p(&h, "read")["windowShown"], false);
}
#[test]
fn disabled_items_can_be_found_and_enabled_without_execution() {
    let h = host();
    let mut cfg = actions::native_bridge::runtime_host::default_config();
    cfg["commands"][0]["enabled"] = json!(false);
    fixture(&h, cfg["commands"].clone());
    show(&h, "finder");
    assert!(!h.cli(&["search", "finder"])["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "app.finder"));
    assert_eq!(
        rows(&h)
            .iter()
            .filter(|r| r["group"] == false)
            .map(|r| r["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("app.finder")]
    );
    key_settings(&h, 40, CMD, "k");
    let v = view(&h, |v| v["actionsVisible"] == true);
    assert!(v["actionLabels"]
        .as_array()
        .unwrap()
        .contains(&json!("Enable")));
    assert!(!v["actionLabels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().starts_with("Open")));
    key_settings(&h, 53, 0, "");
    nav(&h, "library-filter", json!({"filter":"disabled"}));
    capture(&h, "disabled");
    key_settings(&h, 40, CMD, "k");
    let v = view(&h, |v| v["actionsVisible"] == true);
    let index = v["actionLabels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|s| s.as_str().unwrap().starts_with("Enable"))
        .unwrap();
    settings(&h, "action-button", json!({"index":index}));
    view(&h, |v| v["actionsVisible"] == false);
    settled(&h, |_| true);
    assert_eq!(command(&h, "app.finder")["enabled"], true);
    assert!(h.cli(&["search", "finder"])["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "app.finder"));
    assert!(rows(&h).iter().all(|r| r["group"] == true));
}
#[test]
fn tabs_groups_filters_and_query_reset_keep_all_saved_items() {
    let h = host();
    let mut cfg = actions::native_bridge::runtime_host::default_config();
    cfg["commands"].as_array_mut().unwrap().extend([json!({"id":"snippet.test","title":"Test snippet","kind":"snippet","target":"Text","aliases":["snippetfind"]}),json!({"id":"link.test","title":"Test link","kind":"url","target":"https://example.org","aliases":["linkfind"]})]);
    fixture(&h, cfg["commands"].clone());
    show(&h, "");
    let row = rows(&h)
        .iter()
        .position(|r| r["group"] == true && r["title"].as_str().unwrap().contains("Applications"))
        .unwrap();
    settings(&h, "click", json!({"row":row}));
    settled(&h, |_| true);
    assert!(!rows(&h).iter().any(|r| r["id"] == "app.finder"));
    settings(&h, "query", json!({"query":"finder"}));
    settled(&h, |_| true);
    assert!(rows(&h).iter().any(|r| r["id"] == "app.finder"));
    settings(&h, "query", json!({"query":""}));
    settled(&h, |_| true);
    assert!(!rows(&h).iter().any(|r| r["id"] == "app.finder"));
    assert_eq!(
        nav(&h, "library-tab", json!({"tab":"snippets"}))["query"],
        ""
    );
    assert_eq!(
        rows(&h)
            .iter()
            .filter(|r| r["group"] == false)
            .map(|r| r["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("snippet.test")]
    );
    settings(&h, "query", json!({"query":"snippetfind"}));
    settled(&h, |_| true);
    nav(&h, "library-tab", json!({"tab":"quicklinks"}));
    assert_eq!(
        rows(&h)
            .iter()
            .filter(|r| r["group"] == false)
            .map(|r| r["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("path.documents"), json!("link.test")]
    );
    nav(&h, "library-tab", json!({"tab":"snippets"}));
    settings(&h, "query", json!({"query":"snippetfind"}));
    settled(&h, |_| true);
    assert_eq!(rows(&h).iter().filter(|r| r["group"] == false).count(), 1);
}
#[test]
fn library_has_no_launcher_hundred_item_cap() {
    let h = host();
    let commands=(0..5000).map(|i|json!({"id":format!("url.{i:04}"),"title":format!("Item {i:04}"),"kind":"url","target":format!("https://example.org/{i}"),"aliases":[format!("alias{i}")]})).collect::<Vec<_>>();
    fixture(&h, json!(commands));
    show(&h, "");
    assert_eq!(
        rows(&h)
            .iter()
            .filter(|r| r["group"] == false && !r["id"].as_str().unwrap().starts_with("builtin."))
            .count(),
        5000
    );
    settings(&h, "query", json!({"query":"alias4999"}));
    settled(&h, |_| true);
    assert_eq!(
        rows(&h)
            .iter()
            .filter(|r| r["group"] == false)
            .map(|r| r["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("url.4999")]
    );
    capture(&h, "last-of-5000");
}
