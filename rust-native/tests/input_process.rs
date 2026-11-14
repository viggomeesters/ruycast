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
fn set_execution(host: &Bridge, target: &str, args: Value, timeout: f64) {
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    command["target"] = json!(target);
    command["arguments"] = args;
    command["timeout"] = json!(timeout);
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
}
#[test]
fn failure_preserves_text_focus_typing_and_undo() {
    let host = host();
    set_execution(
        &host,
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["execution-fail"]),
        3.0,
    );
    let token = open(&host)["editorToken"].clone();
    input_type(&host, "Keep me");
    actions::probe(&host, "save-editor");
    host.wait(|s| s["scriptsRunning"] == 0);
    view(&host, |v| actions::text_contains(v, "7"));
    let state = actions::probe(&host, "run-input-read");
    assert_eq!(state["editable"], true);
    assert_eq!(state["focused"], true);
    assert_eq!(state["canUndo"], true);
    assert_eq!(state["text"], "Keep me");
    assert_eq!(actions::probe(&host, "read")["editorToken"], token);
    input_type(&host, " and this");
    assert_eq!(
        actions::probe(&host, "run-input-read")["text"],
        "Keep me and this"
    );
    key(&host, 6, "z", actions::CMD);
    assert_ne!(
        actions::probe(&host, "run-input-read")["text"],
        "Keep me and this"
    );
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn launch_failure_keeps_input_and_typing() {
    let host = host();
    set_execution(
        &host,
        "/nonexistent-ruycast-fixture",
        json!(["{input}"]),
        3.0,
    );
    open(&host);
    input_type(&host, "launch draft");
    actions::probe(&host, "save-editor");
    view(&host, |v| {
        v["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().to_lowercase().contains("such file"))
    });
    host.wait(|s| s["scriptsRunning"] == 0);
    assert_eq!(
        actions::probe(&host, "run-input-read")["text"],
        "launch draft"
    );
    input_type(&host, " retained");
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn timeout_keeps_input_and_typing() {
    let host = host();
    set_execution(&host, "/bin/sleep", json!(["1"]), 0.1);
    open(&host);
    input_type(&host, "timeout draft");
    actions::probe(&host, "save-editor");
    view(&host, |v| {
        v["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().to_lowercase().contains("timed out"))
    });
    host.wait(|s| s["scriptsRunning"] == 0);
    assert_eq!(
        actions::probe(&host, "run-input-read")["text"],
        "timeout draft"
    );
    input_type(&host, " retained");
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn completed_detail_replaces_input_without_reoffering() {
    let host = host();
    let marker = host.base.join("detail-marker");
    set_execution(
        &host,
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["fake", "date-detail", marker, "own fixture detail"]),
        3.0,
    );
    open(&host);
    input_type(&host, "own fixture input");
    actions::probe(&host, "save-editor");
    host.wait(|s| s["scriptsRunning"] == 0);
    let state = view(&host, |v| {
        v["formVisible"] == false && !v["scriptResult"].is_null()
    });
    assert!(state.get("editorToken").unwrap().is_null());
    assert_eq!(state["resultPanel"]["detailVisible"], true);
    assert_eq!(state["resultPanel"]["editable"], false);
    assert_eq!(state["resultPanel"]["detail"], "own fixture detail");
    assert_eq!(
        host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
        1
    );
    host.probe("result-control", json!({"name":"close"}));
    host.settled(|_| true);
    let state = actions::probe(&host, "read");
    assert!(state.get("editorToken").unwrap().is_null());
    assert_eq!(state["formVisible"], false);
    assert!(state.get("scriptResult").unwrap().is_null());
}
#[test]
fn usage_write_failure_never_reoffers_completed_action() {
    let host = host();
    let counter = host.base.join("counter");
    set_execution(
        &host,
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["execution-write", counter, "once"]),
        3.0,
    );
    open(&host);
    input_type(&host, "own fixture input");
    fs::create_dir(host.profile.join("usage.json")).unwrap();
    actions::probe(&host, "save-editor");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !counter.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "fixture did not execute"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    host.wait(|s| s["scriptsRunning"] == 0);
    view(&host, |v| {
        v["formVisible"] == false && actions::text_contains(v, "Action completed")
    });
    actions::probe(&host, "save-editor");
    assert_eq!(fs::read_to_string(counter).unwrap(), "once");
    assert_eq!(
        host.rpc(&json!({"op":"logs"}))["result"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
struct RunningHost(Bridge);
impl std::ops::Deref for RunningHost {
    type Target = Bridge;
    fn deref(&self) -> &Bridge {
        &self.0
    }
}
impl Drop for RunningHost {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.0.host.stop(true)));
        }
    }
}
fn wait_exists(path: &std::path::Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "fixture did not publish {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
fn waiting_input() -> (
    RunningHost,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let host = RunningHost(host());
    let ready = host.base.join("ready");
    let gate = host.base.join("gate");
    let done = host.base.join("done");
    set_execution(
        &host,
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["fake", "gated", ready, gate, done, "completed"]),
        3.0,
    );
    (host, ready, gate, done)
}
fn start_waiting_input(host: &Bridge, ready: &std::path::Path) {
    open(host);
    input_type(host, "own fixture input");
    actions::probe(host, "save-editor");
    wait_exists(ready);
}
fn finish_waiting_input(host: &Bridge, gate: &std::path::Path, done: &std::path::Path) {
    fs::write(gate, "go").unwrap();
    wait_exists(done);
    host.wait(|s| s["scriptsRunning"] == 0);
}
#[test]
fn unrelated_cli_and_config_refresh_do_not_strand_completion() {
    for mode in ["usage", "config"] {
        let (host, ready, gate, done) = waiting_input();
        let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
        config["commands"].as_array_mut().unwrap().push(json!({"id":"script.other","title":"Other fixture","kind":"script","target":"/usr/bin/true","arguments":[]}));
        host.replace(&config, None);
        assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
        start_waiting_input(&host, &ready);
        if mode == "usage" {
            assert_eq!(
                host.rpc(&json!({"op":"execute","id":"script.other"}))["ok"],
                true
            );
            assert_eq!(
                host.rpc(&json!({"op":"usage"}))["result"]["script.other"],
                1
            );
        } else {
            config["commands"]
                .as_array_mut()
                .unwrap()
                .last_mut()
                .unwrap()["title"] = json!("Changed unrelated title");
            host.replace(&config, None);
            assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
        }
        finish_waiting_input(&host, &gate, &done);
        view(&host, |v| v["formVisible"] == false);
        assert_eq!(
            host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
            1
        );
    }
}
#[test]
fn newer_query_preserves_palette_and_finishes_owned_input() {
    let (host, ready, gate, done) = waiting_input();
    start_waiting_input(&host, &ready);
    host.probe("run-input-new-query", json!({"query":"finder"}));
    host.settled(|_| true);
    finish_waiting_input(&host, &gate, &done);
    let state = view(&host, |v| v["formVisible"] == false);
    assert_eq!(state["input"], "finder");
    assert_eq!(
        host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        true
    );
    assert_eq!(
        host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
        1
    );
}
#[test]
fn changed_command_after_effect_closes_input_with_warning() {
    let (host, ready, gate, done) = waiting_input();
    start_waiting_input(&host, &ready);
    set_execution(&host, "/usr/bin/true", json!([]), 3.0);
    finish_waiting_input(&host, &gate, &done);
    view(&host, |v| {
        v["formVisible"] == false && actions::text_contains(v, "Action completed")
    });
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn cancel_running_input_does_not_close_new_draft() {
    let host = RunningHost(host());
    let pidfile = host.base.join("leader");
    set_execution(
        &host,
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["execution-delay", pidfile, "2000", ""]),
        3.0,
    );
    let old = open(&host)["editorToken"].clone();
    input_type(&host, "cancel me");
    actions::probe(&host, "save-editor");
    wait_exists(&pidfile);
    actions::probe(&host, "cancel-editor");
    view(&host, |v| v["formVisible"] == false);
    host.wait(|s| s["scriptsRunning"] == 0);
    let pid: i32 = fs::read_to_string(&pidfile).unwrap().parse().unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    let new = open(&host)["editorToken"].clone();
    assert_ne!(old, new);
    input_type(&host, "new draft");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(actions::probe(&host, "run-input-read")["text"], "new draft");
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn light_dark_input_and_validation_capture() {
    let host = host();
    for theme in ["light", "dark"] {
        let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
        config["theme"] = json!(theme);
        host.replace(&config, None);
        assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
        open(&host);
        actions::probe(&host, "save-editor");
        view(&host, |v| actions::text_contains(v, "Enter Message"));
        host.capture(&format!("input-validation-{theme}"));
        actions::probe(&host, "cancel-editor");
        view(&host, |v| v["formVisible"] == false);
        open(&host);
        input_type(&host, "Unicode büro 日本語\nSecond line");
        host.capture(&format!("input-{theme}"));
        actions::probe(&host, "cancel-editor");
        view(&host, |v| v["formVisible"] == false);
    }
}
fn inline_open(host: &Bridge, tab: bool) {
    actions::show(host, "runfixture");
    assert_eq!(actions::probe(host, "read")["windowShown"], false);
    key(
        host,
        if tab { 48 } else { 36 },
        if tab { "\t" } else { "\r" },
        0,
    );
    view(host, |v| v["runInputInline"] == true);
}
fn inline_type(host: &Bridge, text: &str) {
    host.probe("run-inline-type", json!({"text":text}));
}
fn inline_closed(host: &Bridge) -> Value {
    view(host, |v| {
        v["runInputInline"] == false && v["formVisible"] == false
    })
}
#[test]
fn return_and_tab_open_inline_input_cancel_restores_query() {
    let host = host();
    let before = fs::read(&host.config).unwrap();
    for tab in [false, true] {
        inline_open(&host, tab);
        inline_type(&host, "PRIVATE literal Ω");
        assert_eq!(
            actions::probe(&host, "run-inline-read")["text"],
            "PRIVATE literal Ω"
        );
        assert_eq!(actions::probe(&host, "read")["modelQuery"], "runfixture");
        assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
        key(&host, 53, "", 0);
        assert_eq!(inline_closed(&host)["input"], "runfixture");
        assert_eq!(fs::read(&host.config).unwrap(), before);
    }
    actions::probe(&host, "assert-hidden-order-block");
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
    assert!(
        actions::probe(&host, "read")["blockedOrders"]
            .as_u64()
            .unwrap()
            > 0
    );
}
#[test]
fn alias_capture_cancel_restores_only_alias() {
    let host = host();
    assert_eq!(
        host.rpc(&json!({"op":"show","query":"runfixture\t  raw tail"}))["ok"],
        true
    );
    host.settled(|s| s["nativeUI"]["visible"] == true);
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
    view(&host, |v| v["runInputInline"] == true);
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "  raw tail"
    );
    key(&host, 53, "", 0);
    assert_eq!(inline_closed(&host)["input"], "runfixture");
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn composing_alias_does_not_capture_until_commit() {
    let host = host();
    actions::show(&host, "runfixture");
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
    host.probe("query-composing", json!({"text":"runfixture 日本語"}));
    host.settled(|_| true);
    assert_eq!(actions::probe(&host, "read")["runInputInline"], false);
    actions::probe(&host, "query-commit");
    view(&host, |v| v["runInputInline"] == true);
    assert_eq!(actions::probe(&host, "run-inline-read")["text"], "日本語");
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
