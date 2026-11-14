//! Actual hidden native snippet choice window with owned clipboard guard.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Owned host and direct probe share bounded child support"
)]
#[path = "support/child_output.rs"]
mod child_output;
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn invoke(text: &str, steps: Value, extra: Value, use_guard: bool, name: Option<&str>) -> Value {
    let owned = runtime_host::OwnedDir::new();
    let base = &owned.0;
    fs::write(base.join(".ruycast-choice-fixture"), b"own-choice-v1").unwrap();
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_native-snippet-choice-probe"));
    let artifact = std::env::var_os("RUYCAST_TEST_CHOICE_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            probe
                .parent()
                .unwrap()
                .join("examples/libchoice_fixture.dylib")
        });
    assert!(
        artifact.is_absolute() && artifact.is_file(),
        "build choice-fixture first"
    );
    let shim = base.join("choice-guard.dylib");
    fs::copy(&artifact, &shim).unwrap();
    assert_eq!(
        Sha256::digest(fs::read(&artifact).unwrap()),
        Sha256::digest(fs::read(&shim).unwrap())
    );
    if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        static EVIDENCE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = EVIDENCE.lock().unwrap();
        let folder = PathBuf::from(folder);
        fs::create_dir_all(&folder).unwrap();
        let root = runtime_host::root();
        let sources = [
            "rust-native/tests/fixtures/choice_board_sdk.rs",
            "rust-native/tests/fixtures/choice_monitor_sdk.rs",
            "rust-native/tests/fixtures/choice_fixture.rs",
            "rust-native/Cargo.toml",
            "rust-native/Cargo.lock",
            "rust/Cargo.toml",
            "rust-toolchain.toml",
        ]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                format!("{:x}", Sha256::digest(fs::read(root.join(name)).unwrap())),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
        let (path, digest) = runtime_host::bind_artifact(&artifact, "native-snippet-choice-guard");
        fs::write(folder.join("native-snippet-choice-bindings.json"), serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":{path:digest}})).unwrap()).unwrap();
    }
    let title = extra
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Own snippet");
    let theme = extra.get("theme").and_then(Value::as_str).unwrap_or("dark");
    let mut case = json!({"command":{"id":"own.choice","title":title,"kind":"snippet","target":text,"keyword":";own","aliases":[],"arguments":[]},"steps":steps,"theme":theme});
    for (k, v) in extra.as_object().unwrap() {
        if k != "title" && k != "theme" {
            case[k] = v.clone();
        }
    }
    let mut command = Command::new(probe);
    command
        .arg(base)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars_os() {
        let text = key.to_string_lossy();
        if text.starts_with("RUYCAST_")
            || text.starts_with("RUSTCAST_")
            || text == "DYLD_INSERT_LIBRARIES"
        {
            command.env_remove(key);
        }
    }
    if use_guard {
        command.env("DYLD_INSERT_LIBRARIES", &shim);
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&case).unwrap())
        .unwrap();
    let output = child_output::wait(child, Duration::from_secs(15));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    if !use_guard {
        assert_eq!(value["ok"], false);
        return value;
    }
    if extra.get("dropDuring").is_some() {
        assert_eq!(value["ok"], true);
        assert_eq!(value["rootDropped"], true);
        assert_eq!(value["ownerRetired"], true);
        assert_eq!(value["panelRetired"], true);
        assert_eq!(value["events"], 1);
        return value;
    }
    assert_eq!(value["ok"], true, "{value}");
    for field in ["visible", "canBecomeKey", "canBecomeMain"] {
        assert_eq!(value[field], false);
    }
    for field in [
        "prohibited",
        "nonactivating",
        "ownerRetired",
        "generalBoardSubstitute",
    ] {
        assert_eq!(value[field], true);
    }
    if let (Some(name), Some(folder)) = (name, std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR")) {
        let out = PathBuf::from(folder)
            .parent()
            .unwrap()
            .join("native-snippet-choice");
        fs::create_dir_all(&out).unwrap();
        fs::copy(base.join("choice.png"), out.join(format!("{name}.png"))).unwrap();
        fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
    value
}
const DEFAULT: &str = "{field name=\"name\" default=\"Own\"}";
#[test]
fn fixture_requires_complete_guard() {
    let v = invoke(DEFAULT, json!([]), json!({}), false, None);
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("Complete owned clipboard substitute"));
}
#[test]
fn required_field_does_not_read_clipboard_until_eligible() {
    let v = invoke(
        "{name} {clipboard}",
        json!([{"op":"key","code":36}]),
        json!({"clipboard":"OWN CLIP"}),
        true,
        None,
    );
    assert_eq!(v["clipboardReads"], 0);
    assert_eq!(v["events"], json!([]));
    assert_eq!(v["live"], true);
    let v = invoke(
        "{name} {clipboard}",
        json!([{"op":"set","field":0,"value":"Own"},{"op":"key","code":36}]),
        json!({"clipboard":"OWN CLIP"}),
        true,
        None,
    );
    assert_eq!(v["clipboardReads"], 1);
    assert_eq!(v["events"], json!([{"submit":[["name","Own"]]}]));
    assert_eq!(v["live"], false);
}
#[test]
fn real_buttons_are_single_use_with_callback_reentry() {
    let v = invoke(
        DEFAULT,
        json!([{"op":"insert"},{"op":"insert"},{"op":"cancel"}]),
        json!({"reenter":true}),
        true,
        None,
    );
    assert_eq!(v["events"], json!([{"submit":[["name","Own"]]}]));
    assert_eq!(v["failed"], false);
}
#[test]
fn cancel_and_external_hide_have_distinct_restore_policy() {
    for (step, restore) in [
        (json!({"op":"cancel"}), true),
        (json!({"op":"key","code":53}), true),
        (json!({"op":"hide"}), false),
        (json!({"op":"resign"}), false),
    ] {
        let v = invoke(
            DEFAULT,
            json!([step,{"op":"insert"}]),
            json!({}),
            true,
            None,
        );
        assert_eq!(v["events"], json!([{"cancel":true,"restore":restore}]));
    }
}
#[test]
fn field_controls_buttons_and_hidden_renders() {
    for theme in ["light", "dark"] {
        let v = invoke(
            "{field name=\"name\" default=\"Own\"} {field name=\"kind\" options=\"One,Two\"}",
            json!([{"op":"render"},{"op":"show"}]),
            json!({"theme":theme}),
            true,
            Some(&format!("fields-{theme}")),
        );
        assert_eq!(v["width"].as_f64().unwrap(), 456.0);
        assert_eq!(v["height"].as_f64().unwrap(), 270.0);
        assert_eq!(
            v["controls"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["label"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["name", "kind"]
        );
        assert_eq!(v["steps"], json!([{"show":false}]));
    }
}
#[test]
fn repeat_modifiers_and_marked_text_do_not_submit() {
    for step in [
        json!({"op":"key","code":36,"repeat":true}),
        json!({"op":"route","code":36,"flags":1<<20}),
    ] {
        let v = invoke(DEFAULT, json!([step]), json!({}), true, None);
        assert_eq!(v["events"], json!([]));
    }
    let v = invoke(
        DEFAULT,
        json!([{"op":"focus","field":0},{"op":"marked"},{"op":"equivalent","code":36}]),
        json!({}),
        true,
        None,
    );
    assert_eq!(v["events"], json!([]));
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap(),
        &json!({"handled":false})
    );
}
#[test]
fn option_selection_and_arrow_clamping_without_os_menu() {
    let v = invoke(
        "{field name=\"kind\" options=\"One,Two,Three\"}",
        json!([{"op":"choose","field":0,"index":1},{"op":"route","code":125},{"op":"route","code":125},{"op":"insert"}]),
        json!({}),
        true,
        None,
    );
    assert_eq!(v["events"], json!([{"submit":[["kind","Three"]]}]));
}
#[test]
fn callback_panic_fails_closed() {
    let v = invoke(
        DEFAULT,
        json!([{"op":"insert"},{"op":"insert"}]),
        json!({"panic":true}),
        true,
        None,
    );
    assert_eq!(v["failed"], true);
    assert_eq!(v["live"], false);
    assert_eq!(v["events"].as_array().unwrap().len(), 1);
}
#[test]
fn undo_redo_and_leaving_editor_submit_actual_values() {
    for (operation, expected, leave) in [
        (json!({"op":"undo"}), "Own", false),
        (json!({"op":"edit-key"}), "Own", false),
        (json!({"op":"edit-key","redo":true}), "OwnX", false),
        (json!({"op":"undo"}), "Own", true),
    ] {
        let mut steps = vec![
            json!({"op":"focus","field":0}),
            json!({"op":"type","value":"X"}),
        ];
        if operation["redo"] == true {
            steps.push(json!({"op":"undo"}));
        }
        steps.push(operation);
        if leave {
            steps.push(json!({"op":"focus","field":1}));
        }
        steps.push(json!({"op":"insert"}));
        let v = invoke(
            "{field name=\"name\" default=\"Own\"} {other default=\"Other\"}",
            json!(steps),
            json!({}),
            true,
            None,
        );
        assert_eq!(v["events"][0]["submit"][0], json!(["name", expected]));
        assert_eq!(v["steps"][1]["canUndo"], true);
    }
}
#[test]
fn popup_tracking_is_contained_in_fixture() {
    let v = invoke(
        "{field name=\"kind\" options=\"One,Two\"}",
        json!([{"op":"popup-track"}]),
        json!({}),
        true,
        None,
    );
    assert_eq!(v["events"], json!([]));
    assert!(v["controls"][0]["blockedTracking"].as_u64().unwrap() > 0);
}
#[test]
fn callback_drops_last_window_during_actual_native_lifecycle() {
    for phase in ["close", "hide", "resign"] {
        invoke(DEFAULT, json!([]), json!({"dropDuring":phase}), true, None);
    }
}
#[test]
fn scrollable_long_fields_and_two_line_title() {
    let mut text=String::from("{field name=\"A longer label that needs more than one line at the available field width to remain legible\" default=\"Own\"} ");
    text.push_str(
        &(0..8)
            .map(|i| format!("{{field name=\"item{i}\" default=\"own{i}\"}}"))
            .collect::<Vec<_>>()
            .join(" "),
    );
    for theme in ["light", "dark"] {
        let v = invoke(
            &text,
            json!([{"op":"render"}]),
            json!({"theme":theme,"title":"A longer snippet title that uses two lines while preserving the form and its actions"}),
            true,
            Some(&format!("long-{theme}")),
        );
        assert_eq!(v["height"].as_f64().unwrap(), 440.0);
        assert_eq!(v["controls"].as_array().unwrap().len(), 9);
    }
}
#[test]
fn canonical_default_representation_is_preserved_on_submit() {
    let v = invoke(
        "{field name=\"name\" options=\"é,Other\" default=\"é\"}",
        json!([{"op":"insert"}]),
        json!({}),
        true,
        None,
    );
    assert_eq!(v["events"], json!([{"submit":[["name","é"]]}]));
}
#[test]
fn undo_then_real_tab_keeps_restored_value() {
    let v = invoke(
        "{field name=\"name\" default=\"Own\"} {other}",
        json!([{"op":"focus","field":0},{"op":"type","value":"X"},{"op":"undo"},{"op":"key","code":48},{"op":"set","field":1,"value":"Other"},{"op":"insert"}]),
        json!({}),
        true,
        None,
    );
    assert_eq!(v["events"][0]["submit"][0], json!(["name", "Own"]));
}
