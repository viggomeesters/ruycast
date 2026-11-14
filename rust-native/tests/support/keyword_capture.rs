//! Shared isolated keyword capture probe and fixture.
#![allow(dead_code)]
#[path = "child_output.rs"]
mod child_output;
#[allow(dead_code)]
#[path = "runtime_host.rs"]
mod runtime_host;
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
pub fn config(text: &str, keyword: &str) -> Value {
    let mut c = runtime_host::default_config();
    c["commands"] = json!([{"id":"own.snippet","title":"Own snippet","kind":"snippet","target":text,"keyword":keyword}]);
    c["snippetExpansionEnabled"] = json!(true);
    c
}
pub fn sdk(control: &str, fields: Value) -> Value {
    let mut o = fields.as_object().unwrap().clone();
    o.insert("op".into(), json!("sdk"));
    o.insert("control".into(), json!(control));
    Value::Object(o)
}
pub fn event(text: &str, fields: Value) -> Value {
    let mut o = fields.as_object().unwrap().clone();
    o.insert("text".into(), json!(text));
    sdk("event", Value::Object(o))
}
pub fn keys(text: &str) -> Vec<Value> {
    text.chars()
        .map(|c| event(&c.to_string(), json!({})))
        .collect()
}
pub fn invoke(ops: Vec<Value>, library: Option<&str>, ok: bool) -> Value {
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
pub fn run_keys(text: &str, c: Value, between: Vec<Value>) -> Value {
    let mut ops = vec![json!({"op":"config","value":c}), json!({"op":"refresh"})];
    ops.extend(between);
    ops.extend(keys(text));
    invoke(ops, Some("complete"), true)
}
pub fn inserted(state: &Value) -> String {
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
