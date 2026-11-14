//! Native script-input tests on the isolated hidden host.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::native_bridge::{
    runtime_host::{default_config, Config},
    Bridge,
};
use serde_json::{json, Value};
use std::fs;
fn host() -> Bridge {
    let mut host = Bridge::new(Config::current(), false);
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":"/usr/bin/true","arguments":["PRIVATE-TARGET-ARG"],"timeout":3,"inputPrompt":"Message"}));
    host.replace(&config, None);
    host.start();
    host.settled(|_| true);
    host
}
fn view(host: &Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
    actions::view(host, predicate)
}
fn key(host: &Bridge, code: u32, chars: &str, flags: u64) {
    actions::key(host, code, flags, chars, false);
}
fn open(host: &Bridge) -> Value {
    actions::show(host, "runfixture");
    assert_eq!(actions::probe(host, "read")["windowShown"], false);
    key(host, 36, "\r", 0);
    view(host, |v| v["runInputInline"] == true);
    key(host, 36, "\r", actions::CMD);
    view(host, |v| {
        v["formVisible"] == true
            && v["screenText"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s == "Run ⌘↵")
    })
}
fn input_type(host: &Bridge, value: &str) {
    host.probe("run-input-type", json!({"text":value}));
}
#[test]
fn open_and_cancel_leave_configuration_untouched() {
    let host = host();
    let before = fs::read(&host.config).unwrap();
    let state = open(&host);
    assert!(state["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "Message"));
    assert!(!serde_json::to_string(&state)
        .unwrap()
        .contains("PRIVATE-TARGET-ARG"));
    assert_eq!(actions::probe(&host, "run-input-read")["focused"], true);
    input_type(&host, "PRIVATE-DRAFT");
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
    actions::probe(&host, "cancel-editor");
    view(&host, |v| v["formVisible"] == false);
    assert_eq!(fs::read(&host.config).unwrap(), before);
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
}
#[test]
fn plain_return_inserts_newline_without_running() {
    let host = host();
    open(&host);
    input_type(&host, "First line");
    key(&host, 36, "\r", 0);
    input_type(&host, "Second line");
    assert_eq!(
        actions::probe(&host, "run-input-read")["text"],
        "First line\nSecond line"
    );
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn marked_text_blocks_run_and_escape_without_effects() {
    let host = host();
    open(&host);
    assert_eq!(
        actions::probe(&host, "run-input-ime")["runAndEscapeIgnored"],
        true
    );
    assert_eq!(actions::probe(&host, "read")["formVisible"], true);
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
    host.probe("editor-text-command", json!({"marked":false}));
    view(&host, |v| v["formVisible"] == false);
}
