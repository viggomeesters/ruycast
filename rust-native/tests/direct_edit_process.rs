#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod native_actions;
use native_actions::*;
use native_bridge::{runtime_host::{default_config, Config}, Bridge};
use serde_json::json;

#[test]
fn command_e_opens_selected_snippet_from_list_and_menu() {
    let mut h = Bridge::new(Config::current(), false);
    let mut fixture = default_config();
    fixture["commands"] = json!([
        {"id":"snippet.first","title":"Fixture Alpha","kind":"snippet","target":"One"},
        {"id":"snippet.second","title":"Fixture Beta","kind":"snippet","target":"Two"}
    ]);
    h.replace(&fixture, None);
    h.start();
    h.settled(|_| true);
    show(&h, "Fixture");
    view(&h, |v| v["rows"].as_u64().is_some_and(|n| n >= 2));
    h.probe("select", json!({"row": 1}));
    let selected = view(&h, |v| v["selectedId"] == "snippet.second");
    assert_eq!(selected["input"], "Fixture");
    key(&h, 14, CMD, "e", true);
    assert_eq!(probe(&h, "read")["formVisible"], false);
    key(&h, 14, CMD, "e", false);
    let editor = view(&h, |v| v["formVisible"] == true);
    assert_eq!(editor["editorId"], "snippet.second");
    key(&h, 14, CMD, "e", false);
    assert_eq!(probe(&h, "read")["editorToken"], editor["editorToken"]);
    key(&h, 53, 0, "", false);
    let restored = saved(&h);
    assert_eq!(restored["selectedId"], "snippet.second");
    assert_eq!(restored["input"], "Fixture");
    key(&h, 40, CMD, "k", false);
    let menu = view(&h, |v| v["actionsVisible"] == true);
    assert!(menu["actionLabels"].as_array().unwrap().iter().any(|v| {
        v.as_str().is_some_and(|s| s.starts_with("Edit") && s.contains("⌘") && s.contains('E'))
    }));
    key(&h, 14, CMD, "e", false);
    let editor = view(&h, |v| v["formVisible"] == true);
    assert_eq!(editor["editorId"], "snippet.second");
    key(&h, 53, 0, "", false);
    saved(&h);
    show(&h, "no-such-snippet-unique");
    view(&h, |v| v["rows"] == 0);
    key(&h, 14, CMD, "e", false);
    assert_eq!(probe(&h, "read")["formVisible"], false);
}
