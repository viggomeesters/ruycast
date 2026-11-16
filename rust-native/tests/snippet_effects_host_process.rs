//! Real headless/hidden host callers with private Rust SDK and named clipboard fixtures.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_bridge.rs"]
mod native_bridge;
use native_bridge::{runtime_host, Bridge};
use runtime_host::{default_config, Config, Entry, Host};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

// Preserve original unittest/matrix sequencing; the three callers inside one scenario remain concurrent.
static HOST_SCENARIOS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn host(target: &str, flags: &[&str], clipboard: Option<&str>) -> Host {
    configured_host(command(target), flags, clipboard, Entry::HeadlessTest, true)
}
fn command(target: &str) -> Value {
    json!({"id":"snippet.fixture", "title":"Snippet Fixture", "kind":"snippet", "target":target, "aliases":["snipfixture"]})
}
fn hidden_host(target: &str) -> Bridge {
    hidden_host_options(target, &[], true)
}
fn hidden_host_options(target: &str, flags: &[&str], complete: bool) -> Bridge {
    hidden_command_host(command(target), flags, complete)
}
fn hidden_command_host(command: Value, flags: &[&str], complete: bool) -> Bridge {
    let host = Bridge {
        host: configured_host(command, flags, None, Entry::HiddenTest, complete),
    };
    host.settled(|_| true);
    host
}
fn configured_host(
    command: Value,
    flags: &[&str],
    clipboard: Option<&str>,
    entry: Entry,
    complete: bool,
) -> Host {
    let mut host = Host::new(Config::current(), false);
    let original = std::env::var_os("RUYCAST_TEST_SNIPPET_EFFECTS_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            host.settings
                .host
                .parent()
                .unwrap()
                .join("examples/libsnippet_effects_fixture.dylib")
        });
    assert!(
        original.is_absolute() && original.is_file(),
        "Build snippet-effects-fixture first"
    );
    let copy = host.base.join("snippet-effects.dylib");
    fs::copy(&original, &copy).unwrap();
    assert_eq!(
        Sha256::digest(fs::read(&original).unwrap()),
        Sha256::digest(fs::read(&copy).unwrap())
    );
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(command);
    host.replace(&config, None);
    for (key, value) in [
        ("DYLD_INSERT_LIBRARIES", copy.into_os_string()),
        (
            "RUYCAST_WORKSPACE_FIXTURE_HOME",
            host.base.as_os_str().to_owned(),
        ),
        (
            "RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD",
            match entry {
                Entry::HeadlessTest => "1",
                Entry::HiddenTest => "trace",
            }
            .into(),
        ),
    ] {
        host.env.insert(key.into(), value);
    }
    if complete {
        host.env
            .insert("RUYCAST_NATIVE_TEST_SNIPPET".into(), "1".into());
    }
    if matches!(entry, Entry::HiddenTest) {
        host.env
            .insert("RUYCAST_NATIVE_TEST_REQUIRE_HIDE".into(), "1".into());
    }
    if let Some(text) = clipboard {
        host.env
            .insert("RUYCAST_NATIVE_TEST_CLIPBOARD".into(), "named".into());
        host.env
            .insert("RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT".into(), text.into());
    }
    for flag in flags {
        fs::write(host.base.join(flag), "own fault").unwrap();
    }
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        fs::create_dir_all(&directory).unwrap();
        let sources = [
            "rust-native/tests/snippet_effects_host_process.rs",
            "rust-native/tests/support/runtime_host.rs",
            "rust-native/tests/support/native_bridge.rs",
            "rust-native/tests/fixtures/workspace_sdk.rs",
            "rust-native/tests/fixtures/snippet_effects_sdk.rs",
            "rust-native/tests/fixtures/snippet_effects_fixture.rs",
            "rust-native/Cargo.toml",
            "rust-native/Cargo.lock",
            "rust/Cargo.toml",
            "rust-toolchain.toml",
        ]
        .into_iter()
        .map(|path| {
            (
                path,
                format!(
                    "{:x}",
                    Sha256::digest(fs::read(runtime_host::root().join(path)).unwrap())
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
        let artifacts = [
            (&host.settings.host, "host"),
            (&host.settings.cli, "cli"),
            (&original, "snippet-effects-sdk"),
        ]
        .into_iter()
        .map(|(path, label)| runtime_host::bind_artifact(path, label))
        .collect::<BTreeMap<_, _>>();
        fs::write(PathBuf::from(directory).join("snippet-effects-host-bindings.json"),serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":artifacts,"sdkCopyHashChecked":true,"scope":"Owned headless and prohibited/offscreen hidden hosts, named clipboard and synthetic target processes; no desktop effects"})).unwrap()).unwrap();
    }
    host.start_entry(entry);
    host.env
        .remove(std::ffi::OsStr::new("DYLD_INSERT_LIBRARIES"));
    host.env
        .remove(std::ffi::OsStr::new("RUYCAST_WORKSPACE_FIXTURE_HOME"));
    host
}

fn show(host: &Bridge) {
    assert_eq!(
        host.rpc(&json!({"op":"show","query":"snipfixture"}))["ok"],
        true
    );
    host.settled(|status| status["nativeUI"]["visible"] == true);
}

fn assert_keys(posts: &[&Value], keys: &[u64]) {
    assert_eq!(
        posts
            .iter()
            .map(|event| event["key"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        keys
    );
    assert!(posts.iter().all(|event| event["mainThread"] == true
        && event["pid"] == 4260
        && event["tag"] == 0x52554859504552_u64));
    assert_eq!(
        posts
            .iter()
            .map(|event| event["down"].as_bool().unwrap())
            .collect::<Vec<_>>(),
        (0..posts.len())
            .map(|index| index % 2 == 0)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        posts
            .iter()
            .map(|event| event["flags"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        keys.iter()
            .map(|key| if *key == 9 { 1 << 20 } else { 0 })
            .collect::<Vec<_>>()
    );
}

#[test]
fn unchanged_cli_on_gui_host_receives_original_success_shape_after_event_phase() {
    let _scenario = HOST_SCENARIOS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut command = command("PRIVATE [Guide](https://example.com)");
    command["snippetOutput"] = json!("teams");
    let host = hidden_command_host(command, &[], true);
    show(&host);
    let result = host.cli(&["run", "snipfixture"]);
    assert_eq!(result, json!({"ok":true,"result":{"state":"pasted"}}));
    let trace = trace(&host);
    let posts: Vec<_> = trace
        .iter()
        .filter(|event| event["phase"] == "snippet-post")
        .collect();
    assert_keys(&posts, &[9, 9]);
    let writes: Vec<_> = trace
        .iter()
        .filter(|event| event["phase"] == "snippet-write")
        .collect();
    assert_eq!(writes[0]["text"], "PRIVATE Guide");
    assert!(writes[0]["html"]
        .as_str()
        .unwrap()
        .contains("<p style=\"margin:0\">PRIVATE <a href="));
    let raw = fs::read_to_string(host.profile.join("command-log.json")).unwrap();
    assert!(!raw.contains("PRIVATE"));
    assert!(!raw.contains("https://example.com"));
    let usage = host.cli(&["usage"]);
    assert_eq!(usage["result"]["snippet.fixture"], 1);
    save(
        "gui-cli-private-payload",
        &json!({"result":result,"usage":usage,"payloadAbsentFromCommandLog":true}),
        &trace,
    );
}

#[test]
fn gui_missing_target_and_incomplete_fixture_never_mutate_board() {
    let _scenario = HOST_SCENARIOS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (name, flags, complete, error) in [
        (
            "gui-missing-target",
            &["missing-target"][..],
            true,
            "Choose an app",
        ),
        (
            "gui-incomplete-fixture",
            &[][..],
            false,
            "Complete snippet fixture",
        ),
    ] {
        let host = hidden_host_options("Plain", flags, complete);
        show(&host);
        let response = host.rpc(&json!({"op":"execute","id":"snipfixture"}));
        assert_eq!(response["ok"], false);
        assert!(response["error"].as_str().unwrap().contains(error));
        let trace = trace(&host);
        for phase in ["snippet-write", "snippet-post", "snippet-clear"] {
            assert!(!trace.iter().any(|event| event["phase"] == phase));
        }
        save(name, &response, &trace);
    }
}

#[test]
fn cli_uses_current_app_without_show_and_refreshes_after_external_switch() {
    let _scenario = HOST_SCENARIOS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let host = hidden_host("CLI");
    let first = host.cli(&["run", "snipfixture"]);
    assert_eq!(first, json!({"ok":true,"result":{"state":"pasted"}}));
    let pids = |events: &[Value]| {
        events
            .iter()
            .filter(|event| event["phase"] == "snippet-post")
            .map(|event| event["pid"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(pids(&trace(&host)), vec![4260, 4260]);
    host.probe("snippet-fixture-control", json!({"operation":2}));
    let second = host.cli(&["run", "snipfixture"]);
    assert_eq!(second["ok"], true);
    let trace = trace(&host);
    assert_eq!(pids(&trace), vec![4260, 4260, 4261, 4261]);
    save(
        "cli-current-target",
        &json!({"first":first,"second":second}),
        &trace,
    );
}

#[test]
fn three_concurrent_cli_requests_keep_payload_until_own_paste_completes() {
    let _scenario = HOST_SCENARIOS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let host = hidden_host("{x}");
    let mut workers = Vec::new();
    for value in ["First", "Second", "Third"] {
        let socket = host.socket();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let response = runtime_host::status_rpc::request(
                &socket,
                &json!({"op":"execute","id":"snipfixture","values":{"x":value}}),
                Instant::now() + Duration::from_secs(3),
            )
            .unwrap();
            sender.send(response).unwrap();
        });
        workers.push((worker, receiver));
    }
    let mut results = Vec::new();
    for (worker, receiver) in workers {
        results.push(
            receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("Own snippet caller did not finish"),
        );
        worker.join().unwrap();
    }
    assert_eq!(
        results,
        vec![json!({"ok":true,"result":{"state":"pasted"}}); 3]
    );
    let trace = trace(&host);
    let phases: Vec<_> = trace
        .iter()
        .filter_map(|event| {
            let phase = event["phase"].as_str().unwrap();
            ["snippet-write", "snippet-post"]
                .contains(&phase)
                .then_some(phase)
        })
        .collect();
    assert_eq!(
        phases,
        ["snippet-write", "snippet-post", "snippet-post"].repeat(3)
    );
    let texts: BTreeSet<_> = trace
        .iter()
        .filter(|event| event["phase"] == "snippet-write")
        .map(|event| event["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, BTreeSet::from(["First", "Second", "Third"]));
    assert_eq!(
        host.rpc(&json!({"op":"status"}))["result"]["snippetTransactionsPending"],
        0
    );
    save("concurrent-callers", &json!(results), &trace);
}

fn trace(host: &Host) -> Vec<Value> {
    let text = fs::read_to_string(host.base.join("workspace-trace.jsonl")).unwrap_or_default();
    let complete = text
        .rsplit_once('\n')
        .map(|(complete, _)| complete)
        .unwrap_or("");
    complete
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn save(name: &str, response: &Value, trace: &[Value]) {
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        fs::write(
            PathBuf::from(directory).join(format!("snippet-headless-{name}.json")),
            serde_json::to_vec_pretty(&json!({"response":response,"trace":trace})).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn clipboard_demand_and_validation_error_priority_match_existing_pipeline() {
    let _scenario = HOST_SCENARIOS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let host = host("{missing} {clipboard}", &["deny-ax"], Some("Own input"));
    let response = host.rpc(&json!({"op":"execute","id":"snipfixture"}));
    assert_eq!(response["ok"], false);
    assert!(response["error"].as_str().unwrap().contains("missing"));
    assert!(host.base.join("clipboard-read").exists());
    let trace = trace(&host);
    assert!(!trace.iter().any(|event| event["phase"] == "snippet-write"));
    assert!(!trace.iter().any(|event| event["phase"] == "snippet-trust"));
    save("validation-priority", &response, &trace);
}
#[test]
fn headless_missing_target_and_denied_ax_never_mutate_board() {
    let _scenario = HOST_SCENARIOS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (name, flags, error) in [
        ("missing-target", &[][..], "Choose an app"),
        ("denied-ax", &["deny-ax"][..], "Allow Ruycast"),
    ] {
        let host = host("Plain", flags, None);
        let response = host.rpc(&json!({"op":"execute","id":"snipfixture"}));
        assert_eq!(response["ok"], false);
        assert!(response["error"].as_str().unwrap().contains(error));
        let trace = trace(&host);
        for phase in ["snippet-write", "snippet-post", "snippet-clear"] {
            assert!(!trace.iter().any(|event| event["phase"] == phase));
        }
        save(name, &response, &trace);
    }
}
