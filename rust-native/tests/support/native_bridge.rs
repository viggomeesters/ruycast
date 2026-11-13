//! Source-driven prohibited/offscreen native fixture on the shared owned host.
#[allow(dead_code)] // Separate fixture consumers use different parts of this shared API.
#[path = "runtime_host.rs"]
pub mod runtime_host;
use runtime_host::{Config, Entry, Host};
use serde_json::{json, Value};
use std::{
    fs,
    ops::{Deref, DerefMut},
    path::PathBuf,
};
pub fn normal_host() -> PathBuf {
    let path = std::env::var_os("RUYCAST_TEST_NORMAL_HOST")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_host::root().join("rust-native/target/debug/ruycast-native"));
    assert!(
        path.is_absolute() && path.is_file(),
        "Verified normal-feature refusal host required"
    );
    path
}
pub struct Bridge {
    pub host: Host,
}
impl Bridge {
    pub fn new(settings: Config, start: bool) -> Self {
        let mut bridge = Self {
            host: Host::new(settings, false),
        };
        if start {
            bridge.start();
        }
        bridge
    }
    pub fn start(&mut self) {
        self.host.start_entry(Entry::HiddenTest);
    }
    pub fn settled(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        self.host.wait(|s| {
            s["configurationUpdating"] != true
                && s["nativeUI"]["revision"].as_u64().unwrap_or(0) > 0
                && s["nativeUI"]["revision"] == s["nativeUI"]["appliedRevision"]
                && predicate(s)
        })
    }
    pub fn probe(&self, action: &str, fields: Value) -> Value {
        let mut request = fields.as_object().unwrap().clone();
        request.insert("op".into(), json!("native.test"));
        request.insert("action".into(), json!(action));
        let response = self.host.rpc(&Value::Object(request));
        assert_eq!(response["ok"], true, "{response}");
        response["result"].clone()
    }
    pub fn capture(&self, name: &str) -> Value {
        let value = self.probe("screenshot", json!({}));
        let image = self.profile.join("native-probe.png");
        assert!(fs::read(&image).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        if let Some(evidence) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
            let directory = PathBuf::from(evidence).parent().unwrap().join("bridge");
            fs::create_dir_all(&directory).unwrap();
            fs::copy(image, directory.join(format!("{name}.png"))).unwrap();
            fs::write(
                directory.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&value).unwrap(),
            )
            .unwrap();
        }
        value
    }
}
impl Deref for Bridge {
    type Target = Host;
    fn deref(&self) -> &Host {
        &self.host
    }
}
impl DerefMut for Bridge {
    fn deref_mut(&mut self) -> &mut Host {
        &mut self.host
    }
}
