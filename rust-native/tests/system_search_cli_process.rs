//! Real owned headless and hidden native system-search routes and synthetic effects.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::{
    action,
    native_bridge::{
        runtime_host::{default_config, Config, Host},
        Bridge,
    },
    view,
};
use serde_json::{json, Value};
use std::{fs, io::Write, time::Duration};
fn configure(h: &mut Host) {
    configure_with(h, "light", 760);
}
fn configure_with(h: &mut Host, theme: &str, width: u32) {
    h.env
        .insert("RUYCAST_NATIVE_TEST_SYSTEM_SEARCH".into(), "owned".into());
    h.env
        .insert("RUYCAST_NATIVE_TEST_MAIN_TABS".into(), "1".into());
    h.env
        .insert("RUYCAST_NATIVE_TEST_WIDTH".into(), width.to_string().into());
    let mut cfg = default_config();
    cfg["theme"] = json!(theme);
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"fixture.alias","title":"Local Exact Alias","kind":"url","target":"https://example.invalid/local","aliases":["owned"]}));
    h.replace(&cfg, None);
}
fn host() -> Host {
    let mut h = Host::new(Config::current(), false);
    configure(&mut h);
    h.start();
    h
}
fn gui_host() -> Bridge {
    gui_host_with("light", 760)
}
fn gui_host_with(theme: &str, width: u32) -> Bridge {
    let mut h = Bridge::new(Config::current(), false);
    configure_with(&mut h.host, theme, width);
    h.start();
    h.settled(|_| true);
    h
}
fn show_mode(h: &Bridge, mode: &str) -> Value {
    assert_eq!(
        h.rpc(&json!({"op":"show","mode":mode,"query":""}))["ok"],
        true
    );
    h.settled(|s| s["nativeUI"]["visible"] == true && s["nativeUI"]["systemSearchBusy"] == false);
    h.probe("read", json!({}))
}
fn capture(h: &Bridge, name: &str) -> Value {
    h.probe("screenshot", json!({}));
    let out = actions::native_bridge::runtime_host::root()
        .join("rust-native/target/system-search-evidence");
    fs::create_dir_all(&out).unwrap();
    fs::copy(
        h.profile.join("native-probe.png"),
        out.join(format!("{name}.png")),
    )
    .unwrap();
    let v = h.probe("read", json!({}));
    fs::write(
        out.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&v).unwrap(),
    )
    .unwrap();
    v
}
#[test]
fn native_modes_return_and_hidden_renders() {
    for theme in ["light", "dark"] {
        for width in [600, 760] {
            let h = gui_host_with(theme, width);
            for (scope, expected, source) in [
                (
                    "tabs",
                    "Search Safari tabs…",
                    "https://example.invalid/owned",
                ),
                ("windows", "Search windows…", "Owned Editor"),
                ("menus", "Search app menu…", "Owned Editor › File"),
                ("shortcuts", "Search Apple Shortcuts…", "eeeeeeee"),
            ] {
                let v = show_mode(&h, scope);
                assert_eq!(v["windowShown"], false);
                assert!(v["accessibilityText"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(expected)));
                assert!(
                    v["screenText"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|s| s.as_str().unwrap().contains(source)),
                    "{}",
                    v["screenText"]
                );
                if scope == "menus" {
                    assert!(v["accessibilityText"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|s| s.as_str().unwrap().contains("Command + O")));
                }
                capture(&h, &format!("{scope}-{theme}-{width}"));
            }
        }
    }
}
#[test]
fn native_return_and_refresh_permission_recovery() {
    let h = gui_host();
    for (scope, expected) in [
        ("windows", json!(["activate", "AXRaise"])),
        ("menus", json!(["activate", "AXPress"])),
        ("tabs", json!(["select-tab", "front-window", "activate"])),
        (
            "shortcuts",
            json!(["shortcut:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"]),
        ),
    ] {
        fixture(&h, "clear-effects", json!({}));
        show_mode(&h, scope);
        h.probe("key-event", json!({"code":36,"chars":"\r","flags":0}));
        h.wait(|s| s["nativeUI"]["visible"] == false && s["nativeUI"]["systemSearchBusy"] == false);
        assert_eq!(effects(&h), expected);
        assert_eq!(h.probe("read", json!({}))["windowShown"], false);
    }
    fixture(&h, "trust", json!({"value":false}));
    let v = show_mode(&h, "windows");
    assert!(v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("Accessibility")));
    capture(&h, "windows-permission");
    fixture(&h, "trust", json!({"value":true}));
    action(&h, "Refresh");
    h.settled(|s| s["nativeUI"]["systemSearchBusy"] == false);
    assert_eq!(h.probe("read", json!({}))["rows"], 2);
    show_mode(&h, "tabs");
    fixture(&h, "clear-effects", json!({}));
    action(&h, "Allow Safari Access");
    h.settled(|s| s["nativeUI"]["systemSearchBusy"] == false);
    assert_eq!(effects(&h), json!(["consent"]));
}
#[test]
fn native_shortcut_alias_override_and_reopen() {
    let h = gui_host();
    show_mode(&h, "shortcuts");
    action(&h, "Set Alias");
    view(&h, |v| v["formVisible"] == true);
    h.probe("alias-input", json!({"text":"savedshort"}));
    h.probe("save-editor", json!({}));
    view(&h, |v| v["formVisible"] == false);
    h.settled(|_| true);
    let cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    let saved = cfg["commands"].as_array().unwrap().last().unwrap();
    assert_eq!(
        saved["id"],
        "apple-shortcut.aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
    );
    assert_eq!(saved["aliases"], json!(["savedshort"]));
    assert_eq!(
        search(&h, "shortcuts", "savedshort")["result"][0]["id"],
        saved["id"]
    );
}
#[test]
fn superseded_discovery_config_change_and_cli_disconnect_cannot_publish() {
    let h = gui_host();
    fixture(&h, "delay", json!({"milliseconds":150}));
    assert_eq!(h.rpc(&json!({"op":"show","mode":"windows"}))["ok"], true);
    assert_eq!(
        h.rpc(&json!({"op":"show","mode":"commands","query":"missing-owned-term"}))["ok"],
        true
    );
    fixture(&h, "delay", json!({"milliseconds":0}));
    h.settled(|s| s["nativeUI"]["systemSearchBusy"] == false);
    assert_eq!(h.probe("read", json!({}))["rows"], 0);
    fixture(&h, "delay", json!({"milliseconds":200}));
    let mut stream = h.connect(Duration::from_secs(3));
    stream
        .write_all(b"{\"op\":\"search\",\"mode\":\"windows\"}\n")
        .unwrap();
    drop(stream);
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    cfg["theme"] = json!("dark");
    h.replace(&cfg, None);
    h.rpc(&json!({"op":"reload"}));
    fixture(&h, "delay", json!({"milliseconds":0}));
    assert_eq!(h.rpc(&json!({"op":"status"}))["ok"], true);
    assert_eq!(effects(&h), json!([]));
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
