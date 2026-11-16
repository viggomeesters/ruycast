//! Real headless execute caller with private Rust SDK and named clipboard fixtures.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn host(target: &str, flags: &[&str], clipboard: Option<&str>) -> Host {
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
    config["commands"].as_array_mut().unwrap().push(json!({
        "id":"snippet.fixture", "title":"Snippet Fixture", "kind":"snippet",
        "target":target, "aliases":["snipfixture"]
    }));
    host.replace(&config, None);
    for (key, value) in [
        ("DYLD_INSERT_LIBRARIES", copy.into_os_string()),
        (
            "RUYCAST_WORKSPACE_FIXTURE_HOME",
            host.base.as_os_str().to_owned(),
        ),
        ("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD", "1".into()),
        ("RUYCAST_NATIVE_TEST_SNIPPET", "1".into()),
    ] {
        host.env.insert(key.into(), value);
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
        fs::write(PathBuf::from(directory).join("snippet-effects-host-bindings.json"),serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":artifacts,"sdkCopyHashChecked":true,"scope":"Owned headless hosts, named clipboard and synthetic target processes; no desktop effects"})).unwrap()).unwrap();
    }
    host.start();
    host.env
        .remove(std::ffi::OsStr::new("DYLD_INSERT_LIBRARIES"));
    host.env
        .remove(std::ffi::OsStr::new("RUYCAST_WORKSPACE_FIXTURE_HOME"));
    host
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
