//! Fixed outputs from the actual Swift AnalyticsStore/Foundation compatibility oracle.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Bounded probe capture shares owned fixture support"
)]
#[path = "support/child_output.rs"]
mod child_output;
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use serde_json::{json, Value};
use std::{
    fs,
    process::{Command, Stdio},
    time::Duration,
};

fn normalize(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(normalize),
        Value::Object(values) => {
            values.remove("id");
            values.values_mut().for_each(normalize);
        }
        _ => {}
    }
}
fn compare(case: &str) -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/analytics_cast_reference.json")).unwrap();
    let entry = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == case)
        .unwrap();
    let root = runtime_host::OwnedDir::new();
    fs::write(root.0.join(".ruycast-analytics-fixture"), b"").unwrap();
    let input = json!({"root":root.0,"now":fixture["provenance"]["now"],"zone":fixture["provenance"]["zone"],"calendar":fixture["provenance"]["calendar"],"actions":entry["actions"]});
    let mut child = Command::new(env!("CARGO_BIN_EXE_analytics-probe"))
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || serde_json::to_writer(stdin, &input));
    let output = child_output::wait(child, Duration::from_secs(30));
    assert!(writer.join().unwrap().is_ok());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    normalize(&mut actual);
    assert_eq!(actual, entry["expected"], "{case}");
    actual
}
#[test]
fn private_serde_magic_keys_remain_literal_objects() {
    let result = compare("private_serde_magic_keys");
    assert_eq!(result["results"], json!(vec![json!({"bool":null,"int":null}); 6]));
}
#[test]
fn foundation_request_casts_are_scoped_to_analytics() {
    compare("foundation_request_casts");
}
