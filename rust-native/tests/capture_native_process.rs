//! Native incomplete-output handling on the owned hidden host.
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
fn host(args: Value) -> Bridge {
    let mut h = Bridge::new(Config::current(), false);
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_script-probe"),"arguments":args,"timeout":3,"inputPrompt":"Message"}));
    h.replace(&cfg, None);
    h.start();
    h.settled(|_| true);
    h
}
fn open(h: &Bridge) -> Value {
    show(h, "runfixture");
    assert_eq!(probe(h, "read")["windowShown"], false);
    key(h, 36, 0, "\r", false);
    view(h, |v| v["runInputInline"] == true);
    key(h, 36, CMD, "\r", false);
    view(h, |v| {
        v["formVisible"] == true
            && v["screenText"]
                .as_array()
                .unwrap()
                .contains(&json!("Run ⌘↵"))
    })
}
fn type_text(h: &Bridge, text: &str) {
    h.probe("run-input-type", json!({"text":text}));
}
fn idle(h: &Bridge) {
    h.wait(|s| s["scriptsRunning"] == 0);
}
#[test]
fn incomplete_or_altered_success_warns_and_does_not_offer_completed_input_again() {
    for (index, args) in [
        json!(["capture-fixture", "bytes", "stdout", "65537"]),
        json!(["capture-fixture", "native-invalid"]),
        json!(["capture-fixture", "spoof-prefix"]),
    ]
    .into_iter()
    .enumerate()
    {
        let h = host(args);
        open(&h);
        type_text(&h, "PRIVATE input");
        probe(&h, "save-editor");
        idle(&h);
        view(&h, |v| {
            v["formVisible"] == false && text_contains(v, "incomplete or invalid")
        });
        assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
        assert!(!fs::read_to_string(h.profile.join("command-log.json"))
            .unwrap()
            .contains("PRIVATE"));
        h.capture(&format!("capture-success-warning-{index}"));
    }
}
#[test]
fn incomplete_process_failure_retains_input_and_actual_process_error() {
    let h = host(json!(["capture-fixture", "native-failure"]));
    let token = open(&h)["editorToken"].clone();
    type_text(&h, "PRIVATE keep");
    probe(&h, "save-editor");
    idle(&h);
    view(&h, |v| {
        v["formVisible"] == true && text_contains(v, "status 7")
    });
    assert_eq!(probe(&h, "read")["editorToken"], token);
    assert_eq!(probe(&h, "run-input-read")["text"], "PRIVATE keep");
    type_text(&h, " continued");
    assert_eq!(
        probe(&h, "run-input-read")["text"],
        "PRIVATE keep continued"
    );
    h.capture("capture-failure-keeps-input");
}
#[test]
fn stderr_loss_does_not_reject_otherwise_complete_stdout() {
    let h = host(json!(["capture-fixture", "stderr-invalid"]));
    open(&h);
    type_text(&h, "input");
    probe(&h, "save-editor");
    idle(&h);
    let v = view(&h, |v| {
        v["formVisible"] == false && !v["scriptResult"].is_null()
    });
    assert_eq!(v["resultPanel"]["detailVisible"], true);
    assert_eq!(v["resultPanel"]["editable"], false);
    assert_eq!(v["resultPanel"]["detail"], "valid");
    assert!(!text_contains(&v, "incomplete or invalid"));
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"]["script.fixture"], 1);
}
