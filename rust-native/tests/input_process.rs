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
#[test]
fn alias_tail_preserves_literal_whitespace_and_arguments() {
    let host = host();
    let out = host.base.join("result");
    set_execution(
        &host,
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["execution-write", out, "{input}"]),
        3.0,
    );
    let tail = " Ω\t$(echo literal)  end";
    assert_eq!(
        host.rpc(&json!({"op":"show","query":format!("runfixture {tail}")}))["ok"],
        true
    );
    host.settled(|s| s["nativeUI"]["visible"] == true);
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
    view(&host, |v| v["runInputInline"] == true);
    assert_eq!(actions::probe(&host, "run-inline-read")["text"], tail);
    key(&host, 36, "\r", 0);
    wait_exists(&out);
    host.wait(|s| s["scriptsRunning"] == 0);
    inline_closed(&host);
    assert_eq!(fs::read_to_string(out).unwrap(), tail);
    assert_eq!(
        host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
        1
    );
    assert!(!fs::read_to_string(host.profile.join("command-log.json"))
        .unwrap()
        .contains("literal"));
}
#[test]
fn expand_preserves_actual_field_editor_text_and_token_without_execution() {
    let host = host();
    inline_open(&host, false);
    inline_type(&host, "Unicode büro 日本語 $(literal)");
    let before = actions::probe(&host, "run-inline-read");
    key(&host, 36, "\r", actions::CMD);
    let state = view(&host, |v| {
        v["formVisible"] == true && v["runInputInline"] == false
    });
    assert_eq!(
        state.get("editorToken").unwrap(),
        before.get("token").unwrap()
    );
    assert_eq!(
        actions::probe(&host, "run-input-read")["text"],
        before["text"]
    );
    assert_eq!(actions::probe(&host, "run-input-read")["focused"], true);
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
    host.probe("run-input-type", json!({"text":"\nSecond line"}));
    assert!(actions::probe(&host, "run-input-read")["text"]
        .as_str()
        .unwrap()
        .ends_with("\nSecond line"));
}
#[test]
fn empty_run_validation_retains_inline_focus_and_typing() {
    let host = host();
    inline_open(&host, false);
    key(&host, 36, "\r", 0);
    view(&host, |v| actions::text_contains(v, "Enter Message"));
    inline_type(&host, "still editable");
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "still editable"
    );
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn inline_menu_escape_keeps_text_and_keyboard_expand_uses_selection() {
    let host = host();
    inline_open(&host, false);
    inline_type(&host, "menu value");
    key(&host, 40, "k", actions::CMD);
    let menu = view(&host, |v| v["actionsVisible"] == true);
    assert_eq!(
        menu["actionLabels"],
        json!([
            "Run",
            "Expand Input…",
            "Cancel Input",
            "Edit…",
            "Set Alias…",
            "Set Hotkey…",
            "Disable",
            "Remove…",
            "Settings"
        ])
    );
    key(&host, 53, "", 0);
    view(&host, |v| v["actionsVisible"] == false);
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "menu value"
    );
    inline_type(&host, " retained");
    key(&host, 40, "k", actions::CMD);
    view(&host, |v| v["actionsVisible"] == true);
    key(&host, 125, "", 0);
    key(&host, 36, "\r", 0);
    view(&host, |v| {
        v["formVisible"] == true && v["runInputInline"] == false
    });
    assert_eq!(
        actions::probe(&host, "run-input-read")["text"],
        "menu value retained"
    );
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn inline_menu_clicked_expand_and_cancel() {
    let host = host();
    inline_open(&host, false);
    inline_type(&host, "clicked");
    key(&host, 40, "k", actions::CMD);
    view(&host, |v| v["actionsVisible"] == true);
    host.probe("action-button", json!({"index":1}));
    view(&host, |v| {
        v["formVisible"] == true && v["runInputInline"] == false
    });
    assert_eq!(actions::probe(&host, "run-input-read")["text"], "clicked");
    actions::probe(&host, "cancel-editor");
    inline_closed(&host);
    inline_open(&host, false);
    inline_type(&host, "discard");
    key(&host, 40, "k", actions::CMD);
    view(&host, |v| v["actionsVisible"] == true);
    host.probe("action-button", json!({"index":2}));
    inline_closed(&host);
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
fn delayed_inline(host: &Bridge, fields: Value) {
    use std::io::{BufRead, Write};
    host.probe("schedule-inline-control", fields);
    let mut stream = host.connect(std::time::Duration::from_secs(4));
    stream.write_all(b"{\"op\":\"native.delay\"}\n").unwrap();
    wait_exists(&host.profile.join("inline-control-fired"));
    let mut line = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut line)
        .unwrap();
    let result: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(result["result"]["newIntentDuringDelay"], true);
    host.settled(|_| true);
}
#[test]
fn pending_menu_cancel_keeps_inline_input_and_typing() {
    let host = host();
    inline_open(&host, false);
    inline_type(&host, "menu pending");
    let initial = actions::probe(&host, "run-inline-read");
    let token = initial.get("token").unwrap().clone();
    delayed_inline(&host, json!({"operation":"menu","cancel":true}));
    assert_eq!(actions::probe(&host, "read")["runInputInline"], true);
    assert_eq!(actions::probe(&host, "read")["actionsVisible"], false);
    let state = actions::probe(&host, "run-inline-read");
    assert_eq!(state["text"], "menu pending continued");
    assert_eq!(state.get("token").unwrap(), &token);
    inline_type(&host, " now");
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn expired_expand_releases_inline_input_without_losing_text() {
    let host = host();
    inline_open(&host, false);
    inline_type(&host, "expired expand");
    delayed_inline(&host, json!({"operation":"expand","expire":true}));
    view(&host, |v| actions::text_contains(v, "expired"));
    assert_eq!(actions::probe(&host, "read")["runInputInline"], true);
    assert_eq!(actions::probe(&host, "read")["formVisible"], false);
    let state = actions::probe(&host, "run-inline-read");
    assert_eq!(state["pending"], false);
    assert_eq!(state["text"], "expired expand");
    inline_type(&host, " retained");
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn empty_menu_run_restores_direct_typing() {
    let host = host();
    inline_open(&host, false);
    key(&host, 40, "k", actions::CMD);
    view(&host, |v| v["actionsVisible"] == true);
    host.probe("action-button", json!({"index":0}));
    view(&host, |v| {
        v["actionsVisible"] == false && actions::text_contains(v, "Enter Message")
    });
    inline_type(&host, "repaired");
    assert_eq!(actions::probe(&host, "run-inline-read")["text"], "repaired");
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn native_expand_selectors_preserve_text_without_executing() {
    let host = host();
    for selector in ["line-break", "newline-ignore"] {
        inline_open(&host, false);
        inline_type(&host, "selector value");
        host.probe("run-inline-selector", json!({"selector":selector}));
        view(&host, |v| {
            v["formVisible"] == true && v["runInputInline"] == false
        });
        assert_eq!(
            actions::probe(&host, "run-input-read")["text"],
            "selector value"
        );
        actions::probe(&host, "cancel-editor");
        inline_closed(&host);
    }
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn reload_usage_and_theme_keep_inline_editor_undo_and_composition() {
    let host = host();
    inline_open(&host, false);
    inline_type(&host, "before");
    let initial = actions::probe(&host, "run-inline-read");
    assert_eq!(initial["canUndo"], true);
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    config["theme"] = json!("light");
    config["commands"].as_array_mut().unwrap().push(json!({"id":"script.other","title":"Other","kind":"script","target":"/usr/bin/true","arguments":[]}));
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    assert_eq!(
        host.rpc(&json!({"op":"execute","id":"script.other"}))["ok"],
        true
    );
    host.settled(|_| true);
    let after = actions::probe(&host, "run-inline-read");
    assert_eq!(after["text"], "before");
    assert_eq!(
        after.get("editorAddress").unwrap(),
        initial.get("editorAddress").unwrap()
    );
    assert_eq!(after["canUndo"], true);
    assert_eq!(after.get("token").unwrap(), initial.get("token").unwrap());
    host.probe("run-inline-mark", json!({"text":"にほん"}));
    let marked = actions::probe(&host, "run-inline-read");
    assert_eq!(marked["marked"], true);
    config["theme"] = json!("dark");
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    host.settled(|_| true);
    let after = actions::probe(&host, "run-inline-read");
    assert_eq!(after["marked"], true);
    for field in ["text", "editorAddress", "selectedLocation"] {
        assert_eq!(after.get(field).unwrap(), marked.get(field).unwrap());
    }
    actions::probe(&host, "run-inline-ime-guards");
    assert_eq!(
        host.rpc(&json!({"op":"logs"}))["result"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    actions::probe(&host, "run-inline-unmark");
    inline_type(&host, " continued");
    assert!(actions::probe(&host, "run-inline-read")["text"]
        .as_str()
        .unwrap()
        .ends_with(" continued"));
}
#[test]
fn old_menu_button_cannot_act_on_reopened_menu_or_new_input() {
    let host = host();
    inline_open(&host, false);
    inline_type(&host, "original");
    key(&host, 40, "k", actions::CMD);
    view(&host, |v| v["actionsVisible"] == true);
    host.probe("retain-input-button", json!({"index":1}));
    key(&host, 53, "", 0);
    view(&host, |v| v["actionsVisible"] == false);
    key(&host, 40, "k", actions::CMD);
    view(&host, |v| v["actionsVisible"] == true);
    actions::probe(&host, "old-input-button");
    assert_eq!(actions::probe(&host, "read")["runInputInline"], true);
    assert_eq!(actions::probe(&host, "read")["actionsVisible"], true);
    key(&host, 53, "", 0);
    view(&host, |v| v["actionsVisible"] == false);
    key(&host, 53, "", 0);
    inline_closed(&host);
    inline_open(&host, false);
    inline_type(&host, "new");
    actions::probe(&host, "old-input-button");
    assert_eq!(actions::probe(&host, "run-inline-read")["text"], "new");
    assert_eq!(actions::probe(&host, "read")["formVisible"], false);
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn light_dark_inline_and_menu_captures() {
    let host = host();
    for theme in ["light", "dark"] {
        let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
        config["theme"] = json!(theme);
        host.replace(&config, None);
        assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
        inline_open(&host, false);
        inline_type(&host, "Unicode büro 日本語");
        host.capture(&format!("run-inline-{theme}"));
        key(&host, 40, "k", actions::CMD);
        view(&host, |v| v["actionsVisible"] == true);
        host.capture(&format!("run-inline-menu-{theme}"));
        key(&host, 53, "", 0);
        view(&host, |v| v["actionsVisible"] == false);
        key(&host, 53, "", 0);
        inline_closed(&host);
    }
}
#[test]
fn script_failure_restores_inline_text_token_and_typing() {
    let host = host();
    set_execution(
        &host,
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["execution-fail"]),
        3.0,
    );
    inline_open(&host, false);
    inline_type(&host, "keep retry text");
    let initial = actions::probe(&host, "run-inline-read");
    let token = initial.get("token").unwrap().clone();
    key(&host, 36, "\r", 0);
    host.wait(|s| s["scriptsRunning"] == 0);
    view(&host, |v| actions::text_contains(v, "7"));
    let state = actions::probe(&host, "run-inline-read");
    assert_eq!(state["text"], "keep retry text");
    assert_eq!(state.get("token").unwrap(), &token);
    inline_type(&host, " retained");
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "keep retry text retained"
    );
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn cancel_running_inline_input_does_not_reopen_old_draft() {
    let host = RunningHost(host());
    let pidfile = host.base.join("leader");
    set_execution(
        &host,
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["execution-delay", pidfile, "2000", ""]),
        3.0,
    );
    inline_open(&host, false);
    inline_type(&host, "cancel active");
    key(&host, 36, "\r", 0);
    wait_exists(&pidfile);
    key(&host, 53, "", 0);
    inline_closed(&host);
    host.wait(|s| s["scriptsRunning"] == 0);
    let pid: i32 = fs::read_to_string(&pidfile).unwrap().parse().unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    inline_open(&host, false);
    inline_type(&host, "new draft");
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "new draft"
    );
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn newer_query_during_inline_run_finishes_without_hiding_search() {
    let (host, ready, gate, done) = waiting_input();
    inline_open(&host, false);
    inline_type(&host, "own fixture input");
    key(&host, 36, "\r", 0);
    wait_exists(&ready);
    host.probe("run-input-new-query", json!({"query":"finder"}));
    host.settled(|_| true);
    finish_waiting_input(&host, &gate, &done);
    assert_eq!(inline_closed(&host)["input"], "finder");
    assert_eq!(
        host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        true
    );
    assert_eq!(
        host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
        1
    );
}
fn input_actions_fixture() -> Bridge {
    let host = host();
    set_execution(&host, "/usr/bin/printf", json!(["fixture result"]), 3.0);
    inline_open(&host, false);
    inline_type(&host, "PRIVATE literal Ω");
    host
}
fn input_action(host: &Bridge, title: &str) {
    key(host, 40, "k", actions::CMD);
    let menu = view(host, |v| v["actionsVisible"] == true);
    let index = menu["actionLabels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|v| v == title)
        .unwrap();
    host.probe("action-button", json!({"index":index}));
}
#[test]
fn menu_cancel_while_busy_preserves_new_typing() {
    use std::io::{BufRead, Write};
    let host = input_actions_fixture();
    key(&host, 40, "k", actions::CMD);
    view(&host, |v| v["actionsVisible"] == true);
    let before = fs::read(&host.config).unwrap();
    actions::probe(&host, "schedule-input-action-cancel");
    let mut stream = host.connect(std::time::Duration::from_secs(4));
    stream.write_all(b"{\"op\":\"native.delay\"}\n").unwrap();
    wait_exists(&host.profile.join("input-action-cancel-fired"));
    let mut line = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut line)
        .unwrap();
    let value: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["result"]["newIntentDuringDelay"], true);
    host.settled(|_| true);
    view(&host, |v| {
        v["runInputInline"] == true && v["actionsVisible"] == false
    });
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω continued"
    );
    assert_eq!(fs::read(&host.config).unwrap(), before);
    assert_eq!(actions::probe(&host, "read")["formVisible"], false);
}
#[test]
fn external_removal_before_edit_keeps_input() {
    let host = input_actions_fixture();
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    config["commands"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["id"] != "script.fixture");
    host.replace(&config, None);
    input_action(&host, "Set Alias…");
    view(&host, |v| {
        v["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().to_lowercase().contains("changed"))
    });
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
    assert_eq!(actions::probe(&host, "read")["formVisible"], false);
}
#[test]
fn relocated_config_before_edit_keeps_suspended_input() {
    let host = input_actions_fixture();
    let folder = host.base.join("relocated");
    fs::create_dir(&folder).unwrap();
    assert_eq!(
        host.cli(&["config", "directory", folder.to_str().unwrap()])["ok"],
        true
    );
    let before = fs::read(&host.config).unwrap();
    input_action(&host, "Set Alias…");
    view(&host, |v| {
        v["screenText"].as_array().unwrap().iter().any(|s| {
            s.as_str()
                .unwrap()
                .to_lowercase()
                .contains("folder changed")
        })
    });
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
    assert_eq!(fs::read(&host.config).unwrap(), before);
}
fn input_action_form(host: &Bridge) -> Value {
    view(host, |v| {
        v["formVisible"] == true && v["runInputInline"] == false
    })
}
fn input_action_resumed(host: &Bridge) -> Value {
    host.settled(|_| true);
    view(host, |v| {
        v["runInputInline"] == true && v["actionsVisible"] == false
    })
}
fn input_saved_command(host: &Bridge) -> Value {
    let config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    config["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "script.fixture")
        .unwrap()
        .clone()
}
#[test]
fn each_config_editor_cancel_resumes_fresh_input_without_writing() {
    let host = input_actions_fixture();
    let before = fs::read(&host.config).unwrap();
    for action in ["Set Alias…", "Set Hotkey…", "Edit…", "Remove…"] {
        let old = actions::probe(&host, "run-inline-read")["token"]
            .as_u64()
            .unwrap();
        input_action(&host, action);
        let child = input_action_form(&host)["editorToken"].as_u64().unwrap();
        assert!(child > old);
        actions::probe(&host, "cancel-editor");
        let state = input_action_resumed(&host);
        assert!(state["editorToken"].as_u64().unwrap() > child);
        assert_eq!(
            actions::probe(&host, "run-inline-read")["text"],
            "PRIVATE literal Ω"
        );
        assert_eq!(fs::read(&host.config).unwrap(), before);
    }
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn alias_and_hotkey_saves_resume_same_input() {
    let host = input_actions_fixture();
    input_action(&host, "Set Alias…");
    input_action_form(&host);
    host.probe("alias-input", json!({"text":"newalias"}));
    actions::probe(&host, "save-editor");
    input_action_resumed(&host);
    assert_eq!(input_saved_command(&host)["aliases"], json!(["newalias"]));
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
    input_action(&host, "Set Hotkey…");
    input_action_form(&host);
    key(&host, 45, "n", actions::CMD);
    input_action_resumed(&host);
    assert_eq!(input_saved_command(&host)["hotkey"], "cmd+n");
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
    assert!(!fs::read_to_string(&host.config)
        .unwrap()
        .contains("PRIVATE"));
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn failed_alias_save_retains_both_drafts_until_cancel() {
    let host = input_actions_fixture();
    let before = fs::read(&host.config).unwrap();
    input_action(&host, "Set Alias…");
    input_action_form(&host);
    host.probe("alias-input", json!({"text":"safari"}));
    actions::probe(&host, "save-editor");
    view(&host, |v| {
        v["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().to_lowercase().contains("duplicate"))
    });
    assert_eq!(fs::read(&host.config).unwrap(), before);
    assert_eq!(actions::probe(&host, "read")["aliasValue"], "safari");
    actions::probe(&host, "cancel-editor");
    input_action_resumed(&host);
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
}
#[test]
fn alias_save_does_not_adopt_external_timeout_or_prompt() {
    let host = input_actions_fixture();
    input_action(&host, "Set Alias…");
    input_action_form(&host);
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let c = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    c["timeout"] = json!(2);
    c["inputPrompt"] = json!("Other prompt");
    host.replace(&config, None);
    host.probe("alias-input", json!({"text":"mine"}));
    actions::probe(&host, "save-editor");
    input_action_resumed(&host);
    key(&host, 36, "\r", 0);
    view(&host, |v| {
        v["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().to_lowercase().contains("changed"))
    });
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn cancel_unpresented_child_preserves_live_ime_and_undo() {
    let host = input_actions_fixture();
    let before = fs::read(&host.config).unwrap();
    key(&host, 40, "k", actions::CMD);
    let menu = view(&host, |v| v["actionsVisible"] == true);
    host.probe("defer-input-child", json!({"defer":true}));
    let index = menu["actionLabels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|v| v == "Set Alias…")
        .unwrap();
    host.probe("action-button", json!({"index":index}));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while host.probe("defer-input-child", json!({"defer":true}))["pendingChild"] != true {
        assert!(
            std::time::Instant::now() < deadline,
            "owner did not publish deferred child"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    key(&host, 53, "", 0);
    inline_type(&host, " continued");
    host.probe("run-inline-mark", json!({"text":"日本"}));
    let marked = actions::probe(&host, "run-inline-read");
    assert_eq!(marked["marked"], true);
    host.probe("defer-input-child", json!({"defer":false}));
    input_action_resumed(&host);
    let after = actions::probe(&host, "run-inline-read");
    assert_eq!(after.get("text").unwrap(), marked.get("text").unwrap());
    assert_eq!(
        after.get("editorAddress").unwrap(),
        marked.get("editorAddress").unwrap()
    );
    assert_eq!(after["marked"], true);
    assert_eq!(after["canUndo"], true);
    assert_eq!(fs::read(&host.config).unwrap(), before);
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn queued_child_save_cancel_resumes_without_publishing() {
    let host = input_actions_fixture();
    let before = fs::read(&host.config).unwrap();
    input_action(&host, "Set Alias…");
    let child = input_action_form(&host)["editorToken"].as_u64().unwrap();
    host.probe("alias-input", json!({"text":"must-not-save"}));
    actions::probe(&host, "schedule-save-cancel");
    assert_eq!(host.rpc(&json!({"op":"native.delay"}))["ok"], true);
    let state = input_action_resumed(&host);
    assert!(state["editorToken"].as_u64().unwrap() > child);
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
    assert_eq!(fs::read(&host.config).unwrap(), before);
    inline_type(&host, " still typing");
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω still typing"
    );
}
#[test]
fn hiding_child_drops_suspended_input() {
    let host = input_actions_fixture();
    input_action(&host, "Set Alias…");
    input_action_form(&host);
    actions::probe(&host, "hide-palette");
    host.settled(|s| s["nativeUI"]["visible"] == false);
    actions::show(&host, "runfixture");
    let state = actions::probe(&host, "read");
    assert_eq!(state["windowShown"], false);
    assert_eq!(state["runInputInline"], false);
    assert_eq!(state["formVisible"], false);
}
#[test]
fn settings_peer_preserves_primary_input() {
    let host = input_actions_fixture();
    let before = fs::read(&host.config).unwrap();
    input_action(&host, "Settings");
    input_action_resumed(&host);
    host.wait(|s| s["managementUI"]["visible"] == true);
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
    assert_eq!(fs::read(&host.config).unwrap(), before);
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn disable_and_enable_keep_input_but_disabled_run_cannot_spawn() {
    let host = input_actions_fixture();
    input_action(&host, "Disable");
    input_action_resumed(&host);
    assert_eq!(input_saved_command(&host)["enabled"], false);
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
    key(&host, 36, "\r", 0);
    view(&host, |v| {
        v["screenText"].as_array().unwrap().iter().any(|s| {
            let s = s.as_str().unwrap().to_lowercase();
            s.contains("disabled") || s.contains("changed")
        })
    });
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
    key(&host, 40, "k", actions::CMD);
    let menu = view(&host, |v| v["actionsVisible"] == true);
    let labels = menu["actionLabels"].as_array().unwrap();
    assert!(!labels.iter().any(|v| v == "Run"));
    let index = labels.iter().position(|v| v == "Enable").unwrap();
    host.probe("action-button", json!({"index":index}));
    input_action_resumed(&host);
    assert_eq!(input_saved_command(&host)["enabled"], true);
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
}
#[test]
fn remove_commit_discards_input_and_never_executes() {
    let host = input_actions_fixture();
    input_action(&host, "Remove…");
    input_action_form(&host);
    actions::probe(&host, "save-editor");
    inline_closed(&host);
    let config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    assert!(!config["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "script.fixture"));
    assert_eq!(host.rpc(&json!({"op":"logs"}))["result"], json!([]));
}
#[test]
fn light_dark_input_actions_and_resumed_captures() {
    let host = input_actions_fixture();
    for theme in ["light", "dark"] {
        let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
        config["theme"] = json!(theme);
        host.replace(&config, None);
        assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
        host.settled(|_| true);
        key(&host, 40, "k", actions::CMD);
        view(&host, |v| v["actionsVisible"] == true);
        host.capture(&format!("input-actions-menu-{theme}"));
        let menu = actions::probe(&host, "read");
        let index = menu["actionLabels"]
            .as_array()
            .unwrap()
            .iter()
            .position(|v| v == "Set Alias…")
            .unwrap();
        host.probe("action-button", json!({"index":index}));
        input_action_form(&host);
        host.capture(&format!("input-actions-alias-{theme}"));
        actions::probe(&host, "cancel-editor");
        input_action_resumed(&host);
        host.capture(&format!("input-actions-resumed-{theme}"));
    }
}
#[test]
fn full_editor_adopts_own_arguments_after_save_without_running() {
    let host = RunningHost(host());
    let out = host.base.join("result");
    set_execution(
        &host,
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["execution-write", "unused"]),
        3.0,
    );
    inline_open(&host, false);
    inline_type(&host, "PRIVATE literal Ω");
    input_action(&host, "Edit…");
    input_action_form(&host);
    host.probe("command-argument", json!({"index":1,"text":out}));
    actions::probe(&host, "add-argument");
    host.probe("command-argument", json!({"index":2,"text":"{input}"}));
    actions::probe(&host, "save-editor");
    input_action_resumed(&host);
    assert!(!out.exists());
    assert_eq!(
        actions::probe(&host, "run-inline-read")["text"],
        "PRIVATE literal Ω"
    );
    key(&host, 36, "\r", 0);
    wait_exists(&out);
    host.wait(|s| s["scriptsRunning"] == 0);
    inline_closed(&host);
    assert_eq!(fs::read_to_string(out).unwrap(), "PRIVATE literal Ω");
}
