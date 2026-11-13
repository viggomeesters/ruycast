//! Creation actions on owned hidden native profiles.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::{
    native_bridge::{runtime_host::default_config, Bridge},
    *,
};
use serde_json::{json, Value};
use std::{fs, time::Duration};
fn probe(h: &Bridge, action: &str) -> Value {
    actions::probe(h, action)
}
fn field(h: &Bridge, key: &str, text: &str) {
    h.probe("command-input", json!({"key":key,"text":text}));
}
fn rows(h: &Bridge) -> Vec<Value> {
    actions::config(h)["commands"].as_array().unwrap().clone()
}
fn new_command(h: &Bridge, title: &str) -> Value {
    action(h, title);
    view(h, |v| v["formVisible"] == true && text_contains(v, "Name"));
    probe(h, "command-read")["value"].clone()
}
fn press(h: &Bridge, code: u32, flags: u64, chars: &str) {
    key(h, code, flags, chars, false);
}
fn action_index(value: &Value, label: &str) -> usize {
    value["actionLabels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|v| v == label)
        .unwrap()
}
fn start_slow(h: &Bridge) {
    show(h, "finder");
    press(h, 40, CMD, "k");
    let v = view(h, |v| v["actionsVisible"] == true);
    probe(h, "arm-slow-prefill");
    h.probe(
        "action-button",
        json!({"index":action_index(&v,"New Quicklink…")}),
    );
    h.wait(|s| s["nativeUI"]["creationPreparing"] == true);
    assert_eq!(probe(h, "read")["formVisible"], false);
}

#[test]
fn empty_results_keep_all_three_creation_actions_available() {
    let h = host();
    show(&h, "not-found-Ω");
    let before = fs::read(&h.config).unwrap();
    press(&h, 40, CMD, "k");
    let v = view(&h, |v| v["actionsVisible"] == true);
    assert_eq!(v["selectedRow"], -1);
    for name in ["New Snippet…", "New Quicklink…", "New Shell Command…"] {
        assert!(v["actionLabels"].as_array().unwrap().contains(&json!(name)));
    }
    h.capture("create-empty-menu");
    new_command(&h, "New Snippet");
    probe(&h, "cancel-editor");
    saved(&h);
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn snippet_is_created_only_after_save_and_resolves_by_alias() {
    let h = host();
    show(&h, "finder");
    let before = fs::read(&h.config).unwrap();
    let count = rows(&h).len();
    let draft = new_command(&h, "New Snippet");
    assert!(draft["id"].as_str().unwrap().starts_with("snippet."));
    assert_eq!(fs::read(&h.config).unwrap(), before);
    for (key, text) in [
        ("title", "My Snippet"),
        ("target", "Hello\nUnicode büro"),
        ("aliases", "mynew"),
        ("keyword", ";new"),
        ("snippetOutput", "teams"),
    ] {
        field(&h, key, text);
    }
    h.capture("create-snippet");
    probe(&h, "save-editor");
    saved(&h);
    let commands = rows(&h);
    assert_eq!(commands.len(), count + 1);
    let created = commands.iter().find(|c| c["id"] == draft["id"]).unwrap();
    assert_eq!(created["target"], "Hello\nUnicode büro");
    assert_eq!(created["snippetOutput"], "teams");
    assert_eq!(h.cli(&["search", "mynew"])["result"][0]["id"], draft["id"]);
    probe(&h, "save-editor");
    assert_eq!(rows(&h).len(), count + 1);
}
#[test]
fn validation_retains_new_identity_then_repairs() {
    let h = host();
    show(&h, "finder");
    let before = fs::read(&h.config).unwrap();
    let draft = new_command(&h, "New Snippet");
    probe(&h, "save-editor");
    view(&h, |v| text_contains(v, "need id, title and target"));
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(probe(&h, "command-read")["value"]["id"], draft["id"]);
    field(&h, "target", "Valid");
    probe(&h, "save-editor");
    saved(&h);
    assert_eq!(rows(&h).last().unwrap()["id"], draft["id"]);
}
#[test]
fn quicklinks_normalize_domain_and_file_path_without_hidden_general_clipboard() {
    let h = host();
    show(&h, "finder");
    let draft = new_command(&h, "New Quicklink");
    assert_eq!(draft["target"], "https://");
    field(&h, "target", "example.org/path");
    probe(&h, "save-editor");
    saved(&h);
    assert_eq!(
        rows(&h).last().unwrap()["target"],
        "https://example.org/path"
    );
    let draft = new_command(&h, "New Quicklink");
    field(&h, "target", "file:///tmp/hello%20world");
    probe(&h, "save-editor");
    saved(&h);
    let commands = rows(&h);
    let created = commands.iter().find(|c| c["id"] == draft["id"]).unwrap();
    assert_eq!(created["kind"], "path");
    assert_eq!(created["target"], "/tmp/hello world");
}
#[test]
fn shell_command_arguments_and_draft_hotkey_are_literal() {
    let h = host();
    show(&h, "finder");
    let before = fs::read(&h.config).unwrap();
    let draft = new_command(&h, "New Shell Command");
    probe(&h, "add-argument");
    h.probe(
        "command-argument",
        json!({"index":0,"text":"one\ntwo $(no expansion)"}),
    );
    probe(&h, "record");
    view(&h, |v| v["recording"] == true);
    press(&h, 45, CMD, "n");
    assert_eq!(fs::read(&h.config).unwrap(), before);
    probe(&h, "save-editor");
    saved(&h);
    let commands = rows(&h);
    let created = commands.iter().find(|c| c["id"] == draft["id"]).unwrap();
    assert_eq!(created["arguments"], json!(["one\ntwo $(no expansion)"]));
    assert_eq!(created["hotkey"], "cmd+n");
}
#[test]
fn external_changes_survive_and_identity_collision_never_overwrites() {
    let h = host();
    show(&h, "finder");
    new_command(&h, "New Shell Command");
    let mut cfg = config(&h);
    cfg["clipboard"]["limit"] = json!(92);
    h.replace(&cfg, None);
    probe(&h, "save-editor");
    saved(&h);
    assert_eq!(config(&h)["clipboard"]["limit"], 92);
    let draft = new_command(&h, "New Shell Command");
    let mut cfg = config(&h);
    let mut external = draft;
    external["title"] = json!("External owner");
    cfg["commands"].as_array_mut().unwrap().push(external);
    h.replace(&cfg, None);
    let before = fs::read(&h.config).unwrap();
    probe(&h, "save-editor");
    view(&h, |v| text_contains(v, "identity already exists"));
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn full_creation_menu_light_dark_and_maximum_actions_fit() {
    let h = host();
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"snippet.fixture","title":"Snippet fixture","kind":"snippet","target":"Text","aliases":[]}));
    for theme in ["light", "dark"] {
        cfg["theme"] = json!(theme);
        h.replace(&cfg, None);
        assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
        h.settled(|s| s["theme"] == theme);
        show(&h, "snippet.fixture");
        press(&h, 40, CMD, "k");
        let v = view(&h, |v| v["actionsVisible"] == true);
        assert_eq!(v["actionLabels"].as_array().unwrap().len(), 11);
        h.capture(&format!("create-full-menu-{theme}"));
        press(&h, 53, 0, "");
    }
}
#[test]
fn queued_creation_menu_cancel_during_busy_owner_stays_cancelled() {
    let h = host();
    show(&h, "finder");
    let before = fs::read(&h.config).unwrap();
    press(&h, 40, CMD, "k");
    let value = view(&h, |v| v["actionsVisible"] == true);
    let index = action_index(&value, "New Snippet…");
    for _ in 0..index {
        press(&h, 125, 0, "");
    }
    h.settled(|_| true);
    probe(&h, "schedule-menu-cancel");
    assert_eq!(h.rpc(&json!({"op":"native.delay"}))["ok"], true);
    let value = view(&h, |v| v["actionsVisible"] == false);
    assert_eq!(value["formVisible"], false);
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn menu_escape_during_in_progress_prefill_does_not_open_a_zombie_editor() {
    let h = host();
    show(&h, "finder");
    let before = fs::read(&h.config).unwrap();
    press(&h, 40, CMD, "k");
    let value = view(&h, |v| v["actionsVisible"] == true);
    let index = action_index(&value, "New Quicklink…");
    probe(&h, "arm-slow-prefill");
    probe(&h, "schedule-menu-escape");
    h.probe("action-button", json!({"index":index}));
    h.settled(|_| true);
    let value = view(&h, |v| v["actionsVisible"] == false);
    assert_eq!(value["prefillTestRuns"], 1);
    assert_eq!(value["formVisible"], false);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    new_command(&h, "New Snippet");
    probe(&h, "cancel-editor");
    saved(&h);
}
#[test]
fn pending_prefill_keeps_owner_responsive_and_new_query_retires_draft() {
    let h = host();
    let before = fs::read(&h.config).unwrap();
    start_slow(&h);
    assert_eq!(h.rpc(&json!({"op":"commands"}))["ok"], true);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["creationPreparing"],
        true
    );
    h.probe("query", json!({"query":"safari"}));
    h.wait(|s| s["nativeUI"]["creationPreparing"] == false);
    h.settled(|_| true);
    assert_eq!(probe(&h, "read")["formVisible"], false);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    show(&h, "finder");
    new_command(&h, "New Snippet");
    probe(&h, "cancel-editor");
    saved(&h);
}
#[test]
fn pending_prefill_config_change_cancels_without_a_late_form() {
    let h = host();
    start_slow(&h);
    let mut cfg = config(&h);
    cfg["theme"] = json!("light");
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    h.wait(|s| s["nativeUI"]["creationPreparing"] == false);
    h.settled(|_| true);
    assert_eq!(probe(&h, "read")["formVisible"], false);
    assert_eq!(config(&h), cfg);
}
#[test]
fn pending_prefill_does_not_hold_shutdown_open() {
    let mut h = host();
    start_slow(&h);
    assert_eq!(h.rpc(&json!({"op":"quit"}))["ok"], true);
    let output = h.wait_exit(Duration::from_secs(3), false);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn cancelled_origin_filters_an_already_published_creation_view() {
    let h = host();
    show(&h, "finder");
    let before = fs::read(&h.config).unwrap();
    new_command(&h, "New Snippet");
    probe(&h, "cancel-creation-origin");
    saved(&h);
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
