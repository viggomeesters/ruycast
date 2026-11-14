//! Real CLI and headless owner with the owned Workspace interposition fixture.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    time::Duration,
};
fn command(kind: &str, target: &str) -> Value {
    json!({"id":"fixture.open","title":"Fixture","kind":kind,"target":target,"aliases":["fixture-alias"]})
}
fn host_options_with_dispatch(
    dispatch: u32,
    commands: Vec<Value>,
    named: bool,
    clipboard: Option<&str>,
    use_shim: bool,
    discovered: bool,
) -> Host {
    let settings = Config::current();
    let shim = std::env::var_os("RUYCAST_TEST_WORKSPACE_SHIM")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            settings
                .host
                .parent()
                .unwrap()
                .join("examples/libworkspace_fixture.dylib")
        });
    assert!(
        shim.is_absolute() && shim.is_file(),
        "Build workspace-fixture first: {shim:?}"
    );
    let mut h = Host::new(settings, false);
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().extend(commands);
    h.replace(&config, None);
    let home = h.base.as_os_str().to_owned();
    h.env.insert("RUYCAST_WORKSPACE_FIXTURE_HOME".into(), home);
    h.env.insert(
        "RUYCAST_NATIVE_TEST_DISPATCH_MS".into(),
        dispatch.to_string().into(),
    );
    h.env
        .insert("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD".into(), "1".into());
    if named {
        h.env
            .insert("RUYCAST_NATIVE_TEST_CLIPBOARD".into(), "named".into());
        if let Some(value) = clipboard {
            h.env
                .insert("RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT".into(), value.into());
        }
    }
    if use_shim {
        h.env
            .insert("DYLD_INSERT_LIBRARIES".into(), shim.into_os_string());
    }
    if discovered {
        let contents = h.applications.join("Fixture.app/Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("Info.plist"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.fixture.discovered</string><key>CFBundleName</key><string>Fixture Discovered</string></dict></plist>").unwrap();
    }
    h.start();
    h.env.remove(std::ffi::OsStr::new("DYLD_INSERT_LIBRARIES"));
    h.env
        .remove(std::ffi::OsStr::new("RUYCAST_WORKSPACE_FIXTURE_HOME"));
    if discovered {
        h.wait(|s| s["appDiscoveryPending"] == false);
    }
    h
}
fn host_options(
    commands: Vec<Value>,
    named: bool,
    clipboard: Option<&str>,
    use_shim: bool,
    discovered: bool,
) -> Host {
    host_options_with_dispatch(2000, commands, named, clipboard, use_shim, discovered)
}

fn host_clipboard(command: Value, named: bool, clipboard: Option<&str>) -> Host {
    host_options(vec![command], named, clipboard, true, false)
}
fn host(command: Value) -> Host {
    host_clipboard(command, false, None)
}

