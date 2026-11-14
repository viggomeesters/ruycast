//! Owned native keyboard scenarios; test SDK prevents global registration.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Direct refusal probes and the owned host share the same bounded child helper"
)]
#[path = "support/child_output.rs"]
mod child_output;
#[allow(dead_code)]
#[path = "support/native_keyboard.rs"]
mod native_keyboard;
use native_keyboard::{
    actions,
    actions::native_bridge::{runtime_host::default_config, Bridge},
    *,
};
use serde_json::{json, Value};
use std::{fs, process::Stdio, time::Duration};

fn normal() -> Bridge {
    host(&[], true, true, true, None)
}
fn press(h: &Bridge) -> Value {
    emit(h, 49, 6144, json!({}))
}
fn config(h: &Bridge) -> Value {
    actions::config(h)
}
fn key_script(target: &str, args: &[String]) -> Value {
    script("script.key", "cmd+a", target, args)
}
fn read(h: &Bridge) -> Value {
    actions::probe(h, "read")
}
fn marker_path(h: &Bridge, name: &str) -> String {
    h.base.join(name).to_str().unwrap().to_owned()
}
fn wait_json(path: &std::path::Path) -> Value {
    let until = std::time::Instant::now() + Duration::from_secs(4);
    loop {
        assert!(
            std::time::Instant::now() < until,
            "Complete owned JSON not observed: {path:?}"
        );
        if let Ok(bytes) = fs::read(path) {
            if let Ok(value) = serde_json::from_slice(&bytes) {
                assert!(
                    std::time::Instant::now() < until,
                    "Complete owned JSON arrived too late: {path:?}"
                );
                return value;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn native_callback_toggles_launcher_and_two_queued_presses_preserve_toggle_order() {
    let h = normal();
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        false
    );
    assert_eq!(press(&h), json!([0]));
    h.settled(|s| s["nativeUI"]["visible"] == true);
    assert_eq!(read(&h)["windowShown"], false);
    press(&h);
    h.settled(|s| s["nativeUI"]["visible"] == false);
    assert_eq!(emit(&h, 49, 6144, json!({"count":2})), json!([0, 0]));
    h.settled(|s| s["nativeUI"]["visible"] == false);
}

#[test]
fn item_callback_uses_exact_command_not_selected_row_and_logs_hotkey_without_showing_palette() {
    let mut h = host(&[], false, true, true, None);
    let marker = marker_path(&h, "own-action");
    let mut config = default_config();
    config["commands"]
        .as_array_mut()
        .unwrap()
        .push(key_script("/usr/bin/touch", std::slice::from_ref(&marker)));
    h.replace(&config, None);
    h.start();
    h.settled(|s| s["keyboardGeneration"].as_u64().unwrap_or(0) > 0);
    assert_ne!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["selectedId"],
        "script.key"
    );
    assert_eq!(emit(&h, 0, 256, json!({})), json!([0]));
    wait_file(std::path::Path::new(&marker));
    h.wait(|s| s["scriptsRunning"] == 0);
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"]["script.key"], 1);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        false
    );
    let logs = h.rpc(&json!({"op":"logs"}));
    assert!(logs["result"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["source"] == "hotkey" && x["commandID"] == "script.key"));
    let analytics = h.rpc(&json!({"op":"analytics"}));
    let result = &analytics["result"];
    assert_eq!(result["totalSuccesses"], 1);
    assert_eq!(
        result["commands"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()["sources"],
        json!({"hotkey":1})
    );
}

#[test]
fn hotkey_snippet_failure_keeps_exact_analytics_source_before_missing_sdk_refusal() {
    let command = json!({"id":"snippet.key","title":"Owned hotkey snippet","kind":"snippet","target":"Owned text","hotkey":"cmd+a"});
    let h = host(&[command], true, true, true, None);
    assert_eq!(emit(&h, 0, 256, json!({})), json!([0]));
    let until = std::time::Instant::now() + Duration::from_secs(3);
    let result = loop {
        let value = h.rpc(&json!({"op":"analytics"}))["result"].clone();
        if value["totalFailures"] == 1 {
            break value;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(result["totalAttempts"], 1);
    assert_eq!(
        result["commands"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()["sources"],
        json!({"hotkey":1})
    );
    assert_eq!(read(&h)["windowShown"], false);
}

#[test]
fn reload_retires_old_ids_and_disabled_bindings() {
    let mut h = host(&[], false, true, true, None);
    let old = marker_path(&h, "old");
    let new = marker_path(&h, "new");
    let mut cfg = default_config();
    cfg["commands"]
        .as_array_mut()
        .unwrap()
        .push(key_script("/usr/bin/touch", std::slice::from_ref(&old)));
    h.replace(&cfg, None);
    h.start();
    h.settled(|s| s["keyboardGeneration"].as_u64().unwrap_or(0) > 0);
    let prior = trace(&h)
        .into_iter()
        .find(|x| x["phase"] == "register" && x["a"] == 0)
        .unwrap()["c"]
        .clone();
    let command = cfg["commands"].as_array_mut().unwrap().last_mut().unwrap();
    command["hotkey"] = json!("cmd+b");
    command["arguments"] = json!([new]);
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    h.settled(|_| true);
    assert_eq!(emit(&h, 49, 6144, json!({"id":prior})), json!([-9874]));
    assert_eq!(emit(&h, 0, 256, json!({})), json!([-9874]));
    assert!(!std::path::Path::new(&old).exists());
    assert!(!std::path::Path::new(&new).exists());
    assert_eq!(emit(&h, 11, 256, json!({})), json!([0]));
    wait_file(std::path::Path::new(&new));
    h.wait(|s| s["scriptsRunning"] == 0);
    cfg["commands"].as_array_mut().unwrap().last_mut().unwrap()["enabled"] = json!(false);
    h.replace(&cfg, None);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    h.settled(|_| true);
    assert_eq!(emit(&h, 11, 256, json!({})), json!([-9874]));
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"]["script.key"], 1);
}

#[test]
fn actual_recording_waits_for_physical_unregistration_and_restores_accepted_plan() {
    let h = normal();
    h.rpc(&json!({"op":"show","query":"finder"}));
    h.settled(|_| true);
    actions::hotkey(&h);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["recordingShortcut"],
        true
    );
    assert_eq!(press(&h), json!([-9874]));
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        true
    );
    assert!(trace(&h).iter().any(|x| x["phase"] == "unregister"));
    actions::probe(&h, "cancel-editor");
    h.settled(|s| s["recordingShortcut"] == false && s["nativeUI"]["editorOpen"] == false);
    assert_eq!(press(&h), json!([0]));
    h.settled(|s| s["nativeUI"]["visible"] == false);
}

#[test]
fn two_window_recorders_do_not_resume_on_other_window_cancel() {
    let h = normal();
    h.rpc(&json!({"op":"show","query":"finder"}));
    h.settled(|_| true);
    actions::hotkey(&h);
    h.rpc(&json!({"op":"show","mode":"settings"}));
    h.settled(|_| true);
    assert_eq!(h.rpc(&json!({"op":"native.test","surface":"settings","action":"inline-hotkey","id":"app.safari"}))["ok"],true);
    h.wait(|s| s["managementUI"]["editorOpen"] == true);
    assert_eq!(
        h.rpc(&json!({"op":"native.test","surface":"settings","action":"read"}))["result"]
            ["recording"],
        true
    );
    actions::probe(&h, "cancel-editor");
    h.settled(|s| s["nativeUI"]["editorOpen"] == false);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["recordingShortcut"],
        true
    );
    assert_eq!(press(&h), json!([-9874]));
    h.rpc(&json!({"op":"native.test","surface":"settings","action":"cancel-editor"}));
    h.wait(|s| s["recordingShortcut"] == false);
    assert_eq!(press(&h), json!([0]));
}

#[test]
fn headless_hidden_does_not_enable_keyboard_without_explicit_optin() {
    let h = host(&[], true, false, true, None);
    assert!(!h.base.join("keyboard-trace.jsonl").exists());
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeKeyboardService"],
        false
    );
}

#[test]
fn missing_owned_sdk_stamp_fails_before_any_global_target_or_handler() {
    let h = host(&[], false, true, false, None);
    let mut command = h.command(&h.settings.host);
    let child = command
        .arg("--native-hidden")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child_output::wait(child, Duration::from_secs(4));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Complete keyboard fixture"));
    assert!(!h.base.join("keyboard-trace.jsonl").exists());
}

struct Release(std::path::PathBuf);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::write(&self.0, b"");
    }
}

