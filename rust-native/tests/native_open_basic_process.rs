//! Real offscreen native activation with the owned synthetic Workspace SDK.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::{
    native_bridge::{
        runtime_host::{default_config, Config},
        Bridge,
    },
    *,
};
use serde_json::{json, Value};
use std::{
    fs,
    time::{Duration, Instant},
};

fn host(command: Value) -> Bridge {
    host_with_apps(command, false, false, None)
}
fn host_with_picker(command: Value, picker: bool) -> Bridge {
    host_with_apps(command, picker, false, None)
}
fn host_with_apps(
    command: Value,
    picker: bool,
    discovered: bool,
    clipboard: Option<&str>,
) -> Bridge {
    host_with_deadline(command, picker, discovered, clipboard, None)
}
fn host_with_deadline(
    command: Value,
    picker: bool,
    discovered: bool,
    clipboard: Option<&str>,
    deadline_ms: Option<u32>,
) -> Bridge {
    host_with_width(
        command,
        picker,
        discovered,
        clipboard,
        deadline_ms,
        760,
        None,
    )
}
fn host_with_width(
    command: Value,
    picker: bool,
    discovered: bool,
    clipboard: Option<&str>,
    deadline_ms: Option<u32>,
    width: u32,
    theme: Option<&str>,
) -> Bridge {
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
    assert!(shim.is_absolute() && shim.is_file());
    let mut h = Bridge::new(settings, false);
    if let Some(deadline) = deadline_ms {
        h.env.insert(
            "RUYCAST_NATIVE_TEST_OPEN_DEADLINE_MS".into(),
            deadline.to_string().into(),
        );
    }
    if picker {
        let contents = h.applications.join("Picker.app/Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("Info.plist"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.fixture.success</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleName</key><string>Fixture Selected Browser</string></dict></plist>").unwrap();
    }
    if discovered {
        let contents = h.applications.join("Fixture.app/Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("Info.plist"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.fixture.discovered</string><key>CFBundleName</key><string>Discovered Native Fixture</string></dict></plist>").unwrap();
    }
    let mut cfg = default_config();
    if let Some(theme) = theme {
        cfg["theme"] = json!(theme);
    }
    cfg["commands"].as_array_mut().unwrap().push(command);
    h.replace(&cfg, None);
    h.env
        .insert("RUYCAST_NATIVE_TEST_WIDTH".into(), width.to_string().into());
    h.env
        .insert("DYLD_INSERT_LIBRARIES".into(), shim.into_os_string());
    let fixture_home = h.base.as_os_str().to_owned();
    h.env
        .insert("RUYCAST_WORKSPACE_FIXTURE_HOME".into(), fixture_home);
    h.env
        .insert("RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD".into(), "trace".into());
    h.env
        .insert("RUYCAST_NATIVE_TEST_REQUIRE_HIDE".into(), "1".into());
    if let Some(value) = clipboard {
        h.env
            .insert("RUYCAST_NATIVE_TEST_CLIPBOARD".into(), "named".into());
        h.env
            .insert("RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT".into(), value.into());
    }
    h.start();
    h.settled(|_| true);
    if picker || discovered {
        h.wait(|s| s["appDiscoveryPending"] == false);
    }
    h.env.remove(std::ffi::OsStr::new("DYLD_INSERT_LIBRARIES"));
    h.env
        .remove(std::ffi::OsStr::new("RUYCAST_WORKSPACE_FIXTURE_HOME"));
    h
}
#[test]
fn post_effect_storage_failure_and_ranking_warning_are_visible() {
    for file in ["usage.json", "search-ranking.json"] {
        let mut cmd = command("url", "ruycast-fixture://hold-sync");
        cmd["timeout"] = json!(3);
        let h = host_with_width(cmd, false, false, None, None, 600, None);
        let gate = Release(h.base.join("workspace-release"));
        show(&h, "nativeopen");
        key(&h, 36, 0, "\r", false);
        wait_open_started(&h);
        assert_eq!(
            h.rpc(&json!({"op":"status"}))["result"]["openRequestsRunning"],
            1
        );
        let blocked = h.profile.join(file);
        if blocked.exists() {
            fs::remove_file(&blocked).unwrap();
        }
        fs::create_dir(&blocked).unwrap();
        fs::write(&gate.0, "release").unwrap();
        done(&h);
        h.settled(|_| true);
        assert_eq!(
            h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
            true
        );
        let view = capture(&h, &format!("{file}-warning"));
        let text = view["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("usage could not be saved")
                || text.contains("Opened, but recent choices could not be saved"),
            "{text}"
        );
        assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
    }
}
#[test]
fn missing_application_and_template_validation_keep_action_uncommitted() {
    for theme in ["light", "dark"] {
        let h = host_with_width(
            command("app", "dev.fixture.missing"),
            false,
            false,
            None,
            None,
            760,
            Some(theme),
        );
        show(&h, "nativeopen");
        key(&h, 36, 0, "\r", false);
        h.wait(|s| s["openRequestsRunning"] == 0 && s["nativeUI"]["executionRunning"] != true);
        let view = capture(&h, theme);
        let text = view["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("App is not installed:"));
        assert!(rows(&h.profile.join("native-hide.jsonl")).is_empty());
        assert!(rows(&h.base.join("workspace-trace.jsonl"))
            .iter()
            .all(|r| !r["phase"].as_str().unwrap().starts_with("open-")));
    }
}
#[test]
fn expiry_cancel_and_config_change_before_hide_never_open() {
    for reason in ["expiry", "cancel", "config"] {
        let mut cmd = command("app", "dev.fixture.hold-resolve");
        cmd["timeout"] = json!(1);
        let h = host_with_deadline(
            cmd,
            false,
            false,
            None,
            if reason == "expiry" { Some(250) } else { None },
        );
        let gate = Release(h.base.join("workspace-release"));
        show(&h, "nativeopen");
        key(&h, 36, 0, "\r", false);
        let marker = h.base.join("resolve-entered");
        let end = Instant::now() + Duration::from_secs(4);
        while !marker.exists() {
            assert!(Instant::now() < end, "own fixture did not enter resolve");
            std::thread::sleep(Duration::from_millis(10));
        }
        if reason == "cancel" {
            key(&h, 53, 0, "", false);
        } else if reason == "config" {
            let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
            cfg["commands"].as_array_mut().unwrap().last_mut().unwrap()["target"] =
                json!("dev.fixture.changed");
            h.replace(&cfg, None);
            h.cli(&["reload"]);
        }
        h.wait(|s| s["openRequestsRunning"] == 0 && s["nativeUI"]["executionRunning"] != true);
        fs::write(&gate.0, "release").unwrap();
        h.wait(|s| s["liveExecutionWorkers"] == 0);
        assert!(rows(&h.base.join("workspace-trace.jsonl"))
            .iter()
            .all(|r| !r["phase"].as_str().unwrap().starts_with("open-")));
        assert!(rows(&h.profile.join("native-hide.jsonl")).is_empty());
    }
}
#[test]
fn native_picker_save_then_run_uses_selected_bundle_with_exact_hide() {
    let h = host_with_picker(command("url", "https://example.invalid/picker"), true);
    show(&h, "nativeopen");
    action(&h, "Edit");
    view(&h, |v| v["formVisible"] == true);
    h.settled(|_| true);
    h.probe("app-picker", json!({"command":"open"}));
    h.probe("app-picker", json!({"command":"click","index":1}));
    probe(&h, "save-editor");
    saved(&h);
    let cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    assert_eq!(
        cfg["commands"].as_array().unwrap().last().unwrap()["applicationID"],
        "dev.fixture.success"
    );
    activate(&h);
    let trace = rows(&h.base.join("workspace-trace.jsonl"));
    let opens = trace
        .iter()
        .filter(|r| r["phase"].as_str().unwrap().starts_with("open-"))
        .collect::<Vec<_>>();
    assert_eq!(
        opens.iter().map(|r| r["phase"].clone()).collect::<Vec<_>>(),
        vec![json!("open-with-targets"), json!("open-with")]
    );
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
    assert_eq!(
        trace
            .iter()
            .find(|r| r["phase"] == "open-with-targets")
            .unwrap()["urls"],
        json!(["https://example.invalid/picker"])
    );
    assert!(trace
        .iter()
        .any(|r| r["phase"] == "resolve" && r["bundleID"] == "dev.fixture.success"));
}
fn command(kind: &str, target: &str) -> Value {
    json!({"id":"fixture.open","title":"Native Open Fixture","kind":kind,"target":target,"aliases":["nativeopen"]})
}
fn rows(path: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn template_fields(h: &Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        h.settled(|_| true);
        let value = h.probe("template-read", json!({}));
        if predicate(&value) {
            return value;
        }
        assert!(
            Instant::now() < end,
            "template fields did not settle: {value}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn template_key(h: &Bridge, code: u32, chars: &str) {
    key(h, code, 0, chars, false);
    h.settled(|_| true);
}
fn template_key_flags(h: &Bridge, code: u32, chars: &str, flags: u64) {
    key(h, code, flags, chars, false);
    h.settled(|_| true);
}
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (
                (bytes[index + 1] as char).to_digit(16),
                (bytes[index + 2] as char).to_digit(16),
            ) {
                decoded.push((a * 16 + b) as u8);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}
fn opens(h: &Bridge) -> Vec<Value> {
    rows(&h.base.join("workspace-trace.jsonl"))
        .into_iter()
        .filter(|r| r["phase"].as_str().unwrap().starts_with("open-"))
        .collect()
}
#[test]
fn real_tab_and_backtab_cross_readonly_choice_without_focus_forcing() {
    let h = host(command("url", "https://example.invalid/{a default=\"A\"}/{b default=\"One\" options=\"One,Two\"}/{c default=\"C\"}"));
    template_form(&h);
    h.probe("template-type", json!({"index":0,"text":"First"}));
    template_key(&h, 48, "\t");
    template_key(&h, 48, "\t");
    template_key(&h, 7, "x");
    assert_eq!(
        template_fields(&h, |_| true)["values"],
        json!(["First", "One", "x"])
    );
    template_key_flags(&h, 48, "\u{19}", 1 << 17);
    template_key_flags(&h, 48, "\u{19}", 1 << 17);
    template_key(&h, 12, "q");
    assert_eq!(
        template_fields(&h, |_| true)["values"],
        json!(["q", "One", "x"])
    );
    template_key(&h, 36, "\r");
    done(&h);
    assert_eq!(
        unquote(opens(&h)[0]["url"].as_str().unwrap()),
        "https://example.invalid/q/One/x"
    );
}
#[test]
fn actual_local_option_mouse_click_commits_once_and_restores_form() {
    let h = host(command(
        "url",
        "https://example.invalid/{a default=\"A\"}/{b default=\"One\" options=\"One,Two\"}",
    ));
    template_form(&h);
    template_key(&h, 48, "\t");
    template_key(&h, 49, " ");
    h.probe("template-choice", json!({"command":"click","index":1}));
    h.settled(|_| true);
    let v = template_fields(&h, |_| true);
    assert_eq!(v["choice"]["open"], false);
    assert_eq!(v["values"], json!(["A", "Two"]));
    assert_eq!(v["fieldsAXHidden"], false);
    assert_eq!(probe(&h, "read")["saveEnabled"], true);
    assert!(opens(&h).is_empty());
    template_key_flags(&h, 36, "\r", CMD);
    done(&h);
    let result = opens(&h);
    assert_eq!(result.len(), 1);
    assert_eq!(
        unquote(result[0]["url"].as_str().unwrap()),
        "https://example.invalid/A/Two"
    );
}
fn template_form(h: &Bridge) {
    show(h, "nativeopen");
    key(h, 36, 0, "\r", false);
    template_fields(h, |v| !v["fields"].is_null());
}
#[test]
fn many_fields_are_virtualized_and_last_value_remains_reachable() {
    let target = format!(
        "https://example.invalid/{}",
        (0..5000)
            .map(|i| format!("{{f{i} default=\"x\"}}"))
            .collect::<Vec<_>>()
            .join("/")
    );
    let h = host(command("url", &target));
    template_form(&h);
    h.probe("template-type", json!({"index":4999,"text":"last-value"}));
    let v = template_fields(&h, |_| true);
    assert_eq!(v["fields"], 5000);
    assert_eq!(
        v["values"].as_array().unwrap().last().unwrap(),
        "last-value"
    );
    assert!(v["liveRows"].as_u64().unwrap() <= 32);
    probe(&h, "save-editor");
    done(&h);
    assert!(unquote(opens(&h)[0]["url"].as_str().unwrap()).ends_with("last-value"));
}
#[test]
fn long_choice_and_label_render_with_complete_accessibility_names() {
    let label = format!("Place {}", "long label ".repeat(12));
    let value = format!("Long option {}", "Ω value ".repeat(30))
        .trim_end()
        .to_owned();
    let target = format!("https://example.invalid/{{a default=\"fixed\"}}/{{b name=\"{label}\" default=\"{value}\" options=\"{value},Other\"}}");
    for theme in ["light", "dark"] {
        for width in [600, 760] {
            let h = host_with_width(
                command("url", &target),
                false,
                false,
                None,
                None,
                width,
                Some(theme),
            );
            template_form(&h);
            let v = capture(&h, &format!("choice-long-{theme}-{width}"));
            assert!(v["accessibilityText"]
                .as_array()
                .unwrap()
                .contains(&json!(label)));
            template_key(&h, 48, "\t");
            template_key(&h, 49, " ");
            let v = capture(&h, &format!("choice-long-open-{theme}-{width}"));
            assert!(v["accessibilityText"]
                .as_array()
                .unwrap()
                .contains(&json!(value)));
            template_key(&h, 53, "");
            assert_eq!(template_fields(&h, |_| true)["values"][1], value);
        }
    }
}
fn large_choices() -> String {
    (0..10_000)
        .map(|i| format!("Choice{i}"))
        .collect::<Vec<_>>()
        .join(",")
}
#[test]
fn open_large_choice_virtualizes_last_option_and_retains_exact_value() {
    let target = format!("https://example.invalid/{{a default=\"x\"}}/{{b name=\"Place\" default=\"Choice0\" options=\"{}\"}}", large_choices());
    let h = host(command("url", &target));
    template_form(&h);
    template_key(&h, 48, "\t");
    template_key(&h, 49, " ");
    let v = template_fields(&h, |_| true);
    assert_eq!(v["choice"]["rows"], 10_000);
    assert!(v["choice"]["visibleRows"].as_u64().unwrap() <= 32);
    assert!(v["choice"]["reads"].as_u64().unwrap() < 64);
    h.probe("template-choice", json!({"command":"select","index":9999}));
    h.settled(|_| true);
    let v = template_fields(&h, |_| true);
    assert_eq!(v["values"][1], "Choice0");
    assert!(v["choice"]["reads"].as_u64().unwrap() < 128);
    capture(&h, "choice-last");
    template_key(&h, 36, "\r");
    assert_eq!(template_fields(&h, |_| true)["values"][1], "Choice9999");
    template_key_flags(&h, 36, "\r", CMD);
    done(&h);
    assert!(unquote(opens(&h)[0]["url"].as_str().unwrap()).contains("Choice9999"));
}
#[test]
fn large_choice_datasource_and_canonical_default() {
    let target = format!("https://example.invalid/{{a name=\"Text\" default=\"fixed\"}}/{{b name=\"Mode\" options=\"{}\"}}", large_choices());
    let h = host(command("url", &target));
    template_form(&h);
    let v = template_fields(&h, |_| true);
    assert!(v["optionReads"].as_u64().unwrap() < 64);
    h.probe("template-choose", json!({"index":1,"selected":9999}));
    assert_eq!(template_fields(&h, |_| true)["values"][1], "Choice9999");
    probe(&h, "save-editor");
    done(&h);
    assert!(opens(&h)[0]["url"].as_str().unwrap().contains("Choice9999"));
}
#[test]
fn single_query_field_inline_and_literal_alias_tail() {
    for query in ["nativeopen", "nativeopen literal&value"] {
        let h = host(command("url", "https://example.invalid/{query}"));
        assert_eq!(h.rpc(&json!({"op":"show","query":query}))["ok"], true);
        h.settled(|s| s["nativeUI"]["visible"] == true);
        if query == "nativeopen" {
            key(&h, 36, 0, "\r", false);
        }
        let end = Instant::now() + Duration::from_secs(5);
        while probe(&h, "read")["runInputInline"] != true {
            assert!(Instant::now() < end, "inline input did not open");
            h.settled(|_| true);
            std::thread::sleep(Duration::from_millis(10));
        }
        if query == "nativeopen" {
            h.probe("run-inline-type", json!({"text":"literal&value"}));
        }
        key(&h, 36, 0, "\r", false);
        done(&h);
        assert!(opens(&h)[0]["url"]
            .as_str()
            .unwrap()
            .contains("literal%26value"));
    }
}
#[test]
fn multi_field_text_and_lazy_choice_open_exact_rendered_url() {
    let h = host(command("url", "https://example.invalid/{first name=\"Message\"}/{second name=\"Country\" options=\"One,Two\"}"));
    let before = fs::read(&h.config).unwrap();
    show(&h, "nativeopen");
    key(&h, 36, 0, "\r", false);
    let v = template_fields(&h, |v| v["fields"] == 2);
    assert_eq!(v["values"], json!(["", ""]));
    h.probe("template-type", json!({"index":0,"text":"a&b Ω"}));
    h.probe("template-choose", json!({"index":1,"selected":1}));
    assert_eq!(
        template_fields(&h, |_| true)["values"],
        json!(["a&b Ω", "Two"])
    );
    capture(&h, "template-basic");
    assert!(probe(&h, "read")["accessibilityText"]
        .as_array()
        .unwrap()
        .contains(&json!("Message")));
    assert!(probe(&h, "read")["accessibilityText"]
        .as_array()
        .unwrap()
        .contains(&json!("Country")));
    probe(&h, "save-editor");
    done(&h);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
    let results = opens(&h);
    let url = results[0]["url"].as_str().unwrap();
    assert!(url.contains("a%26b"));
    assert!(url.contains("Two"));
}
#[test]
fn actual_undo_then_run_uses_current_field_editor_and_no_secret_logs() {
    let h = host(command(
        "url",
        "https://example.invalid/{a name=\"Text\"}/{b name=\"Other\" default=\"fixed\"}",
    ));
    template_form(&h);
    h.probe("template-type", json!({"index":0,"text":"PRIVATE-first"}));
    h.settled(|_| true);
    h.probe("template-type", json!({"index":0,"text":"PRIVATE-second"}));
    h.settled(|_| true);
    key(&h, 6, CMD, "z", false);
    h.settled(|_| true);
    capture(&h, "template-undo");
    probe(&h, "save-editor");
    done(&h);
    let target = unquote(opens(&h)[0]["url"].as_str().unwrap());
    assert!(target.contains("PRIVATE-first"));
    assert!(!target.contains("PRIVATE-second"));
    let raw = serde_json::to_string(&h.rpc(&json!({"op":"logs"}))).unwrap();
    assert!(!raw.contains("PRIVATE-first"));
    assert!(!raw.contains("PRIVATE-second"));
}
#[test]
fn old_callbacks_cannot_save_or_cancel_an_alias_editor() {
    let h = host(command("url", "https://example.invalid/{a}/{b}"));
    template_form(&h);
    h.probe("template-type", json!({"index":0,"text":"old secret"}));
    h.probe("template-retain", json!({"index":0}));
    probe(&h, "cancel-editor");
    h.settled(|_| true);
    show(&h, "nativeopen");
    key(&h, 40, CMD, "k", false);
    h.settled(|_| true);
    let labels = probe(&h, "read")["actionLabels"]
        .as_array()
        .unwrap()
        .clone();
    let index = labels
        .iter()
        .position(|v| v.as_str().unwrap().starts_with("Set Alias"))
        .unwrap();
    h.probe("action-button", json!({"index":index}));
    h.settled(|_| true);
    h.probe("alias-input", json!({"text":"UNSAVED-ALIAS"}));
    let before = fs::read(&h.config).unwrap();
    h.probe("template-stale", json!({}));
    h.settled(|_| true);
    let view = probe(&h, "read");
    assert_eq!(view["formVisible"], true);
    assert_eq!(view["aliasValue"], "UNSAVED-ALIAS");
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert!(opens(&h).is_empty());
}
#[test]
fn config_replacement_rejects_run_without_losing_main_draft() {
    let h = host(command(
        "url",
        "https://example.invalid/{a name=\"Text\"}/{b name=\"Other\"}",
    ));
    template_form(&h);
    h.probe("template-type", json!({"index":0,"text":"keep draft"}));
    h.probe("template-type", json!({"index":1,"text":"second"}));
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    cfg["commands"].as_array_mut().unwrap().last_mut().unwrap()["target"] =
        json!("https://example.invalid/changed/{a}/{b}");
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    h.settled(|_| true);
    probe(&h, "save-editor");
    h.settled(|_| true);
    assert_eq!(
        template_fields(&h, |_| true)["values"],
        json!(["keep draft", "second"])
    );
    assert_eq!(template_fields(&h, |_| true)["enabled"], true);
    assert!(opens(&h).is_empty());
    let view = probe(&h, "read");
    let text = view["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Command changed"));
}
#[test]
fn old_option_table_cannot_commit_after_same_form_reopen_or_cancel() {
    let h = host(command(
        "url",
        "https://example.invalid/{a default=\"x\"}/{b default=\"One\" options=\"One,Two\"}",
    ));
    template_form(&h);
    template_key(&h, 48, "\t");
    template_key(&h, 49, " ");
    let old = template_fields(&h, |_| true);
    h.probe("template-choice", json!({"command":"retain"}));
    template_key(&h, 53, "");
    template_key(&h, 49, " ");
    let before = template_fields(&h, |_| true);
    assert!(
        before["choice"]["generation"].as_u64().unwrap()
            > old["choice"]["generation"].as_u64().unwrap()
    );
    h.probe("template-choice", json!({"command":"stale","index":1}));
    let after = template_fields(&h, |_| true);
    assert_eq!(after["choice"]["open"], true);
    assert_eq!(after["values"], before["values"]);
    assert_eq!(
        after["choice"]["generation"],
        before["choice"]["generation"]
    );
    probe(&h, "cancel-editor");
    h.settled(|_| true);
    h.probe("template-choice", json!({"command":"stale","index":1}));
    assert_eq!(template_fields(&h, |_| true)["values"], json!([]));
    assert!(opens(&h).is_empty());
}
#[test]
fn cancel_wipes_values_and_old_callbacks_cannot_operate_new_form() {
    let h = host(command(
        "url",
        "https://example.invalid/{a name=\"First\"}/{b name=\"Second\"}",
    ));
    template_form(&h);
    h.probe(
        "template-type",
        json!({"index":0,"text":"PRIVATE-OLD-VALUE"}),
    );
    h.probe("template-retain", json!({"index":0}));
    probe(&h, "cancel-editor");
    h.settled(|_| true);
    assert_eq!(h.probe("template-read", json!({}))["values"], json!([]));
    let old = h.probe("template-stale", json!({}));
    assert_eq!(old["oldValue"], "");
    assert_eq!(old["oldToken"], 0);
    template_form(&h);
    h.probe("template-type", json!({"index":0,"text":"new value"}));
    let before = h.probe("template-read", json!({}));
    h.probe("template-stale", json!({}));
    let after = h.probe("template-read", json!({}));
    assert_eq!(after["token"], before["token"]);
    assert_eq!(after["values"], before["values"]);
    assert!(opens(&h).is_empty());
}
#[test]
fn marked_input_defers_choice_and_unrelated_reload_preserves_open_list() {
    let h = host(command(
        "url",
        "https://example.invalid/{a}/{b options=\"One,Two\"}",
    ));
    template_form(&h);
    h.probe("template-mark", json!({"index":0,"text":"に"}));
    h.probe("template-choice", json!({"command":"open","index":1}));
    let v = template_fields(&h, |_| true);
    assert_eq!(v["marked"], true);
    assert_eq!(v["choice"]["open"], false);
    h.probe("template-unmark", json!({}));
    h.probe("template-choice", json!({"command":"open","index":1}));
    let before = template_fields(&h, |_| true);
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    cfg["theme"] = json!("light");
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    h.settled(|_| true);
    let after = template_fields(&h, |_| true);
    assert_eq!(after["choice"]["open"], true);
    assert_eq!(
        after["choice"]["generation"],
        before["choice"]["generation"]
    );
    assert_eq!(after["values"], before["values"]);
    assert_eq!(after["fieldsAXHidden"], true);
    assert!(after["fieldControlsEnabled"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v == false));
    template_key(&h, 53, "");
    assert_eq!(template_fields(&h, |_| true)["fieldsAXHidden"], false);
}
#[test]
fn marked_text_and_unrelated_reload_preserve_live_native_editor() {
    let h = host(command(
        "url",
        "https://example.invalid/{a name=\"Text\"}/{b name=\"Other\" default=\"fixed\"}",
    ));
    template_form(&h);
    h.probe("template-mark", json!({"index":0,"text":"日本語"}));
    let before = h.probe("template-read", json!({}));
    assert_eq!(before["marked"], true);
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    cfg["theme"] = json!("light");
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    h.settled(|_| true);
    let after = h.probe("template-read", json!({}));
    assert_eq!(after["marked"], true);
    assert_eq!(after["token"], before["token"]);
    probe(&h, "save-editor");
    h.settled(|_| true);
    assert!(opens(&h).is_empty());
    h.probe("template-unmark", json!({}));
    h.probe("template-type", json!({"index":0,"text":"日本語"}));
    probe(&h, "save-editor");
    done(&h);
    assert!(unquote(opens(&h)[0]["url"].as_str().unwrap()).contains("日本語"));
}
#[test]
fn empty_choice_never_silently_commits_first_option() {
    let h = host(command(
        "url",
        "https://example.invalid/{a}/{b name=\"Place\" options=\"One,Two\"}",
    ));
    template_form(&h);
    template_key(&h, 48, "\t");
    template_key(&h, 49, " ");
    assert_eq!(template_fields(&h, |_| true)["choice"]["selected"], -1);
    template_key(&h, 36, "\r");
    let v = template_fields(&h, |_| true);
    assert_eq!(v["choice"]["open"], true);
    assert_eq!(v["values"][1], "");
    template_key(&h, 125, "");
    assert_eq!(template_fields(&h, |_| true)["values"][1], "");
    template_key(&h, 36, "\r");
    assert_eq!(template_fields(&h, |_| true)["values"][1], "One");
}
#[test]
fn native_list_type_selection_remains_tentative_until_return() {
    let h = host(command("url", "https://example.invalid/{a default=\"A\"}/{b default=\"Alpha\" options=\"Alpha,Beta,Gamma\"}"));
    template_form(&h);
    template_key(&h, 48, "\t");
    template_key(&h, 49, " ");
    template_key(&h, 11, "b");
    let v = template_fields(&h, |_| true);
    assert_eq!(v["choice"]["open"], true);
    assert_eq!(v["choice"]["selected"], 1);
    assert_eq!(v["values"][1], "Alpha");
    template_key(&h, 36, "\r");
    assert_eq!(template_fields(&h, |_| true)["values"][1], "Beta");
}
#[test]
fn contained_choice_keys_tentative_cancel_commit_and_native_ax_modality() {
    let h = host(command("url", "https://example.invalid/{a default=\"A\"}/{b name=\"Place\" default=\"One\" options=\"One,Two\"}/{c default=\"C\"}"));
    template_form(&h);
    template_key(&h, 48, "\t");
    template_key(&h, 49, " ");
    let v = template_fields(&h, |_| true);
    assert_eq!(v["choice"]["open"], true);
    assert_eq!(v["fieldsAXHidden"], true);
    assert!(v["fieldControlsEnabled"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v == false));
    assert_eq!(probe(&h, "read")["saveEnabled"], false);
    template_key(&h, 125, "");
    let v = template_fields(&h, |_| true);
    assert_eq!(v["choice"]["selected"], 1);
    assert_eq!(v["values"][1], "One");
    capture(&h, "choice-open");
    template_key(&h, 7, "x");
    template_key_flags(&h, 36, "\r", CMD);
    probe(&h, "save-editor");
    h.settled(|_| true);
    assert_eq!(template_fields(&h, |_| true)["choice"]["open"], true);
    assert_eq!(
        template_fields(&h, |_| true)["values"],
        json!(["A", "One", "C"])
    );
    assert!(opens(&h).is_empty());
    template_key(&h, 53, "");
    let v = template_fields(&h, |_| true);
    assert_eq!(v["choice"]["open"], false);
    assert_eq!(v["fieldsAXHidden"], false);
    assert_eq!(v["values"][1], "One");
    assert!(v["fieldControlsEnabled"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v == true));
    assert_eq!(probe(&h, "read")["saveEnabled"], true);
    template_key(&h, 49, " ");
    assert_eq!(template_fields(&h, |_| true)["choice"]["open"], true);
    template_key(&h, 125, "");
    template_key(&h, 36, "\r");
    let v = template_fields(&h, |_| true);
    assert_eq!(v["choice"]["open"], false);
    assert_eq!(v["values"][1], "Two");
    template_key(&h, 48, "\t");
    template_key(&h, 7, "x");
    assert_eq!(
        template_fields(&h, |_| true)["values"],
        json!(["A", "Two", "x"])
    );
}
fn capture(h: &Bridge, name: &str) -> Value {
    let view = h.probe("screenshot", json!({}));
    if let Some(out) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let path = std::path::PathBuf::from(out);
        let directory = path.parent().unwrap().join("native-open");
        fs::create_dir_all(&directory).unwrap();
        fs::copy(
            h.profile.join("native-probe.png"),
            directory.join(format!("{name}.png")),
        )
        .unwrap();
        fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&view).unwrap(),
        )
        .unwrap();
    }
    view
}
#[test]
fn failed_callback_and_post_sdk_timeout_surface_the_exact_owned_error() {
    for (kind, target) in [
        ("app", "dev.fixture.error"),
        ("url", "ruycast-fixture://hold-sync"),
    ] {
        let mut cmd = command(kind, target);
        cmd["timeout"] = json!(1);
        let h = host_with_deadline(
            cmd,
            false,
            false,
            None,
            if kind == "url" { Some(250) } else { None },
        );
        let gate = Release(h.base.join("workspace-release"));
        show(&h, "nativeopen");
        key(&h, 36, 0, "\r", false);
        done(&h);
        assert_eq!(
            h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
            true
        );
        h.settled(|_| true);
        let view = capture(
            &h,
            if kind == "url" {
                "timeout"
            } else {
                "callback-error"
            },
        );
        let text = view["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("Fixture native error") || text.contains("Check its outcome"),
            "{text}"
        );
        assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
        assert_eq!(view["windowShown"], false);
        if kind == "url" {
            fs::write(&gate.0, "release").unwrap();
        }
    }
}
fn activate(h: &Bridge) {
    show(h, "nativeopen");
    key(h, 36, 0, "\r", false);
    done(h);
}
struct Release(std::path::PathBuf);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::write(&self.0, "release");
    }
}
struct ReleaseCallbacks<'a> {
    host: &'a Bridge,
    active: bool,
}
impl Drop for ReleaseCallbacks<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.host.probe("workspace-release-callbacks", json!({}));
            }));
        }
    }
}
#[test]
fn gui_workspace_ignores_script_timeout_and_finishes_only_on_sdk_result() {
    for (kind, target) in [
        ("url", "ruycast-fixture://hold-sync"),
        ("app", "dev.fixture.hold"),
    ] {
        let mut cmd = command(kind, target);
        cmd["timeout"] = json!(0.1);
        let h = host(cmd);
        let gate = Release(h.base.join("workspace-release"));
        let mut callbacks = if kind == "app" {
            Some(ReleaseCallbacks {
                host: &h,
                active: true,
            })
        } else {
            None
        };
        show(&h, "nativeopen");
        key(&h, 36, 0, "\r", false);
        let marker = h.base.join("open-started");
        let end = Instant::now() + Duration::from_secs(4);
        while !marker.exists() {
            assert!(
                Instant::now() < end,
                "own fixture did not enter open-started"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(1200));
        let status = h.rpc(&json!({"op":"status"}))["result"].clone();
        assert_eq!(status["openRequestsRunning"], 1);
        assert_eq!(status["scriptsRunning"], 0);
        assert_eq!(status["nativeUI"]["visible"], false);
        assert!(h.cli(&["usage"])["result"].get("fixture.open").is_none());
        assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
        if kind == "app" {
            h.probe("workspace-release-callbacks", json!({}));
            callbacks.as_mut().unwrap().active = false;
        } else {
            fs::write(&gate.0, "release").unwrap();
        }
        h.wait(|s| s["openRequestsRunning"] == 0 && s["nativeUI"]["executionRunning"] != true);
        assert_eq!(h.cli(&["usage"])["result"]["fixture.open"], 1);
        assert_eq!(
            rows(&h.base.join("workspace-trace.jsonl"))
                .iter()
                .filter(|r| r["phase"].as_str().unwrap().starts_with("open-"))
                .count(),
            1
        );
    }
}
#[test]
fn new_show_and_query_revoke_blocked_old_lookup() {
    for action_name in ["show", "query"] {
        let h = host(command("app", "dev.fixture.hold-resolve"));
        let gate = h.base.join("workspace-release");
        let _release = Release(gate.clone());
        show(&h, "nativeopen");
        key(&h, 36, 0, "\r", false);
        let marker = h.base.join("resolve-entered");
        let end = Instant::now() + Duration::from_secs(4);
        while !marker.exists() {
            assert!(Instant::now() < end, "own fixture did not enter resolve");
            std::thread::sleep(Duration::from_millis(10));
        }
        if action_name == "show" {
            show(&h, "nativeopen");
        } else {
            h.probe("query", json!({"query":"new-intent"}));
        }
        fs::write(gate, "release").unwrap();
        h.wait(|s| s["openRequestsRunning"] == 0 && s["nativeUI"]["executionRunning"] != true);
        assert!(rows(&h.base.join("workspace-trace.jsonl"))
            .iter()
            .all(|r| !r["phase"].as_str().unwrap().starts_with("open-")));
        assert!(rows(&h.profile.join("native-hide.jsonl")).is_empty());
        assert!(h.cli(&["usage"])["result"].get("fixture.open").is_none());
    }
}
fn done(h: &Bridge) {
    wait_open_started(h);
    h.wait(|s| s["openRequestsRunning"] == 0 && s["nativeUI"]["executionRunning"] != true);
}
fn wait_open_started(h: &Bridge) {
    let marker = h.base.join("open-started");
    let end = Instant::now() + Duration::from_secs(4);
    while !marker.exists() {
        assert!(
            Instant::now() < end,
            "own fixture did not enter open-started"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn gui_without_wall_timer_still_cancels_on_new_show() {
    let mut cmd = command("url", "ruycast-fixture://hold-sync");
    cmd["timeout"] = json!(0.1);
    let h = host(cmd);
    let gate = Release(h.base.join("workspace-release"));
    show(&h, "nativeopen");
    key(&h, 36, 0, "\r", false);
    wait_open_started(&h);
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["openRequestsRunning"],
        1
    );
    show(&h, "nativeopen");
    done(&h);
    fs::write(&gate.0, "release").unwrap();
    h.wait(|s| s["liveExecutionWorkers"] == 0);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        true
    );
    assert!(!probe(&h, "read")["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        .contains("Check its outcome"));
}
#[test]
fn new_show_after_sdk_entry_cannot_be_hidden_or_receive_old_failure() {
    let mut cmd = command("url", "ruycast-fixture://hold-sync");
    cmd["timeout"] = json!(1);
    let h = host(cmd);
    let gate = Release(h.base.join("workspace-release"));
    show(&h, "nativeopen");
    key(&h, 36, 0, "\r", false);
    wait_open_started(&h);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["openRequestsRunning"],
        1
    );
    show(&h, "nativeopen");
    done(&h);
    fs::write(&gate.0, "release").unwrap();
    h.wait(|s| s["liveExecutionWorkers"] == 0);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        true
    );
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
    assert!(!probe(&h, "read")["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        .contains("Check its outcome"));
}
#[test]
fn discovered_app_is_not_required_to_be_saved_for_usage() {
    let h = host_with_apps(command("app", "dev.fixture.inline"), false, true, None);
    show(&h, "Discovered Native Fixture");
    key(&h, 36, 0, "\r", false);
    done(&h);
    assert_eq!(
        h.cli(&["usage"])["result"]["discovered.dev.fixture.discovered"],
        1
    );
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
}
#[test]
fn url_clipboard_and_literal_path_share_native_admission() {
    let h = host_with_apps(
        command("url", "https://example.invalid/{clipboard}"),
        false,
        false,
        Some("synthetic-Ω"),
    );
    activate(&h);
    let trace = rows(&h.base.join("workspace-trace.jsonl"));
    assert_eq!(
        trace
            .iter()
            .find(|r| r["phase"] == "clipboard-read")
            .unwrap()["mainThread"],
        true
    );
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
    fs::remove_file(h.base.join("open-started")).unwrap();
    let path = h.base.join("own literal Ω");
    fs::write(&path, "own").unwrap();
    let mut cfg: Value = serde_json::from_slice(&fs::read(&h.config).unwrap()).unwrap();
    *cfg["commands"].as_array_mut().unwrap().last_mut().unwrap() =
        command("path", path.to_str().unwrap());
    h.replace(&cfg, None);
    h.cli(&["reload"]);
    activate(&h);
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 2);
    assert_eq!(h.cli(&["usage"])["result"]["fixture.open"], 2);
}
#[test]
fn new_quicklink_prefill_uses_real_named_main_service_without_execution() {
    for (text, expected, name) in [
        (
            "  https://example.invalid/new  ",
            "https://example.invalid/new",
            "value",
        ),
        ("   ", "https://", "empty"),
    ] {
        let h = host_with_apps(
            command("app", "dev.fixture.inline"),
            false,
            false,
            Some(text),
        );
        show(&h, "nativeopen");
        let before = fs::read(&h.config).unwrap();
        action(&h, "New Quicklink");
        view(&h, |v| v["formVisible"] == true);
        let value = h.probe("command-read", json!({}))["value"].clone();
        assert!(value["id"].as_str().unwrap().starts_with("url."));
        assert_eq!(value["target"], expected);
        assert_eq!(
            fs::read_to_string(h.base.join("creation-prefill-read")).unwrap(),
            "own-named-main-thread\n"
        );
        assert_eq!(fs::read(&h.config).unwrap(), before);
        assert!(rows(&h.base.join("workspace-trace.jsonl"))
            .iter()
            .all(|r| !r["phase"].as_str().unwrap().starts_with("open-")));
        assert!(rows(&h.profile.join("native-hide.jsonl")).is_empty());
        assert_eq!(h.cli(&["usage"])["result"], json!({}));
        let screenshot = h.probe("screenshot", json!({}));
        if let Some(out) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
            let path = std::path::PathBuf::from(out);
            let directory = path.parent().unwrap().join("native-open");
            fs::create_dir_all(&directory).unwrap();
            fs::copy(
                h.profile.join("native-probe.png"),
                directory.join(format!("creation-prefill-{name}.png")),
            )
            .unwrap();
            fs::write(
                directory.join(format!("creation-prefill-{name}.json")),
                serde_json::to_vec_pretty(&screenshot).unwrap(),
            )
            .unwrap();
        }
        probe(&h, "cancel-editor");
        saved(&h);
    }
}
#[test]
fn existing_prompt_input_is_validated_and_executes_without_config_write() {
    let mut cmd = command("app", "dev.fixture.inline");
    cmd["inputPrompt"] = json!("Label");
    let h = host(cmd);
    let before = fs::read(&h.config).unwrap();
    show(&h, "nativeopen");
    key(&h, 36, 0, "\r", false);
    h.settled(|_| true);
    assert_eq!(probe(&h, "read")["runInputInline"], true);
    key(&h, 36, CMD, "\r", false);
    h.settled(|_| true);
    h.probe("run-input-type", json!({"text":"own prompt value"}));
    key(&h, 36, CMD, "\r", false);
    done(&h);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
}
#[test]
fn open_with_uses_exact_hide_before_its_native_callback() {
    let mut cmd = command("url", "https://example.invalid/own");
    cmd["applicationID"] = json!("dev.fixture.inline");
    let h = host(cmd);
    activate(&h);
    assert!(rows(&h.base.join("workspace-trace.jsonl"))
        .iter()
        .any(|r| r["phase"] == "open-with"));
    assert_eq!(rows(&h.profile.join("native-hide.jsonl")).len(), 1);
}
#[test]
fn saved_app_hides_exact_panel_before_sdk_and_records_success() {
    let h = host(command("app", "dev.fixture.inline"));
    activate(&h);
    let hides = rows(&h.profile.join("native-hide.jsonl"));
    assert_eq!(hides.len(), 1);
    assert_eq!(hides[0]["mainThread"], true);
    assert_eq!(hides[0]["afterVisible"], false);
    assert_eq!(probe(&h, "read")["windowShown"], false);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        false
    );
    assert_eq!(h.cli(&["usage"])["result"]["fixture.open"], 1);
    assert!(rows(&h.base.join("workspace-trace.jsonl"))
        .iter()
        .filter(|r| r["phase"].as_str().unwrap().starts_with("open-"))
        .all(|r| r["mainThread"] == false));
}