fn trace(h: &Host) -> Vec<Value> {
    let p = h.base.join("workspace-trace.jsonl");
    if !p.exists() {
        return vec![];
    }
    fs::read_to_string(p)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn opens(h: &Host) -> Vec<Value> {
    trace(h)
        .into_iter()
        .filter(|v| v["phase"].as_str().unwrap().starts_with("open-"))
        .collect()
}
#[test]
fn unchanged_cli_saved_app_alias_and_usage() {
    let h = host(command("app", "dev.fixture.inline"));
    assert_eq!(
        h.cli(&["run", "fixture-alias"]),
        json!({"ok":true,"result":{"command":"fixture.open","state":"opened"}})
    );
    assert_eq!(h.cli(&["usage"])["result"]["fixture.open"], 1);
    let trace = trace(&h);
    assert_eq!(
        trace
            .iter()
            .map(|v| v["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["installed", "resolve", "open-app", "callback"]
    );
    assert!(trace[1..].iter().all(|v| v["mainThread"] == false));
}
#[test]
fn received_canonical_value_duplicates_follow_wire_order() {
    let h = host(command("url", "https://example.invalid/{café}"));
    for (first, second) in [(r"caf\u00e9", r"cafe\u0301"), (r"cafe\u0301", r"caf\u00e9")] {
        let wire=format!("{{\"op\":\"execute\",\"id\":\"fixture.open\",\"values\":{{\"{first}\":\"first\",\"{second}\":\"last\"}}}}\n");
        let mut stream = h.connect(Duration::from_secs(3));
        stream.write_all(wire.as_bytes()).unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let result: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(
            opens(&h).last().unwrap()["url"],
            "https://example.invalid/last"
        );
    }
}
#[test]
fn unchanged_cli_keeps_sorted_serialization_policy() {
    let h = host(command("url", "https://example.invalid/{café}"));
    for input in [
        "{\"café\":\"NFC\",\"café\":\"NFD\"}",
        "{\"café\":\"NFD\",\"café\":\"NFC\"}",
    ] {
        assert_eq!(
            h.cli(&["run", "fixture.open", "--input", input])["ok"],
            true
        );
        assert_eq!(
            opens(&h).last().unwrap()["url"],
            "https://example.invalid/NFC"
        );
    }
}

fn config(h: &Host) -> Value {
    serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap()
}
#[test]
fn canonical_alias_and_disabled_name_guard() {
    let mut cmd = command("app", "dev.fixture.inline");
    cmd["aliases"] = json!(["café"]);
    let h = host(cmd);
    assert_eq!(h.cli(&["run", "cafe\u{301}"])["ok"], true);
    let mut cfg = config(&h);
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"fixture.disabled","title":"Disabled","kind":"url","target":"https://example.invalid","aliases":["cafe\u{301}"],"enabled":false}));
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    let before = opens(&h).len();
    let result = h.rpc(&json!({"op":"execute","id":"café"}));
    assert_eq!(result["ok"], false);
    assert!(result["error"].as_str().unwrap().contains("disabled"));
    assert_eq!(opens(&h).len(), before);
}
#[test]
fn url_fields_encoding_open_with_and_literal_path() {
    let mut cmd = command(
        "url",
        "https://example.invalid/{query}?place={x name=\"Place\"}",
    );
    cmd["applicationID"] = json!("dev.fixture.inline");
    let h = host(cmd);
    let input = r#"{"Query": "a&b /\u03a9", "Place": "New York"}"#;
    assert_eq!(
        h.cli(&["run", "fixture.open", "--input", input]),
        json!({"ok":true,"result":{"state":"opened"}})
    );
    let trace = trace(&h);
    let targets = trace
        .iter()
        .find(|s| s["phase"] == "open-with-targets")
        .unwrap()["urls"]
        .as_array()
        .unwrap();
    let url = targets[0].as_str().unwrap();
    assert!(url.contains("a%26b%20"));
    assert!(url.contains("New%20York"));
    let path = h.base.join("literal Ω\n$(not-executed)");
    fs::write(&path, "fixture").unwrap();
    let mut cfg = config(&h);
    *cfg["commands"].as_array_mut().unwrap().last_mut().unwrap() =
        command("path", path.to_str().unwrap());
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    assert_eq!(
        h.cli(&["run", "fixture.open"]),
        json!({"ok":true,"result":{"command":"fixture.open","state":"opened"}})
    );
    let opened = opens(&h);
    let last = opened.last().unwrap();
    assert_eq!(last["phase"], "open-default");
    assert!(last["url"].as_str().unwrap().contains("%0A"));
}
#[test]
fn missing_apps_paths_and_fields_do_not_open() {
    for (kind, target, expected) in [
        ("app", Some("dev.fixture.missing"), "App is not installed:"),
        ("path", None, "Path does not exist:"),
        (
            "url",
            Some("https://example.invalid/{query}"),
            "Enter Query.",
        ),
    ] {
        let h = host(command(kind, target.unwrap_or("/own-placeholder")));
        if target.is_none() {
            let mut cfg = config(&h);
            cfg["commands"].as_array_mut().unwrap().last_mut().unwrap()["target"] =
                json!(h.base.join("missing"));
            h.replace(&cfg, None);
            h.cli(&["reload"]);
        }
        let result = h.rpc(&json!({"op":"execute","id":"fixture.open"}));
        assert_eq!(result["ok"], false);
        assert!(
            result["error"].as_str().unwrap().contains(expected),
            "{result}"
        );
        assert!(opens(&h).is_empty());
    }
}

#[test]
fn fixture_clipboard_without_explicit_named_enablement_is_never_read() {
    let h = host(command("url", "https://example.invalid/{clipboard}"));
    let result = h.rpc(&json!({"op":"execute","id":"fixture.open"}));
    assert_eq!(result["ok"], false);
    assert!(result["error"]
        .as_str()
        .unwrap()
        .contains("fixture boundary"));
    let events = trace(&h);
    assert_eq!(events.len(), 1);
}
#[test]
fn named_clipboard_url_uses_main_thread_and_preserves_raw_substitution() {
    let h = host_clipboard(
        command("url", "https://example.invalid/?q={clipboard}"),
        true,
        Some("raw/Ω&x=1"),
    );
    let value = h.cli(&["run", "fixture.open"]);
    assert_eq!(value["ok"], true, "{value}");
    let events = trace(&h);
    let reads: Vec<_> = events
        .iter()
        .filter(|s| s["phase"] == "clipboard-read")
        .collect();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0]["mainThread"], true);
    let opened = opens(&h);
    assert_eq!(opened[0]["mainThread"], false);
    assert!(opened[0]["url"].as_str().unwrap().contains("&x=1"));
    assert!(h.base.join("clipboard-read").exists());
    assert_eq!(h.cli(&["usage"])["result"]["fixture.open"], 1);
}
#[test]
fn missing_named_clipboard_and_nonliteral_demand_do_not_read_general() {
    let h = host_clipboard(
        command("url", "https://example.invalid/{clipboard}"),
        true,
        None,
    );
    let value = h.rpc(&json!({"op":"execute","id":"fixture.open"}));
    assert_eq!(value["ok"], false);
    assert!(opens(&h).is_empty());
    let marker = h.base.join("clipboard-read");
    assert!(marker.exists());
    let mut cfg = config(&h);
    cfg["commands"].as_array_mut().unwrap().last_mut().unwrap()["target"] =
        json!("https://example.invalid/{clipboard}\u{301}");
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    fs::remove_file(&marker).unwrap();
    let value = h.rpc(&json!({"op":"execute","id":"fixture.open"}));
    assert_eq!(value["ok"], false);
    assert!(!marker.exists());
    assert!(opens(&h).is_empty());
}