#[test]
fn positive_held_main_registration_keeps_owner_responsive_and_profile_locked_until_retirement() {
    let mut h = normal();
    fs::write(h.base.join("hold-register"), b"").unwrap();
    let mut cfg = config(&h);
    cfg["launcherHotkey"] = json!("cmd+a");
    h.replace(&cfg, None);
    wait_file(&h.base.join("keyboard-register-entered"));
    let release = Release(h.base.join("keyboard-register-release"));
    let at = std::time::Instant::now();
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["configurationUpdating"],
        true
    );
    assert_eq!(h.rpc(&json!({"op":"search","query":"finder"}))["ok"], true);
    assert_eq!(h.rpc(&json!({"op":"show","query":"safari"}))["ok"], true);
    assert_eq!(h.rpc(&json!({"op":"quit"}))["ok"], true);
    assert!(at.elapsed() < Duration::from_millis(600));
    let mut command = h.command(&h.settings.host);
    let child = command
        .arg("--native-hidden")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child_output::wait(child, Duration::from_secs(3));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("already running"));
    assert!(h.process.as_mut().unwrap().try_wait().unwrap().is_none());
    drop(release);
    let output = h.wait_exit(Duration::from_secs(5), false);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!h.profile.join("control.sock").exists());
    assert!(trace(&h).iter().any(|x| x["phase"] == "unregister"));
}

