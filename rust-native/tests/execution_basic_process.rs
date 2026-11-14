//! Actual CLI execution and privacy checks on an isolated owned runtime.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host};
use serde_json::{json, Value};
use std::{fs, os::unix::fs::PermissionsExt};
fn command(target: &str, args: Value) -> Value {
    json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":target,"arguments":args,"timeout":3})
}
fn host(cmd: Value) -> Host {
    let mut h = Host::new(Config::current(), false);
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(cmd);
    h.replace(&cfg, None);
    h.start();
    h
}
fn replace_command(h: &Host, cmd: Value) {
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    cfg["commands"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["id"] != cmd["id"]);
    cfg["commands"].as_array_mut().unwrap().push(cmd);
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
}
#[test]
fn cli_literal_input_uuid_usage_and_private_logs() {
    let mut cmd = command(
        "/usr/bin/printf",
        json!([
            "%s|%s|%s|%s",
            "{input}",
            "prefix{input}",
            "{uuid}",
            "{uuid}"
        ]),
    );
    cmd["inputPrompt"] = json!("Text");
    let h = host(cmd);
    let secret = "literal $(echo secret) Ω https://private.invalid/token";
    let response = h.cli(&["run", "runfixture", "--input", secret]);
    let result: Vec<_> = response["result"]["stdout"]
        .as_str()
        .unwrap()
        .split('|')
        .collect();
    assert_eq!(&result[..2], &[secret, "prefix{input}"]);
    assert_eq!(result[2].len(), 36);
    assert_ne!(result[2], result[3]);
    assert_eq!(h.cli(&["usage"])["result"]["script.fixture"], 1);
    let logs = h.cli(&["logs"]);
    let logs = logs["result"].as_array().unwrap();
    assert_eq!(
        logs.iter()
            .map(|v| v["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["command.finish", "command.start"]
    );
    assert_eq!(logs[0]["source"], "cli");
    let raw = fs::read_to_string(h.profile.join("command-log.json")).unwrap();
    for private in [secret, "private.invalid", "stdout"] {
        assert!(!raw.contains(private));
    }
    assert_eq!(
        fs::metadata(h.profile.join("command-log.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(h.profile.join("usage.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
#[test]
fn dry_run_disabled_and_missing_prompt_do_not_spawn_or_log() {
    let mut h = Host::new(Config::current(), false);
    let marker = h.base.join("should-not-run");
    let mut cmd = command(
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["fake", "echo", marker, "fixture result"]),
    );
    cmd["inputPrompt"] = json!("Text");
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(cmd.clone());
    h.replace(&cfg, None);
    h.start();
    assert_eq!(h.cli(&["run", "runfixture", "--dry-run"])["dryRun"], true);
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":"runfixture"}))["ok"],
        false
    );
    cmd["enabled"] = json!(false);
    replace_command(&h, cmd);
    assert_eq!(
        h.rpc(&json!({"op":"execute","id":"runfixture","values":{"input":"text"}}))["ok"],
        false
    );
    assert_eq!(h.rpc(&json!({"op":"logs"}))["result"], json!([]));
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
    assert!(!marker.exists());
}

#[test]
fn expanded_arguments_reach_os_limit_with_private_failure_logs() {
    let mut h = Host::new(Config::current(), false);
    let args: Vec<Value> = (0..32).map(|_| json!("{input}")).collect();
    let mut cmd = command("/usr/bin/true", json!(args));
    cmd["inputPrompt"] = json!("Text");
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(cmd);
    h.replace(&cfg, None);
    h.start();
    let result =
        h.rpc(&json!({"op":"execute","id":"runfixture","values":{"input":"x".repeat(65_536)}}));
    assert_eq!(result["ok"], false);
    assert!(result["error"]
        .as_str()
        .unwrap()
        .contains("Argument list too long"));
    let logs = h.rpc(&json!({"op":"logs"}))["result"].clone();
    assert_eq!(
        logs.as_array()
            .unwrap()
            .iter()
            .map(|v| v["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["command.finish", "command.start"]
    );
    assert_eq!(logs[0]["success"], false);
    assert!(!serde_json::to_string(&logs)
        .unwrap()
        .contains(&"x".repeat(100)));
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn invalid_existing_log_is_preserved_until_confirmed_clear() {
    let mut h = Host::new(Config::current(), false);
    let original = b"invalid preserved log";
    let log = h.profile.join("command-log.json");
    fs::write(&log, original).unwrap();
    let cmd = command("/usr/bin/printf", json!(["fixture result"]));
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(cmd);
    h.replace(&cfg, None);
    h.start();
    assert_eq!(h.cli(&["run", "runfixture"])["ok"], true);
    assert_eq!(fs::read(&log).unwrap(), original);
    assert!(!h.rpc(&json!({"op":"status"}))["result"]["commandLogError"].is_null());
    assert_eq!(h.rpc(&json!({"op":"logs.clear"}))["ok"], false);
    assert_eq!(
        h.rpc(&json!({"op":"logs.clear","confirmed":true}))["ok"],
        true
    );
    let saved: Value = serde_json::from_slice(&fs::read(&log).unwrap()).unwrap();
    assert_eq!(saved["entries"], json!([]));
}
#[test]
fn cli_result_envelope_preserves_existing_provider_contract() {
    let mut h = Host::new(Config::current(), false);
    let marker = h.base.join("report-marker");
    let cmd = command(
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["fake", "date-detail", marker, "fixture detail"]),
    );
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(cmd);
    h.replace(&cfg, None);
    h.start();
    let result = h.rpc(&json!({"op":"execute","id":"runfixture"}));
    assert_eq!(result["ok"], true);
    assert_eq!(result["detail"], "fixture detail");
    assert_eq!(result["exitCode"], 0);
}
