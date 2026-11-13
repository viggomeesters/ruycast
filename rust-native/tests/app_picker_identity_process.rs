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
    let mut h = Bridge::new(Config::current(), false);
    for (id, title) in [
        ("dev.fixture.a", "Fixture Browser A"),
        ("dev.fixture.b", "Fixture Browser B"),
    ] {
        let contents = h.applications.join(format!("{id}.app/Contents"));
        fs::create_dir_all(&contents).unwrap();
        let name = if duplicate { "Fixture Same" } else { title };
        let plist=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleName</key><string>{name}</string></dict></plist>");
        fs::write(contents.join("Info.plist"), plist).unwrap();
    }
    let mut cfg = default_config();
    let mut command = json!({"id":"url.fixture","title":"Fixture Link","kind":"url","target":"https://example.invalid/own","aliases":["fixtureurl"]});
    if let Some(id) = application {
        command["applicationID"] = json!(id);
    }
    cfg["commands"].as_array_mut().unwrap().push(command);
    h.replace(&cfg, None);
    h.start();
    h.wait(|s| s["appDiscoveryPending"] == false);
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
