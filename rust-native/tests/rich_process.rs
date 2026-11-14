//! Native rich result lifecycle with an owned Rust provider.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::native_bridge::{
    runtime_host::{default_config, Config},
    Bridge,
};
use serde_json::{json, Value};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn host(mode: &str) -> Bridge {
    let mut host = Bridge::new(Config::current(), false);
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["rich-provider","{input}",host.base,mode],"timeout":3}));
    host.replace(&config, None);
    host.start();
    host.settled(|_| true);
    host
}
fn view(host: &Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        host.settled(|_| true);
        let v = actions::probe(host, "read");
        if !v["scriptResult"].is_null() && predicate(&v["scriptResult"]) {
            return v;
        }
        assert!(Instant::now() < deadline, "rich result missing: {v}");
        thread::sleep(Duration::from_millis(20));
    }
}
fn open(host: &Bridge) -> Value {
    actions::show(host, "runfixture");
    assert_eq!(actions::probe(host, "read")["windowShown"], false);
    actions::key(host, 36, 0, "\r", false);
    view(host, |_| true)
}
fn click(host: &Bridge, name: &str) {
    host.probe("result-control", json!({"name":name}));
    host.settled(|_| true);
}
fn capture(host: &Bridge, name: &str) -> Value {
    let v = host.probe("screenshot", json!({}));
    assert_eq!(v["windowShown"], false);
    assert_eq!(v["windowKey"], false);
    if let Some(output) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let folder = std::path::PathBuf::from(output)
            .parent()
            .unwrap()
            .join("rich");
        fs::create_dir_all(&folder).unwrap();
        fs::copy(
            host.profile.join("native-probe.png"),
            folder.join(format!("{name}.png")),
        )
        .unwrap();
        fs::write(
            folder.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&v).unwrap(),
        )
        .unwrap();
    }
    v
}
#[test]
fn choices_filter_literal_selection_and_ok_only_closes() {
    let host = host("close");
    let v = open(&host);
    let result = &v["scriptResult"];
    assert_eq!(
        result["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["reused", "two"]
    );
    assert!(!serde_json::to_string(result)
        .unwrap()
        .contains("PRIVATE $(literal)"));
    assert_eq!(v["rows"], 2);
    assert!(v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "EX / PLAN"));
    host.probe("query", json!({"query":"CAFE PLAN"}));
    let v = view(&host, |r| r["rows"].as_array().unwrap().len() == 1);
    assert_eq!(v["scriptResult"]["rows"][0]["id"], "reused");
    actions::key(&host, 36, 0, "\r", false);
    host.wait(|s| s["scriptsRunning"] == 0);
    assert!(actions::probe(&host, "read")
        .get("scriptResult")
        .unwrap()
        .is_null());
    assert_eq!(
        fs::read_to_string(host.base.join("selected")).unwrap(),
        "PRIVATE $(literal) Ω 1"
    );
    assert_eq!(
        host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
        2
    );
}
#[test]
fn reload_replaces_token_and_stale_button_is_detached() {
    let host = host("close");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    host.probe("retain-result-button", json!({"name":"choose"}));
    host.probe("query", json!({"query":"PLAN"}));
    view(&host, |r| r["query"] == "PLAN");
    click(&host, "reload");
    let new = view(&host, |r| r.get("token").unwrap() != &old);
    assert_eq!(new["scriptResult"]["query"], "");
    assert_eq!(new["scriptResult"]["selection"], 0);
    assert!(!host.base.join("selected").exists());
    assert_eq!(fs::read_to_string(host.base.join("phase")).unwrap(), "2");
    actions::probe(&host, "stale-result-button");
    host.settled(|_| true);
    assert!(!host.base.join("selected").exists());
    click(&host, "choose");
    host.wait(|s| s["scriptsRunning"] == 0);
    assert_eq!(
        fs::read_to_string(host.base.join("selected")).unwrap(),
        "PRIVATE $(literal) Ω 2"
    );
}
#[test]
fn failure_keeps_query_selection_and_readable_detail() {
    let host = host("fail");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    host.probe("query", json!({"query":"PLAN"}));
    view(&host, |r| r["query"] == "PLAN");
    click(&host, "choose");
    let v = view(&host, |r| {
        r["busy"] == false && r["error"] == "Provider refused"
    });
    assert_eq!(v["scriptResult"].get("token").unwrap(), &old);
    assert_eq!(v["scriptResult"]["query"], "PLAN");
    capture(&host, "provider-failure");
    click(&host, "details");
    let v = view(&host, |r| r["detail_visible"] == true);
    assert_eq!(v["resultPanel"]["detail"], "Failure detail");
    assert_eq!(v["resultPanel"]["editable"], false);
    assert_eq!(v["resultPanel"]["selectable"], true);
    click(&host, "choices");
    assert_eq!(view(&host, |_| true)["scriptResult"]["query"], "PLAN");
}
