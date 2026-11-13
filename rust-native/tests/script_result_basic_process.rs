//! Script provider result interpretation through the real headless host.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{Config, Host};
use serde_json::{json, Value};
use std::fs;
fn host() -> Host {
    let mut h = Host::new(Config::current(), false);
    let mut cfg = runtime_host::default_config();
    cfg["commands"]
        .as_array_mut()
        .unwrap()
        .push(command(&h, &json!("fixture result"), 0));
    h.replace(&cfg, None);
    h.start();
    h
}
fn command(h: &Host, value: &Value, status: i32) -> Value {
    json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake",if status==7 {"exit7"} else {"write"},h.base.join("result-marker"),format!("{}\n",value)],"timeout":3})
}
fn run_value(h: &Host, value: &Value, status: i32) -> Value {
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    let commands = cfg["commands"].as_array_mut().unwrap();
    commands.retain(|c| c["id"] != "script.fixture");
    commands.push(command(h, value, status));
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    h.rpc(&json!({"op":"execute","id":"runfixture"}))
}
fn choices() -> Value {
    json!({"title":"Project Koppelen","placeholder":"Zoek project…","items":[{"id":"one","title":"Café Project","subtitle":"EX / PLAN","value":"PRIVATE literal $(value) Ω","extra":"unchanged"}]})
}
fn with_exit(value: &Value) -> Value {
    let mut v = value.clone();
    v["exitCode"] = json!(0);
    v
}
#[test]
fn private_serde_magic_keys_stay_literal_in_real_provider_results() {
    let h = host();
    for value in ["true", "42", "null", "\"literal\"", "{\"enabled\":false}"] {
        let payload = json!({"ok":true,"detail":"Own literal report","extra":{"$serde_json::private::RawValue":value}});
        assert_eq!(run_value(&h, &payload, 0), with_exit(&payload));
    }
    let payload = json!({"ok":true,"detail":"Own literal report","extra":{"$serde_json::private::Number":"1"}});
    assert_eq!(run_value(&h, &payload, 0), with_exit(&payload));
}
#[test]
fn process_failure_overrides_provider_success_but_keeps_detail() {
    let h = host();
    let v = run_value(
        &h,
        &json!({"ok":true,"choices":choices(),"detail":"PRIVATE failure report"}),
        7,
    );
    assert_eq!(v["ok"], false);
    assert_eq!(v["exitCode"], 7);
    assert!(v["error"].as_str().unwrap().contains("status 7"));
    assert_eq!(v["detail"], "PRIVATE failure report");
    assert_eq!(v["choices"], choices());
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
    assert!(!fs::read_to_string(h.profile.join("command-log.json"))
        .unwrap()
        .contains("PRIVATE"));
}
#[test]
fn provider_failure_detail_and_message_survive_a_zero_process_exit() {
    let h = host();
    let mut v = run_value(
        &h,
        &json!({"ok":false,"error":"Provider asks attention","detail":"PRIVATE partial result"}),
        0,
    );
    assert_eq!(
        v.as_object_mut()
            .unwrap()
            .remove("diagnosticID")
            .unwrap()
            .as_str()
            .unwrap()
            .len(),
        36
    );
    assert_eq!(
        v,
        json!({"ok":false,"error":"Provider asks attention","detail":"PRIVATE partial result","exitCode":0})
    );
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn cli_does_not_apply_gui_choice_validation() {
    let h = host();
    for change in [
        "duplicate",
        "missing",
        "empty-title",
        "wrong-value-type",
        "canonical-duplicate",
    ] {
        let mut c = choices();
        match change {
            "duplicate" => {
                let item = c["items"][0].clone();
                c["items"].as_array_mut().unwrap().push(item);
            }
            "missing" => {
                c.as_object_mut().unwrap().remove("placeholder");
            }
            "empty-title" => c["items"][0]["title"] = json!(""),
            "wrong-value-type" => c["items"][0]["value"] = json!(42),
            _ => {
                let mut a = c["items"][0].clone();
                a["id"] = json!("é");
                let mut b = a.clone();
                b["id"] = json!("e\u{301}");
                c["items"] = json!([a, b]);
            }
        }
        let payload = json!({"ok":true,"choices":c,"detail":"PRIVATE validation report"});
        assert_eq!(run_value(&h, &payload, 0), with_exit(&payload));
    }
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"]["script.fixture"], 5);
}
#[test]
fn failed_provider_does_not_validate_or_replace_its_choice_payload() {
    let h = host();
    let payload = json!({"ok":false,"error":"Reload refused","choices":{"not":"a valid list"},"detail":"Keep existing list"});
    let mut v = run_value(&h, &payload, 0);
    assert_eq!(
        v.as_object_mut()
            .unwrap()
            .remove("diagnosticID")
            .unwrap()
            .as_str()
            .unwrap()
            .len(),
        36
    );
    assert_eq!(v, with_exit(&payload));
}
#[test]
fn empty_choice_list_is_valid_and_metadata_extensions_stay_present() {
    let h = host();
    let mut c = choices();
    c["items"] = json!([]);
    c["future"] = json!(true);
    let payload = json!({"ok":true,"choices":c});
    assert_eq!(run_value(&h, &payload, 0), with_exit(&payload));
}
#[test]
fn generic_ok_only_and_plain_json_keep_the_ordinary_runner_wrapper() {
    let h = host();
    for value in [
        json!({"ok":true}),
        json!({"detail":"no boolean discriminator"}),
        json!({"ok":"true","choices":choices()}),
    ] {
        let response = run_value(&h, &value, 0);
        assert_eq!(response["ok"], true);
        assert_eq!(
            serde_json::from_str::<Value>(response["result"]["stdout"].as_str().unwrap()).unwrap(),
            value
        );
    }
}
