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
// Python's original JSON comparison considers finite integral floats equal to
// equal integers; restrict cross-type conversion to exactly representable ints.
fn equivalent(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if x == y {
                return true;
            }
            let exact = |n: &serde_json::Number| {
                n.as_i64()
                    .map(|v| v.unsigned_abs() <= (1u64 << 53))
                    .or_else(|| n.as_u64().map(|v| v <= (1u64 << 53)))
                    .unwrap_or(false)
            };
            ((x.is_f64() && exact(y)) || (y.is_f64() && exact(x))) && x.as_f64() == y.as_f64()
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| equivalent(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, a)| y.get(k).is_some_and(|b| equivalent(a, b)))
        }
        _ => a == b,
    }
}

fn compare_from(data: &str, case: &str) -> Value {
    let fixture: Value = serde_json::from_str(data).unwrap();
    let entry = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == case)
        .unwrap();
    let root = runtime_host::OwnedDir::new();
    fs::write(root.0.join(".ruycast-analytics-fixture"), b"").unwrap();
    let input = json!({"root":root.0,"now":entry.get("now").unwrap_or(&fixture["provenance"]["now"]),"zone":entry.get("zone").unwrap_or(&fixture["provenance"]["zone"]),"calendar":entry.get("calendar").unwrap_or(&fixture["provenance"]["calendar"]),"actions":entry["actions"]});
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
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut normalized = actual.clone();
    normalize(&mut normalized);
    assert!(
        equivalent(&normalized, &entry["expected"]),
        "{case}: actual={normalized} expected={}",
        entry["expected"]
    );
    actual
}
#[test]
fn private_serde_magic_keys_remain_literal_objects() {
    let result = compare_from(
        include_str!("fixtures/analytics_cast_reference.json"),
        "private_serde_magic_keys",
    );
    assert_eq!(
        result["results"],
        json!(vec![json!({"bool":null,"int":null}); 6])
    );
}
#[test]
fn foundation_request_casts_are_scoped_to_analytics() {
    compare_from(
        include_str!("fixtures/analytics_cast_reference.json"),
        "foundation_request_casts",
    );
}

#[test]
fn attempt_completion_pause_reset_and_private_export() {
    let out = compare_from(
        include_str!("fixtures/analytics_lifecycle_reference.json"),
        "attempt_completion_pause_reset_private_export",
    );
    assert!(!serde_json::to_string(&out)
        .unwrap()
        .contains("own private URL token"));
    assert_eq!(out["document"]["totalFailures"], 1);
}
#[test]
fn calendar_year_and_local_day_match_foundation() {
    for calendar in ["gregorian", "buddhist"] {
        for zone in ["UTC", "Pacific/Kiritimati", "America/Los_Angeles"] {
            compare_from(
                include_str!("fixtures/analytics_lifecycle_reference.json"),
                &format!("calendar_{calendar}_{zone}"),
            );
        }
    }
}

#[test]
fn reference_comparison_preserves_numeric_boundaries() {
    assert!(equivalent(&json!(3), &json!(3.0)));
    assert!(!equivalent(&json!(3), &json!(3.5)));
    assert!(!equivalent(&json!(true), &json!(1)));
    assert!(!equivalent(
        &json!(9007199254740993u64),
        &json!(9007199254740992.0)
    ));
}
