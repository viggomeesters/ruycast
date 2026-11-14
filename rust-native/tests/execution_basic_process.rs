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

fn delayed(h: &Host, millis: u32, text: &str) -> (Value, std::path::PathBuf) {
    let marker = h.base.join("leader");
    let cmd = command(
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["execution-delay", marker, millis.to_string(), text]),
    );
    (cmd, marker)
}
fn pending(h: &Host) -> std::os::unix::net::UnixStream {
    use std::io::Write;
    let mut stream = h.connect(std::time::Duration::from_secs(6));
    stream
        .write_all(b"{\"op\": \"execute\", \"id\": \"runfixture\"}\n")
        .unwrap();
    stream
}
fn response(stream: std::os::unix::net::UnixStream) -> Value {
    use std::io::BufRead;
    let mut line = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut line)
        .unwrap();
    serde_json::from_str(&line).unwrap()
}
fn wait_file(path: &std::path::Path) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if path.exists() {
            return fs::read_to_string(path).unwrap();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Own fixture did not publish {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
fn assert_dead(pid: &str) {
    let pid: libc::pid_t = pid.parse().unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
#[test]
fn status_and_configuration_remain_responsive_during_script() {
    let h = host(command("/usr/bin/printf", json!(["fixture result"])));
    let (cmd, pid) = delayed(&h, 500, "done");
    replace_command(&h, cmd);
    let stream = pending(&h);
    wait_file(&pid);
    let start = std::time::Instant::now();
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["scriptsRunning"],
        1
    );
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    assert_eq!(response(stream)["ok"], true);
    assert_dead(&fs::read_to_string(&pid).unwrap());
    h.wait(|s| s["scriptsRunning"] == 0);
}
#[test]
fn half_closed_client_receives_result() {
    let mut h = Host::new(Config::current(), false);
    let (cmd, _) = delayed(&h, 150, "half-close");
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(cmd);
    h.replace(&cfg, None);
    h.start();
    let stream = pending(&h);
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(response(stream)["result"]["stdout"], "half-close\n");
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"]["script.fixture"], 1);
}
#[test]
fn old_command_completion_is_not_attributed_to_replacement() {
    let h = host(command("/usr/bin/printf", json!(["fixture result"])));
    let (cmd, marker) = delayed(&h, 400, "original");
    replace_command(&h, cmd);
    let stream = pending(&h);
    wait_file(&marker);
    replace_command(&h, command("/usr/bin/true", json!([])));
    let result = response(stream);
    assert_eq!(result["ok"], true);
    assert_eq!(result["result"]["stdout"], "original\n");
    assert!(result.get("usageWarning").is_some());
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
}

#[test]
fn disconnect_cancels_process_group_and_recovers_capacity() {
    let h = host(command("/usr/bin/printf", json!(["fixture result"])));
    let pid = h.base.join("leader");
    let sentinel = h.base.join("orphan");
    let cmd = command(
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["execution-tree", pid, sentinel]),
    );
    replace_command(&h, cmd);
    let stream = pending(&h);
    wait_file(&pid);
    drop(stream);
    h.wait(|s| s["scriptsRunning"] == 0);
    assert_dead(&fs::read_to_string(&pid).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(850));
    assert!(!sentinel.exists());
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
    assert_eq!(h.rpc(&json!({"op":"logs"}))["result"][0]["success"], false);
}
#[test]
fn usage_failure_reports_completed_action_without_retry() {
    let h = host(command("/usr/bin/printf", json!(["fixture result"])));
    let gate = h.base.join("gate");
    let ready = h.base.join("ready");
    let counter = h.base.join("counter");
    let cmd = command(
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["fake", "gated", ready, gate, counter, ""]),
    );
    replace_command(&h, cmd);
    let stream = pending(&h);
    wait_file(&ready);
    fs::create_dir(h.profile.join("usage.json")).unwrap();
    fs::write(&gate, "go").unwrap();
    let result = response(stream);
    assert_eq!(result["ok"], false);
    assert_eq!(result["actionCompleted"], true);
    assert_eq!(fs::read_to_string(&counter).unwrap(), "once");
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["scriptsRunning"],
        0
    );
}

struct GracefulHost(Host);
impl std::ops::Deref for GracefulHost {
    type Target = Host;
    fn deref(&self) -> &Host {
        &self.0
    }
}
impl std::ops::DerefMut for GracefulHost {
    fn deref_mut(&mut self) -> &mut Host {
        &mut self.0
    }
}
impl Drop for GracefulHost {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.0.stop(true)));
        }
    }
}
#[test]
fn saturated_scripts_leave_status_and_quit_admission() {
    use std::io::Write;
    let mut h = GracefulHost(host(command("/usr/bin/printf", json!(["fixture result"]))));
    let pids = h.base.join("pids");
    fs::create_dir(&pids).unwrap();
    let mut cmd = command(
        env!("CARGO_BIN_EXE_private-results-fixture"),
        json!(["execution-delay", "{input}", "10000", ""]),
    );
    cmd["timeout"] = json!(15);
    replace_command(&h, cmd);
    let mut streams = Vec::new();
    for i in 0..15 {
        let mut stream = h.connect(std::time::Duration::from_secs(6));
        serde_json::to_writer(
            &mut stream,
            &json!({"op":"execute","id":"runfixture","values":{"input":pids.join(i.to_string())}}),
        )
        .unwrap();
        stream.write_all(b"\n").unwrap();
        streams.push(stream);
    }
    h.wait(|s| s["scriptsRunning"] == 15);
    for i in 0..15 {
        wait_file(&pids.join(i.to_string()));
    }
    let busy =
        h.rpc(&json!({"op":"execute","id":"runfixture","values":{"input":pids.join("extra")}}));
    assert_eq!(busy["ok"], false);
    assert!(busy["error"].as_str().unwrap().contains("busy"));
    let start = std::time::Instant::now();
    assert_eq!(h.rpc(&json!({"op":"status"}))["ok"], true);
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    assert_eq!(h.rpc(&json!({"op":"quit"}))["ok"], true);
    let output = h.wait_exit(std::time::Duration::from_secs(5), false);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for entry in fs::read_dir(&pids).unwrap() {
        assert_dead(&fs::read_to_string(entry.unwrap().path()).unwrap());
    }
    assert!(!h.profile.join("control.sock").exists());
    drop(streams);
}
