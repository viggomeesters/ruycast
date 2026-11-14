//! Owned real Carbon ABI with a complete synthetic SDK and no physical input.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Owned fixture and direct probe share bounded child support"
)]
#[path = "support/child_output.rs"]
mod child_output;
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn config(launcher: &str, clipboard: &str) -> Value {
    let mut c = runtime_host::default_config();
    c["launcherHotkey"] = json!(launcher);
    c["clipboardHotkey"] = json!(clipboard);
    c["commands"] = json!([]);
    c
}
fn stage(c: Value) -> Value {
    json!({"action":"stage","config":c})
}
fn run_probe(operations: Vec<Value>, flags: &[&str], shim: Option<&str>) -> (Value, Vec<Value>) {
    let owned = runtime_host::OwnedDir::new();
    let home = &owned.0;
    if !flags.contains(&"missing-marker") {
        fs::write(
            home.join(".ruycast-keyboard-fixture"),
            "own-keyboard-fixture-v1",
        )
        .unwrap();
    }
    for flag in flags {
        if *flag != "missing-marker" {
            fs::write(home.join(flag), "own fault").unwrap();
        }
    }
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_keyboard-probe"));
    let dir = probe.parent().unwrap().join("examples");
    let mut cmd = Command::new(&probe);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "DYLD_INSERT_LIBRARIES",
        "RUYCAST_HOME",
        "RUYCAST_CONFIG_DIR",
        "RUSTCAST_HOME",
        "RUYCAST_KEYBOARD_FIXTURE_HOME",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("HOME", home)
        .env("RUYCAST_KEYBOARD_FIXTURE_HOME", home);
    let mut artifacts = std::collections::BTreeMap::new();
    let mut originals = std::collections::BTreeMap::new();
    for (label, var, name) in [
        (
            "complete",
            "RUYCAST_TEST_KEYBOARD_SHIM",
            "libkeyboard_fixture.dylib",
        ),
        (
            "partial",
            "RUYCAST_TEST_KEYBOARD_PARTIAL",
            "libkeyboard_missing_parameter.dylib",
        ),
    ] {
        let artifact = std::env::var_os(var)
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.join(name));
        assert!(artifact.is_absolute() && artifact.is_file());
        let copy = home.join(name);
        fs::copy(&artifact, &copy).unwrap();
        assert_eq!(
            Sha256::digest(fs::read(&artifact).unwrap()),
            Sha256::digest(fs::read(&copy).unwrap())
        );
        originals.insert(label, artifact);
        artifacts.insert(label, copy);
    }
    if let Some(which) = shim {
        cmd.env("DYLD_INSERT_LIBRARIES", artifacts.get(which).unwrap());
    }
    if let Some(evidence) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap();
        let folder = PathBuf::from(evidence);
        fs::create_dir_all(&folder).unwrap();
        let root = runtime_host::root();
        let sources = [
            "rust-native/tests/fixtures/keyboard_sdk.rs",
            "rust-native/tests/fixtures/keyboard_fixture.rs",
            "rust-native/tests/fixtures/keyboard_missing_parameter.rs",
            "Sources/LauncherCore/RustBridge.swift",
            "Sources/RuycastApp/Hotkeys.swift",
        ]
        .into_iter()
        .map(|p| {
            (
                p.to_owned(),
                format!("{:x}", Sha256::digest(fs::read(root.join(p)).unwrap())),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
        let mut copies = std::collections::BTreeMap::new();
        for (label, source) in &originals {
            let (path, digest) = runtime_host::bind_artifact(source, &format!("keyboard-{label}"));
            copies.insert(path, digest);
        }
        fs::write(
            folder.join("keyboard-reference-bindings.json"),
            serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":copies})).unwrap(),
        )
        .unwrap();
    }
    let mut child = cmd.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let bytes = serde_json::to_vec(&operations).unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let out = child_output::wait(child, Duration::from_secs(8));
    writer.join().unwrap().unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v = serde_json::from_slice(&out.stdout).unwrap();
    let path = home.join("keyboard-trace.jsonl");
    let trace = if path.exists() {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect()
    } else {
        vec![]
    };
    assert!(trace.iter().all(|e| e["mainThread"] == true));
    (v, trace)
}
fn phases<'a>(trace: &'a [Value], name: &str) -> Vec<&'a Value> {
    trace.iter().filter(|e| e["phase"] == name).collect()
}
#[test]
fn complete_boundary_missing_symbols_and_marker_fail_closed() {
    for (shim, flags) in [
        (None, vec![]),
        (Some("partial"), vec![]),
        (Some("complete"), vec!["missing-marker"]),
    ] {
        let (v, t) = run_probe(vec![], &flags, shim);
        assert_eq!(v["initialization"]["ok"], false);
        assert!(t.is_empty());
    }
}
#[test]
fn real_registration_callback_identity_staging_and_retirement() {
    let (v, t) = run_probe(
        vec![
            stage(config("cmd+a", "cmd+s")),
            json!({"action":"emit","id":1}),
            json!({"action":"commit"}),
            json!({"action":"emit","id":1}),
            stage(config("cmd+d", "cmd+s")),
            json!({"action":"emit","id":3}),
            json!({"action":"commit"}),
            json!({"action":"emit","id":1}),
            json!({"action":"emit","id":3}),
            json!({"action":"emit","id":2}),
            json!({"action":"drop"}),
            json!({"action":"emit","id":3}),
        ],
        &["emit-on-register", "emit-on-unregister"],
        Some("complete"),
    );
    assert_eq!(v["initialization"]["ok"], true);
    let presses = v["presses"].as_array().unwrap();
    for (field, expected) in [
        ("action", json!(["@launcher", "@launcher", "@clipboard"])),
        ("generation", json!([1, 2, 2])),
        ("key", json!(["cmd+a", "cmd+d", "cmd+s"])),
    ] {
        assert_eq!(
            Value::Array(
                presses
                    .iter()
                    .map(|p| p.get(field).unwrap().clone())
                    .collect()
            ),
            expected
        );
    }
    assert!(t
        .iter()
        .filter(|e| e["phase"] == "register-callback" || e["phase"] == "unregister-callback")
        .all(|e| e["b"] == -9874));
    let pairs = phases(&t, "register")
        .iter()
        .map(|e| json!([e["a"], e["b"]]))
        .collect::<Vec<_>>();
    assert_eq!(
        pairs,
        vec![json!([0, 256]), json!([1, 256]), json!([2, 256])]
    );
    assert_eq!(phases(&t, "unregister").len(), 3);
    assert_eq!(phases(&t, "remove").len(), 1);
}
#[test]
fn malformed_carbon_events_never_dispatch() {
    let variants = [
        ("class", json!(0)),
        ("kind", json!(6)),
        ("signature", json!(0)),
        ("parameterType", json!(0)),
        ("size", json!(4)),
        ("size", json!(16)),
        ("status", json!(-50)),
    ];
    let mut ops = vec![stage(config("cmd+a", "cmd+s")), json!({"action":"commit"})];
    for (field, value) in variants {
        let mut v = json!({"action":"emit","id":1});
        v[field] = value;
        ops.push(v);
    }
    ops.push(json!({"action":"emit","id":9999}));
    let (v, t) = run_probe(ops, &[], Some("complete"));
    assert_eq!(v["presses"], json!([]));
    assert!(v["steps"].as_array().unwrap()[2..]
        .iter()
        .all(|s| s.get("status").unwrap_or(&json!(-9874)) == -9874));
    assert_eq!(phases(&t, "parameter").len(), 6);
}
#[test]
fn carbon_modifier_masks_match_sdk_and_hyper() {
    let (v, t) = run_probe(
        vec![
            stage(config("ctrl+alt+shift+cmd+f12", "alt+f1")),
            json!({"action":"commit"}),
        ],
        &[],
        Some("complete"),
    );
    assert_eq!(v["steps"][0]["ok"], true);
    let mut pairs = phases(&t, "register")
        .iter()
        .map(|e| (e["a"].as_i64().unwrap(), e["b"].as_i64().unwrap()))
        .collect::<Vec<_>>();
    pairs.sort();
    assert_eq!(pairs, vec![(111, 6912), (122, 2048)]);
}
#[test]
fn physical_suspension_and_latest_plan_resume_keep_generation() {
    let (v, t) = run_probe(
        vec![
            stage(config("cmd+a", "cmd+s")),
            json!({"action":"commit"}),
            json!({"action":"suspend"}),
            json!({"action":"emit","id":1}),
            stage(config("cmd+d", "cmd+s")),
            json!({"action":"commit"}),
            json!({"action":"resume"}),
            json!({"action":"emit","id":3}),
        ],
        &[],
        Some("complete"),
    );
    assert_eq!(
        v["presses"],
        json!([{"generation":2,"key":"cmd+d","action":"@launcher"}])
    );
    assert_eq!(phases(&t, "register").len(), 4);
}
#[test]
fn callback_gate_and_panic_are_fail_closed() {
    let (v, _) = run_probe(
        vec![
            stage(config("cmd+a", "cmd+s")),
            json!({"action":"commit"}),
            json!({"action":"gate","enabled":false}),
            json!({"action":"emit","id":1}),
            json!({"action":"gate","enabled":true}),
            json!({"action":"panic"}),
            json!({"action":"emit","id":1}),
            json!({"action":"emit","id":1}),
        ],
        &[],
        Some("complete"),
    );
    assert_eq!(v["presses"], json!([]));
    let statuses = v["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s.get("status"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(statuses, vec![json!(-9874); 3]);
}
#[test]
fn partial_registration_and_failed_cleanup_stay_bounded() {
    let (v, t) = run_probe(
        vec![
            stage(config("cmd+a", "cmd+s")),
            stage(config("cmd+d", "cmd+s")),
        ],
        &["partial-register", "fail-unregister"],
        Some("complete"),
    );
    assert_eq!(v["steps"][0]["ok"], false);
    assert_eq!(v["steps"][1]["ok"], false);
    assert_eq!(v["presses"], json!([]));
    assert_eq!(phases(&t, "register").len(), 1);
    assert!(phases(&t, "unregister").len() >= 2);
}
#[test]
fn unresolved_old_registration_blocks_new_candidates() {
    let (v, t) = run_probe(
        vec![
            stage(config("cmd+a", "cmd+s")),
            json!({"action":"commit"}),
            stage(config("cmd+d", "cmd+s")),
            json!({"action":"commit"}),
            stage(config("cmd+f", "cmd+s")),
            stage(config("cmd+g", "cmd+s")),
        ],
        &["fail-unregister"],
        Some("complete"),
    );
    assert_eq!(v["steps"][2]["ok"], true);
    assert_eq!(v["steps"][4]["ok"], false);
    assert_eq!(v["steps"][5]["ok"], false);
    assert_eq!(phases(&t, "register").len(), 3);
}
#[test]
fn failed_remove_and_null_handler_keep_callback_disabled() {
    let (v, _) = run_probe(
        vec![
            stage(config("cmd+a", "cmd+s")),
            json!({"action":"commit"}),
            json!({"action":"drop"}),
            json!({"action":"gate","enabled":true}),
            json!({"action":"emit","id":1}),
        ],
        &["fail-remove"],
        Some("complete"),
    );
    assert_eq!(v["presses"], json!([]));
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap()["status"],
        -9874
    );
    let (v, t) = run_probe(
        vec![json!({"action":"emit","id":1})],
        &["null-install"],
        Some("complete"),
    );
    assert_eq!(v["initialization"]["ok"], false);
    assert_eq!(v["presses"], json!([]));
    assert_eq!(v["steps"][0]["status"], -9874);
    assert!(phases(&t, "register").is_empty());
}
#[test]
fn every_literal_swift_shortcut_keycode_including_escape() {
    let bytes =
        fs::read(runtime_host::root().join("rust-native/tests/keyboard_keycodes_reference.json"))
            .unwrap();
    let gold: Value = serde_json::from_slice(&bytes).unwrap();
    let source = fs::read(runtime_host::root().join(gold["source"].as_str().unwrap())).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(source)),
        gold["sourceSHA256"]
    );
    let codes = gold["keyCodes"].as_object().unwrap();
    assert_eq!(codes.len(), 71);
    for (key, code) in codes {
        if key == "left-control" || key == "left-option" {
            continue;
        }
        let (v, t) = run_probe(
            vec![
                stage(config(&format!("ctrl+alt+{key}"), "cmd+s")),
                json!({"action":"commit"}),
            ],
            &[],
            Some("complete"),
        );
        assert_eq!(v["steps"][0]["ok"], true, "{key}");
        let registered = phases(&t, "register")
            .iter()
            .filter(|e| e["b"] == ((1 << 12) | (1 << 11)))
            .map(|e| e["a"].clone())
            .collect::<Vec<_>>();
        assert_eq!(registered, vec![code.clone()], "{key}");
    }
}
