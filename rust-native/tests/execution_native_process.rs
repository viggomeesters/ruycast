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