#[test]
fn launcher_from_hotkey_report_returns_to_commands_before_toggle_hides() {
    let fixture = env!("CARGO_BIN_EXE_private-results-fixture");
    let mut h = host(&[], false, true, true, None);
    let marker = marker_path(&h, "keyboard-report-marker");
    let args = [
        "fake".to_owned(),
        "report".to_owned(),
        marker,
        json!({"ok":true,"detail":"Own keyboard report"}).to_string(),
    ];
    let mut cfg = default_config();
    cfg["commands"]
        .as_array_mut()
        .unwrap()
        .push(key_script(fixture, &args));
    h.replace(&cfg, None);
    h.start();
    h.settled(|s| s["keyboardGeneration"].as_u64().unwrap_or(0) > 0);
    assert_eq!(emit(&h, 0, 256, json!({})), json!([0]));
    actions::view(&h, |v| !v["scriptResult"].is_null());
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        true
    );
    press(&h);
    h.settled(|s| s["nativeUI"]["visible"] == true);
    actions::view(&h, |v| v["scriptResult"].is_null());
    press(&h);
    h.settled(|s| s["nativeUI"]["visible"] == false);
}

#[test]
fn failed_registration_does_not_publish_editor_save_and_old_routes_survive() {
    let h = normal();
    h.rpc(&json!({"op":"show","query":"finder"}));
    h.settled(|_| true);
    let before = fs::read(&h.config).unwrap();
    let generation = h.rpc(&json!({"op":"status"}))["result"]["keyboardGeneration"].clone();
    fs::write(h.base.join("fail-code"), "11").unwrap();
    actions::hotkey(&h);
    actions::key(&h, 11, actions::CMD, "b", false);
    actions::view(&h, |v| {
        v["saveEnabled"] == true && actions::text_contains(v, "unavailable (-9878)")
    });
    h.settled(|_| true);
    assert_eq!(fs::read(&h.config).unwrap(), before);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["keyboardGeneration"],
        generation
    );
    actions::probe(&h, "cancel-editor");
    h.settled(|s| s["recordingShortcut"] == false);
    assert_eq!(press(&h), json!([0]));
    h.settled(|s| s["nativeUI"]["visible"] == false);
    fs::remove_file(h.base.join("fail-code")).unwrap();
}

