//! Reusable editor operations on an owned hidden Actions host.
#[allow(dead_code)]
#[path = "native_actions.rs"]
pub mod actions;
use actions::native_bridge::Bridge;
use serde_json::{json, Value};
pub fn edit(host: &Bridge) -> Value {
    actions::action(host, "Edit");
    actions::view(host, |v| {
        v["formVisible"] == true && actions::text_contains(v, "Name")
    })
}
pub fn field(host: &Bridge, key: &str, text: &str) {
    host.probe("command-input", json!({"key":key,"text":text}));
}
