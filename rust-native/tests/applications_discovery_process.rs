//! Actual discovery in exclusively owned synthetic application roots.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn bundle(root: &Path, name: &str, id: Option<&str>, display: Option<&str>, label: Option<&str>) {
    let contents = root.join(format!("{name}.app/Contents"));
    fs::create_dir_all(&contents).unwrap();
    let mut fields = String::new();
    for (key, value) in [
        ("CFBundleIdentifier", id),
        ("CFBundlePackageType", Some("APPL")),
        ("CFBundleDisplayName", display),
        ("CFBundleName", label),
    ] {
        if let Some(value) = value {
            fields.push_str(&format!("<key>{key}</key><string>{value}</string>"));
        }
    }
    fs::write(contents.join("Info.plist"),format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>{fields}</dict></plist>")).unwrap();
}
fn ready(h: &Host) -> Value {
    h.wait(|s| s["appDiscoveryPending"] == false)
}
fn search(h: &Host, query: &str) -> Vec<Value> {
    h.rpc(&json!({"op":"search","query":query,"limit":0}))["result"]
        .as_array()
        .unwrap()
        .clone()
}
fn write_cfg(h: &Host, cfg: &Value) {
    fs::write(&h.config, serde_json::to_vec(cfg).unwrap()).unwrap();
}
#[test]
fn actual_metadata_depth_hidden_and_unchanged_cli_search() {
    let mut h = Host::new(Config::current(), false);
    h.env
        .insert("RUYCAST_NATIVE_TEST_APP_BARRIER".into(), "1".into());
    for (name, id, display, label) in [
        (
            "Folder/Visible",
            Some("dev.fixture.visible"),
            Some("Fixture Display"),
            Some("Not this"),
        ),
        (
            "OnlyName",
            Some("dev.fixture.name"),
            None,
            Some("Fixture Name"),
        ),
        (
            "Fixture Filename Ω",
            Some("dev.fixture.filename"),
            None,
            None,
        ),
        (".Hidden", Some("dev.fixture.hidden"), None, None),
        (
            "Folder/Visible.app/Contents/Embedded",
            Some("dev.fixture.embedded"),
            None,
            Some("Fixture Embedded"),
        ),
        (
            "a/b/c/AtLevel4",
            Some("dev.fixture.depth"),
            None,
            Some("Fixture Depth"),
        ),
        (
            "a/b/c/d/TooDeep",
            Some("dev.fixture.toodeep"),
            None,
            Some("Fixture Too Deep"),
        ),
        ("Empty", Some(""), None, Some("Fixture Invalid Empty")),
        ("Blank", Some("   "), None, Some("Fixture Invalid Blank")),
        ("Missing", None, None, Some("Fixture Invalid Missing")),
    ] {
        bundle(&h.applications, name, id, display, label);
    }
    h.start();
    let marker = h.base.join("app-discovery-ready");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "discovery barrier missing");
        thread::sleep(Duration::from_millis(10));
    }
    let before = h.rpc(&json!({"op":"commands"}))["result"].clone();
    fs::write(h.base.join("app-discovery-release"), "go").unwrap();
    let status = ready(&h);
    assert!(status["appDiscoveryError"].is_null());
    assert_eq!(status["discoveredApps"], 4);
    let result = h.cli(&["search", "Fixture", "--limit", "0"]);
    let found = result["result"].as_array().unwrap();
    assert_eq!(
        found.iter().map(|c| c["title"].clone()).collect::<Vec<_>>(),
        vec![
            json!("Fixture Depth"),
            json!("Fixture Display"),
            json!("Fixture Filename Ω"),
            json!("Fixture Name")
        ]
    );
    assert!(found.iter().all(|c| c["kind"] == "app"
        && c["id"].as_str().unwrap() == format!("discovered.{}", c["target"].as_str().unwrap())));
    assert_eq!(h.rpc(&json!({"op":"commands"}))["result"], before);
}
#[test]
fn roots_precedence_and_configured_target_override_disable() {
    let mut h = Host::new(Config::current(), false);
    let system = h.base.join("SystemApps");
    let user = h.base.join("UserApps");
    fs::create_dir(&system).unwrap();
    fs::create_dir(&user).unwrap();
    bundle(
        &system,
        "Old",
        Some("dev.fixture.same"),
        None,
        Some("Fixture Old"),
    );
    bundle(
        &user,
        "New",
        Some("dev.fixture.same"),
        None,
        Some("Fixture New"),
    );
    bundle(
        &user,
        "Other",
        Some("dev.fixture.other"),
        None,
        Some("Fixture Other"),
    );
    h.env.insert(
        "RUYCAST_NATIVE_TEST_APP_ROOTS".into(),
        json!([system, user]).to_string().into(),
    );
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"saved.fixture","title":"Saved Fixture","kind":"app","target":"dev.fixture.same","aliases":["fixtureapp"],"enabled":false}));
    write_cfg(&h, &cfg);
    h.start();
    assert_eq!(ready(&h)["discoveredApps"], 2);
    assert_eq!(
        search(&h, "Fixture")
            .iter()
            .map(|c| c["target"].clone())
            .collect::<Vec<_>>(),
        vec![json!("dev.fixture.other")]
    );
    cfg["commands"].as_array_mut().unwrap().last_mut().unwrap()["enabled"] = json!(true);
    write_cfg(&h, &cfg);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    assert_eq!(
        search(&h, "fixtureapp")
            .iter()
            .map(|c| c["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("saved.fixture")]
    );
    cfg["commands"].as_array_mut().unwrap().pop();
    write_cfg(&h, &cfg);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    assert_eq!(
        search(&h, "Fixture")
            .iter()
            .map(|c| c["title"].clone())
            .collect::<Vec<_>>(),
        vec![json!("Fixture New"), json!("Fixture Other")]
    );
}
#[test]
fn saved_id_collision_never_publishes_two_rows() {
    let mut h = Host::new(Config::current(), false);
    bundle(
        &h.applications,
        "App",
        Some("dev.fixture.collision"),
        None,
        Some("Fixture Discovered"),
    );
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"discovered.dev.fixture.collision","title":"Fixture Saved","kind":"script","target":"/usr/bin/true"}));
    write_cfg(&h, &cfg);
    h.start();
    ready(&h);
    assert_eq!(
        search(&h, "Fixture")
            .iter()
            .map(|c| c["title"].clone())
            .collect::<Vec<_>>(),
        vec![json!("Fixture Saved")]
    );
}

