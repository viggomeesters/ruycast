//! Hidden Settings dashboard with an owned save-dialog fixture.
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
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    thread,
    time::{Duration, Instant},
};
fn host() -> Bridge {
    let mut h = Bridge::new(Config::current(), false);
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().extend([
        json!({"id":"analytics.fixture","title":"Own analytics fixture","kind":"script","target":"/usr/bin/true"}),
        json!({"id":"analytics.failure","title":"Own failure fixture","kind":"script","target":"/usr/bin/false"}),
    ]);
    h.replace(&cfg, None);
    h.start();
    settled(&h, |_| true);
    h
}
fn settled(h: &Bridge, predicate: impl Fn(&Value) -> bool) {
    h.wait(|s| {
        s["configurationUpdating"] != true
            && s["managementUI"]["revision"].as_u64().unwrap_or(0) > 0
            && s["managementUI"]["revision"] == s["managementUI"]["appliedRevision"]
            && predicate(s)
    });
}
fn probe(h: &Bridge, action: &str, mut fields: Value) -> Value {
    let request = fields.as_object_mut().unwrap();
    request.insert("op".into(), json!("native.test"));
    request.insert("surface".into(), json!("settings"));
    request.insert("action".into(), json!(action));
    let response = h.rpc(&fields);
    assert_eq!(response["ok"], true, "{response}");
    response["result"].clone()
}
fn read(h: &Bridge) -> Value {
    probe(h, "dashboard-read", json!({}))
}
fn open(h: &Bridge) -> Value {
    assert_eq!(h.rpc(&json!({"op":"show","mode":"analytics"}))["ok"], true);
    settled(h, |s| {
        s["managementUI"]["visible"] == true && s["managementUI"]["tab"] == "dashboard"
    });
    read(h)
}
fn click(h: &Bridge, index: u32) {
    probe(h, "dashboard-control", json!({"index":index}));
}
fn wait(h: &Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let v = read(h);
        if predicate(&v) {
            return v;
        }
        assert!(Instant::now() < end, "dashboard did not settle: {v}");
        thread::sleep(Duration::from_millis(20));
    }
}
fn entries(path: &std::path::Path) -> BTreeSet<std::ffi::OsString> {
    fs::read_dir(path)
        .unwrap()
        .map(|v| v.unwrap().file_name())
        .collect()
}
#[test]
fn standard_builtin_cli_native_sources_usage_logs_and_override() {
    let h = host();
    let rows = h.rpc(&json!({"op":"search","query":"dashboard"}))["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(rows, vec![json!("builtin.dashboard")]);
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":"analytics","dryRun":true}))["dryRun"],
        true
    );
    assert_eq!(h.rpc(&json!({"op":"execute","id":"analytics"}))["ok"], true);
    settled(&h, |s| s["managementUI"]["tab"] == "dashboard");
    let d = h.rpc(&json!({"op":"analytics"}))["result"].clone();
    assert_eq!(d["totalAttempts"], 1);
    assert_eq!(d["totalSuccesses"], 1);
    assert_eq!(
        h.rpc(&json!({"op":"usage"}))["result"]["builtin.dashboard"],
        1
    );
    let logs = h.rpc(&json!({"op":"logs","limit":100}))["result"].clone();
    assert!(logs
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["commandID"] == "builtin.dashboard"
            && r["source"] == "cli"
            && r["phase"] == "command.finish"));
    h.rpc(&json!({"op":"show","query":"dashboard"}));
    h.wait(|s| s["nativeUI"]["visible"] == true);
    h.probe("key-event", json!({"code":36,"flags":0,"chars":"\r"}));
    settled(&h, |s| {
        s["managementUI"]["tab"] == "dashboard" && s["nativeUI"]["visible"] == false
    });
    assert_eq!(
        h.rpc(&json!({"op":"usage"}))["result"]["builtin.dashboard"],
        2
    );
    let d = h.rpc(&json!({"op":"analytics"}))["result"].clone();
    assert_eq!(d["totalAttempts"], 2);
    assert_eq!(d["totalSuccesses"], 2);
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"own.dashboard","title":"Own dashboard","kind":"builtin","target":"analytics","aliases":["myactivity"],"enabled":false}));
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    settled(&h, |_| true);
    assert_eq!(
        h.rpc(&json!({"op":"search","query":"dashboard"}))["result"],
        json!([])
    );
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":"myactivity"}))["ok"],
        false
    );
}
#[test]
fn builtin_input_validation_and_native_prompt_do_not_log_private_values() {
    let h = host();
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"own.prompted-dashboard","title":"Prompted analytics","kind":"builtin","target":"analytics","aliases":["promptactivity"],"inputPrompt":"Reason"}));
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    settled(&h, |_| true);
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":"promptactivity","dryRun":true}))["dryRun"],
        true
    );
    for values in [json!({}), json!({"input":" \n\t"}), json!({"input":2})] {
        assert_eq!(
            h.rpc(&json!({"op":"execute","id":"promptactivity","values":values}))["ok"],
            false
        );
    }
    assert_eq!(
        h.rpc(&json!({"op":"analytics"}))["result"]["totalAttempts"],
        0
    );
    h.rpc(&json!({"op":"show","query":"promptactivity"}));
    h.wait(|s| s["nativeUI"]["visible"] == true);
    h.probe("key-event", json!({"code":36,"flags":0,"chars":"\r"}));
    h.wait(|s| s["nativeUI"]["editorOpen"] == true);
    assert_eq!(h.probe("read", json!({}))["runInputInline"], true);
    assert_eq!(
        h.rpc(&json!({"op":"analytics"}))["result"]["totalAttempts"],
        0
    );
    h.probe(
        "run-inline-type",
        json!({"text":"PRIVATE dashboard reason"}),
    );
    h.probe("key-event", json!({"code":36,"flags":0,"chars":"\r"}));
    settled(&h, |s| {
        s["managementUI"]["tab"] == "dashboard" && s["nativeUI"]["visible"] == false
    });
    assert_eq!(
        h.rpc(&json!({"op":"analytics"}))["result"]["totalAttempts"],
        1
    );
    assert!(!serde_json::to_string(&h.rpc(&json!({"op":"logs"})))
        .unwrap()
        .contains("PRIVATE"));
}
#[test]
fn management_routing_controls_custom_retention_and_reset() {
    let h = host();
    let v = open(&h);
    assert_eq!(v["visible"], true);
    assert_eq!(v["surface"]["hours"].as_array().unwrap().len(), 24);
    assert!(v["text"].as_array().unwrap().contains(&json!("Dashboard")));
    assert_eq!(probe(&h, "read", json!({}))["windowShown"], false);
    click(&h, 1);
    wait(&h, |v| {
        v["surface"]["busy"] == false && v["surface"]["enabled"] == false
    });
    assert_eq!(
        h.rpc(&json!({"op":"analytics"}))["result"]["enabled"],
        false
    );
    assert_eq!(
        h.rpc(&json!({"op":"analytics.settings","retentionDays":14}))["ok"],
        true
    );
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":"analytics.fixture"}))["ok"],
        true
    );
    let v = wait(&h, |v| v["surface"]["retention_days"] == 14);
    assert!(v["text"].as_array().unwrap().contains(&json!("14 days")));
    probe(&h, "dashboard-retention", json!({"days":7}));
    wait(&h, |v| {
        v["surface"]["busy"] == false && v["surface"]["retention_days"] == 7
    });
    let before = fs::read(&h.config).unwrap();
    click(&h, 3);
    wait(&h, |v| v["surface"]["confirming_reset"] == true);
    click(&h, 5);
    wait(&h, |v| v["surface"]["confirming_reset"] == false);
    click(&h, 3);
    wait(&h, |v| v["surface"]["confirming_reset"] == true);
    click(&h, 4);
    wait(&h, |v| {
        v["surface"]["busy"] == false && v["surface"]["confirming_reset"] == false
    });
    assert_eq!(fs::read(&h.config).unwrap(), before);
}
#[test]
fn repeated_confirmation_retires_old_queued_reset_callback() {
    let h = host();
    open(&h);
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":"analytics.fixture"}))["ok"],
        true
    );
    click(&h, 3);
    let first = wait(&h, |v| v["surface"]["confirming_reset"] == true);
    probe(&h, "dashboard-retain", json!({"index":4}));
    click(&h, 5);
    wait(&h, |v| v["surface"]["confirming_reset"] == false);
    click(&h, 3);
    let second = wait(&h, |v| v["surface"]["confirming_reset"] == true);
    assert_ne!(
        first["surface"]["confirmation_token"],
        second["surface"]["confirmation_token"]
    );
    let before = h.rpc(&json!({"op":"analytics"}))["result"]["totalAttempts"].clone();
    probe(&h, "dashboard-stale", json!({}));
    assert_eq!(
        h.rpc(&json!({"op":"analytics"}))["result"]["totalAttempts"],
        before
    );
    assert_eq!(read(&h)["surface"]["confirming_reset"], true);
    click(&h, 4);
    wait(&h, |v| {
        v["surface"]["busy"] == false && v["surface"]["confirming_reset"] == false
    });
    assert_eq!(
        h.rpc(&json!({"op":"analytics"}))["result"]["totalAttempts"],
        0
    );
}
#[test]
fn owned_save_dialog_takes_current_data_and_cancel_writes_nothing() {
    let h = host();
    open(&h);
    click(&h, 2);
    let d = read(&h);
    assert_eq!(d["fixtureDialog"], true);
    assert_eq!(d["dialogOpen"], true);
    let before = entries(&h.base);
    assert_eq!(
        probe(
            &h,
            "dashboard-dialog-complete",
            json!({"serial":d["dialogSerial"],"token":d["surface"]["token"],"accepted":false})
        )["accepted"],
        true
    );
    assert_eq!(entries(&h.base), before);
    click(&h, 2);
    let d = read(&h);
    assert_eq!(
        h.rpc(&json!({"op":"analytics.reset","confirmed":true}))["ok"],
        true
    );
    assert_eq!(
        h.rpc(&json!({"op":"analytics.settings","retentionDays":14}))["ok"],
        true
    );
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":"analytics.fixture"}))["ok"],
        true
    );
    assert_eq!(read(&h)["dialogOpen"], true);
    let output = h.base.join("diagnostics.json");
    assert_eq!(
        probe(
            &h,
            "dashboard-dialog-complete",
            json!({"serial":d["dialogSerial"],"token":d["surface"]["token"],"accepted":true,"path":output})
        )["accepted"],
        true
    );
    wait(&h, |v| {
        v["surface"]["busy"] == false && v["surface"]["message"] == "Diagnostics exported"
    });
    let raw = fs::read_to_string(&output).unwrap();
    let result: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(result["attempts"], 1);
    assert_eq!(result["successes"], 1);
    assert_eq!(
        fs::metadata(&output).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!raw.contains(h.base.to_str().unwrap()));
}
#[test]
fn retired_controls_and_dialog_cannot_mutate_a_reopened_dashboard() {
    let h = host();
    let token = open(&h)["surface"]["token"].clone();
    probe(&h, "dashboard-retain", json!({"index":1}));
    click(&h, 2);
    let d = read(&h);
    probe(&h, "library-tab", json!({"tab":"commands"}));
    settled(&h, |s| s["managementUI"]["tab"] == "commands");
    assert_eq!(read(&h)["dialogOpen"], false);
    assert_ne!(open(&h)["surface"]["token"], token);
    let before = h.rpc(&json!({"op":"analytics"}))["result"].clone();
    probe(&h, "dashboard-stale", json!({}));
    let output = h.base.join("stale-export.json");
    assert_eq!(
        probe(
            &h,
            "dashboard-dialog-complete",
            json!({"serial":d["dialogSerial"],"token":token,"accepted":true,"path":output})
        )["accepted"],
        false
    );
    assert_eq!(h.rpc(&json!({"op":"analytics"}))["result"], before);
    assert!(!output.exists());
}
