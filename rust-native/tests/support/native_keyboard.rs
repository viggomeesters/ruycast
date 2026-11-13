//! Keyboard fixture for owned hidden hosts; no global keyboard registration.
#[allow(dead_code)]
#[path = "native_actions.rs"]
pub mod actions;
use actions::native_bridge::{
    runtime_host::{self, Config},
    Bridge,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

fn sdk(owner: &Bridge, role: &'static str) -> PathBuf {
    assert!(matches!(role, "NativeKeyboard" | "GeneralKeyboard"));
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_ruycast-native"));
    let source = std::env::var_os("RUYCAST_TEST_KEYBOARD_HOST_SHIM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            executable
                .parent()
                .unwrap()
                .join("examples/libkeyboard_host.dylib")
        });
    assert!(
        source.is_absolute() && source.is_file(),
        "Build keyboard-host example first: {source:?}"
    );
    let path = owner.base.join("keyboard.dylib");
    fs::copy(&source, &path).unwrap();
    let current = format!("{:x}", Sha256::digest(fs::read(&path).unwrap()));
    static RECEIPT: OnceLock<(String, &'static str)> = OnceLock::new();
    let expected = RECEIPT.get_or_init(|| {
        let Some(evidence) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") else {
            return (current.clone(), role);
        };
        let directory = PathBuf::from(evidence).join("host-sdks");
        fs::create_dir_all(&directory).unwrap();
        let artifact = directory.join(format!("{role}.dylib"));
        if path != artifact {
            fs::copy(&path, &artifact).unwrap();
        }
        let root = runtime_host::root();
        let sources = [
            "rust-native/Cargo.toml",
            "rust-native/Cargo.lock",
            "rust/Cargo.toml",
            "rust-toolchain.toml",
            "rust-native/tests/fixtures/keyboard_host_loader.json",
            "rust-native/tests/fixtures/keyboard_host.rs",
            "rust-native/tests/fixtures/caps_sdk.rs",
            "rust-native/tests/fixtures/hid_sdk.rs",
            "rust-native/tests/fixtures/keyboard_sdk.rs",
            "rust-native/tests/fixtures/keyword_core.rs",
            "rust-native/tests/fixtures/keyword_notification.rs",
            "rust-native/tests/fixtures/keyword_board.rs",
            "rust-native/tests/fixtures/keyword_ax.rs",
            "rust-native/tests/fixtures/keyword_events.rs",
            "rust-native/tests/fixtures/workspace_sdk.rs",
        ]
        .into_iter()
        .map(|name| {
            let source = root.join(name);
            let hash = format!("{:x}", Sha256::digest(fs::read(&source).unwrap()));
            (source, hash)
        })
        .collect::<BTreeMap<_, _>>();
        let sha = format!("{:x}", Sha256::digest(fs::read(&artifact).unwrap()));
        assert_eq!(sha, current);
        fs::write(
            directory.join(format!("{role}.json")),
            serde_json::to_vec_pretty(&json!({"sources":sources,"artifact":artifact,"sha256":sha}))
                .unwrap(),
        )
        .unwrap();
        (current.clone(), role)
    });
    assert_eq!(
        (&current, role),
        (&expected.0, expected.1),
        "Keyboard SDK changed between owned hosts"
    );
    path
}
pub fn host(
    commands: &[Value],
    start: bool,
    optin: bool,
    marker: bool,
    theme: Option<&str>,
) -> Bridge {
    host_for_role(commands, start, optin, marker, theme, "NativeKeyboard")
}
pub fn host_for_role(
    commands: &[Value],
    start: bool,
    optin: bool,
    marker: bool,
    theme: Option<&str>,
    role: &'static str,
) -> Bridge {
    let mut host = Bridge::new(Config::current(), false);
    let fixture = sdk(&host, role);
    if !commands.is_empty() || theme.is_some() {
        let mut config = runtime_host::default_config();
        config["commands"]
            .as_array_mut()
            .unwrap()
            .extend_from_slice(commands);
        if let Some(theme) = theme {
            config["theme"] = json!(theme);
        }
        host.replace(&config, None);
    }
    if marker {
        fs::write(
            host.base.join(".ruycast-keyboard-fixture"),
            b"own-keyboard-fixture-v1",
        )
        .unwrap();
    }
    host.env
        .insert("DYLD_INSERT_LIBRARIES".into(), fixture.into_os_string());
    let home = host.base.as_os_str().to_owned();
    for variable in [
        "RUYCAST_KEYWORD_FIXTURE_HOME",
        "RUYCAST_KEYBOARD_FIXTURE_HOME",
        "RUYCAST_WORKSPACE_FIXTURE_HOME",
    ] {
        host.env.insert(variable.into(), home.clone());
    }
    host.env
        .insert("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD".into(), "trace".into());
    if optin {
        host.env
            .insert("RUYCAST_NATIVE_TEST_KEYBOARD".into(), "1".into());
    }
    if start {
        host.start();
        host.settled(|s| {
            if optin {
                s["keyboardGeneration"].as_u64().unwrap_or(0) > 0
            } else {
                s["nativeKeyboardService"] == false
            }
        });
    }
    host
}
pub fn emit(host: &Bridge, code: u32, modifiers: u32, fields: Value) -> Value {
    let mut fields = fields.as_object().unwrap().clone();
    fields.insert("code".into(), json!(code));
    fields.insert("modifiers".into(), json!(modifiers));
    host.probe("keyboard-press", Value::Object(fields))["statuses"].clone()
}
pub fn trace(host: &Bridge) -> Vec<Value> {
    fs::read_to_string(host.base.join("keyboard-trace.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
pub fn wait_file(path: &Path) {
    let until = Instant::now() + Duration::from_secs(4);
    while !path.exists() {
        assert!(Instant::now() < until, "Own marker not observed: {path:?}");
        thread::sleep(Duration::from_millis(2));
    }
}
pub fn script(id: &str, key: &str, target: &str, args: &[String]) -> Value {
    json!({"id":id,"title":"Own keyboard command","kind":"script","target":target,"arguments":args,"aliases":["keyfixture"],"hotkey":key,"timeout":5})
}
pub fn capture(host: &Bridge, name: &str) -> Value {
    let value = host.probe("screenshot", json!({}));
    assert_eq!(value["windowShown"], false);
    assert_eq!(value["windowKey"], false);
    if let Some(out) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let directory = PathBuf::from(out).parent().unwrap().join("native-keyboard");
        fs::create_dir_all(&directory).unwrap();
        fs::copy(
            host.profile.join("native-probe.png"),
            directory.join(format!("{name}.png")),
        )
        .unwrap();
        fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
    value
}
