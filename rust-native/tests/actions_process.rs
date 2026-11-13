//! Original sixteen native Actions cases using shared owned Rust fixtures.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod native_actions;
use native_actions::*;
use native_bridge::{runtime_host::default_config, Bridge};
use serde_json::{json, Value};
use std::{fs, os::fd::AsRawFd};
fn p(h: &Bridge, action: &str, fields: Value) -> Value {
    h.probe(action, fields)
}
fn k(h: &Bridge, code: u32, flags: u64, chars: &str) -> Value {
    key(h, code, flags, chars, false)
}
fn cancel(h: &Bridge) {
    probe(h, "cancel-editor");
    saved(h);
}
fn lower_contains(v: &Value, expected: &str) -> bool {
    v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().to_lowercase().contains(expected))
}

#[test]
fn actions_keyboard_badges_and_escape_keeps_palette() {
    let h = host();
    show(&h, "finder");
    k(&h, 40, CMD, "k");
    let value = view(&h, |v| v["actionsVisible"] == true);
    for (label, badge) in [("Set Alias…", "⌥"), ("Set Hotkey…", "⇧")] {
        assert!(value["actionLabels"].as_array().unwrap().iter().any(|v| v
            .as_str()
            .unwrap()
            .starts_with(label)
            && v.as_str().unwrap().contains(badge)));
    }
    h.capture("actions-menu");
    k(&h, 53, 0, "");
    view(&h, |v| v["actionsVisible"] == false);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        true
    );
    key(&h, 43, CMD | OPT, ",", true);
    assert_eq!(probe(&h, "read")["formVisible"], false);
    alias(&h);
    k(&h, 53, 0, "");
    saved(&h);
    k(&h, 40, CMD, "k");
    view(&h, |v| v["actionsVisible"] == true);
    k(&h, 125, 0, "");
    k(&h, 125, 0, "");
    k(&h, 36, 0, "");
    view(&h, |v| {
        v["formVisible"] == true && text_contains(v, "Alias")
    });
    k(&h, 53, 0, "");
    saved(&h);
}
#[test]
fn alias_save_clear_and_cli_resolution() {
    let h = host();
    show(&h, "finder");
    alias(&h);
    p(&h, "alias-input", json!({"text":"myfinder, büro"}));
    h.capture("alias-edit");
    k(&h, 36, CMD, "\r");
    saved(&h);
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["myfinder", "büro"])
    );
    assert_eq!(h.cli(&["search", "büro"])["result"][0]["id"], "app.finder");
    alias(&h);
    p(&h, "alias-input", json!({"text":""}));
    probe(&h, "save-editor");
    saved(&h);
    assert_eq!(command(&h, "app.finder")["aliases"], json!([]));
}
#[test]
fn alias_conflict_keeps_draft_and_allows_repair() {
    let h = host();
    show(&h, "finder");
    alias(&h);
    let before = fs::read(&h.config).unwrap();
    p(&h, "alias-input", json!({"text":"SAFARI"}));
    probe(&h, "save-editor");
    let value = view(&h, |v| lower_contains(v, "duplicate"));
    assert_eq!(value["aliasValue"], "SAFARI");
    assert_eq!(fs::read(&h.config).unwrap(), before);
    h.capture("alias-conflict");
    p(&h, "alias-type", json!({"text":"x"}));
    assert!(probe(&h, "read")["aliasValue"]
        .as_str()
        .unwrap()
        .contains('x'));
    p(&h, "alias-input", json!({"text":"repairedfinder"}));
    probe(&h, "save-editor");
    saved(&h);
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["repairedfinder"])
    );
}
#[test]
fn failed_async_save_restores_native_field_focus_and_undo() {
    let h = host();
    show(&h, "finder");
    alias(&h);
    let before = fs::read(&h.config).unwrap();
    p(&h, "alias-input", json!({"text":""}));
    p(&h, "alias-type", json!({"text":"SAFARI"}));
    assert_eq!(probe(&h, "alias-undo-state")["canUndo"], true);
    probe(&h, "save-editor");
    view(&h, |v| {
        v["saveEnabled"] == true && lower_contains(v, "duplicate")
    });
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(probe(&h, "alias-undo-state")["canUndo"], true);
    k(&h, 6, CMD, "z");
    assert_ne!(probe(&h, "read")["aliasValue"], "SAFARI");
    k(&h, 6, CMD | SHIFT, "z");
    assert_eq!(probe(&h, "read")["aliasValue"], "SAFARI");
}
#[test]
fn other_external_fields_merge_but_same_alias_change_conflicts() {
    let h = host();
    show(&h, "finder");
    alias(&h);
    let mut value = config(&h);
    value["clipboard"]["limit"] = json!(77);
    value["commands"][0]["title"] = json!("Externally renamed Finder");
    value["commands"][1]["title"] = json!("Other title");
    h.replace(&value, None);
    p(&h, "alias-input", json!({"text":"mine"}));
    probe(&h, "save-editor");
    saved(&h);
    let mut current = config(&h);
    assert_eq!(current["clipboard"]["limit"], 77);
    assert_eq!(current["commands"][0]["title"], "Externally renamed Finder");
    assert_eq!(current["commands"][1]["title"], "Other title");
    alias(&h);
    current["commands"][0]["aliases"] = json!(["outside"]);
    h.replace(&current, None);
    let before = fs::read(&h.config).unwrap();
    p(&h, "alias-input", json!({"text":"draft"}));
    probe(&h, "save-editor");
    view(&h, |v| text_contains(v, "changed elsewhere"));
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(probe(&h, "read")["aliasValue"], "draft");
}
#[test]
fn deleted_item_and_invalid_schema_are_not_overwritten() {
    let h = host();
    show(&h, "finder");
    alias(&h);
    let mut value = config(&h);
    value["commands"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["id"] != "app.finder");
    h.replace(&value, None);
    let before = fs::read(&h.config).unwrap();
    p(&h, "alias-input", json!({"text":"mine"}));
    probe(&h, "save-editor");
    view(&h, |v| text_contains(v, "removed"));
    assert_eq!(fs::read(&h.config).unwrap(), before);
    cancel(&h);
    h.replace(&default_config(), None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    show(&h, "finder");
    alias(&h);
    let mut value = default_config();
    value["commands"][0]["unknownField"] = json!("PRIVATE-SENTINEL");
    h.replace(&value, None);
    let before = fs::read(&h.config).unwrap();
    p(&h, "alias-input", json!({"text":"mine"}));
    probe(&h, "save-editor");
    view(&h, |v| text_contains(v, "unknown field"));
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert!(!probe(&h, "read").to_string().contains("PRIVATE-SENTINEL"));
}
#[test]
fn hotkey_sides_hyper_delete_and_repeat() {
    let h = host();
    show(&h, "finder");
    let mut value = config(&h);
    value["separateCommandKeys"] = json!(true);
    h.replace(&value, None);
    h.settled(|s| s["separateCommandKeys"] == true);
    hotkey(&h);
    let before = fs::read(&h.config).unwrap();
    key(&h, 43, CMD | SHIFT, ",", true);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    h.capture("hotkey-recording");
    k(&h, 45, CMD | SHIFT | 0x10, "different layout");
    saved(&h);
    assert_eq!(command(&h, "app.finder")["hotkey"], "shift+right-cmd+n");
    for (flags, expected) in [
        (CMD | 0x8, "left-cmd+n"),
        (CMD | CTRL | SHIFT | OPT | 0x10, "ctrl+alt+shift+cmd+n"),
    ] {
        hotkey(&h);
        k(&h, 45, flags, "n");
        saved(&h);
        assert_eq!(command(&h, "app.finder")["hotkey"], expected);
    }
    hotkey(&h);
    k(&h, 51, 0, "");
    saved(&h);
    assert!(command(&h, "app.finder").get("hotkey").is_none());
    hotkey(&h);
    k(&h, 9, CMD, "v");
    saved(&h);
    assert_eq!(command(&h, "app.finder")["hotkey"], "cmd+v");
}
#[test]
fn hotkey_conflict_and_solo_modifiers() {
    let h = host();
    show(&h, "finder");
    hotkey(&h);
    let before = fs::read(&h.config).unwrap();
    k(&h, 9, CTRL | OPT, "v");
    view(&h, |v| text_contains(v, "Duplicate hotkey"));
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(probe(&h, "read")["recording"], false);
    h.capture("hotkey-conflict");
    for (code, flags, expected) in [
        (59, CTRL, "ctrl+left-control"),
        (58, OPT, "alt+left-option"),
    ] {
        if probe(&h, "read")["formVisible"] != true {
            hotkey(&h);
        } else {
            probe(&h, "record");
            view(&h, |v| v["recording"] == true);
        }
        p(&h, "flags-event", json!({"code":code,"flags":flags}));
        p(&h, "flags-event", json!({"code":code,"flags":0}));
        saved(&h);
        assert_eq!(command(&h, "app.finder")["hotkey"], expected);
    }
}
#[test]
fn recorder_stops_on_window_app_resign_unfocus_hide_and_cancel() {
    let h = host();
    show(&h, "finder");
    for event in ["resign-window", "resign-app", "unfocus"] {
        hotkey(&h);
        let before = fs::read(&h.config).unwrap();
        probe(&h, event);
        assert_eq!(probe(&h, "read")["recording"], false);
        p(&h, "flags-event", json!({"code":59,"flags":CTRL}));
        p(&h, "flags-event", json!({"code":59,"flags":0}));
        assert_eq!(fs::read(&h.config).unwrap(), before);
        cancel(&h);
    }
    hotkey(&h);
    let before = fs::read(&h.config).unwrap();
    probe(&h, "hide-palette");
    h.settled(|s| s["nativeUI"]["visible"] == false);
    assert_eq!(probe(&h, "read")["recording"], false);
    probe(&h, "record");
    assert_eq!(probe(&h, "read")["recording"], false);
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn alias_native_composition_does_not_save_or_cancel() {
    let h = host();
    show(&h, "finder");
    alias(&h);
    let before = fs::read(&h.config).unwrap();
    let token = probe(&h, "read")["editorToken"].clone();
    probe(&h, "alias-ime");
    assert_eq!(probe(&h, "read")["editorToken"], token);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    cancel(&h);
}
#[test]
fn remove_confirmation_cancel_then_commit_and_disable() {
    let h = host();
    show(&h, "finder");
    action(&h, "Remove");
    view(&h, |v| v["formVisible"] == true);
    let before = fs::read(&h.config).unwrap();
    h.capture("remove-confirmation");
    cancel(&h);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    action(&h, "Remove");
    view(&h, |v| v["formVisible"] == true);
    probe(&h, "save-editor");
    saved(&h);
    assert!(!config(&h)["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "app.finder"));
    show(&h, "safari");
    action(&h, "Disable");
    view(&h, |v| v["actionsVisible"] == false);
    h.settled(|s| s["configurationUpdating"] == false);
    assert_eq!(command(&h, "app.safari")["enabled"], false);
    assert!(!h.cli(&["search", "safari"])["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "app.safari"));
}
#[test]
fn visible_selection_is_bound_and_unseen_selection_is_refused() {
    let h = host();
    show(&h, "");
    p(&h, "select-and-alias", json!({"row":1}));
    view(&h, |v| v["formVisible"] == true);
    assert!(probe(&h, "read")["screenText"]
        .as_array()
        .unwrap()
        .contains(&json!("Safari")));
    cancel(&h);
    show(&h, "");
    p(&h, "unseen-selection-and-alias", json!({"row":1}));
    view(&h, |v| text_contains(v, "Selection changed"));
    assert_eq!(probe(&h, "read")["formVisible"], false);
}
#[test]
fn busy_config_lock_keeps_draft_and_retries() {
    let h = host();
    show(&h, "finder");
    alias(&h);
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
        p(&h, "alias-input", json!({"text":"locked-draft"}));
        probe(&h, "save-editor");
        view(&h, |v| {
            lower_contains(v, "lock") || lower_contains(v, "busy")
        });
        assert_eq!(fs::read(&h.config).unwrap(), before);
    }
    probe(&h, "save-editor");
    saved(&h);
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["locked-draft"])
    );
}
#[test]
fn queued_save_cancel_and_hide_prevent_dispatch_and_zombie_editor() {
    let h = host();
    for mode in ["schedule-save-cancel", "schedule-save-hide"] {
        show(&h, "finder");
        alias(&h);
        let token = probe(&h, "read")["editorToken"].as_u64().unwrap();
        let before = fs::read(&h.config).unwrap();
        p(&h, "alias-input", json!({"text":"must-not-save"}));
        probe(&h, mode);
        assert_eq!(h.rpc(&json!({"op":"native.delay"}))["ok"], true);
        let value = saved(&h);
        assert!(value["editorToken"].is_null());
        assert_eq!(fs::read(&h.config).unwrap(), before);
        h.settled(|s| s["nativeUI"]["visible"] == (mode == "schedule-save-cancel"));
        show(&h, "finder");
        alias(&h);
        assert!(probe(&h, "read")["editorToken"].as_u64().unwrap() > token);
        cancel(&h);
    }
}
#[test]
fn queued_menu_return_cancel_does_not_open_editor() {
    let h = host();
    show(&h, "finder");
    k(&h, 40, CMD, "k");
    view(&h, |v| v["actionsVisible"] == true);
    k(&h, 125, 0, "");
    k(&h, 125, 0, "");
    h.settled(|_| true);
    probe(&h, "schedule-menu-cancel");
    assert_eq!(h.rpc(&json!({"op":"native.delay"}))["ok"], true);
    let value = view(&h, |v| v["actionsVisible"] == false);
    assert_eq!(value["formVisible"], false);
    assert!(value["editorToken"].is_null());
    alias(&h);
    cancel(&h);
}
#[test]
fn forms_and_actions_render_with_long_titles_in_light_and_dark() {
    let h = host();
    for theme in ["light", "dark"] {
        let mut value = default_config();
        value["theme"] = json!(theme);
        value["commands"][0]["title"] = json!(format!(
            "Finder — {}",
            "Long Unicode title büro 日本語 · ".repeat(7)
        ));
        h.replace(&value, None);
        assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
        h.settled(|s| s["theme"] == theme);
        show(&h, "finder");
        k(&h, 40, CMD, "k");
        view(&h, |v| v["actionsVisible"] == true);
        h.capture(&format!("actions-long-{theme}"));
        k(&h, 53, 0, "");
        alias(&h);
        p(&h, "alias-input", json!({"text":"büro, 文件, myfinder"}));
        h.capture(&format!("alias-long-{theme}"));
        cancel(&h);
        hotkey(&h);
        h.capture(&format!("hotkey-long-{theme}"));
        cancel(&h);
    }
}