#[test]
fn pending_recorder_consumes_save_keys_and_cancels_during_positive_physical_hold() {
    for (theme, management) in [
        ("light", false),
        ("dark", false),
        ("light", true),
        ("dark", true),
    ] {
        let mut h = host(&[], true, true, true, Some(theme));
        let before = fs::read(&h.config).unwrap();
        let surface = if management {
            h.rpc(&json!({"op":"show","mode":"settings"}));
            "settings"
        } else {
            h.rpc(&json!({"op":"show","query":"finder"}));
            "palette"
        };
        h.settled(|_| true);
        fs::write(h.base.join("hold-unregister"), b"").unwrap();
        assert_eq!(h.rpc(&json!({"op":"native.test","surface":surface,"action":"schedule-pending-recorder","close":management}))["ok"],true);
        if management {
            h.rpc(&json!({"op":"native.test","surface":"settings","action":"inline-hotkey","id":"app.safari"}));
        } else {
            actions::key(&h, 43, actions::CMD | actions::SHIFT, ",", false);
        }
        let path = h.base.join("keyboard-pending.json");
        let mut proof = wait_json(&path);
        let text = proof.as_object_mut().unwrap().remove("screenText").unwrap();
        let accessibility = proof
            .as_object_mut()
            .unwrap()
            .remove("accessibilityText")
            .unwrap();
        assert!(text
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x.as_str().unwrap().contains("Preparing")));
        assert!(accessibility
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x.as_str().unwrap().contains("Preparing shortcut recording")));
        if let Some(out) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
            let dir = std::path::PathBuf::from(out)
                .parent()
                .unwrap()
                .join("native-keyboard");
            fs::create_dir_all(&dir).unwrap();
            let name = format!(
                "pending-{theme}-{}",
                if management { "True" } else { "False" }
            );
            fs::copy(
                h.base.join("keyboard-pending.png"),
                dir.join(format!("{name}.png")),
            )
            .unwrap();
            let mut evidence = proof.clone();
            evidence["screenText"] = text;
            evidence["accessibilityText"] = accessibility;
            fs::write(
                dir.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(
            proof,
            json!({"preparing":true,"recording":false,"consumed":[true,true],"unchanged":true,"cancelled":true,"physicalHold":true})
        );
        h.settled(|s| s["recordingShortcut"] == false);
        assert_eq!(fs::read(&h.config).unwrap(), before);
        assert!(h.process.as_mut().unwrap().try_wait().unwrap().is_none());
    }
}

#[test]
fn callback_during_commit_before_main_ack_is_deferred_until_exact_store_adoption() {
    let mut h = host(&[], false, true, true, None);
    let marker = marker_path(&h, "after-commit");
    let mut cfg = default_config();
    cfg["commands"]
        .as_array_mut()
        .unwrap()
        .push(key_script("/usr/bin/touch", std::slice::from_ref(&marker)));
    h.replace(&cfg, None);
    h.start();
    h.settled(|s| s["keyboardGeneration"].as_u64().unwrap_or(0) > 0);
    let prior = h.rpc(&json!({"op":"status"}))["result"]["keyboardGeneration"]
        .as_u64()
        .unwrap();
    fs::write(h.base.join("emit-latest-on-unregister"), b"").unwrap();
    fs::write(h.base.join("hold-unregister"), b"").unwrap();
    cfg["commands"].as_array_mut().unwrap().last_mut().unwrap()["hotkey"] = json!("cmd+b");
    h.replace(&cfg, None);
    wait_file(&h.base.join("keyboard-unregister-entered"));
    let release = Release(h.base.join("keyboard-unregister-release"));
    let status = h.rpc(&json!({"op":"status"}));
    assert_eq!(status["result"]["configurationUpdating"], true);
    assert_eq!(status["result"]["keyboardGeneration"], prior);
    assert!(!std::path::Path::new(&marker).exists());
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
    assert!(trace(&h)
        .iter()
        .any(|x| x["phase"] == "unregister-latest-callback" && x["b"] == 0));
    fs::remove_file(h.base.join("emit-latest-on-unregister")).unwrap();
    drop(release);
    h.settled(|s| s["keyboardGeneration"].as_u64().unwrap() > prior);
    wait_file(std::path::Path::new(&marker));
    h.wait(|s| s["scriptsRunning"] == 0);
    assert_eq!(h.rpc(&json!({"op":"usage"}))["result"]["script.key"], 1);
}

