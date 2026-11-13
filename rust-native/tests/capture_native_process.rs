//! Native incomplete-output handling on the owned hidden host.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
#[path = "support/capture_holder.rs"]
mod capture_holder;
use actions::{
    native_bridge::{
        runtime_host::{default_config, Config},
        Bridge,
    },
    *,
};
use capture_holder::Holder;
use serde_json::{json, Value};
use std::fs;
#[test]
fn late_discovery_preserves_real_input_text_token_and_undo() {
    use std::time::{Duration, Instant};
    let mut h = Bridge::new(Config::current(), false);
    h.env
        .insert("RUYCAST_NATIVE_TEST_APP_BARRIER".into(), "1".into());
    let contents = h.applications.join("App.app/Contents");
    fs::create_dir_all(&contents).unwrap();
    fs::write(contents.join("Info.plist"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.fixture.late</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleName</key><string>Fixture Late</string></dict></plist>").unwrap();
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Input Fixture","kind":"script","target":"/usr/bin/true","aliases":["runfixture"],"inputPrompt":"Message"}));
    fs::write(&h.config, serde_json::to_vec(&cfg).unwrap()).unwrap();
    h.start();
    let marker = h.base.join("app-discovery-ready");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "discovery barrier missing");
        std::thread::sleep(Duration::from_millis(10));
    }
    let token = open(&h)["editorToken"].clone();
    type_text(&h, "PRIVATE own input");
    assert_eq!(probe(&h, "run-input-read")["canUndo"], true);
    fs::write(h.base.join("app-discovery-release"), "go").unwrap();
    h.wait(|s| s["appDiscoveryPending"] == false);
    h.settled(|_| true);
    assert_eq!(probe(&h, "read")["editorToken"], token);
    assert_eq!(probe(&h, "run-input-read")["text"], "PRIVATE own input");
    assert_eq!(probe(&h, "run-input-read")["canUndo"], true);
    type_text(&h, " retained");
    assert_eq!(
        probe(&h, "run-input-read")["text"],
        "PRIVATE own input retained"
    );
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
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

#[test]
fn native_requires_stdout_eof_but_not_stderr_eof() {
    for held in ["stdout", "stderr", "both"] {
        let h = host(json!(["capture-fixture", "invalid"]));
        let mut holder = Holder::new(&h.base, held);
        let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
        let command = cfg["commands"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|c| c["id"] == "script.fixture")
            .unwrap();
        command["arguments"] = json!(["capture-holder", "parent", held, h.base]);
        h.replace(&cfg, None);
        assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
        open(&h);
        type_text(&h, "input");
        probe(&h, "save-editor");
        idle(&h);
        if held == "stderr" {
            let v = view(&h, |v| {
                v["formVisible"] == false && !v["scriptResult"].is_null()
            });
            assert_eq!(v["resultPanel"]["detailVisible"], true);
            assert_eq!(v["resultPanel"]["editable"], false);
            assert_eq!(v["resultPanel"]["detail"], "prefix while pipe is open");
            assert_eq!(h.rpc(&json!({"op":"usage"}))["result"]["script.fixture"], 1);
        } else {
            view(&h, |v| {
                v["formVisible"] == false && text_contains(v, "incomplete or invalid")
            });
            assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
        }
        assert!(holder.ready.exists());
        assert!(!holder.done.exists());
        holder.release();
    }
}
