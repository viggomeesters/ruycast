//! Inline delayed-write and cancellation behavior in an owned hidden native host.
#![cfg(feature = "test-tools")]
#![allow(dead_code)]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::{native_bridge::Bridge, *};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn settings(h: &Bridge, action: &str, fields: Value) -> Value {
    let mut req = fields.as_object().unwrap().clone();
    req.insert("op".into(), json!("native.test"));
    req.insert("surface".into(), json!("settings"));
    req.insert("action".into(), json!(action));
    let response = h.rpc(&Value::Object(req));
    assert_eq!(response["ok"], true, "{response}");
    response["result"].clone()
}
fn p(h: &Bridge, action: &str) -> Value {
    settings(h, action, json!({}))
}
fn settled(h: &Bridge, pred: impl Fn(&Value) -> bool) {
    h.wait(|s| {
        s["configurationUpdating"] != true
            && s["managementUI"]["revision"].as_u64().unwrap_or(0) > 0
            && s["managementUI"]["revision"] == s["managementUI"]["appliedRevision"]
            && pred(s)
    });
}
fn view(h: &Bridge, pred: impl Fn(&Value) -> bool) -> Value {
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        assert!(
            Instant::now() < until,
            "Settings did not reach expected state"
        );
        let value = p(h, "read");
        if pred(&value) {
            return value;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn show(h: &Bridge, query: &str) {
    assert_eq!(
        h.rpc(&json!({"op":"show","mode":"settings","query":query}))["ok"],
        true
    );
    settled(h, |s| s["managementUI"]["visible"] == true);
    view(h, |v| v["input"] == query);
}
fn capture(h: &Bridge, name: &str) {
    let value = p(h, "screenshot");
    let image = h.profile.join("native-settings-probe.png");
    assert!(fs::read(&image).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    if let Some(evidence) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let directory = std::path::PathBuf::from(evidence)
            .parent()
            .unwrap()
            .join("library");
        fs::create_dir_all(&directory).unwrap();
        fs::copy(image, directory.join(format!("{name}.png"))).unwrap();
        fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}
fn begin(h: &Bridge, id: &str, hotkey: bool) -> Value {
    let before = p(h, "read")["editorToken"].clone();
    settings(
        h,
        if hotkey {
            "inline-hotkey"
        } else {
            "inline-alias"
        },
        json!({"id":id}),
    );
    view(h, |v| {
        v["inlineVisible"] == true
            && v["editorId"] == id
            && v["editorToken"] != before
            && (!hotkey || v["recording"] == true)
    })
}
fn done(h: &Bridge) {
    settled(h, |s| s["managementUI"]["editorOpen"] == false);
}
fn press(h: &Bridge, code: u32, flags: u64, chars: &str) {
    settings(
        h,
        "key-event",
        json!({"code":code,"flags":flags,"chars":if (code==36||code==76)&&chars.is_empty(){"\r"} else {chars},"repeat":false}),
    );
}
fn duplicate(v: &Value) -> bool {
    v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().to_lowercase().contains("duplicate"))
}
fn slow_host(held: bool) -> Bridge {
    let mut h = Bridge::new(
        actions::native_bridge::runtime_host::Config::current(),
        false,
    );
    let shim = std::env::var_os("RUYCAST_TEST_FCNTL_SHIM")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            h.settings
                .host
                .parent()
                .unwrap()
                .join("examples/libfcntl_sync_shim.dylib")
        });
    assert!(
        shim.is_absolute() && shim.is_file(),
        "build fcntl-sync-shim first"
    );
    let marker = h.profile.join("inline-sync-entered");
    for (key, value) in [
        ("DYLD_INSERT_LIBRARIES", shim.into_os_string()),
        (
            "RUYCAST_TEST_FSYNC_DIRECTORY",
            h.profile.as_os_str().to_owned(),
        ),
        ("RUYCAST_TEST_FSYNC_MARKER", marker.as_os_str().to_owned()),
        ("RUYCAST_TEST_FSYNC_DELAY_MS", "500".into()),
        ("RUYCAST_TEST_FSYNC_DELAY_ONLY", "1".into()),
    ] {
        h.env.insert(key.into(), value);
    }
    if held {
        let release = h.profile.join("inline-sync-release").into_os_string();
        h.env.insert("RUYCAST_TEST_FSYNC_RELEASE".into(), release);
    }
    h.start();
    settled(&h, |_| true);
    assert!(!marker.exists());
    h
}
fn wait_marker(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "actual directory sync not reached"
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn cancel_during_save(checkbox: bool) {
    let h = slow_host(false);
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"publishedbeforecancel"}));
    let before = command(&h, "app.safari");
    p(&h, "schedule-inline-cancel");
    settings(
        &h,
        if checkbox {
            "library-enable"
        } else {
            "inline-alias"
        },
        json!({"id":"app.safari"}),
    );
    done(&h);
    assert_eq!(
        fs::read(h.profile.join("inline-sync-entered")).unwrap(),
        b"injected"
    );
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["publishedbeforecancel"])
    );
    assert_eq!(command(&h, "app.safari"), before);
}
#[test]
fn cancel_during_actual_directory_sync_prevents_next_binding() {
    cancel_during_save(false);
}
#[test]
fn cancel_during_actual_directory_sync_prevents_next_checkbox_write() {
    cancel_during_save(true);
}
#[test]
fn command_w_closes_settings_during_held_async_alias_save() {
    let h = slow_host(false);
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"published-before-close"}));
    p(&h, "save-editor");
    wait_marker(&h.profile.join("inline-sync-entered"));
    press(&h, 13, CMD, "w");
    h.wait(|s| s["managementUI"]["visible"] == false);
    h.wait(|s| s["configurationUpdating"] == false);
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["published-before-close"])
    );
}
struct Release(std::path::PathBuf);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::write(&self.0, "release");
    }
}
#[test]
fn focus_end_after_interim_ack_does_not_repeat_transition_save() {
    let h = slow_host(true);
    let release = Release(h.profile.join("inline-sync-release"));
    show(&h, "");
    begin(&h, "app.finder", false);
    settings(&h, "alias-type", json!({"text":"once-before-switch"}));
    settings(&h, "inline-alias", json!({"id":"app.safari"}));
    wait_marker(&h.profile.join("inline-sync-entered"));
    view(&h, |v| {
        v["editorSaving"] == true && v["editorRequestPending"] == false
    });
    settings(&h, "inline-alias", json!({"id":"app.terminal"}));
    p(&h, "save-editor");
    settings(&h, "library-enable", json!({"id":"app.safari"}));
    settings(&h, "library-tab", json!({"tab":"snippets"}));
    settings(&h, "library-add", json!({"kind":"snippet"}));
    assert_eq!(p(&h, "read")["editorSaving"], true);
    p(&h, "inline-focus-search");
    drop(release);
    view(&h, |v| {
        v["inlineVisible"] == true && v["editorId"] == "app.safari" && v["editorSaving"] == false
    });
    assert_eq!(
        command(&h, "app.finder")["aliases"],
        json!(["once-before-switch"])
    );
    assert!(command(&h, "app.safari")
        .get("enabled")
        .map(|value| value.as_bool() == Some(true))
        .unwrap_or(true));
    assert_eq!(p(&h, "library-read")["tab"], "commands");
    settings(&h, "alias-type", json!({"text":"still-typable"}));
    assert_eq!(p(&h, "inline-control-read")["hitEditor"], true);
}
