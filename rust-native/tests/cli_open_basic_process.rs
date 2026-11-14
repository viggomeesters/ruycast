//! Real CLI and headless owner with the owned Workspace interposition fixture.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    time::Duration,
};
fn command(kind: &str, target: &str) -> Value {
    json!({"id":"fixture.open","title":"Fixture","kind":kind,"target":target,"aliases":["fixture-alias"]})
}
fn host(command: Value) -> Host {
    let settings = Config::current();
    let shim = std::env::var_os("RUYCAST_TEST_WORKSPACE_SHIM")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            settings
                .host
                .parent()
                .unwrap()
                .join("examples/libworkspace_fixture.dylib")
        });
    assert!(
        shim.is_absolute() && shim.is_file(),
        "Build workspace-fixture first: {shim:?}"
    );
    let mut h = Host::new(settings, false);
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(command);
    h.replace(&config, None);
    let home = h.base.as_os_str().to_owned();
    h.env.insert("RUYCAST_WORKSPACE_FIXTURE_HOME".into(), home);
    h.env
        .insert("RUYCAST_NATIVE_TEST_DISPATCH_MS".into(), "2000".into());
    h.env
        .insert("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD".into(), "1".into());
    h.env
        .insert("DYLD_INSERT_LIBRARIES".into(), shim.into_os_string());
    h.start();
    h.env.remove(std::ffi::OsStr::new("DYLD_INSERT_LIBRARIES"));
    h.env
        .remove(std::ffi::OsStr::new("RUYCAST_WORKSPACE_FIXTURE_HOME"));
    h
}
fn trace(h: &Host) -> Vec<Value> {
    let p = h.base.join("workspace-trace.jsonl");
    if !p.exists() {
        return vec![];
    }
    fs::read_to_string(p)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn opens(h: &Host) -> Vec<Value> {
    trace(h)
        .into_iter()
        .filter(|v| v["phase"].as_str().unwrap().starts_with("open-"))
        .collect()
}
#[test]
fn unchanged_cli_saved_app_alias_and_usage() {
    let h = host(command("app", "dev.fixture.inline"));
    assert_eq!(
        h.cli(&["run", "fixture-alias"]),
        json!({"ok":true,"result":{"command":"fixture.open","state":"opened"}})
    );
    assert_eq!(h.cli(&["usage"])["result"]["fixture.open"], 1);
    let trace = trace(&h);
    assert_eq!(
        trace
            .iter()
            .map(|v| v["phase"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["installed", "resolve", "open-app", "callback"]
    );
    assert!(trace[1..].iter().all(|v| v["mainThread"] == false));
}
#[test]
fn received_canonical_value_duplicates_follow_wire_order() {
    let h = host(command("url", "https://example.invalid/{café}"));
    for (first, second) in [(r"caf\u00e9", r"cafe\u0301"), (r"cafe\u0301", r"caf\u00e9")] {
        let wire=format!("{{\"op\":\"execute\",\"id\":\"fixture.open\",\"values\":{{\"{first}\":\"first\",\"{second}\":\"last\"}}}}\n");
        let mut stream = h.connect(Duration::from_secs(3));
        stream.write_all(wire.as_bytes()).unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let result: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(
            opens(&h).last().unwrap()["url"],
            "https://example.invalid/last"
        );
    }
}
#[test]
fn unchanged_cli_keeps_sorted_serialization_policy() {
    let h = host(command("url", "https://example.invalid/{café}"));
    for input in [
        "{\"café\":\"NFC\",\"café\":\"NFD\"}",
        "{\"café\":\"NFD\",\"café\":\"NFC\"}",
    ] {
        assert_eq!(
            h.cli(&["run", "fixture.open", "--input", input])["ok"],
            true
        );
        assert_eq!(
            opens(&h).last().unwrap()["url"],
            "https://example.invalid/NFC"
        );
    }
}