#[test]
fn discovered_app_runs_from_shared_catalog_without_saved_presence() {
    let h = host_options(
        vec![command("app", "dev.fixture.inline")],
        false,
        None,
        true,
        true,
    );
    let search = h.cli(&["search", "Fixture Discovered"]);
    let row = search["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "discovered.dev.fixture.discovered")
        .unwrap();
    let id = row["id"].as_str().unwrap();
    assert_eq!(h.cli(&["run", id, "--dry-run"])["dryRun"], true);
    let result = h.cli(&["run", id]);
    assert_eq!(result["ok"], true);
    assert!(result.get("usageWarning").is_none());
    assert_eq!(h.cli(&["usage"])["result"][id], 1);
    assert!(!h.cli(&["commands"])["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == id));
}
#[test]
fn saved_disabled_target_suppresses_discovered_and_saved_copy() {
    let mut first = command("app", "dev.fixture.discovered");
    first["enabled"] = json!(false);
    let second =
        json!({"id":"fixture.copy","title":"Copy","kind":"app","target":"dev.fixture.discovered"});
    let h = host_options(vec![first, second], false, None, true, true);
    for id in [
        "fixture.open",
        "fixture.copy",
        "discovered.dev.fixture.discovered",
    ] {
        assert_eq!(h.rpc(&json!({"op":"execute","id":id}))["ok"], false);
    }
    assert!(opens(&h).is_empty());
}
#[test]
fn missing_interposition_fails_closed_in_headless_fixture() {
    let h = host_options(
        vec![command("app", "dev.fixture.inline")],
        false,
        None,
        false,
        false,
    );
    let result = h.rpc(&json!({"op":"execute","id":"fixture.open"}));
    assert_eq!(result["ok"], false);
    assert!(result["error"]
        .as_str()
        .unwrap()
        .contains("fixture boundary"));
    assert!(trace(&h).is_empty());
}

struct Release(std::path::PathBuf);
impl Release {
    fn release(self) {
        fs::write(&self.0, b"release").unwrap();
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::write(&self.0, b"release");
    }
}
fn wait_file(path: &std::path::Path) {
    let end = std::time::Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < end,
            "Own SDK fixture did not enter"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn execute_async(h: &Host) -> std::sync::mpsc::Receiver<std::io::Result<Value>> {
    let socket = h.socket();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = runtime_host::status_rpc::request(
            &socket,
            &json!({"op":"execute","id":"fixture.open"}),
            std::time::Instant::now() + Duration::from_secs(4),
        );
        let _ = tx.send(result);
    });
    rx
}
#[test]
fn blocked_main_clipboard_does_not_block_owner_or_revive_expired_execution() {
    let h = host_options_with_dispatch(
        400,
        vec![command("url", "https://example.invalid/{clipboard}")],
        true,
        Some("hold-main"),
        true,
        false,
    );
    let release = Release(h.base.join("workspace-release"));
    let worker = execute_async(&h);
    wait_file(&h.base.join("clipboard-entered"));
    assert_eq!(h.rpc(&json!({"op":"status"}))["ok"], true);
    let result = worker
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert_eq!(result["ok"], false);
    assert!(result.get("actionCompleted").is_none());
    release.release();
    wait_file(&h.base.join("clipboard-read"));
    h.wait(|s| s["liveExecutionWorkers"] == 0);
    assert!(opens(&h).is_empty());
    assert!(h.cli(&["usage"])["result"].get("fixture.open").is_none());
}
#[test]
fn config_change_during_main_capture_retires_old_origin() {
    let h = host_clipboard(
        command("url", "https://example.invalid/{clipboard}"),
        true,
        Some("hold-main"),
    );
    let release = Release(h.base.join("workspace-release"));
    let worker = execute_async(&h);
    wait_file(&h.base.join("clipboard-entered"));
    let mut cfg = config(&h);
    cfg["commands"].as_array_mut().unwrap().last_mut().unwrap()["target"] =
        json!("https://example.invalid/changed");
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    let result = worker
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert_eq!(result["ok"], false);
    release.release();
    h.wait(|s| s["liveExecutionWorkers"] == 0);
    assert!(opens(&h).is_empty());
    assert_eq!(h.cli(&["run", "fixture.open"])["ok"], true);
}

#[test]
fn small_execute_packet_can_open_expanded_output_above_one_mib() {
    let h = host(command("url", "https://example.invalid/{x}{x}"));
    let value = h.rpc_timeout(
        &json!({"op":"execute","id":"fixture.open","values":{"x":"a".repeat(600_000)}}),
        Duration::from_secs(10),
    );
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(
        opens(&h)[0]["url"],
        format!("https://example.invalid/{}", "a".repeat(1_200_000))
    );
}