#[test]
fn nested_app_symlink_outside_home_is_not_followed() {
    use std::os::unix::fs::symlink;
    let mut host = Host::new(Config::current(), false);
    let other = Host::new(Config::current(), false);
    bundle(
        &other.applications,
        "PrivateExternal",
        Some("dev.fixture.external"),
        None,
        Some("Fixture External"),
    );
    symlink(
        other.applications.join("PrivateExternal.app"),
        host.applications.join("Link.app"),
    )
    .unwrap();
    host.start();
    assert_eq!(ready(&host)["discoveredApps"], 0);
    assert!(search(&host, "Fixture").is_empty());
}

#[test]
fn malformed_metadata_types_do_not_abort_worker() {
    let mut host = Host::new(Config::current(), false);
    for (name, value) in [
        ("NumberID", "<dict><key>CFBundleIdentifier</key><integer>12</integer></dict>"),
        ("ArrayRoot", "<array/>"),
        ("WrongLabel", "<dict><key>CFBundleIdentifier</key><string>dev.fixture.wronglabel</string><key>CFBundleDisplayName</key><array><string>wrong</string></array><key>CFBundleName</key><string>Fixture fallback</string></dict>"),
    ] {
        let contents = host.applications.join(format!("{name}.app/Contents"));
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("Info.plist"), format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\">{value}</plist>")).unwrap();
    }
    host.start();
    let status = ready(&host);
    assert!(status["appDiscoveryError"].is_null());
    assert_eq!(status["discoveredApps"], 1);
    assert_eq!(
        search(&host, "Fixture")
            .iter()
            .map(|c| c["title"].clone())
            .collect::<Vec<_>>(),
        vec![json!("Fixture fallback")]
    );
}
