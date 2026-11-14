//! Owned analytics runtime and source-built write shim.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{Config, Host};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

fn host() -> Host {
    let mut h = Host::new(Config::current(), false);
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_ruycast-native"));
    let shim = std::env::var_os("RUYCAST_TEST_ANALYTICS_SHIM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            executable
                .parent()
                .unwrap()
                .join("examples/libanalytics_write_shim.dylib")
        });
    assert!(
        shim.is_absolute() && shim.is_file(),
        "Build analytics-write-shim first: {shim:?}"
    );
    let owned = h.base.join("analytics-write-shim.dylib");
    fs::copy(shim, &owned).unwrap();
    h.env
        .insert("DYLD_INSERT_LIBRARIES".into(), owned.into_os_string());
    let home = h.base.as_os_str().to_owned();
    h.env.insert("RUYCAST_ANALYTICS_FIXTURE_HOME".into(), home);
    h.start();
    h
}
#[test]
fn idle_cli_settings_flush_wakes_owner_and_preserves_existing_file() {
    let h = host();
    let started = Instant::now();
    let paused = h.rpc(&json!({"op":"analytics.settings","enabled":false}));
    assert_eq!(paused["ok"], true, "{paused}");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(paused["result"]["enabled"], false);
    let stored: Value =
        serde_json::from_slice(&fs::read(h.profile.join("analytics.json")).unwrap()).unwrap();
    assert_eq!(stored["enabled"], false);
    assert_eq!(h.cli(&["analytics", "resume"])["result"]["enabled"], true);
    assert_eq!(h.cli(&["analytics", "reset", "--confirm"])["ok"], true);
    let exported = h.cli(&["analytics", "export"])["result"].clone();
    assert!(exported.get("commands").is_none());
    assert_eq!(exported["attempts"], 0);
}
#[test]
fn raw_analytics_request_casts_match_foundation_without_config_coercion() {
    let h = host();
    for (value, expected) in [
        (json!(0), false),
        (json!(1), true),
        (json!(2), true),
        (json!(-1), true),
        (json!(1.5), true),
        (json!("1"), true),
        (Value::Null, true),
    ] {
        let result = h.rpc(&json!({"op":"analytics.settings","enabled":value}));
        assert_eq!(result["ok"], true);
        assert_eq!(result["result"]["enabled"], expected);
    }
    for value in [json!(true), json!(1.0), json!(1.5), json!("7")] {
        let result = h.rpc(&json!({"op":"analytics.settings","retentionDays":value}));
        assert_eq!(result["ok"], true);
        assert_eq!(result["result"]["retentionDays"], 1);
    }
    assert_eq!(
        h.rpc(&json!({"op":"analytics.settings","retentionDays":false}))["ok"],
        false
    );
    assert_eq!(
        h.rpc(&json!({"op":"analytics.reset","confirmed":0}))["ok"],
        false
    );
    assert_eq!(
        h.rpc(&json!({"op":"analytics.reset","confirmed":1}))["ok"],
        true
    );
}
