//! Native editor cases on owned hidden Actions fixtures.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_editor.rs"]
mod native_editor;
use native_editor::{
    actions::{
        self,
        native_bridge::{runtime_host::default_config, Bridge},
        *,
    },
    edit, field,
};
use serde_json::{json, Value};
use std::fs;
fn p(h: &Bridge, action: &str) -> Value {
    actions::probe(h, action)
}
fn press(h: &Bridge, code: u32, flags: u64, chars: &str) {
    key(h, code, flags, chars, false);
}
fn fixture(h: &Bridge, command: Value) {
    let mut cfg = default_config();
    cfg["commands"]
        .as_array_mut()
        .unwrap()
        .push(command.clone());
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    show(h, command["id"].as_str().unwrap());
}
fn text_lower(v: &Value, needle: &str) -> bool {
    v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().to_lowercase().contains(needle))
}
#[test]
fn name_alias_and_native_typing_save_and_cancel() {
    let h = host();
    show(&h, "finder");
    let before = fs::read(&h.config).unwrap();
    edit(&h);
    assert!(!p(&h, "read")["screenText"]
        .as_array()
        .unwrap()
        .contains(&json!("Enabled")));
    h.probe("command-type", json!({"text":"Finder Renamed"}));
    field(&h, "aliases", "myfinder, büro");
    h.capture("edit-app");
    press(&h, 36, CMD, "");
    saved(&h);
    assert_eq!(command(&h, "app.finder")["title"], "Finder Renamed");
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["myfinder", "büro"])
    );
    edit(&h);
    field(&h, "title", "Must not save");
    p(&h, "cancel-editor");
    saved(&h);
    assert_eq!(command(&h, "app.finder")["title"], "Finder Renamed");
    assert_ne!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn validation_preserves_full_draft_and_native_name_focus() {
    let h = host();
    show(&h, "finder");
    edit(&h);
    let before = fs::read(&h.config).unwrap();
    field(&h, "aliases", "safari");
    p(&h, "save-editor");
    view(&h, |v| text_lower(v, "duplicate"));
    assert_eq!(fs::read(&h.config).unwrap(), before);
    h.capture("edit-validation");
    h.probe("command-type", json!({"text":"After rejection"}));
    assert_eq!(p(&h, "command-read")["value"]["title"], "After rejection");
    field(&h, "aliases", "repaired");
    p(&h, "save-editor");
    saved(&h);
    assert_eq!(command(&h, "app.finder")["title"], "After rejection");
}
#[test]
fn external_change_merges_unedited_fields_and_conflict_keeps_draft() {
    let h = host();
    show(&h, "finder");
    edit(&h);
    field(&h, "title", "Local name");
    let mut cfg = config(&h);
    cfg["commands"][0]["aliases"] = json!(["external"]);
    cfg["clipboard"]["limit"] = json!(87);
    h.replace(&cfg, None);
    p(&h, "save-editor");
    saved(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!(["external"]));
    assert_eq!(config(&h)["clipboard"]["limit"], 87);
    edit(&h);
    field(&h, "title", "My draft");
    let mut cfg = config(&h);
    cfg["commands"][0]["title"] = json!("Outside");
    h.replace(&cfg, None);
    let before = fs::read(&h.config).unwrap();
    p(&h, "save-editor");
    view(&h, |v| text_contains(v, "changed elsewhere"));
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(p(&h, "command-read")["value"]["title"], "My draft");
}
#[test]
fn script_arguments_are_independent_literal_multiline_values() {
    let h = host();
    let args = [
        "first\nsecond",
        "$(touch /tmp/never-execute)",
        "büro \"quoted\"",
    ];
    fixture(
        &h,
        json!({"id":"script.fixture","title":"Script fixture","kind":"script","target":"/usr/bin/printf","arguments":args,"aliases":[]}),
    );
    edit(&h);
    h.probe("command-scroll", json!({"y":440}));
    h.capture("edit-script-arguments");
    p(&h, "add-argument");
    h.probe("command-argument", json!({"index":3,"text":"extra\nline"}));
    h.probe("remove-argument", json!({"index":1}));
    field(&h, "timeout", "45.5");
    p(&h, "save-editor");
    saved(&h);
    let cmd = command(&h, "script.fixture");
    assert_eq!(cmd["arguments"], json!([args[0], args[2], "extra\nline"]));
    assert_eq!(cmd["timeout"], 45.5);
}
#[test]
fn snippet_text_output_keyword_preserved() {
    let h = host();
    fixture(
        &h,
        json!({"id":"snippet.fixture","title":"Snippet fixture","kind":"snippet","target":"Old text","aliases":[],"keyword":";old"}),
    );
    edit(&h);
    let text = "Hello [world](https://example.com)\n\nUnicode büro 日本語";
    field(&h, "target", text);
    field(&h, "snippetOutput", "teams");
    field(&h, "keyword", ";new");
    h.capture("edit-snippet-top");
    h.probe("command-scroll", json!({"y":200}));
    h.capture("edit-snippet");
    p(&h, "save-editor");
    saved(&h);
    let cmd = command(&h, "snippet.fixture");
    assert_eq!(cmd["target"], text);
    assert_eq!(cmd["snippetOutput"], "teams");
    assert_eq!(cmd["keyword"], ";new");
}
#[test]
fn recording_changes_draft_only_and_escape_stops_without_closing() {
    let h = host();
    show(&h, "finder");
    edit(&h);
    let before = fs::read(&h.config).unwrap();
    p(&h, "record");
    view(&h, |v| v["recording"] == true);
    press(&h, 53, 0, "");
    assert_eq!(p(&h, "read")["recording"], false);
    assert_eq!(p(&h, "read")["formVisible"], true);
    p(&h, "record");
    press(&h, 45, CMD, "n");
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(p(&h, "command-read")["value"]["hotkey"], "cmd+n");
    p(&h, "save-editor");
    saved(&h);
    assert_eq!(command(&h, "app.finder")["hotkey"], "cmd+n");
}
#[test]
fn quicklink_domain_and_file_url_normalize() {
    let h = host();
    fixture(
        &h,
        json!({"id":"link.fixture","title":"Link fixture","kind":"url","target":"https://example.com","aliases":[]}),
    );
    edit(&h);
    field(&h, "target", "example.org/path");
    p(&h, "save-editor");
    saved(&h);
    assert_eq!(
        command(&h, "link.fixture")["target"],
        "https://example.org/path"
    );
    edit(&h);
    field(&h, "target", "file:///tmp/hello%20world");
    p(&h, "save-editor");
    saved(&h);
    let cmd = command(&h, "link.fixture");
    assert_eq!(cmd["kind"], "path");
    assert_eq!(cmd["target"], "/tmp/hello world");
}
#[test]
fn noop_save_preserves_explicit_plain_and_empty_optional_fields() {
    let h = host();
    fixture(
        &h,
        json!({"id":"snippet.fixture","title":"Snippet fixture","kind":"snippet","target":"Text","snippetOutput":"plain","inputPrompt":"","aliases":["a,b"]}),
    );
    edit(&h);
    let before = config(&h);
    p(&h, "save-editor");
    saved(&h);
    assert_eq!(config(&h), before);
}
#[test]
fn snippet_undo_is_local_to_the_opened_draft() {
    let h = host();
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().extend([json!({"id":"snippet.a","title":"Snippet A","kind":"snippet","target":"First","aliases":[]}),json!({"id":"snippet.b","title":"Snippet B","kind":"snippet","target":"Second","aliases":[]})]);
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    show(&h, "snippet.a");
    edit(&h);
    h.probe("snippet-type", json!({"text":"PRIVATE-DRAFT"}));
    assert_eq!(p(&h, "snippet-undo-state")["canUndo"], true);
    p(&h, "cancel-editor");
    saved(&h);
    show(&h, "snippet.b");
    edit(&h);
    assert_eq!(p(&h, "snippet-undo-state")["canUndo"], false);
    assert_eq!(p(&h, "command-read")["value"]["target"], "Second");
}
#[test]
fn native_composition_and_multiline_escape_routes() {
    let h = host();
    show(&h, "finder");
    edit(&h);
    let before = fs::read(&h.config).unwrap();
    p(&h, "command-ime");
    assert_eq!(p(&h, "read")["formVisible"], true);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    p(&h, "cancel-editor");
    saved(&h);
    for (command, argument) in [
        (
            json!({"id":"snippet.fixture","title":"Snippet","kind":"snippet","target":"Text","aliases":[]}),
            Value::Null,
        ),
        (
            json!({"id":"script.fixture","title":"Script","kind":"script","target":"/usr/bin/printf","arguments":["one"],"aliases":[]}),
            json!(0),
        ),
    ] {
        fixture(&h, command);
        edit(&h);
        h.probe(
            "editor-text-command",
            json!({"marked":true,"argument":argument}),
        );
        assert_eq!(p(&h, "read")["formVisible"], true);
        h.probe(
            "editor-text-command",
            json!({"marked":false,"argument":argument}),
        );
        saved(&h);
    }
}
#[test]
fn alias_form_noop_preserves_literal_comma_alias() {
    let h = host();
    let mut cfg = default_config();
    cfg["commands"][0]["aliases"] = json!(["literal,alias"]);
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    show(&h, "finder");
    alias(&h);
    p(&h, "save-editor");
    saved(&h);
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["literal,alias"])
    );
}
#[test]
fn full_editor_queued_save_cancel_does_not_publish() {
    let h = host();
    show(&h, "finder");
    edit(&h);
    let before = fs::read(&h.config).unwrap();
    field(&h, "title", "Must not publish");
    p(&h, "schedule-save-cancel");
    assert_eq!(h.rpc(&json!({"op":"native.delay"}))["ok"], true);
    saved(&h);
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn detached_old_argument_cannot_cancel_a_new_editor() {
    let h = host();
    fixture(
        &h,
        json!({"id":"script.fixture","title":"Script","kind":"script","target":"/usr/bin/printf","arguments":["one"],"aliases":[]}),
    );
    edit(&h);
    p(&h, "retain-argument");
    p(&h, "cancel-editor");
    saved(&h);
    show(&h, "finder");
    edit(&h);
    let token = p(&h, "read")["editorToken"].clone();
    p(&h, "old-text-command");
    assert_eq!(p(&h, "read")["formVisible"], true);
    assert_eq!(p(&h, "read")["editorToken"], token);
}
#[test]
fn long_editor_heading_and_scrolled_fields_in_light_and_dark() {
    let h = host();
    for theme in ["light", "dark"] {
        let mut cfg = default_config();
        cfg["theme"] = json!(theme);
        cfg["commands"][0]["title"] =
            json!(format!("Finder — {}", "Unicode büro 日本語 ".repeat(12)));
        h.replace(&cfg, None);
        assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
        h.settled(|s| s["theme"] == theme);
        show(&h, "finder");
        edit(&h);
        h.capture(&format!("edit-long-{theme}"));
        let value = h.probe("command-scroll", json!({"y":260}));
        assert!(value["commandScrollY"].as_f64().unwrap() >= 0.0);
        assert!(
            value["commandScrollY"].as_f64().unwrap()
                <= value["commandScrollMaxY"].as_f64().unwrap()
        );
        h.capture(&format!("edit-bottom-{theme}"));
        p(&h, "cancel-editor");
        saved(&h);
    }
}
