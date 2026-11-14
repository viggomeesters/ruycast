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
    if let Some(initial) = entry.get("initial").filter(|v| !v.is_null()) {
        fs::write(
            root.0.join("analytics.json"),
            serde_json::to_vec(initial).unwrap(),
        )
        .unwrap();
    }
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

#[test]
fn retention_utc_hours_and_calendar_days_across_dst() {
    let data = include_str!("fixtures/analytics_retention_reference.json");
    compare_from(data, "empty");
    for date in ["2026-03-29T12:30:00+00:00", "2026-10-25T12:30:00+00:00"] {
        for zone in ["Europe/Amsterdam", "America/New_York", "UTC"] {
            let out = compare_from(data, &format!("retention_{date}_{zone}"));
            assert_eq!(out["document"]["hours"].as_object().unwrap().len(), 2);
        }
    }
}
#[test]
fn schema_one_missing_and_null_hours_are_upgraded() {
    let data = include_str!("fixtures/analytics_retention_reference.json");
    compare_from(data, "empty");
    for variant in ["absent", "null", "empty"] {
        compare_from(data, &format!("schema_one_{variant}"));
    }
}

#[test]
fn existing_document_rejection_and_date_codec_match_foundation() {
    compare_from(
        include_str!("fixtures/analytics_retention_reference.json"),
        "empty",
    );
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/analytics_decoder_reference.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    for case in cases {
        let root = runtime_host::OwnedDir::new();
        fs::write(root.0.join(".ruycast-analytics-fixture"), b"").unwrap();
        let raw = case["raw"].as_str().unwrap().as_bytes();
        let path = root.0.join("analytics.json");
        fs::write(&path, raw).unwrap();
        let request = json!({"root":root.0,"now":1790000000,"zone":"UTC","calendar":"gregorian","actions":[]});
        let mut child = Command::new(env!("CARGO_BIN_EXE_analytics-probe"))
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || serde_json::to_writer(stdin, &request));
        let output = child_output::wait(child, Duration::from_secs(20));
        assert!(writer.join().unwrap().is_ok());
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(&path).unwrap(), raw, "{}", case["name"]);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let mut actual = if value["storageError"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
        {
            value["storageError"].clone()
        } else {
            value["document"].clone()
        };
        normalize(&mut actual);
        assert!(
            equivalent(&actual, &case["expected"]),
            "{}: actual={actual} expected={}",
            case["name"],
            case["expected"]
        );
    }
}
