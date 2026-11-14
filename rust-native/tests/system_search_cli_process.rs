//! Real owned headless system-search routes and synthetic effects.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host};
use serde_json::{json, Value};
use std::fs;
fn host() -> Host {
    let mut h = Host::new(Config::current(), false);
    h.env
        .insert("RUYCAST_NATIVE_TEST_SYSTEM_SEARCH".into(), "owned".into());
    h.env
        .insert("RUYCAST_NATIVE_TEST_MAIN_TABS".into(), "1".into());
    h.env
        .insert("RUYCAST_NATIVE_TEST_WIDTH".into(), "760".into());
    let mut cfg = default_config();
    cfg["theme"] = json!("light");
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"fixture.alias","title":"Local Exact Alias","kind":"url","target":"https://example.invalid/local","aliases":["owned"]}));
    h.replace(&cfg, None);
    h.start();
    h
}
fn search(h: &Host, mode: &str, q: &str) -> Value {
    let result = h.rpc(&json!({"op":"search","mode":mode,"query":q}));
    assert_eq!(result["ok"], true, "{result}");
    result
}
fn fixture(h: &Host, action: &str, mut fields: Value) -> Value {
    let request = fields.as_object_mut().unwrap();
    request.insert("op".into(), json!("system.fixture"));
    request.insert("action".into(), json!(action));
    let result = h.rpc(&fields);
    assert_eq!(result["ok"], true, "{result}");
    result["result"].clone()
}
fn effects(h: &Host) -> Value {
    fixture(h, "read", json!({}))["effects"].clone()
}
#[test]
fn cli_real_provider_routes_metadata_order_and_exact_effects() {
    let h = host();
    assert_eq!(fixture(&h, "read", json!({}))["reads"], 0);
    let main = search(&h, "commands", "owned")["result"].clone();
    assert!(main[0]["id"]
        .as_str()
        .unwrap()
        .starts_with("system.tab:root."));
    assert_eq!(main[1]["id"], "fixture.alias");
    assert!(!main
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"].as_str().unwrap().starts_with("system.window:")));
    let tabs = search(&h, "tabs", "invalid")["result"].clone();
    assert_eq!(tabs[0]["tab"]["url"], "https://example.invalid/owned");
    assert!(!tabs[0]["id"]
        .as_str()
        .unwrap()
        .starts_with("system.tab:root."));
    let windows = search(&h, "windows", "")["result"].clone();
    assert_eq!(windows.as_array().unwrap().len(), 2);
    assert_eq!(windows[1]["window"]["minimized"], true);
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":windows[1]["id"]}))["ok"],
        true
    );
    assert_eq!(effects(&h), json!(["activate", "restore", "AXRaise"]));
    fixture(&h, "clear-effects", json!({}));
    let menus = search(&h, "menus", "");
    let row = &menus["result"][0];
    assert_eq!(row["menu"]["shortcut"]["label"], "Command + O");
    assert!(row.get("hotkey").is_none());
    assert_eq!(h.rpc(&json!({"op":"execute","id":row["id"]}))["ok"], false);
    assert_eq!(effects(&h), json!([]));
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":row["id"],"foreground":menus["foreground"]}))["ok"],
        true
    );
    assert_eq!(effects(&h), json!(["activate", "AXPress"]));
    fixture(&h, "clear-effects", json!({}));
    fixture(&h, "rename-window", json!({}));
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":windows[0]["id"]}))["ok"],
        false
    );
    assert_eq!(effects(&h), json!([]));
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":tabs[0]["id"]}))["ok"],
        true
    );
    assert_eq!(
        effects(&h),
        json!(["select-tab", "front-window", "activate"])
    );
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn cli_shortcut_uuid_duplicate_titles_bindings_and_private_output() {
    let h = host();
    let rows = search(&h, "shortcuts", "")["result"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(rows[0]["title"], rows[1]["title"]);
    assert_ne!(rows[0]["id"], rows[1]["id"]);
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    let mut saved = rows[1].clone();
    saved["aliases"] = json!(["uuidalias"]);
    saved["hotkey"] = json!("cmd+alt+w");
    cfg["commands"].as_array_mut().unwrap().push(saved.clone());
    h.replace(&cfg, None);
    h.rpc(&json!({"op":"reload"}));
    let found = search(&h, "shortcuts", "uuidalias")["result"].clone();
    assert_eq!(found[0]["id"], saved["id"]);
    assert_eq!(found[0]["hotkey"], saved["hotkey"]);
    assert_eq!(h.rpc(&json!({"op":"execute","id":"uuidalias"}))["ok"], true);
    assert_eq!(
        effects(&h),
        json!(["shortcut:11111111-2222-3333-4444-555555555555"])
    );
    saved["enabled"] = json!(false);
    *cfg["commands"].as_array_mut().unwrap().last_mut().unwrap() = saved;
    h.replace(&cfg, None);
    h.rpc(&json!({"op":"reload"}));
    assert_eq!(search(&h, "shortcuts", "uuidalias")["result"], json!([]));
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":"uuidalias"}))["ok"],
        false
    );
}