#[test]
fn recorder_begin_reenters_registration_blocks_immediately_and_retires_new_handle() {
    let h = normal();
    h.rpc(&json!({"op":"show","query":"finder"}));
    h.settled(|_| true);
    actions::hotkey(&h);
    actions::probe(&h, "record");
    actions::view(&h, |v| {
        v["recording"] == false && v["recordingPreparing"] == false
    });
    h.settled(|s| s["recordingShortcut"] == false);
    fs::write(h.base.join("hold-register"), b"").unwrap();
    fs::write(h.base.join("pump-register"), b"").unwrap();
    actions::probe(&h, "schedule-begin-recorder");
    let mut cfg = config(&h);
    cfg["launcherHotkey"] = json!("cmd+a");
    h.replace(&cfg, None);
    let path = h.base.join("keyboard-nested-begin.json");
    let proof = wait_json(&path);
    assert_eq!(
        proof,
        json!({"preparing":true,"recording":false,"blockedCallback":{"Ok":-9874},"physicalHold":true})
    );
    actions::view(&h, |v| v["recording"] == true);
    h.settled(|s| s["configurationUpdating"] == false);
    let events = trace(&h);
    let id = events
        .iter()
        .find(|x| x["phase"] == "register" && x["a"] == 0)
        .unwrap()["c"]
        .clone();
    assert!(events
        .iter()
        .any(|x| x["phase"] == "unregister" && x["c"] == id));
    assert_eq!(emit(&h, 0, 256, json!({})), json!([-9874]));
    actions::probe(&h, "cancel-editor");
    h.settled(|s| s["recordingShortcut"] == false);
    assert_eq!(h.rpc(&json!({"op":"reload"}))["ok"], true);
    h.settled(|_| true);
    assert_eq!(emit(&h, 0, 256, json!({})), json!([0]));
}

#[test]
fn current_native_copy_and_recorder_error_renders_in_both_themes() {
    for theme in ["light", "dark"] {
        let h = host(&[], true, true, true, Some(theme));
        h.rpc(&json!({"op":"show","query":"finder"}));
        h.settled(|_| true);
        actions::hotkey(&h);
        capture(&h, &format!("recording-{theme}"));
        fs::write(h.base.join("fail-code"), "11").unwrap();
        actions::key(&h, 11, actions::CMD, "b", false);
        actions::view(&h, |v| {
            v["saveEnabled"] == true && actions::text_contains(v, "unavailable (-9878)")
        });
        capture(&h, &format!("registration-error-{theme}"));
        actions::probe(&h, "cancel-editor");
        h.settled(|s| s["recordingShortcut"] == false);
    }
}

