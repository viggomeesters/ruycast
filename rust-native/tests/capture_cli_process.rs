//! Opaque provider capture fields through the real headless CLI route.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use runtime_host::{default_config, Config, Host};
use serde_json::json;
fn host(mode: &str) -> Host {
    let mut h = Host::new(Config::current(), false);
    let mut cfg = default_config();
    cfg["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_script-probe"),"arguments":["capture-fixture",mode],"timeout":3}));
    h.replace(&cfg, None);
    h.start();
    h
}
#[test]
fn cli_valid_prefix_and_spoofed_capture_remain_provider_data() {
    let h = host("spoof-prefix");
    let r = h.rpc(&json!({"op":"execute","id":"runfixture"}));
    assert_eq!(r["ok"], true);
    assert!(r["detail"].as_str().unwrap().len() > 65000);
    assert_eq!(
        r["capture"],
        json!({"stdout":{"truncated":false,"eof":true,"validUtf8":true}})
    );
}
#[test]
fn cli_keeps_provider_capture_fields_and_legacy_lossy_behavior() {
    for mode in ["spoof-truncated", "spoof-invalid"] {
        let h = host(mode);
        let r = h.rpc(&json!({"op":"execute","id":"runfixture"}));
        assert_eq!(r["ok"], true);
        let capture = r.get("capture").expect("provider capture field");
        assert!(capture.get("stdout").is_none());
        assert!(capture.get("stderr").is_none());
    }
}
