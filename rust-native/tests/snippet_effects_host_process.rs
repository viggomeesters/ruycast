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

fn run_native(host: &Bridge) {
    host.probe("key-event", json!({"code":36,"chars":"\r","flags":0}));
}

fn wait_event(host: &mut Bridge, phase: &str, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if let Some(event) = trace(host)
            .into_iter()
            .rev()
            .find(|event| event["phase"] == phase && predicate(event))
        {
            return event;
        }
        assert!(
            host.process.as_mut().unwrap().try_wait().unwrap().is_none(),
            "Own snippet host exited"
        );
        thread::sleep(Duration::from_millis(5));
    }
    panic!("Own snippet phase absent: {phase}");
}

fn done(host: &Bridge) -> Value {
    host.wait(|status| status["snippetRequestsRunning"] == 0 && status["openRequestsRunning"] == 0)
}

fn notice(host: &Bridge, name: &str) -> Value {
    let value = host.probe("notice-screenshot", json!({}));
    let image = host.profile.join("native-notice-probe.png");
    assert!(fs::read(&image).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let output = PathBuf::from(directory)
            .parent()
            .unwrap()
            .join("native-snippets");
        fs::create_dir_all(&output).unwrap();
        fs::copy(image, output.join(format!("{name}.png"))).unwrap();
        fs::write(
            output.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(value["present"], true);
    assert_eq!(value["windowShown"], false);
    assert_eq!(value["canBecomeKey"], false);
    assert_eq!(value["nonactivating"], true);
    assert_eq!(value["ignoresMouse"], true);
    value
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
fn postcommit_failures_retire_input_and_use_passive_notice() {
    let _scenario = HOST_SCENARIOS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (fault, needle) in [
        ("fail-write", "clipboard may have changed"),
        ("activation-noop", "target app did not activate"),
        ("fail-event", "input events"),
    ] {
        let mut host = hidden_command_host(command("{message}"), &[fault], true);
        show(&host);
        run_native(&host);
        host.settled(|_| true);
        host.probe("run-inline-type", json!({"text":"Owned text"}));
        run_native(&host);
        wait_event(&mut host, "snippet-clear", |_| true);
        done(&host);
        host.settled(|_| true);
        let notice = notice(&host, &format!("snippet-{fault}"));
        assert_eq!(notice["success"], false);
        assert!(notice["text"].as_str().unwrap().contains(needle));
        assert_eq!(host.probe("read", json!({}))["runInputInline"], false);
        assert_eq!(host.probe("read", json!({}))["formVisible"], false);
        let trace = trace(&host);
        assert!(!trace.iter().any(|event| event["phase"] == "snippet-post"));
        if fault != "fail-write" {
            assert_eq!(
                host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
                false
            );
        }
        let usage = host.cli(&["usage"]);
        assert!(!usage["result"]
            .as_object()
            .unwrap()
            .contains_key("snippet.fixture"));
        save(
            &format!("postcommit-{fault}"),
            &json!({"notice":notice,"usage":usage}),
            &trace,
        );
    }
}

#[test]
fn preparation_failures_keep_the_input_draft_and_never_clear() {
    let _scenario = HOST_SCENARIOS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for fault in ["fail-string", "fail-html"] {
        let mut command = command("{message}");
        command["snippetOutput"] = json!("html");
        let mut host = hidden_command_host(command, &[fault], true);
        show(&host);
        run_native(&host);
        host.settled(|_| true);
        assert_eq!(host.probe("read", json!({}))["runInputInline"], true);
        host.probe("run-inline-type", json!({"text":"Own draft"}));
        run_native(&host);
        wait_event(&mut host, "snippet-item-failed", |_| true);
        done(&host);
        host.settled(|_| true);
        let view = host.probe("read", json!({}));
        assert_eq!(view["runInputInline"], true);
        let screen_text = view["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .map(|text| text.as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen_text.contains("Could not copy text"));
        let failed_trace = trace(&host);
        for phase in ["snippet-write", "snippet-post", "snippet-clear"] {
            assert!(!failed_trace.iter().any(|event| event["phase"] == phase));
        }
        fs::remove_file(host.base.join(fault)).unwrap();
        run_native(&host);
        wait_event(&mut host, "snippet-post", |_| true);
        done(&host);
        let trace = trace(&host);
        let write = trace
            .iter()
            .find(|event| event["phase"] == "snippet-write")
            .unwrap();
        assert_eq!(write["text"], "Own draft");
        save(
            &format!("preparation-{fault}"),
            &json!({"failedView":view,"failedTrace":failed_trace,"draftRetried":true}),
            &trace,
        );
    }
}

#[test]
fn real_palette_run_has_complete_copy_hide_activate_paste_and_cursor_sequence() {
    let _scenario = HOST_SCENARIOS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut command = command("A{cursor}😀end");
    command["snippetOutput"] = json!("html");
    command["timeout"] = json!(0.1);
    let mut host = hidden_command_host(command, &[], true);
    let before = fs::read(&host.config).unwrap();
    show(&host);
    run_native(&host);
    wait_event(&mut host, "snippet-post", |event| {
        event["key"] == 123 && event["down"] == false
    });
    done(&host);
    host.settled(|_| true);
    let usage = host.cli(&["usage"]);
    assert_eq!(usage["result"]["snippet.fixture"], 1);
    assert_eq!(fs::read(&host.config).unwrap(), before);
    let trace = trace(&host);
    let writes: Vec<_> = trace
        .iter()
        .filter(|event| event["phase"] == "snippet-write")
        .collect();
    assert_eq!(writes[0]["text"], "A😀end");
    assert!(writes[0]["html"]
        .as_str()
        .unwrap()
        .contains("<div>A😀end</div>"));
    let posts: Vec<_> = trace
        .iter()
        .filter(|event| event["phase"] == "snippet-post")
        .collect();
    assert_keys(&posts, &[9, 9, 123, 123, 123, 123, 123, 123, 123, 123]);
    let activate = trace
        .iter()
        .find(|event| event["phase"] == "snippet-activate")
        .unwrap();
    let activation_delay = posts[0]["atNs"]
        .as_u64()
        .unwrap()
        .checked_sub(activate["atNs"].as_u64().unwrap())
        .unwrap();
    let cursor_delay = posts[2]["atNs"]
        .as_u64()
        .unwrap()
        .checked_sub(posts[1]["atNs"].as_u64().unwrap())
        .unwrap();
    assert!(activation_delay >= 149_000_000);
    assert!(cursor_delay >= 79_000_000);
    assert_eq!(host.probe("read", json!({}))["windowShown"], false);
    assert_eq!(host.probe("read", json!({}))["formVisible"], false);
    let state: Value =
        serde_json::from_slice(&fs::read(host.base.join("snippet-status.json")).unwrap()).unwrap();
    assert_eq!(state["active"], false);
    assert_eq!(state["timer"], false);
    assert_eq!(state["pending"], 0);
    assert_eq!(state["ownChangeCount"], writes[0]["changeCount"]);
    let notice = notice(&host, "snippet-done");
    assert_eq!(notice["success"], true);
    assert_eq!(notice["text"], "Done");
    assert_eq!(notice["height"].as_f64().unwrap(), 65.0);
    save(
        "palette-success",
        &json!({"usage":usage,"state":state,"notice":notice,"activationDelayNs":activation_delay,"cursorDelayNs":cursor_delay}),
        &trace,
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
