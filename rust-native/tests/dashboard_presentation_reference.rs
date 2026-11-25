//! Captured Swift dashboard presentation compared with the Rust analytics surface.
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
fn compare(case: &Value) -> Value {
    let root = runtime_host::OwnedDir::new();
    fs::write(root.0.join(".ruycast-analytics-fixture"), b"").unwrap();
    let mut request = case["request"].clone();
    request["root"] = json!(root.0);
    let mut child = Command::new(env!("CARGO_BIN_EXE_analytics-probe"))
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || serde_json::to_writer(stdin, &request));
    let output = child_output::wait(child, Duration::from_secs(30));
    assert!(writer.join().unwrap().is_ok());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut rust = value["results"].as_array().unwrap().last().unwrap().clone();
    for day in rust["days"].as_array_mut().unwrap() {
        let day = day.as_object_mut().unwrap();
        assert!(day.remove("date").is_some());
        assert!(day.remove("accessibility").is_some());
    }
    for error in rust["errors"].as_array_mut().unwrap() {
        assert!(error.as_object_mut().unwrap().remove("id").is_some());
    }
    // Preserve the captured Swift reference and override only its known
    // floor-index percentiles for the intentional nearest-rank change.
    let mut current = case["expected"].clone();
    for row in current["commands"].as_array_mut().unwrap() {
        match row["p95"].as_str().unwrap() {
            "0.70 ms" => {
                assert!(["Last wins disabled", "Discovered app", "Decomposed", "Ruycast Dashboard"].contains(&row["name"].as_str().unwrap()));
                row["p95"] = json!("100.0 ms");
            }
            "0.01 ms" => row["p95"] = json!("1.0 ms"),
            _ => {}
        }
    }
    for timing in current["timings"].as_array_mut().unwrap() {
        match (timing[0].as_str().unwrap(), timing[1].as_str().unwrap()) {
            ("Search", "25.6 ms") => timing[1] = json!("25.7 ms"),
            ("Commands", "0.01 ms") => timing[1] = json!("1.0 ms"),
            _ => {}
        }
    }
    for (key, value) in current.as_object().unwrap() {
        assert!(
            equivalent(&rust[key], value),
            "dashboard field {key}: actual={} expected={value}",
            rust[key]
        );
    }
    rust
}
#[test]
fn populated_top30_hash_ties_names_samples_errors_and_legacy() {
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/dashboard_presentation_reference.json"
    ))
    .unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 14);
    let r = compare(&cases[0]);
    let commands = r["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 30);
    assert_eq!(r["errors"].as_array().unwrap().len(), 20);
    assert_eq!(r["prior_successes"], 14);
    assert_eq!(r["timings"], json!([["Commands", "100.0 ms"], ["Search", "25.7 ms"]]));
    assert!(commands
        .iter()
        .any(|row| row["name"] == "Last wins disabled"));
    assert!(commands.iter().any(|row| row["name"] == "Decomposed"));
    assert_eq!(
        commands
            .iter()
            .find(|row| row["name"] == "Last wins disabled")
            .unwrap()["p95"],
        "100.0 ms"
    );
}
#[test]
fn elapsed_hours_retention_dst_and_calendar_year() {
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/dashboard_presentation_reference.json"
    ))
    .unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 14);
    for case in &cases[1..13] {
        let r = compare(case);
        assert_eq!(r["commands"][0]["p95"], "1.0 ms");
        assert_eq!(r["timings"], json!([["Commands", "1.0 ms"]]));
        let hours = r["hours"].as_array().unwrap();
        assert_eq!(hours.len(), 24);
        assert_eq!(r["days"].as_array().unwrap().len(), 5);
        assert!(hours
            .windows(2)
            .all(|p| p[1]["date"].as_f64().unwrap() - p[0]["date"].as_f64().unwrap() == 3600.0));
        assert!(hours.iter().all(|p| !p["uses"].is_null()));
    }
    let r = compare(&cases[13]);
    assert_eq!(r["commands"], json!([]));
    assert_eq!(r["days"], json!([]));
    assert_eq!(
        r["hours"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["uses"].is_null())
            .count(),
        23
    );
}
