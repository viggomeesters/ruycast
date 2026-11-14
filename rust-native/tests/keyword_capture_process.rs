//! Owned native keyword filter/effects with the real Rust probe and SDK fixtures.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Owned directory and direct probe share bounded child support"
)]
#[path = "support/child_output.rs"]
mod child_output;
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn config(text: &str, keyword: &str) -> Value {
    let mut c = runtime_host::default_config();
    c["commands"] = json!([{"id":"own.snippet","title":"Own snippet","kind":"snippet","target":text,"keyword":keyword}]);
    c["snippetExpansionEnabled"] = json!(true);
    c
}
fn sdk(control: &str, fields: Value) -> Value {
    let mut o = fields.as_object().unwrap().clone();
    o.insert("op".into(), json!("sdk"));
    o.insert("control".into(), json!(control));
    Value::Object(o)
}
fn event(text: &str, fields: Value) -> Value {
    let mut o = fields.as_object().unwrap().clone();
    o.insert("text".into(), json!(text));
    sdk("event", Value::Object(o))
}
fn keys(text: &str) -> Vec<Value> {
    text.chars()
        .map(|c| event(&c.to_string(), json!({})))
        .collect()
}
fn invoke(ops: Vec<Value>, library: Option<&str>, ok: bool) -> Value {
    let owned = runtime_host::OwnedDir::new();
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_keyword-capture-probe"));
    let directory = probe.parent().unwrap().join("examples");
    let mut cmd = Command::new(&probe);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars_os() {
        let s = key.to_string_lossy();
        if s.starts_with("RUYCAST_") || s.starts_with("RUSTCAST_") || s == "DYLD_INSERT_LIBRARIES" {
            cmd.env_remove(key);
        }
    }
    if let Some(library) = library {
        let (var, name) = if library == "complete" {
            ("RUYCAST_TEST_KEYWORD_SHIM", "libkeyword_fixture.dylib")
        } else {
            (
                "RUYCAST_TEST_KEYWORD_PARTIAL",
                "libkeyword_missing_value_type.dylib",
            )
        };
        let artifact = std::env::var_os(var)
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.join(name));
        assert!(artifact.is_absolute() && artifact.is_file());
        let copy = owned.0.join(name);
        fs::copy(artifact, &copy).unwrap();
        cmd.env("DYLD_INSERT_LIBRARIES", copy);
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&ops).unwrap())
        .unwrap();
    let out = child_output::wait(child, Duration::from_secs(10));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], ok, "{v}");
    if ok {
        assert_eq!(v["retired"], true);
        assert_eq!(v["final"]["allocated"], 0);
        assert_eq!(v["final"]["apps"], 0);
    }
    v
}
fn run_keys(text: &str, c: Value, between: Vec<Value>) -> Value {
    let mut ops = vec![json!({"op":"config","value":c}), json!({"op":"refresh"})];
    ops.extend(between);
    ops.extend(keys(text));
    invoke(ops, Some("complete"), true)
}
fn inserted(state: &Value) -> String {
    let units = state["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["down"] == true && e["key"] == 0)
        .flat_map(|e| {
            e["units"]
                .as_array()
                .unwrap()
                .iter()
                .map(|u| u16::try_from(u.as_u64().unwrap()).unwrap())
        })
        .collect::<Vec<_>>();
    String::from_utf16(&units).unwrap()
}
#[test]
fn complete_sdk_missing_fails_before_real_discovery() {
    for lib in [None, Some("missing")] {
        assert!(invoke(vec![], lib, false)["error"]
            .as_str()
            .unwrap()
            .contains("Complete keyword SDK fixture"));
    }
}
#[test]
fn actual_tap_plain_replacement_is_synchronous_and_tagged() {
    let v = run_keys("abc", config("HELLO", "abc"), vec![]);
    let consumed = v["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s.get("consumed"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(consumed, vec![json!(false), json!(false), json!(true)]);
    assert_eq!(inserted(&v["final"]), "HELLO");
    let out = v["final"]["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["key"].as_u64().unwrap(), e["down"].as_bool().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        out,
        vec![
            (51, true),
            (51, false),
            (51, true),
            (51, false),
            (0, true),
            (0, false)
        ]
    );
    assert_eq!(v["final"]["valueReads"], 0);
}
#[test]
fn scalar_chunks_and_cursor_steps() {
    let text = format!("{}éZ", "😀".repeat(9));
    let v = run_keys(
        "abc",
        config(&format!("{text}{{cursor}}END"), "abc"),
        vec![],
    );
    let out = v["final"]["output"].as_array().unwrap();
    assert!(out
        .iter()
        .all(|e| e["units"].as_array().unwrap().len() <= 16));
    assert_eq!(inserted(&v["final"]), format!("{text}END"));
    assert_eq!(
        out.iter()
            .filter(|e| e["key"] == 123 && e["down"] == true)
            .count(),
        3
    );
}
#[test]
fn empty_replacement_consumes_one_key_without_posts() {
    let v = run_keys("x", config("{cursor}", "x"), vec![]);
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap()["consumed"],
        true
    );
    assert_eq!(v["final"]["posts"], 0);
}
#[test]
fn clipboard_only_on_actual_matching_demand() {
    let v = run_keys(
        "ab",
        config("{clipboard}", "abc"),
        vec![sdk("clipboard", json!({"value":"OWN CLIP"}))],
    );
    assert_eq!(v["final"]["clipboardReads"], 0);
    let v = run_keys(
        "abc",
        config("{clipboard}", "abc"),
        vec![sdk("clipboard", json!({"value":"OWN CLIP"}))],
    );
    assert_eq!(v["final"]["clipboardReads"], 1);
    assert_eq!(inserted(&v["final"]), "OWN CLIP");
    let v = run_keys("abc", config("HELLO", "abc"), vec![]);
    assert_eq!(v["final"]["clipboardReads"], 0);
}
#[test]
fn modifiers_repeats_navigation_mouse_empty_and_oversized_unicode_reset() {
    for middle in [
        event("x", json!({"flags":1<<20})),
        event("x", json!({"flags":1<<18})),
        event("x", json!({"flags":1<<19})),
        event("x", json!({"repeat":1})),
        event("", json!({"key":123})),
        event("", json!({"type":1})),
        event("", json!({"type":3})),
        event("", json!({})),
        event(&"x".repeat(17), json!({})),
    ] {
        let mut ops = vec![
            json!({"op":"config","value":config("HELLO","abc")}),
            json!({"op":"refresh"}),
        ];
        ops.extend(keys("ab"));
        ops.push(middle.clone());
        ops.push(event("c", json!({})));
        let v = invoke(ops, Some("complete"), true);
        assert_eq!(
            v["steps"].as_array().unwrap().last().unwrap()["consumed"],
            false,
            "{middle}"
        );
        assert_eq!(v["final"]["posts"], 0);
    }
}
#[test]
fn backspace_and_generated_input() {
    for middle in [
        vec![event("z", json!({})), event("", json!({"key":51}))],
        vec![event("z", json!({"tag":0x5255535443415354u64}))],
    ] {
        let mut ops = vec![
            json!({"op":"config","value":config("HELLO","abc")}),
            json!({"op":"refresh"}),
        ];
        ops.extend(keys("ab"));
        ops.extend(middle);
        ops.push(event("c", json!({})));
        let v = invoke(ops, Some("complete"), true);
        assert_eq!(
            v["steps"].as_array().unwrap().last().unwrap()["consumed"],
            true
        );
    }
}
