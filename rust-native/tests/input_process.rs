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
    config["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":"/usr/bin/printf","arguments":["PRIVATE-TARGET-ARG"],"timeout":3,"inputPrompt":"Message"}));
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
#[test]
fn relocated_configuration_refuses_old_draft() {
    let host = host();
    open(&host);
    input_type(&host, "keep this");
    let directory = host.base.join("relocated");
    fs::create_dir(&directory).unwrap();
    assert_eq!(
        host.cli(&["config", "directory", directory.to_str().unwrap()])["ok"],
        true
    );
    actions::probe(&host, "save-editor");
    view(&host, |v| actions::text_contains(v, "folder changed"));
    assert_eq!(actions::probe(&host, "run-input-read")["text"], "keep this");
    input_type(&host, " after relocation");
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn changed_command_preserves_old_input_draft() {
    let host = host();
    open(&host);
    input_type(&host, "old input");
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    command["inputPrompt"] = json!("Different prompt");
    command["target"] = json!("/usr/bin/true");
    command["arguments"] = json!([]);
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    let before = fs::read(&host.config).unwrap();
    actions::probe(&host, "save-editor");
    view(&host, |v| actions::text_contains(v, "changed"));
    assert_eq!(actions::probe(&host, "run-input-read")["text"], "old input");
    input_type(&host, " retained");
    assert_eq!(fs::read(&host.config).unwrap(), before);
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
fn delayed_submit(host: &Bridge, expire: bool) -> Value {
    use std::io::{BufRead, Write};
    host.probe("schedule-input-submit", json!({"expire":expire}));
    let mut stream = host.connect(std::time::Duration::from_secs(4));
    stream.write_all(b"{\"op\":\"native.delay\"}\n").unwrap();
    let path = host.profile.join("input-submit-fired");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "input submission was not scheduled"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut line = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut line)
        .unwrap();
    serde_json::from_str(&line).unwrap()
}
#[test]
fn expired_submission_keeps_editable_draft() {
    let host = host();
    open(&host);
    input_type(&host, "expired draft");
    assert_eq!(
        delayed_submit(&host, true)["result"]["newIntentDuringDelay"],
        true
    );
    view(&host, |v| actions::text_contains(v, "expired"));
    let state = actions::probe(&host, "run-input-read");
    assert_eq!(state["pending"], false);
    assert_eq!(state["editable"], true);
    assert_eq!(state["focused"], true);
    assert_eq!(state["text"], "expired draft");
    input_type(&host, " still editable");
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn queued_cancel_never_starts_execution() {
    let host = host();
    open(&host);
    input_type(&host, "cancel before admission");
    assert_eq!(
        delayed_submit(&host, false)["result"]["newIntentDuringDelay"],
        true
    );
    view(&host, |v| v["formVisible"] == false);
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn validation_preserves_literal_multiline_input_and_private_logs() {
    let host = host();
    let out = host.base.join("result");
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    command["target"] = json!(env!("CARGO_BIN_EXE_private-results-fixture"));
    command["arguments"] = json!(["execution-write", out, "{input}"]);
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    let before = fs::read(&host.config).unwrap();
    open(&host);
    key(&host, 36, "\r", actions::CMD);
    view(&host, |v| actions::text_contains(v, "Enter Message"));
    let state = actions::probe(&host, "run-input-read");
    assert_eq!(state["editable"], true);
    assert_eq!(state["focused"], true);
    assert!(!out.exists());
    let value = "literal $(echo private) Ω\nsecond {uuid} https://private.invalid/token";
    input_type(&host, value);
    assert_eq!(actions::probe(&host, "run-input-read")["text"], value);
    key(&host, 36, "\r", actions::CMD);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !out.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "output was not written"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    host.wait(|s| s["scriptsRunning"] == 0);
    view(&host, |v| v["formVisible"] == false);
    assert_eq!(fs::read_to_string(out).unwrap(), value);
    assert_eq!(fs::read(&host.config).unwrap(), before);
    assert_eq!(
        host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
        1
    );
    let raw = fs::read_to_string(host.profile.join("command-log.json")).unwrap();
    assert!(!raw.contains("private.invalid"));
    assert!(!raw.contains("literal $"));
}