fn keyword_host(hyper: bool) -> Bridge {
    let mut h = host(&[], false, true, true, None);
    let mut config = default_config();
    config["commands"] = json!([{ "id":"own.snippet", "title":"Own snippet", "kind":"snippet", "target":"HELLO", "keyword":"abc" }]);
    config["snippetExpansionEnabled"] = json!(true);
    if hyper {
        config["hyperKeyEnabled"] = json!(true);
    }
    h.replace(&config, None);
    h.start();
    h.wait(|s| s["keywordGeneration"] == s["keyboardGeneration"] && s["snippetExpansion"] == "On");
    h
}
fn keyword(h: &Bridge, control: &str, fields: Value) -> Value {
    let mut fields = fields.as_object().unwrap().clone();
    fields.insert("control".into(), json!(control));
    h.probe("keyboard-keyword", Value::Object(fields))
}
fn keyword_type(h: &Bridge, text: &str) -> Vec<Value> {
    text.chars()
        .map(|c| keyword(h, "event", json!({"text":c.to_string()}))["consumed"].clone())
        .collect()
}
fn keyword_reset(h: &Bridge) {
    keyword(h, "event", json!({"text":""}));
    keyword(h, "clear", json!({}));
}
#[test]
fn native_keyword_adopts_catalog_and_emits_without_showing_window() {
    let h = keyword_host(false);
    assert_eq!(
        keyword_type(&h, "abc"),
        vec![json!(false), json!(false), json!(true)]
    );
    let state = keyword(&h, "state", json!({}));
    let units: Vec<u16> = state["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["down"] == true && e["key"] == 0)
        .flat_map(|e| {
            e["units"]
                .as_array()
                .unwrap()
                .iter()
                .map(|u| u.as_u64().unwrap() as u16)
        })
        .collect();
    assert_eq!(String::from_utf16(&units).unwrap(), "HELLO");
    assert_eq!(state["valueReads"], 0);
    assert_eq!(h.probe("read", json!({}))["windowShown"], false);
    assert_eq!(
        h.rpc(&json!({"op":"status"}))["result"]["snippetExpansionComplete"],
        false
    );
}
#[test]
fn native_keyword_hyper_episode_clears_prefix() {
    let h = keyword_host(true);
    h.wait(|s| s["hyper"] == "On");
    keyword_type(&h, "ab");
    for (down, time) in [(true, 1.0), (false, 1.1)] {
        h.probe(
            "keyboard-hid",
            json!({"control":"input","usage":57,"down":down,"time":time}),
        );
    }
    assert_eq!(keyword_type(&h, "c"), vec![json!(false)]);
    assert_eq!(keyword(&h, "state", json!({}))["posts"], 0);
    keyword_reset(&h);
    assert_eq!(keyword_type(&h, "abc").last(), Some(&json!(true)));
}
#[test]
fn native_keyword_external_activation_roundtrip_clears_prefix() {
    let h = keyword_host(false);
    keyword_type(&h, "ab");
    for _ in 0..2 {
        h.probe(
            "keyboard-hid",
            json!({"control":"notification","name":"activate"}),
        );
    }
    assert_eq!(keyword_type(&h, "c"), vec![json!(false)]);
    assert_eq!(keyword(&h, "state", json!({}))["posts"], 0);
    keyword_reset(&h);
    assert_eq!(keyword_type(&h, "abc").last(), Some(&json!(true)));
}

#[test]
fn native_keyword_sleep_session_interleaving_and_resumption() {
    let h = keyword_host(false);
    keyword_type(&h, "ab");
    for name in ["sleep", "resign", "wake"] {
        h.probe(
            "keyboard-hid",
            json!({"control":"notification", "name":name}),
        );
    }
    h.wait(|s| s["snippetExpansion"] == "Paused");
    assert_eq!(keyword_type(&h, "c").last(), Some(&json!(false)));
    h.probe(
        "keyboard-hid",
        json!({"control":"notification", "name":"active"}),
    );
    h.wait(|s| s["snippetExpansion"] == "On");
    keyword_reset(&h);
    assert_eq!(keyword_type(&h, "abc").last(), Some(&json!(true)));
}
#[test]
fn native_keyword_timeout_recovers_on_main_wake_and_drops_prefix() {
    let h = keyword_host(false);
    keyword_type(&h, "ab");
    keyword(&h, "event", json!({"type":u32::MAX}));
    h.wait(|s| s["snippetExpansion"] == "On");
    assert_eq!(keyword_type(&h, "c").last(), Some(&json!(false)));
    keyword_reset(&h);
    assert_eq!(keyword_type(&h, "abc").last(), Some(&json!(true)));
}
#[test]
fn native_keyword_timer_recovers_failed_setup_without_new_input() {
    let h = keyword_host(false);
    let fault = h.base.join("keyword-fail-tap_create");
    fs::write(&fault, b"").unwrap();
    keyword(&h, "event", json!({"type":u32::MAX}));
    h.wait(|s| s["snippetExpansion"] != "On");
    fs::remove_file(fault).unwrap();
    h.wait(|s| s["snippetExpansion"] == "On");
    assert_eq!(keyword_type(&h, "abc").last(), Some(&json!(true)));
}

fn keyword_reload(h: &Bridge, config: &Value) {
    let old = h.rpc(&json!({"op":"status"}))["result"]["keyboardGeneration"]
        .as_u64()
        .unwrap();
    h.replace(config, None);
    let reply = h.rpc(&json!({"op":"reload"}));
    assert!(
        reply["ok"] == true
            || reply["error"]
                .as_str()
                .unwrap_or("")
                .contains("being updated"),
        "{reply}"
    );
    h.wait(|s| {
        s["keyboardGeneration"].as_u64().unwrap_or(0) > old
            && s["keywordGeneration"] == s["keyboardGeneration"]
            && s["configurationUpdating"] == false
    });
}
fn keyword_inserted(state: &Value) -> String {
    let units: Vec<u16> = state["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["down"] == true && e["key"] == 0)
        .flat_map(|e| {
            e["units"]
                .as_array()
                .unwrap()
                .iter()
                .map(|u| u.as_u64().unwrap() as u16)
        })
        .collect();
    String::from_utf16(&units).unwrap()
}
#[test]
fn native_keyword_activation_burst_reaches_owner_before_coalescing() {
    let h = keyword_host(false);
    keyword(&h, "activation-listen", json!({}));
    keyword_type(&h, "ab");
    h.probe(
        "keyboard-hid",
        json!({"control":"activation-burst","values":["typed","missing","malformed","typed"]}),
    );
    let state = keyword(&h, "activation-state", json!({}));
    let events = state["events"].as_array().unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(
        events
            .iter()
            .map(|v| v["typed"].clone())
            .collect::<Vec<_>>(),
        vec![json!(true), json!(false), json!(false), json!(true)]
    );
    assert_eq!(
        events
            .iter()
            .map(|v| v["ownInstance"].clone())
            .collect::<Vec<_>>(),
        vec![json!(true), json!(false), json!(false), json!(true)]
    );
    let first = events[0]["sequence"].as_u64().unwrap();
    assert_eq!(
        events
            .iter()
            .map(|v| v["sequence"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        (first..first + 4).collect::<Vec<_>>()
    );
    assert_eq!(keyword_type(&h, "c").last(), Some(&json!(false)));
    assert_eq!(keyword(&h, "state", json!({}))["posts"], 0);
    keyword_reset(&h);
    assert_eq!(keyword_type(&h, "abc").last(), Some(&json!(true)));
}
#[test]
fn native_keyword_reload_adopts_new_catalog_and_disable_retires_filter() {
    let h = keyword_host(false);
    let mut config = actions::config(&h);
    keyword_type(&h, "ab");
    config["commands"][0]["target"] = json!("NEW");
    keyword_reload(&h, &config);
    assert_eq!(keyword_type(&h, "c"), vec![json!(false)]);
    keyword_reset(&h);
    assert_eq!(keyword_type(&h, "abc").last(), Some(&json!(true)));
    assert_eq!(keyword_inserted(&keyword(&h, "state", json!({}))), "NEW");
    config["snippetExpansionEnabled"] = json!(false);
    keyword_reload(&h, &config);
    h.wait(|s| s["snippetExpansion"] == "Off");
    assert_eq!(keyword(&h, "state", json!({}))["tapValid"], false);
    assert_eq!(keyword_type(&h, "abc").last(), Some(&json!(false)));
}
#[test]
fn native_keyword_secure_exclusion_and_permission_changes_prevent_expansion() {
    let h = keyword_host(false);
    let mut config = actions::config(&h);
    for flags in [8, 16, 4096] {
        keyword(&h, "environment", json!({"flags":flags}));
        assert_eq!(keyword_type(&h, "abc").last(), Some(&json!(false)));
        assert_eq!(keyword(&h, "state", json!({}))["posts"], 0);
    }
    keyword(&h, "environment", json!({}));
    config["clipboard"]["excludedApps"] = json!(["own.keyword.target"]);
    keyword_reload(&h, &config);
    assert_eq!(keyword_type(&h, "abc").last(), Some(&json!(false)));
    assert_eq!(keyword(&h, "state", json!({}))["posts"], 0);
}
