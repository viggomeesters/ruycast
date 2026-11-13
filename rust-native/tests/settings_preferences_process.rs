//! Hidden Settings preferences on isolated profiles; no global input.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_bridge.rs"]
mod native_bridge;
use native_bridge::{
    runtime_host::{default_config, Config},
    Bridge,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn host() -> Bridge {
    let mut h = Bridge::new(Config::current(), false);
    let mut cfg = default_config();
    cfg["snippetExpansionEnabled"] = json!(false);
    h.replace(&cfg, None);
    h.start();
    settled(&h, |_| true);
    h.rpc(&json!({"op":"show","mode":"settings"}));
    settled(&h, |_| true);
    h
}
fn settled(h: &Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
    h.wait(|s| {
        s["configurationUpdating"] != true
            && s["managementUI"]["revision"].as_u64().unwrap_or(0) > 0
            && s["managementUI"]["revision"] == s["managementUI"]["appliedRevision"]
            && predicate(s)
    })
}
fn probe_timeout(h: &Bridge, action: &str, fields: Value, timeout: Duration) -> Value {
    let mut request = fields.as_object().unwrap().clone();
    request.insert("op".into(), json!("native.test"));
    request.insert("surface".into(), json!("settings"));
    request.insert("action".into(), json!(action));
    let response = h.rpc_timeout(&Value::Object(request), timeout);
    assert_eq!(response["ok"], true, "{response}");
    response["result"].clone()
}
fn probe(h: &Bridge, action: &str, fields: Value) -> Value {
    probe_timeout(h, action, fields, Duration::from_secs(3))
}
fn nav(h: &Bridge, tab: &str) -> Value {
    probe(h, "library-tab", json!({"tab":tab}));
    settled(h, |_| true);
    probe(h, "settings-read", json!({}))
}
fn wait(h: &Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = until.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "Settings preferences did not reach the expected state"
        );
        let status = h.rpc_timeout(
            &json!({"op":"status"}),
            remaining.min(Duration::from_secs(3)),
        )["result"]
            .clone();
        if status["configurationUpdating"] != true
            && status["managementUI"]["revision"].as_u64().unwrap_or(0) > 0
            && status["managementUI"]["revision"] == status["managementUI"]["appliedRevision"]
        {
            let remaining = until.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero());
            let value = probe_timeout(
                h,
                "settings-read",
                json!({}),
                remaining.min(Duration::from_secs(3)),
            );
            if predicate(&value) {
                return value;
            }
        }
        thread::sleep(
            Duration::from_millis(20).min(until.saturating_duration_since(Instant::now())),
        );
    }
}
fn capture(h: &Bridge, name: &str) -> Value {
    let value = probe(h, "settings-screenshot", json!({}));
    if let Some(evidence) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let dir = PathBuf::from(evidence);
        fs::create_dir_all(&dir).unwrap();
        fs::copy(
            h.profile.join("native-settings-preferences-probe.png"),
            dir.join(format!("{name}.png")),
        )
        .unwrap();
        fs::write(
            dir.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
    value
}
fn read_config(h: &Bridge) -> Value {
    serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap()
}

#[test]
fn general_actual_controls_write_config_preserve_unknown_fields_and_themes() {
    let h = host();
    let value = nav(&h, "general");
    assert_eq!(value["visible"], true);
    assert!(value["text"]
        .as_array()
        .unwrap()
        .contains(&json!("Open Launcher")));
    let before = read_config(&h);
    assert!(value["buttons"]
        .as_array()
        .unwrap()
        .iter()
        .all(|b| b["role"] == "AXCheckBox" || b["role"] == "AXButton"));
    for (index, key, default) in [
        (0, "hyperKeyEnabled", false),
        (1, "separateCommandKeys", true),
        (2, "menuBarVisible", true),
    ] {
        probe(&h, "settings-control", json!({"index":index}));
        wait(&h, |v| {
            v["surface"]["configuration"]
                .get(key)
                .cloned()
                .unwrap_or(json!(default))
                != before.get(key).cloned().unwrap_or(json!(default))
        });
    }
    for (index, theme) in [(1, "light"), (2, "dark"), (3, "graphite"), (0, "system")] {
        probe(&h, "settings-theme", json!({"index":index}));
        wait(&h, |v| v["surface"]["configuration"]["theme"] == theme);
        for (width, height) in [(680, 480), (980, 680)] {
            probe(&h, "dashboard-size", json!({"width":width,"height":height}));
            capture(&h, &format!("general-{theme}-{width}-{height}"));
        }
    }
    let after = read_config(&h);
    assert_eq!(after["commands"], before["commands"]);
    assert_eq!(after["snippetExpansionEnabled"], false);
    assert_eq!(probe(&h, "read", json!({}))["windowShown"], false);
}
#[test]
fn record_both_shortcuts_local_events_cancel_and_navigation_release() {
    let h = host();
    nav(&h, "general");
    for (index, key, code, chars) in [
        (0, "launcherHotkey", 0, "a"),
        (1, "clipboardHotkey", 11, "b"),
    ] {
        probe(&h, "settings-record", json!({"index":index}));
        let value = wait(&h, |v| v["recording"][index] == true);
        assert!(value["preparing"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v == false));
        probe(
            &h,
            "key-event",
            json!({"code":code,"flags":(1<<20)|(1<<19),"chars":chars}),
        );
        wait(&h, |v| {
            v["surface"]["configuration"][key] == format!("alt+cmd+{chars}")
        });
    }
    probe(&h, "settings-record", json!({"index":0}));
    wait(&h, |v| v["recording"][0] == true);
    let before = fs::read(&h.config).unwrap();
    probe(&h, "key-event", json!({"code":53,"flags":0,"chars":""}));
    wait(&h, |v| {
        v["recording"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x == false)
    });
    assert_eq!(fs::read(&h.config).unwrap(), before);
    probe(&h, "settings-record", json!({"index":1}));
    wait(&h, |v| v["recording"][1] == true);
    nav(&h, "commands");
    assert!(probe(&h, "settings-read", json!({}))["recording"]
        .as_array()
        .unwrap()
        .iter()
        .all(|x| x == false));
    nav(&h, "general");
    probe(&h, "settings-retain", json!({"index":0}));
    let before = fs::read(&h.config).unwrap();
    probe(&h, "window-close", json!({}));
    settled(&h, |s| s["managementUI"]["visible"] == false);
    h.rpc(&json!({"op":"show","mode":"settings"}));
    settled(&h, |_| true);
    nav(&h, "general");
    probe(&h, "settings-stale", json!({}));
    settled(&h, |_| true);
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn advanced_reload_open_permissions_and_fake_folder_cancel_stale_guard() {
    let h = host();
    let value = nav(&h, "advanced");
    let version = h.rpc(&json!({"op":"status"}))["result"]["version"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(value["text"]
        .as_array()
        .unwrap()
        .contains(&json!(format!("Ruycast {version}"))));
    probe(&h, "dashboard-size", json!({"width":680,"height":480}));
    capture(&h, "advanced-680-480");
    probe(&h, "settings-control", json!({"index":0}));
    let value = wait(&h, |v| !v["opened"].as_array().unwrap().is_empty());
    assert_eq!(
        value["opened"].as_array().unwrap().last().unwrap(),
        &json!(h.config.to_str().unwrap())
    );
    probe(&h, "settings-control", json!({"index":3}));
    let value = wait(&h, |v| v["opened"].as_array().unwrap().len() == 2);
    assert!(value["opened"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .as_str()
        .unwrap()
        .contains("Privacy_Accessibility"));
    probe(&h, "settings-control", json!({"index":2}));
    settled(&h, |_| true);
    assert_eq!(
        probe(&h, "settings-read", json!({}))["surface"]["busy"],
        false
    );
    probe(&h, "settings-control", json!({"index":1}));
    let value = probe(&h, "settings-read", json!({}));
    assert_eq!(value["fixtureDialog"], true);
    assert_eq!(
        probe(
            &h,
            "settings-folder-complete",
            json!({"serial":value["dialogSerial"],"accepted":false})
        )["accepted"],
        true
    );
    probe(&h, "settings-control", json!({"index":1}));
    let value = probe(&h, "settings-read", json!({}));
    nav(&h, "commands");
    nav(&h, "advanced");
    let target = h.base.join("moved");
    fs::create_dir(&target).unwrap();
    assert_eq!(
        probe(
            &h,
            "settings-folder-complete",
            json!({"serial":value["dialogSerial"],"accepted":true,"path":target})
        )["accepted"],
        false
    );
    assert!(!target.join("config.json").exists());
    probe(&h, "settings-control", json!({"index":1}));
    let value = probe(&h, "settings-read", json!({}));
    assert_eq!(
        probe(
            &h,
            "settings-folder-complete",
            json!({"serial":value["dialogSerial"],"accepted":true,"path":target})
        )["accepted"],
        true
    );
    wait(&h, |v| {
        v["surface"]["directory"] == target.to_str().unwrap()
    });
    assert!(target.join("config.json").exists());
    assert!(h.config.exists());
}
#[test]
fn menu_bar_hidden_substitution_live_visibility_and_retired_callbacks() {
    let h = host();
    let menu = h.probe("menu-bar-read", json!({}));
    assert_eq!(menu["hiddenSubstitution"], true);
    assert_eq!(menu["nativeRegistrations"], 0);
    assert_eq!(menu["items"], json!(["Open Launcher", "Settings…", "Quit"]));
    h.probe("menu-bar-retain", json!({}));
    nav(&h, "general");
    probe(&h, "settings-control", json!({"index":2}));
    wait(&h, |v| {
        v["surface"]["configuration"]["menuBarVisible"] == false
    });
    probe(&h, "window-close", json!({}));
    settled(&h, |_| true);
    h.probe("menu-bar-stale", json!({}));
    settled(&h, |_| true);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        false
    );
    h.rpc(&json!({"op":"show","mode":"settings"}));
    settled(&h, |_| true);
    nav(&h, "general");
    probe(&h, "settings-control", json!({"index":2}));
    wait(&h, |v| {
        v["surface"]["configuration"]["menuBarVisible"] == true
    });
    h.probe("menu-bar-click", json!({"index":0}));
    h.wait(|s| s["nativeUI"]["visible"] == true);
    assert_eq!(h.probe("read", json!({}))["windowShown"], false);
    h.probe("menu-bar-click", json!({"index":1}));
    settled(&h, |_| true);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["managementUI"]["visible"],
        true
    );
}
