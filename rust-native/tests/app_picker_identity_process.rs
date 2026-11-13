//! App picker identity behavior using only owned synthetic bundles.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::{
    native_bridge::{
        runtime_host::{default_config, Config},
        Bridge,
    },
    *,
};
use serde_json::{json, Value};
use std::fs;
fn host(application: Option<&str>, duplicate: bool) -> Bridge {
    host_with(Options {
        application,
        duplicate,
        ..Options::default()
    })
}
#[derive(Default)]
struct Options<'a> {
    application: Option<&'a str>,
    duplicate: bool,
    empty: bool,
    long: bool,
    default_name: bool,
    missing_name: bool,
    barrier: bool,
    theme: Option<&'a str>,
    width: Option<u32>,
}
fn host_with(options: Options<'_>) -> Bridge {
    let mut h = Bridge::new(Config::current(), false);
    h.env.insert(
        "RUYCAST_NATIVE_TEST_WIDTH".into(),
        options.width.unwrap_or(760).to_string().into(),
    );
    if options.barrier {
        h.env
            .insert("RUYCAST_NATIVE_TEST_APP_BARRIER".into(), "1".into());
    }
    for (id, title) in [
        ("dev.fixture.a", "Fixture Browser A"),
        ("dev.fixture.b", "Fixture Browser B"),
    ] {
        if options.empty {
            continue;
        }
        let contents = h.applications.join(format!("{id}.app/Contents"));
        fs::create_dir_all(&contents).unwrap();
        let name = if options.default_name && id == "dev.fixture.a" {
            "Default app".to_owned()
        } else if options.missing_name && id == "dev.fixture.a" {
            "dev.fixture.missing".to_owned()
        } else if options.duplicate {
            "Fixture Same".to_owned()
        } else {
            format!(
                "{}{}",
                title,
                if options.long {
                    " long büro 日本語 ".repeat(20)
                } else {
                    String::new()
                }
            )
        };
        let plist=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleName</key><string>{name}</string></dict></plist>");
        fs::write(contents.join("Info.plist"), plist).unwrap();
    }
    let mut cfg = default_config();
    let mut command = json!({"id":"url.fixture","title":"Fixture Link","kind":"url","target":"https://example.invalid/own","aliases":["fixtureurl"]});
    if let Some(id) = options.application {
        command["applicationID"] = json!(id);
    }
    cfg["commands"].as_array_mut().unwrap().push(command);
    if let Some(theme) = options.theme {
        cfg["theme"] = json!(theme);
    }
    h.replace(&cfg, None);
    h.start();
    if !options.barrier {
        h.wait(|s| s["appDiscoveryPending"] == false);
    }
    h.settled(|_| true);
    h
}
fn edit(h: &Bridge) -> Value {
    show(h, "fixtureurl");
    action(h, "Edit");
    view(h, |v| v["formVisible"] == true && text_contains(v, "Name"));
    h.settled(|_| true);
    h.probe("app-picker", json!({"command":"read"}))
}
fn read(h: &Bridge) -> Value {
    h.settled(|_| true);
    h.probe("app-picker", json!({"command":"read"}))
}
fn choose(h: &Bridge, index: usize) {
    h.probe("app-picker", json!({"command":"open"}));
    assert_eq!(read(h)["list"]["open"], true);
    h.probe("app-picker", json!({"command":"select","index":index}));
    key(h, 36, 0, "\r", false);
    h.settled(|_| true);
}
#[test]
fn native_app_name_selection_persists_exact_bundle_and_default_clears() {
    let h = host(None, false);
    let v = edit(&h);
    assert_eq!(v["title"], "Default app");
    assert_eq!(v["enabled"], true);
    let before = fs::read(&h.config).unwrap();
    choose(&h, 2);
    let v = read(&h);
    assert_eq!(v["value"], "dev.fixture.b");
    assert_eq!(v["title"], "Fixture Browser B");
    assert_eq!(fs::read(&h.config).unwrap(), before);
    h.capture("app-selected");
    key(&h, 36, CMD, "\r", false);
    saved(&h);
    assert_eq!(command(&h, "url.fixture")["applicationID"], "dev.fixture.b");
    edit(&h);
    choose(&h, 0);
    probe(&h, "save-editor");
    saved(&h);
    assert!(command(&h, "url.fixture").get("applicationID").is_none());
}
#[test]
fn missing_saved_id_is_retained_without_silent_first_app_fallback() {
    let h = host(Some("dev.fixture.missing"), false);
    let v = edit(&h);
    assert_eq!(v["value"], "dev.fixture.missing");
    assert_eq!(v["title"], "dev.fixture.missing");
    h.probe("app-picker", json!({"command":"open"}));
    let v = read(&h);
    assert_eq!(v["list"]["selected"], 1);
    assert_eq!(v["list"]["rows"], 4);
    key(&h, 53, 0, "", false);
    h.probe("command-input", json!({"key":"title","text":"Renamed"}));
    probe(&h, "save-editor");
    saved(&h);
    assert_eq!(
        command(&h, "url.fixture")["applicationID"],
        "dev.fixture.missing"
    );
}
#[test]
fn duplicate_names_show_bundle_ids_and_never_save_display_text() {
    let h = host(None, true);
    edit(&h);
    h.probe("app-picker", json!({"command":"open"}));
    h.capture("app-duplicates");
    let v = probe(&h, "read");
    assert!(v["screenText"]
        .as_array()
        .unwrap()
        .contains(&json!("Fixture Same — dev.fixture.a")));
    assert!(v["screenText"]
        .as_array()
        .unwrap()
        .contains(&json!("Fixture Same — dev.fixture.b")));
    h.probe("app-picker", json!({"command":"click","index":2}));
    assert_eq!(read(&h)["value"], "dev.fixture.b");
    probe(&h, "save-editor");
    saved(&h);
    assert_eq!(command(&h, "url.fixture")["applicationID"], "dev.fixture.b");
}
#[test]
fn long_app_titles_native_accessibility_and_light_dark_widths() {
    for theme in ["light", "dark"] {
        for width in [600, 760] {
            let h = host_with(Options {
                theme: Some(theme),
                width: Some(width),
                long: true,
                ..Options::default()
            });
            edit(&h);
            choose(&h, 2);
            let v = read(&h);
            assert_eq!(v["axValue"], "dev.fixture.b");
            assert!(v["title"].as_str().unwrap().chars().count() > 200);
            h.capture(&format!("app-long-{theme}-{width}"));
            h.probe("app-picker", json!({"command":"open"}));
            let screen = h.capture(&format!("app-long-open-{theme}-{width}"));
            assert!(screen["accessibilityText"]
                .as_array()
                .unwrap()
                .contains(&v["title"]));
            key(&h, 53, 0, "", false);
            probe(&h, "cancel-editor");
            saved(&h);
        }
    }
}
#[test]
fn empty_discovery_keeps_default_and_tab_closes_tentative_choice() {
    let h = host_with(Options {
        empty: true,
        ..Options::default()
    });
    edit(&h);
    h.probe("app-picker", json!({"command":"open"}));
    assert_eq!(read(&h)["list"]["rows"], 1);
    h.capture("app-empty");
    key(&h, 48, 0, "\t", false);
    assert_eq!(read(&h)["list"]["open"], false);
    assert_eq!(read(&h)["value"], "");
    assert_eq!(read(&h)["fieldAXHidden"], false);
}
#[test]
fn explicit_empty_id_is_preserved_until_default_is_explicitly_chosen() {
    let h = host(Some(""), false);
    edit(&h);
    h.probe(
        "command-input",
        json!({"key":"title","text":"Only title changed"}),
    );
    probe(&h, "save-editor");
    saved(&h);
    assert_eq!(command(&h, "url.fixture")["applicationID"], "");
    assert!(command(&h, "url.fixture").get("applicationID").is_some());
    edit(&h);
    choose(&h, 0);
    probe(&h, "save-editor");
    saved(&h);
    assert!(command(&h, "url.fixture").get("applicationID").is_none());
}
#[test]
fn reserved_default_name_and_saved_id_are_visibly_distinct() {
    let h = host_with(Options {
        default_name: true,
        ..Options::default()
    });
    edit(&h);
    h.probe("app-picker", json!({"command":"open"}));
    let v = h.capture("app-reserved-default");
    assert!(v["accessibilityText"]
        .as_array()
        .unwrap()
        .contains(&json!("Default app")));
    assert!(v["accessibilityText"]
        .as_array()
        .unwrap()
        .contains(&json!("Default app — dev.fixture.a")));
    h.probe("app-picker", json!({"command":"click","index":1}));
    assert_eq!(read(&h)["value"], "dev.fixture.a");
    let h = host_with(Options {
        application: Some("dev.fixture.missing"),
        missing_name: true,
        ..Options::default()
    });
    edit(&h);
    h.probe("app-picker", json!({"command":"open"}));
    let v = h.capture("app-saved-id");
    assert!(v["accessibilityText"]
        .as_array()
        .unwrap()
        .contains(&json!("Saved app — dev.fixture.missing")));
    assert!(v["accessibilityText"]
        .as_array()
        .unwrap()
        .contains(&json!("dev.fixture.missing")));
    h.probe("app-picker", json!({"command":"click","index":1}));
    assert_eq!(read(&h)["value"], "dev.fixture.missing");
}
#[test]
fn modal_keys_cancel_and_stale_callbacks_do_not_change_draft() {
    let h = host(None, false);
    edit(&h);
    let before = fs::read(&h.config).unwrap();
    h.probe("app-picker", json!({"command":"open"}));
    h.probe("app-picker", json!({"command":"retain"}));
    let old = read(&h);
    assert_eq!(old["expanded"], true);
    assert_eq!(old["fieldAXHidden"], true);
    assert_eq!(probe(&h, "read")["saveEnabled"], false);
    h.probe("app-picker", json!({"command":"select","index":2}));
    key(&h, 36, CMD, "\r", false);
    probe(&h, "save-editor");
    assert_eq!(read(&h)["list"]["open"], true);
    assert_eq!(read(&h)["value"], "");
    key(&h, 53, 0, "", false);
    assert_eq!(read(&h)["fieldAXHidden"], false);
    assert_eq!(read(&h)["value"], "");
    assert_eq!(probe(&h, "read")["saveEnabled"], true);
    h.probe("app-picker", json!({"command":"open"}));
    let current = read(&h);
    assert!(
        current["list"]["generation"].as_u64().unwrap()
            > old["list"]["generation"].as_u64().unwrap()
    );
    h.probe("app-picker", json!({"command":"stale","index":2}));
    assert_eq!(read(&h)["value"], "");
    assert_eq!(read(&h)["list"]["open"], true);
    key(&h, 53, 0, "", false);
    probe(&h, "cancel-editor");
    saved(&h);
    edit(&h);
    let token = read(&h)["token"].clone();
    h.probe("app-picker", json!({"command":"stale-button"}));
    assert_eq!(read(&h)["token"], token);
    assert_eq!(read(&h)["list"]["open"], false);
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn late_discovery_and_unrelated_reload_preserve_editor_draft() {
    let h = host_with(Options {
        barrier: true,
        ..Options::default()
    });
    let v = edit(&h);
    assert_eq!(v["pending"], true);
    assert_eq!(v["enabled"], false);
    h.probe("command-input", json!({"key":"title","text":"Kept draft"}));
    fs::write(h.base.join("app-discovery-release"), "go").unwrap();
    h.wait(|s| s["appDiscoveryPending"] == false);
    h.settled(|_| true);
    assert_eq!(probe(&h, "command-read")["value"]["title"], "Kept draft");
    assert_eq!(read(&h)["enabled"], true);
    h.probe("app-picker", json!({"command":"open"}));
    let old = read(&h);
    let mut cfg = config(&h);
    cfg["theme"] = json!("light");
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    h.settled(|_| true);
    assert_eq!(read(&h)["list"]["generation"], old["list"]["generation"]);
    assert_eq!(read(&h)["fieldAXHidden"], true);
    key(&h, 53, 0, "", false);
    assert_eq!(probe(&h, "command-read")["value"]["title"], "Kept draft");
}
#[test]
fn external_application_id_conflict_keeps_draft_and_file() {
    let h = host(None, false);
    edit(&h);
    choose(&h, 2);
    let mut cfg = config(&h);
    cfg["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "url.fixture")
        .unwrap()["applicationID"] = json!("dev.fixture.a");
    h.replace(&cfg, None);
    let before = fs::read(&h.config).unwrap();
    probe(&h, "save-editor");
    view(&h, |v| text_contains(v, "changed elsewhere"));
    assert_eq!(read(&h)["value"], "dev.fixture.b");
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
fn discovered(h: &Bridge, id: &str) -> Value {
    config(h)["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["target"] == id)
        .unwrap()
        .clone()
}
#[test]
fn discovered_app_alias_hotkey_edit_and_disable_persist() {
    let h = host(None, false);
    show(&h, "Fixture Browser A");
    alias(&h);
    h.probe("alias-input", json!({"text":"ownbrowser"}));
    probe(&h, "save-editor");
    saved(&h);
    assert_eq!(
        discovered(&h, "dev.fixture.a")["aliases"],
        json!(["ownbrowser"])
    );
    show(&h, "ownbrowser");
    hotkey(&h);
    key(&h, 18, CMD | SHIFT, "1", false);
    saved(&h);
    assert_eq!(discovered(&h, "dev.fixture.a")["hotkey"], "shift+cmd+1");
    show(&h, "Fixture Browser B");
    action(&h, "Edit");
    view(&h, |v| v["formVisible"] == true);
    h.probe(
        "command-input",
        json!({"key":"title","text":"Own browser B"}),
    );
    probe(&h, "save-editor");
    saved(&h);
    show(&h, "Own browser B");
    action(&h, "Disable");
    h.settled(|_| true);
    assert_eq!(discovered(&h, "dev.fixture.b")["enabled"], false);
}
#[test]
fn discovered_app_disable_creates_suppression_without_alias() {
    let h = host(None, false);
    show(&h, "Fixture Browser B");
    action(&h, "Disable");
    h.settled(|_| true);
    assert_eq!(discovered(&h, "dev.fixture.b")["enabled"], false);
    assert!(
        !h.rpc(&json!({"op":"search","query":"Fixture Browser B"}))["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["target"] == "dev.fixture.b")
    );
}
