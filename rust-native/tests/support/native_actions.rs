//! Shared native Actions fixtures. Every event stays inside the owned hidden host.
#[path = "native_bridge.rs"]
pub mod native_bridge;
use native_bridge::{runtime_host::Config, Bridge};
use serde_json::{json, Value};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
pub const CMD: u64 = 1 << 20;
pub const OPT: u64 = 1 << 19;
pub const SHIFT: u64 = 1 << 17;
pub const CTRL: u64 = 1 << 18;
pub fn host() -> Bridge {
    let host = Bridge::new(Config::current(), true);
    host.settled(|_| true);
    host
}
pub fn probe(host: &Bridge, action: &str) -> Value {
    host.probe(action, json!({}))
}
pub fn view(host: &Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        let remaining = until.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "Native action/editor did not reach expected state"
        );
        let value = host.rpc_timeout(
            &json!({"op":"native.test","action":"read"}),
            remaining.min(Duration::from_secs(3)),
        );
        assert_eq!(value["ok"], true, "{value}");
        let value = value["result"].clone();
        if predicate(&value) {
            return value;
        }
        thread::sleep(
            Duration::from_millis(10).min(until.saturating_duration_since(Instant::now())),
        );
    }
}
pub fn text_contains(value: &Value, expected: &str) -> bool {
    value["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains(expected))
}
pub fn show(host: &Bridge, query: &str) {
    assert_eq!(host.rpc(&json!({"op":"show","query":query}))["ok"], true);
    host.settled(|s| s["nativeUI"]["visible"] == true);
    view(host, |v| v["input"] == query);
}
pub fn key(host: &Bridge, code: u32, flags: u64, chars: &str, repeated: bool) -> Value {
    host.probe(
        "key-event",
        json!({"code":code,"flags":flags,"chars":chars,"repeat":repeated}),
    )
}
pub fn alias(host: &Bridge) -> Value {
    key(host, 43, CMD | OPT, ",", false);
    view(host, |v| {
        v["formVisible"] == true && text_contains(v, "Alias")
    })
}
pub fn hotkey(host: &Bridge) -> Value {
    key(host, 43, CMD | SHIFT, ",", false);
    view(host, |v| v["formVisible"] == true && v["recording"] == true)
}
pub fn saved(host: &Bridge) -> Value {
    view(host, |v| v["formVisible"] == false)
}
pub fn config(host: &Bridge) -> Value {
    serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap()
}
pub fn command(host: &Bridge, id: &str) -> Value {
    config(host)["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap()
        .clone()
}
pub fn action(host: &Bridge, name: &str) {
    if probe(host, "read")["actionsVisible"] != true {
        key(host, 40, CMD, "k", false);
    }
    let value = view(host, |v| v["actionsVisible"] == true);
    let index = value["actionLabels"]
        .as_array()
        .unwrap()
        .iter()
        .position(|label| label.as_str().unwrap().starts_with(name))
        .unwrap();
    host.probe("action-button", json!({"index":index}));
}
