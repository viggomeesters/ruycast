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
struct GracefulBridge(Bridge);
impl std::ops::Deref for GracefulBridge {
    type Target = Bridge;
    fn deref(&self) -> &Bridge {
        &self.0
    }
}
impl Drop for GracefulBridge {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.0.host.stop(true)));
        }
    }
}
fn wait_exists(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "fixture did not publish {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn detail_scroll_back_preserves_choices_and_filter() {
    let host = host("close");
    open(&host);
    host.probe("query", json!({"query":"PLAN"}));
    view(&host, |r| r["query"] == "PLAN");
    click(&host, "details");
    let v = view(&host, |r| r["detail_visible"] == true);
    assert_eq!(v["resultPanel"]["selectable"], true);
    assert_eq!(v["resultPanel"]["editable"], false);
    assert!(v["resultPanel"]["detail"]
        .as_str()
        .unwrap()
        .contains("PRIVATE report"));
    assert!(v["resultPanel"]["scrollMaxY"].as_f64().unwrap() > 0.0);
    capture(&host, "long-detail");
    actions::key(&host, 53, 0, "", false);
    assert_eq!(
        view(&host, |r| r["detail_visible"] == false)["scriptResult"]["query"],
        "PLAN"
    );
}
#[test]
fn unsafe_followup_closes_choices_with_invalid_response() {
    let host = host("invalid");
    open(&host);
    click(&host, "choose");
    host.wait(|s| s["scriptsRunning"] == 0);
    let v = actions::probe(&host, "read");
    assert!(v.get("scriptResult").unwrap().is_null());
    assert!(v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("invalid response")));
}
#[test]
fn new_selection_result_uses_new_token_and_values() {
    let host = host("replace");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    click(&host, "choose");
    let v = view(&host, |r| r.get("token").unwrap() != &old);
    assert_eq!(v["scriptResult"]["title"], "Updated");
    assert_eq!(v["scriptResult"]["query"], "");
}
#[test]
fn changed_command_cannot_execute_old_selection() {
    let host = host("close");
    open(&host);
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    command["target"] = json!("/usr/bin/false");
    command["arguments"] = json!([]);
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    click(&host, "choose");
    let v = view(&host, |r| {
        r["error"].as_str().unwrap_or("").contains("changed")
    });
    assert!(!host.base.join("selected").exists());
    assert_eq!(v["scriptResult"]["busy"], false);
}
#[test]
fn back_during_busy_cancels_only_owned_job() {
    let host = GracefulBridge(host("slow"));
    open(&host);
    click(&host, "choose");
    wait_exists(&host.base.join("ready"));
    view(&host, |r| r["busy"] == true);
    click(&host, "close");
    host.wait(|s| s["scriptsRunning"] == 0);
    let v = actions::probe(&host, "read");
    assert!(v.get("scriptResult").unwrap().is_null());
    assert_eq!(
        host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        true
    );
    assert!(!v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("Late report")));
}
#[test]
fn rejected_query_blocks_old_choice_and_can_be_repaired() {
    let host = host("close");
    open(&host);
    let query = (0..65)
        .map(|n| format!("term{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    host.probe("query", json!({"query":query}));
    let v = view(&host, |r| r["query_valid"] == false);
    assert!(v["scriptResult"].get("selection").unwrap().is_null());
    actions::key(&host, 36, 0, "\r", false);
    host.settled(|_| true);
    assert!(!host.base.join("selected").exists());
    host.probe("query", json!({"query":"cafe"}));
    view(&host, |r| r["query_valid"] == true);
    click(&host, "choose");
    host.wait(|s| s["scriptsRunning"] == 0);
    assert!(host.base.join("selected").exists());
}
#[test]
fn retained_old_detail_text_cannot_close_replacement() {
    let host = host("close");
    open(&host);
    click(&host, "details");
    view(&host, |r| r["detail_visible"] == true);
    actions::probe(&host, "retain-result-text");
    click(&host, "choices");
    let old = view(&host, |_| true)["scriptResult"]
        .get("token")
        .unwrap()
        .clone();
    click(&host, "reload");
    let new = view(&host, |r| r.get("token").unwrap() != &old)["scriptResult"]
        .get("token")
        .unwrap()
        .clone();
    click(&host, "details");
    view(&host, |r| r["detail_visible"] == true);
    actions::probe(&host, "stale-result-text");
    host.settled(|_| true);
    assert_eq!(
        view(&host, |_| true)["scriptResult"].get("token").unwrap(),
        &new
    );
}
#[test]
fn post_effect_usage_warning_keeps_reloaded_choices() {
    let host = host("close");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    let usage = host.profile.join("usage.json");
    fs::remove_file(&usage).unwrap();
    fs::create_dir(&usage).unwrap();
    click(&host, "reload");
    let v = view(&host, |r| r.get("token").unwrap() != &old);
    let r = &v["scriptResult"];
    assert!(!r["rows"].as_array().unwrap().is_empty());
    assert!(r["error"]
        .as_str()
        .unwrap()
        .contains("usage could not be saved"));
    assert_eq!(r["busy"], false);
}
#[test]
fn queued_query_before_close_cannot_error_on_restored_search() {
    let host = host("close");
    open(&host);
    host.probe("schedule-result-close", json!({"queryBeforeClose":true}));
    let delay = host.rpc(&json!({"op":"native.delay","executionBarrier":true}));
    assert_eq!(delay["result"]["newIntentDuringDelay"], true);
    let path = host.profile.join("result-close-barrier.json");
    wait_exists(&path);
    let proof: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(proof["queryBeforeClose"], true);
    assert_eq!(proof["queryEpochAdvanced"], true);
    host.settled(|_| true);
    let v = actions::probe(&host, "read");
    assert!(v.get("scriptResult").unwrap().is_null());
    assert!(!v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("no longer open")));
    assert!(!host.base.join("selected").exists());
}
