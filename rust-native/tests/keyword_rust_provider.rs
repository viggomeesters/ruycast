//! Actual native owners plus the loaded complete Rust synthetic SDK.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn run(capture: bool, library: Option<&str>, packet: Value) -> Value {
    let binary = if capture {
        env!("CARGO_BIN_EXE_keyword-capture-probe")
    } else {
        env!("CARGO_BIN_EXE_keyword-native-probe")
    };
    let mut command = Command::new(binary);
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(name) = library {
        let target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("Own target"));
        assert!(target.is_absolute());
        let artifact = target.join("release/examples").join(name);
        assert!(artifact.is_file());
        command.env("DYLD_INSERT_LIBRARIES", artifact);
    }
    let mut child = command.spawn().unwrap();
    let written = serde_json::to_writer(child.stdin.take().unwrap(), &packet);
    let output = child_output::wait(child, Duration::from_secs(8));
    assert!(
        written.is_ok() && output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("Actual owner JSON")
}
fn complete(capture: bool, packet: Value) -> Value {
    let value = run(capture, Some("libkeyword_fixture.dylib"), packet);
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["final"]["allocated"], 0);
    assert_eq!(value["final"]["apps"], 0);
    if capture {
        assert_eq!(value["retired"], true);
    }
    value
}
#[test]
fn native_context_anchor_identity_and_reentrant_revocation() {
    let value = complete(
        false,
        json!([{"op":"text","value":"abc"},{"op":"front"},{"op":"anchor"},{"op":"verify"},{"op":"env","element":2},{"op":"verify"}]),
    );
    assert_eq!(value["steps"][2], json!({"ok":true}));
    assert_eq!(value["steps"][3], json!({"continuity":"pinned"}));
    assert_eq!(value["steps"][5], json!({"continuity":"changed"}));
    let value = complete(false, json!([{"op":"revoke","at":"copy:0"},{"op":"safe"}]));
    assert_eq!(value["steps"][1], json!({"error":"Revoked"}));
}
fn capture_packet(hook: bool) -> Value {
    let mut config = rustcast_core::default_config();
    config["commands"] = json!([{"id":"own.snippet","title":"Own snippet","kind":"snippet","target":"HELLO","keyword":"abc"}]);
    config["snippetExpansionEnabled"] = json!(true);
    let mut packet = vec![
        json!({"op":"config","value":config}),
        json!({"op":"refresh"}),
    ];
    if hook {
        packet.push(json!({"op":"hook","at":"post:down","action":"stop"}));
    }
    packet.extend(["a", "b", "c"].map(|text| json!({"op":"sdk","control":"event","text":text})));
    json!(packet)
}
#[test]
fn native_plain_replacement_and_reentry_balance_owned_events() {
    let value = complete(true, capture_packet(false));
    let events = value["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row.get("consumed"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(events, vec![json!(false), json!(false), json!(true)]);
    let output = value["final"]["output"].as_array().unwrap();
    let keys = output
        .iter()
        .map(|row| (row["key"].as_i64().unwrap(), row["down"].as_bool().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            (51, true),
            (51, false),
            (51, true),
            (51, false),
            (0, true),
            (0, false)
        ]
    );
    assert_eq!(output[4]["units"], json!([72, 69, 76, 76, 79]));
    assert_eq!(value["final"]["valueReads"], 0);
    let value = complete(true, capture_packet(true));
    let output = value["final"]["output"].as_array().unwrap();
    assert_eq!(output.len(), 2);
    assert_eq!(output[1]["down"], false);
    assert_eq!(output[1]["units"], json!([]));
}
#[test]
fn missing_value_type_and_absent_sdk_refuse_before_discovery() {
    for library in [None, Some("libkeyword_missing_value_type.dylib")] {
        for capture in [false, true] {
            let value = run(capture, library, json!([]));
            assert_eq!(value["ok"], false);
            assert!(value["error"]
                .as_str()
                .unwrap()
                .contains("Complete keyword SDK fixture"));
        }
    }
}
