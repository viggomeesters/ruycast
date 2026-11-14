//! Native launcher execution on a real isolated hidden host.
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
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn host(marker: &str) -> Bridge {
    configured_host(marker, false)
}
fn configured_host(marker: &str, delay: bool) -> Bridge {
    let mut host = Bridge::new(Config::current(), false);
    let mut config = default_config();
    let args = if delay {
        json!(["execution-delay", host.base.join(marker), "10000", ""])
    } else {
        json!(["execution-write", host.base.join(marker), marker])
    };
    config["commands"].as_array_mut().unwrap().push(json!({
        "id":"script.fixture", "title":"Execution Fixture", "aliases":["runfixture"],
        "kind":"script", "target":env!("CARGO_BIN_EXE_private-results-fixture"),
        "arguments":args, "timeout":3
    }));
    host.replace(&config, None);
    host.start();
    host.settled(|_| true);
    actions::show(&host, "runfixture");
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
    host
}
struct GracefulBridge(Bridge);
impl std::ops::Deref for GracefulBridge {
    type Target = Bridge;
    fn deref(&self) -> &Bridge {
        &self.0
    }
}
impl std::ops::DerefMut for GracefulBridge {
    fn deref_mut(&mut self) -> &mut Bridge {
        &mut self.0
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
fn running_script() -> (GracefulBridge, i32) {
    let host = GracefulBridge(configured_host("pid", true));
    actions::key(&host, 36, 0, "\r", false);
    let deadline = Instant::now() + Duration::from_secs(3);
    let pid = loop {
        if let Ok(text) = fs::read_to_string(host.base.join("pid")) {
            if let Ok(pid) = text.parse::<i32>() {
                break pid;
            }
        }
        assert!(Instant::now() < deadline, "fixture did not publish its PID");
        thread::sleep(Duration::from_millis(10));
    };
    (host, pid)
}
fn assert_dead(pid: i32) {
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
#[test]
fn window_close_cancels_running_script() {
    let (host, pid) = running_script();
    actions::probe(&host, "window-close");
    host.wait(|s| s["scriptsRunning"] == 0 && s["nativeUI"]["visible"] == false);
    assert_dead(pid);
}
#[test]
fn escape_cancels_running_script() {
    let (host, pid) = running_script();
    actions::key(&host, 53, 1 << 23, "", false);
    host.wait(|s| s["scriptsRunning"] == 0 && s["nativeUI"]["visible"] == false);
    assert_dead(pid);
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn composition_keeps_escape_until_committed() {
    let (host, pid) = running_script();
    assert_eq!(
        actions::probe(&host, "execution-ime-escape")["imeEscapeIgnored"],
        true
    );
    assert_eq!(
        host.rpc(&json!({"op":"status"}))["result"]["scriptsRunning"],
        1
    );
    actions::key(&host, 53, 0, "", false);
    host.wait(|s| s["scriptsRunning"] == 0);
    assert_dead(pid);
}
fn wait_marker(path: &Path, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if fs::read_to_string(path).as_deref().ok() == Some(expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "fixture marker missing or incorrect"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn return_runs_script_and_records_launcher_ranking() {
    let host = host("native");
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&host.base.join("native"), "native");
    host.wait(|s| s["scriptsRunning"] == 0 && s["nativeUI"]["visible"] == false);
    assert_eq!(
        host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
        1
    );
    let ranking: Value =
        serde_json::from_slice(&fs::read(host.profile.join("search-ranking.json")).unwrap())
            .unwrap();
    assert!(ranking["entries"]
        .as_object()
        .unwrap()
        .contains_key("script.fixture"));
    assert_eq!(
        host.rpc(&json!({"op":"logs"}))["result"][0]["source"],
        "launcher"
    );
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
}
#[test]
fn actions_open_runs_script() {
    let host = host("actions");
    actions::action(&host, "Run");
    wait_marker(&host.base.join("actions"), "actions");
    host.wait(|s| s["scriptsRunning"] == 0);
}
#[test]
fn click_runs_selected_script() {
    let host = host("clicked");
    host.probe("click", json!({"row":0}));
    wait_marker(&host.base.join("clicked"), "clicked");
    host.wait(|s| s["scriptsRunning"] == 0);
}
#[test]
fn query_change_keeps_new_results_visible_after_completion() {
    let host = GracefulBridge(host("native"));
    let ready = host.base.join("ready");
    let done = host.base.join("done");
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["id"] == "script.fixture")
        .unwrap();
    command["arguments"] = json!(["execution-query", ready, done]);
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    actions::show(&host, "runfixture");
    actions::key(&host, 36, 0, "\r", false);
    wait_marker(&ready, "ready");
    assert_eq!(host.rpc(&json!({"op":"show","query":"finder"}))["ok"], true);
    host.settled(|s| s["nativeUI"]["selectedId"] == "app.finder");
    wait_marker(&done, "done");
    host.wait(|s| s["scriptsRunning"] == 0);
    let status = host.rpc(&json!({"op":"status"}));
    assert_eq!(status["result"]["nativeUI"]["visible"], true);
    assert_eq!(status["result"]["nativeUI"]["selectedId"], "app.finder");
    assert_eq!(actions::probe(&host, "read")["input"], "finder");
}
#[test]
fn stale_completion_is_rejected_after_newer_local_intent() {
    let host = host("native");
    actions::probe(&host, "reject-stale-execution-view");
    host.settled(|_| true);
    assert_eq!(actions::probe(&host, "read")["actionsVisible"], true);
}

#[test]
fn pending_local_intents_survive_ready_completion() {
    use std::io::{BufRead, Write};
    for mode in ["query", "menu", "editor", "selection"] {
        let mut bridge = Bridge::new(Config::current(), false);
        let ready = bridge.base.join("ready");
        let gate = bridge.base.join("gate");
        let marker = bridge.base.join("marker");
        let mut config = default_config();
        let commands = config["commands"].as_array_mut().unwrap();
        commands.push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","gated",ready,gate,marker,"completed"],"timeout":3}));
        commands.push(json!({"id":"script.other","title":"Execution Fixture Other","aliases":["otherfixture"],"kind":"script","target":"/usr/bin/true","arguments":[],"timeout":3}));
        bridge.replace(&config, None);
        bridge.start();
        let host = GracefulBridge(bridge);
        host.settled(|_| true);
        actions::show(
            &host,
            if mode == "selection" {
                "fixture"
            } else {
                "runfixture"
            },
        );
        assert_eq!(
            host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["selectedId"],
            "script.fixture"
        );
        actions::key(&host, 36, 0, "\r", false);
        wait_marker(&ready, "ready");
        host.probe("schedule-execution-intent", json!({"mode":mode}));
        let mut delay = host.connect(Duration::from_secs(5));
        delay
            .write_all(b"{\"op\":\"native.delay\",\"executionBarrier\":true}\n")
            .unwrap();
        let started = host.profile.join("owner-delay-started");
        let deadline = Instant::now() + Duration::from_secs(3);
        while !started.exists() {
            assert!(Instant::now() < deadline, "owner delay did not start");
            thread::sleep(Duration::from_millis(10));
        }
        fs::write(&gate, "go").unwrap();
        let fired = host.profile.join("execution-intent-fired");
        let deadline = Instant::now() + Duration::from_secs(3);
        while !fired.exists() {
            assert!(Instant::now() < deadline, "scheduled intent did not fire");
            thread::sleep(Duration::from_millis(10));
        }
        let mut line = String::new();
        std::io::BufReader::new(delay).read_line(&mut line).unwrap();
        let evidence: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(evidence["result"]["newIntentDuringDelay"], true);
        assert!(evidence["result"]["executionReady"].as_u64().unwrap() > 0);
        host.wait(|s| s["scriptsRunning"] == 0);
        let view = actions::probe(&host, "read");
        let status = host.rpc(&json!({"op":"status"}));
        assert_eq!(status["result"]["nativeUI"]["visible"], true);
        match mode {
            "query" => assert_eq!(view["input"], "finder"),
            "menu" => assert_eq!(view["actionsVisible"], true),
            "editor" => assert_eq!(view["formVisible"], true),
            "selection" => assert_eq!(status["result"]["nativeUI"]["selectedId"], "script.other"),
            _ => unreachable!(),
        }
    }
}

fn pending_cli(
    mode: Option<&str>,
) -> (
    GracefulBridge,
    std::path::PathBuf,
    std::os::unix::net::UnixStream,
    std::os::unix::net::UnixStream,
) {
    use std::io::Write;
    let mut bridge = Bridge::new(Config::current(), false);
    let ready = bridge.base.join("ready");
    let gate = bridge.base.join("gate");
    let marker = bridge.base.join("marker");
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","gated",ready,gate,marker,"completed"],"timeout":3}));
    bridge.replace(&config, None);
    bridge.start();
    let host = GracefulBridge(bridge);
    host.settled(|_| true);
    let mut execution = host.connect(Duration::from_secs(6));
    execution
        .write_all(b"{\"op\":\"execute\",\"id\":\"runfixture\"}\n")
        .unwrap();
    wait_marker(&ready, "ready");
    if let Some(mode) = mode {
        host.probe("schedule-execution-intent", json!({"mode":mode}));
    }
    let mut delay = host.connect(Duration::from_secs(5));
    delay
        .write_all(b"{\"op\":\"native.delay\",\"executionBarrier\":true}\n")
        .unwrap();
    wait_exists(&host.profile.join("owner-delay-started"));
    (host, gate, delay, execution)
}
fn wait_exists(path: &Path) {
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
fn response(stream: std::os::unix::net::UnixStream) -> Value {
    use std::io::BufRead;
    let mut line = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut line)
        .unwrap();
    serde_json::from_str(&line).unwrap()
}
#[test]
fn cli_completion_delivered_and_recorded_before_native_stop() {
    let (mut host, gate, delay, execution) = pending_cli(Some("stop"));
    fs::write(gate, "go").unwrap();
    wait_exists(&host.profile.join("execution-intent-fired"));
    let evidence = response(delay);
    assert_eq!(evidence["result"]["nativeStopDuringDelay"], true);
    assert!(evidence["result"]["executionReady"].as_u64().unwrap() > 0);
    assert_eq!(response(execution)["ok"], true);
    let output = host.wait_exit(Duration::from_secs(5), false);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let logs: Value =
        serde_json::from_slice(&fs::read(host.profile.join("command-log.json")).unwrap()).unwrap();
    assert_eq!(
        logs["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["command.start", "command.finish"]
    );
    let usage: Value =
        serde_json::from_slice(&fs::read(host.profile.join("usage.json")).unwrap()).unwrap();
    assert_eq!(usage["script.fixture"], 1);
}
#[test]
fn external_replacement_cannot_receive_completed_command_usage() {
    let (host, gate, delay, execution) = pending_cli(None);
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    command["target"] = json!("/usr/bin/true");
    command["arguments"] = json!([]);
    host.replace(&config, None);
    fs::write(gate, "go").unwrap();
    assert!(
        response(delay)["result"]["executionReady"]
            .as_u64()
            .unwrap()
            > 0
    );
    let result = response(execution);
    assert_eq!(result["ok"], true);
    assert!(result.get("usageWarning").is_some());
    assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
}
#[test]
fn ready_cli_completion_survives_queued_rpc_quit() {
    use std::io::Write;
    let (mut host, gate, delay, execution) = pending_cli(None);
    fs::write(gate, "go").unwrap();
    let mut quit = host.connect(Duration::from_secs(5));
    quit.write_all(b"{\"op\":\"quit\"}\n").unwrap();
    assert!(
        response(delay)["result"]["executionReady"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(response(execution)["ok"], true);
    assert_eq!(response(quit)["ok"], true);
    let output = host.wait_exit(Duration::from_secs(5), false);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let usage: Value =
        serde_json::from_slice(&fs::read(host.profile.join("usage.json")).unwrap()).unwrap();
    assert_eq!(usage["script.fixture"], 1);
}

#[test]
fn config_replacement_and_invalid_config_clear_running_label() {
    use std::io::Write;
    for invalid in [false, true] {
        let mut bridge = Bridge::new(Config::current(), false);
        let ready = bridge.base.join("ready");
        let gate = bridge.base.join("gate");
        let marker = bridge.base.join("marker");
        let mut config = default_config();
        config["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","gated",ready,gate,marker,"completed"],"timeout":3}));
        config["commands"].as_array_mut().unwrap().push(json!({"id":"script.other","title":"Execution Fixture Other","aliases":["otherfixture"],"kind":"script","target":"/usr/bin/true","arguments":[],"timeout":3}));
        bridge.replace(&config, None);
        bridge.start();
        let host = GracefulBridge(bridge);
        host.settled(|_| true);
        actions::show(&host, "runfixture");
        assert_eq!(
            host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["selectedId"],
            "script.fixture"
        );
        actions::key(&host, 36, 0, "\r", false);
        wait_marker(&ready, "ready");
        let mut delay = host.connect(Duration::from_secs(5));
        delay
            .write_all(b"{\"op\":\"native.delay\",\"executionBarrier\":true}\n")
            .unwrap();
        wait_exists(&host.profile.join("owner-delay-started"));
        if invalid {
            host.replace_bytes(b"{invalid", None);
        } else {
            let command = config["commands"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|c| c["id"] == "script.fixture")
                .unwrap();
            command["target"] = json!("/usr/bin/true");
            command["arguments"] = json!([]);
            host.replace(&config, None);
        }
        fs::write(&gate, "go").unwrap();
        assert!(
            response(delay)["result"]["executionReady"]
                .as_u64()
                .unwrap()
                > 0
        );
        host.wait(|s| s["scriptsRunning"] == 0);
        let view = actions::probe(&host, "read");
        assert!(!view["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "Running…"));
        if invalid {
            assert!(view["screenText"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t.as_str().unwrap().starts_with("Config:")));
        }
        assert_eq!(
            host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
            true
        );
        assert_eq!(host.rpc(&json!({"op":"usage"}))["result"], json!({}));
    }
}

#[test]
fn running_and_failure_copy_in_both_themes() {
    for theme in ["light", "dark"] {
        let mut bridge = Bridge::new(Config::current(), false);
        let gate = bridge.base.join("gate");
        let ready = bridge.base.join("ready");
        let marker = bridge.base.join("unused");
        let mut config = default_config();
        config["theme"] = json!(theme);
        config["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake","gated-fail",ready,gate,marker,""],"timeout":3}));
        bridge.replace(&config, None);
        bridge.start();
        let host = GracefulBridge(bridge);
        host.settled(|_| true);
        actions::show(&host, "runfixture");
        actions::key(&host, 36, 0, "\r", false);
        wait_marker(&ready, "ready");
        for state in ["running", "failure"] {
            if state == "failure" {
                fs::write(&gate, "go").unwrap();
                host.wait(|s| s["scriptsRunning"] == 0);
            }
            let view = host.probe("screenshot", json!({}));
            let texts = view["screenText"].as_array().unwrap();
            if state == "running" {
                assert!(texts.iter().any(|text| text == "Running…"));
            } else {
                assert!(texts
                    .iter()
                    .any(|text| text.as_str().unwrap().contains("status 7")));
            }
            if let Some(evidence) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
                let evidence = std::path::PathBuf::from(evidence);
                let folder = evidence.parent().unwrap().join("execution");
                fs::create_dir_all(&folder).unwrap();
                fs::copy(
                    host.profile.join("native-probe.png"),
                    folder.join(format!("{state}-{theme}.png")),
                )
                .unwrap();
                fs::write(
                    folder.join(format!("{state}-{theme}.json")),
                    serde_json::to_vec_pretty(&view).unwrap(),
                )
                .unwrap();
            }
            assert_eq!(view["windowShown"], false);
        }
    }
}
