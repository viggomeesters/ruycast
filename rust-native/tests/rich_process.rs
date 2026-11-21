//! Native rich result lifecycle with an owned Rust provider.
#![cfg(feature = "test-tools")]
#[allow(dead_code)]
#[path = "support/native_actions.rs"]
mod actions;
use actions::native_bridge::{
    runtime_host::{default_config, Config},
    Bridge,
};
use serde_json::{json, Value};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn host(mode: &str) -> Bridge {
    host_width(mode, None)
}
fn host_width(mode: &str, width: Option<&str>) -> Bridge {
    let mut host = Bridge::new(Config::current(), false);
    let mut config = default_config();
    config["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["rich-provider","{input}",host.base,mode],"timeout":3}));
    host.replace(&config, None);
    if let Some(width) = width {
        host.env
            .insert("RUYCAST_NATIVE_TEST_WIDTH".into(), width.into());
    }
    host.start();
    host.settled(|_| true);
    host
}
fn view(host: &Bridge, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        host.settled(|_| true);
        let v = actions::probe(host, "read");
        if !v["scriptResult"].is_null() && predicate(&v["scriptResult"]) {
            return v;
        }
        assert!(Instant::now() < deadline, "rich result missing: {v}");
        thread::sleep(Duration::from_millis(20));
    }
}
fn open(host: &Bridge) -> Value {
    actions::show(host, "runfixture");
    assert_eq!(actions::probe(host, "read")["windowShown"], false);
    actions::key(host, 36, 0, "\r", false);
    view(host, |_| true)
}
fn click(host: &Bridge, name: &str) {
    host.probe("result-control", json!({"name":name}));
    host.settled(|_| true);
}
fn capture(host: &Bridge, name: &str) -> Value {
    let v = host.probe("screenshot", json!({}));
    assert_eq!(v["windowShown"], false);
    assert_eq!(v["windowKey"], false);
    if let Some(output) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let folder = std::path::PathBuf::from(output)
            .parent()
            .unwrap()
            .join("rich");
        fs::create_dir_all(&folder).unwrap();
        fs::copy(
            host.profile.join("native-probe.png"),
            folder.join(format!("{name}.png")),
        )
        .unwrap();
        fs::write(
            folder.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&v).unwrap(),
        )
        .unwrap();
    }
    v
}
#[test]
fn choices_filter_literal_selection_and_ok_only_closes() {
    let host = host("close");
    let v = open(&host);
    let result = &v["scriptResult"];
    assert_eq!(
        result["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["reused", "two"]
    );
    assert!(!serde_json::to_string(result)
        .unwrap()
        .contains("PRIVATE $(literal)"));
    assert_eq!(v["rows"], 2);
    assert!(v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "EX / PLAN"));
    host.probe("query", json!({"query":"CAFE PLAN"}));
    let v = view(&host, |r| r["rows"].as_array().unwrap().len() == 1);
    assert_eq!(v["scriptResult"]["rows"][0]["id"], "reused");
    actions::key(&host, 36, 0, "\r", false);
    host.wait(|s| s["scriptsRunning"] == 0);
    assert!(actions::probe(&host, "read")
        .get("scriptResult")
        .unwrap()
        .is_null());
    assert_eq!(
        fs::read_to_string(host.base.join("selected")).unwrap(),
        "PRIVATE $(literal) Ω 1"
    );
    assert_eq!(
        host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
        2
    );
}
#[test]
fn original_project_part_choices_capture_at_760_and_480_pixels() {
    // Native width/label coverage only; this is not pixel-for-pixel Swift parity.
    for (width, pixels) in [("760", 760_u32), ("480", 480_u32)] {
        let host = host_width("project-part", Some(width));
        open(&host);
        let captured = capture(&host, &format!("project-part-{width}"));
        let png = fs::read(host.profile.join("native-probe.png")).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() >= 24, "PNG header missing at {width}px");
        assert_eq!(&png[12..16], b"IHDR");
        let dimensions = (
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
        );
        // AppKit may capture at 1x or Retina 2x; both axes must share that scale.
        assert!(
            dimensions == (pixels, 490) || dimensions == (pixels * 2, 980),
            "wrong native content dimensions at {width}px: {dimensions:?}"
        );
        assert_eq!(captured["scriptResult"]["title"], "Project Koppelen");
        assert_eq!(captured["scriptResult"]["selection"], 0);
        assert_eq!(
            captured["scriptResult"]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["project", "part"]
        );
        for label in [
            "Project Koppelen",
            "Example Project",
            "EX · Projectbreed",
            "Planning",
            "EX / PLAN · Example Project",
            "Zoek project, onderdeel of code…",
            "Back",
            "Choose ↵",
            "Reload ⌘R",
        ] {
            assert!(
                captured["screenText"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|text| text == label),
                "missing choice label at {width}px: {label}"
            );
        }
        for label in [
            "Project Koppelen",
            "Example Project",
            "Planning",
            "Zoek project, onderdeel of code…",
            "Back",
            "Choose ↵",
            "Reload ⌘R",
        ] {
            assert!(
                captured["accessibilityText"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|text| text == label),
                "missing accessibility label at {width}px: {label}"
            );
        }
        assert!(!host.base.join("selected").exists());
    }
}
#[test]
fn original_project_part_navigation_and_single_submit() {
    let host = GracefulBridge(host("project-part"));
    let initial = open(&host);
    let token = initial["scriptResult"].get("token").unwrap().clone();
    assert_eq!(
        initial["scriptResult"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["project", "part"]
    );
    assert_eq!(initial["scriptResult"]["selection"], 0);
    assert!(!host.base.join("selected").exists());

    // These are NSEvents dispatched inside the owned hidden native host.
    actions::key(&host, 125, 0, "\u{f701}", false);
    view(&host, |r| r["selection"] == 1);
    host.probe("query", json!({"query":"EX PLAN"}));
    let filtered = view(&host, |r| r["query"] == "EX PLAN");
    assert_eq!(filtered["scriptResult"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["scriptResult"]["rows"][0]["id"], "part");
    assert_eq!(filtered["scriptResult"]["selection"], 0);

    host.probe("query", json!({"query":"nothing"}));
    let empty = view(&host, |r| r["query"] == "nothing");
    assert!(empty["scriptResult"]["rows"].as_array().unwrap().is_empty());
    assert!(empty["scriptResult"]["selection"].is_null());
    actions::key(&host, 36, 0, "\r", false);
    host.settled(|s| s["scriptsRunning"] == 0);
    let unmatched = view(&host, |r| r["query"] == "nothing");
    assert_eq!(unmatched["scriptResult"].get("token").unwrap(), &token);
    assert_eq!(unmatched["scriptResult"]["busy"], false);
    assert!(!host.base.join("selected").exists());
    assert!(!host.base.join("ready").exists());

    host.probe("query", json!({"query":"PLAN"}));
    let plan = view(&host, |r| r["query"] == "PLAN");
    assert_eq!(plan["scriptResult"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(plan["scriptResult"]["rows"][0]["id"], "part");
    assert_eq!(plan["scriptResult"]["selection"], 0);
    actions::key(&host, 36, 0, "\r", false);
    wait_exists(&host.base.join("ready"));
    let loading = view(&host, |r| r["busy"] == true);
    let loading_capture = capture(&host, "project-part-provider-loading");
    assert_eq!(loading_capture["scriptResult"]["busy"], true);
    assert!(actions::text_contains(&loading_capture, "Planning"));
    assert_eq!(loading_capture["scriptResult"]["token"], token);
    assert_eq!(
        loading_capture["scriptResult"]["rows"],
        loading["scriptResult"]["rows"]
    );
    assert_eq!(
        loading_capture["scriptResult"]["selection"],
        loading["scriptResult"]["selection"]
    );
    assert_eq!(
        loading_capture["scriptResult"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["part"]
    );
    // Hold the provider response until the second Return crosses the native boundary.
    actions::key(&host, 36, 0, "\r", false);
    fs::write(host.base.join("release"), "go").unwrap();
    host.wait(|s| s["scriptsRunning"] == 0);
    actions::view(&host, |v| v["scriptResult"].is_null());
    assert_eq!(
        fs::read_to_string(host.base.join("selected")).unwrap(),
        "pinned-component"
    );

    let fresh = open(&host);
    assert_ne!(fresh["scriptResult"].get("token").unwrap(), &token);
    actions::key(&host, 53, 0, "", false);
    actions::view(&host, |v| v["scriptResult"].is_null());
    assert_eq!(
        fs::read_to_string(host.base.join("selected")).unwrap(),
        "pinned-component"
    );
}
#[test]
fn stale_project_part_refusal_recovers_with_command_r_reload() {
    let host = GracefulBridge(host("project-part-stale"));
    let initial = open(&host)["scriptResult"].clone();
    assert_eq!(
        initial["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["project", "part"]
    );
    host.probe("query", json!({"query":"PLAN"}));
    let filtered = view(&host, |r| r["query"] == "PLAN")["scriptResult"].clone();
    assert_eq!(filtered["rows"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["rows"][0]["id"], "part");
    fs::write(host.base.join("stale"), "stale").unwrap();
    actions::key(&host, 36, 0, "\r", false);
    let refused = view(&host, |r| {
        r["busy"] == false
            && r["error"] == "Project choices are stale. Reload to try again."
    });
    let result = &refused["scriptResult"];
    assert_eq!(result.get("token"), initial.get("token"));
    assert_eq!(result["title"], initial["title"]);
    assert_eq!(result["query"], "PLAN");
    assert_eq!(result["rows"], filtered["rows"]);
    assert_eq!(result["selection"], filtered["selection"]);
    assert!(actions::text_contains(&refused, "Planning"));
    assert!(actions::text_contains(
        &refused,
        "Project choices are stale. Reload to try again."
    ));
    assert!(!host.base.join("selected").exists());
    assert!(!host.base.join("ready").exists());

    fs::remove_file(host.base.join("stale")).unwrap();
    // Dispatch the actual Command+R NSEvent inside the owned hidden host.
    actions::key(&host, 15, actions::CMD, "r", false);
    let fresh = view(&host, |r| r.get("token") != initial.get("token"));
    let result = &fresh["scriptResult"];
    assert_eq!(result["busy"], false);
    assert_eq!(result["error"], "");
    assert_eq!(result["query"], "");
    assert_eq!(result["rows"], initial["rows"]);
    assert_eq!(result["selection"], 0);
    assert!(actions::text_contains(&fresh, "Example Project"));
    assert!(actions::text_contains(&fresh, "Planning"));
    assert!(!actions::text_contains(&fresh, "Project choices are stale."));
    assert!(!host.base.join("selected").exists());
    assert!(!host.base.join("ready").exists());
}
#[test]
fn reload_replaces_token_and_stale_button_is_detached() {
    let host = host("close");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    host.probe("retain-result-button", json!({"name":"choose"}));
    host.probe("query", json!({"query":"PLAN"}));
    view(&host, |r| r["query"] == "PLAN");
    click(&host, "reload");
    let new = view(&host, |r| r.get("token").unwrap() != &old);
    assert_eq!(new["scriptResult"]["query"], "");
    assert_eq!(new["scriptResult"]["selection"], 0);
    assert!(!host.base.join("selected").exists());
    assert_eq!(fs::read_to_string(host.base.join("phase")).unwrap(), "2");
    actions::probe(&host, "stale-result-button");
    host.settled(|_| true);
    assert!(!host.base.join("selected").exists());
    click(&host, "choose");
    host.wait(|s| s["scriptsRunning"] == 0);
    assert_eq!(
        fs::read_to_string(host.base.join("selected")).unwrap(),
        "PRIVATE $(literal) Ω 2"
    );
}
#[test]
fn failure_keeps_query_selection_and_readable_detail() {
    let host = host("fail");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    host.probe("query", json!({"query":"PLAN"}));
    view(&host, |r| r["query"] == "PLAN");
    click(&host, "choose");
    let v = view(&host, |r| {
        r["busy"] == false && r["error"] == "Provider refused"
    });
    assert_eq!(v["scriptResult"].get("token").unwrap(), &old);
    assert_eq!(v["scriptResult"]["query"], "PLAN");
    capture(&host, "provider-failure");
    click(&host, "details");
    let v = view(&host, |r| r["detail_visible"] == true);
    assert_eq!(v["resultPanel"]["detail"], "Failure detail");
    assert_eq!(v["resultPanel"]["editable"], false);
    assert_eq!(v["resultPanel"]["selectable"], true);
    click(&host, "choices");
    assert_eq!(view(&host, |_| true)["scriptResult"]["query"], "PLAN");
}
struct GracefulBridge(Bridge);
impl std::ops::Deref for GracefulBridge {
    type Target = Bridge;
    fn deref(&self) -> &Bridge {
        &self.0
    }
}
impl Drop for GracefulBridge {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.0.host.stop(true)));
        }
    }
}
fn wait_exists(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "fixture did not publish {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn detail_scroll_back_preserves_choices_and_filter() {
    let host = host("close");
    open(&host);
    host.probe("query", json!({"query":"PLAN"}));
    view(&host, |r| r["query"] == "PLAN");
    click(&host, "details");
    let v = view(&host, |r| r["detail_visible"] == true);
    assert_eq!(v["resultPanel"]["selectable"], true);
    assert_eq!(v["resultPanel"]["editable"], false);
    assert!(v["resultPanel"]["detail"]
        .as_str()
        .unwrap()
        .contains("PRIVATE report"));
    assert!(v["resultPanel"]["scrollMaxY"].as_f64().unwrap() > 0.0);
    capture(&host, "long-detail");
    actions::key(&host, 53, 0, "", false);
    assert_eq!(
        view(&host, |r| r["detail_visible"] == false)["scriptResult"]["query"],
        "PLAN"
    );
}
#[test]
fn unsafe_followup_closes_choices_with_invalid_response() {
    let host = host("invalid");
    open(&host);
    click(&host, "choose");
    host.wait(|s| s["scriptsRunning"] == 0);
    let v = actions::probe(&host, "read");
    assert!(v.get("scriptResult").unwrap().is_null());
    assert!(v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("invalid response")));
}
#[test]
fn new_selection_result_uses_new_token_and_values() {
    let host = host("replace");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    click(&host, "choose");
    let v = view(&host, |r| r.get("token").unwrap() != &old);
    assert_eq!(v["scriptResult"]["title"], "Updated");
    assert_eq!(v["scriptResult"]["query"], "");
}
#[test]
fn changed_command_cannot_execute_old_selection() {
    let host = host("close");
    open(&host);
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    command["target"] = json!("/usr/bin/false");
    command["arguments"] = json!([]);
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    click(&host, "choose");
    let v = view(&host, |r| {
        r["error"].as_str().unwrap_or("").contains("changed")
    });
    assert!(!host.base.join("selected").exists());
    assert_eq!(v["scriptResult"]["busy"], false);
}
#[test]
fn back_during_busy_cancels_only_owned_job() {
    let host = GracefulBridge(host("slow"));
    open(&host);
    click(&host, "choose");
    wait_exists(&host.base.join("ready"));
    view(&host, |r| r["busy"] == true);
    click(&host, "close");
    host.wait(|s| s["scriptsRunning"] == 0);
    let v = actions::probe(&host, "read");
    assert!(v.get("scriptResult").unwrap().is_null());
    assert_eq!(
        host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]["visible"],
        true
    );
    assert!(!v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("Late report")));
}
#[test]
fn rejected_query_blocks_old_choice_and_can_be_repaired() {
    let host = host("close");
    open(&host);
    let query = (0..65)
        .map(|n| format!("term{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    host.probe("query", json!({"query":query}));
    let v = view(&host, |r| r["query_valid"] == false);
    assert!(v["scriptResult"].get("selection").unwrap().is_null());
    actions::key(&host, 36, 0, "\r", false);
    host.settled(|_| true);
    assert!(!host.base.join("selected").exists());
    host.probe("query", json!({"query":"cafe"}));
    view(&host, |r| r["query_valid"] == true);
    click(&host, "choose");
    host.wait(|s| s["scriptsRunning"] == 0);
    assert!(host.base.join("selected").exists());
}
#[test]
fn retained_old_detail_text_cannot_close_replacement() {
    let host = host("close");
    open(&host);
    click(&host, "details");
    view(&host, |r| r["detail_visible"] == true);
    actions::probe(&host, "retain-result-text");
    click(&host, "choices");
    let old = view(&host, |_| true)["scriptResult"]
        .get("token")
        .unwrap()
        .clone();
    click(&host, "reload");
    let new = view(&host, |r| r.get("token").unwrap() != &old)["scriptResult"]
        .get("token")
        .unwrap()
        .clone();
    click(&host, "details");
    view(&host, |r| r["detail_visible"] == true);
    actions::probe(&host, "stale-result-text");
    host.settled(|_| true);
    assert_eq!(
        view(&host, |_| true)["scriptResult"].get("token").unwrap(),
        &new
    );
}
#[test]
fn post_effect_usage_warning_keeps_reloaded_choices() {
    let host = host("close");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    let usage = host.profile.join("usage.json");
    fs::remove_file(&usage).unwrap();
    fs::create_dir(&usage).unwrap();
    click(&host, "reload");
    let v = view(&host, |r| r.get("token").unwrap() != &old);
    let r = &v["scriptResult"];
    assert!(!r["rows"].as_array().unwrap().is_empty());
    assert!(r["error"]
        .as_str()
        .unwrap()
        .contains("usage could not be saved"));
    assert_eq!(r["busy"], false);
}
#[test]
fn queued_query_before_close_cannot_error_on_restored_search() {
    let host = host("close");
    open(&host);
    host.probe("schedule-result-close", json!({"queryBeforeClose":true}));
    let delay = host.rpc(&json!({"op":"native.delay","executionBarrier":true}));
    assert_eq!(delay["result"]["newIntentDuringDelay"], true);
    let path = host.profile.join("result-close-barrier.json");
    wait_exists(&path);
    let proof: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(proof["queryBeforeClose"], true);
    assert_eq!(proof["queryEpochAdvanced"], true);
    host.settled(|_| true);
    let v = actions::probe(&host, "read");
    assert!(v.get("scriptResult").unwrap().is_null());
    assert!(!v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("no longer open")));
    assert!(!host.base.join("selected").exists());
}
#[test]
fn native_ime_blocks_return_escape_details_and_mouse_until_commit() {
    let host = host("close");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    for action in ["return", "escape", "details", "mouse"] {
        assert_eq!(
            host.probe("rich-query-mark", json!({"text":"CA"}))["marked"],
            true
        );
        match action {
            "return" => {
                actions::key(&host, 36, 0, "\r", false);
            }
            "escape" => {
                actions::key(&host, 53, 0, "", false);
            }
            "details" => {
                host.probe("result-control", json!({"name":"details"}));
            }
            "mouse" => {
                host.probe("click", json!({"row":0}));
            }
            _ => unreachable!(),
        }
        host.settled(|_| true);
        assert!(!host.base.join("selected").exists());
        let v = view(&host, |_| true);
        assert_eq!(v["scriptResult"].get("token").unwrap(), &old);
        assert_eq!(v["scriptResult"]["detail_visible"], false);
        actions::probe(&host, "rich-query-unmark");
        host.probe("query", json!({"query":""}));
    }
    host.probe("query", json!({"query":"CAFE"}));
    view(&host, |r| r["query"] == "CAFE");
    host.probe("click", json!({"row":0}));
    host.wait(|s| s["scriptsRunning"] == 0);
    assert_eq!(
        fs::read_to_string(host.base.join("selected")).unwrap(),
        "PRIVATE $(literal) Ω 1"
    );
}
#[test]
fn retired_query_undo_redo_and_typing_cannot_modify_new_session() {
    let host = host("close");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    host.probe("rich-query-type", json!({"text":"cafe"}));
    view(&host, |r| r["query"] == "cafe");
    assert_eq!(actions::probe(&host, "rich-query-read")["canUndo"], true);
    actions::probe(&host, "retain-result-query");
    click(&host, "reload");
    let new = view(&host, |r| r.get("token").unwrap() != &old)["scriptResult"]
        .get("token")
        .unwrap()
        .clone();
    let proof = actions::probe(&host, "stale-result-query");
    assert_eq!(proof["cleared"], true);
    assert_eq!(proof["detached"], true);
    assert_eq!(proof["editable"], false);
    assert_eq!(proof["canUndoAfter"], false);
    host.settled(|_| true);
    let v = view(&host, |_| true);
    assert_eq!(v["scriptResult"].get("token").unwrap(), &new);
    assert_eq!(v["scriptResult"]["query"], "");
    assert!(!host.base.join("selected").exists());
}
#[test]
fn retired_result_actions_are_effectless_after_close_and_replacement() {
    for replacement in [false, true] {
        let host = host("close");
        let old = open(&host)["scriptResult"].get("token").unwrap().clone();
        let current = if replacement {
            click(&host, "reload");
            Some(view(&host, |r| r.get("token").unwrap() != &old)["scriptResult"].clone())
        } else {
            click(&host, "close");
            host.settled(|_| true);
            assert!(actions::probe(&host, "read")
                .get("scriptResult")
                .unwrap()
                .is_null());
            None
        };
        let epoch = host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]
            .get("queryEpoch")
            .unwrap()
            .clone();
        for name in ["details", "select", "choose", "activate", "reload", "close"] {
            host.probe(
                "result-event",
                json!({"token":old,"epoch":epoch,"name":name,"id":"reused"}),
            );
            host.settled(|_| true);
            let v = actions::probe(&host, "read");
            assert!(!v["screenText"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s.as_str().unwrap().contains("no longer open")));
            if let Some(current) = &current {
                assert_eq!(v.get("scriptResult").unwrap(), current);
            } else {
                assert!(v.get("scriptResult").unwrap().is_null());
            }
            assert!(!host.base.join("selected").exists());
        }
    }
}
#[test]
fn close_during_busy_owner_rejects_old_published_view() {
    let host = GracefulBridge(host("slow"));
    open(&host);
    click(&host, "choose");
    wait_exists(&host.base.join("ready"));
    view(&host, |r| r["busy"] == true);
    actions::probe(&host, "schedule-result-close");
    let delay = host.rpc(&json!({"op":"native.delay","executionBarrier":true}));
    assert_eq!(delay["result"]["newIntentDuringDelay"], true);
    let path = host.profile.join("result-close-barrier.json");
    wait_exists(&path);
    let proof: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(proof["rejected"], true);
    assert_eq!(proof["windowShown"], false);
    host.wait(|s| s["scriptsRunning"] == 0);
    host.settled(|_| true);
    assert!(actions::probe(&host, "read")
        .get("scriptResult")
        .unwrap()
        .is_null());
}
#[test]
fn config_change_after_admission_keeps_concrete_usage_warning() {
    let host = GracefulBridge(host("slow"));
    open(&host);
    click(&host, "choose");
    wait_exists(&host.base.join("ready"));
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    command["target"] = json!("/usr/bin/printf");
    command["arguments"] = json!(["changed"]);
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    fs::write(host.base.join("release"), "go").unwrap();
    let v = view(&host, |r| r["detail_visible"] == true);
    assert!(v["scriptResult"]["error"]
        .as_str()
        .unwrap()
        .contains("configuration"));
    assert_eq!(v["scriptResult"]["detail"], "Late report");
}
#[test]
fn light_dark_empty_and_long_labels_render() {
    for theme in ["light", "dark"] {
        let host = host_width("close", Some("600"));
        let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
        config["theme"] = json!(theme);
        host.replace(&config, None);
        host.rpc(&json!({"op":"reload"}));
        open(&host);
        capture(&host, &format!("choices-{theme}"));
        host.probe("query", json!({"query":"no result"}));
        view(&host, |r| r["rows"].as_array().unwrap().is_empty());
        let v = capture(&host, &format!("empty-{theme}"));
        assert!(v["screenText"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "No results"));
    }
}
fn special_host(mode: &str) -> Bridge {
    let host = host("close");
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let c = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    c["arguments"] = json!(["rich-special", mode, "{input}"]);
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
    host
}
#[test]
fn failure_can_add_first_readable_detail_without_replacing_choices() {
    let host = special_host("detail-failure");
    let old = open(&host)["scriptResult"].clone();
    assert!(old.get("detail").unwrap().is_null());
    assert!(!actions::probe(&host, "read")["resultPanel"]["buttons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "Details ⌘D"));
    click(&host, "choose");
    let v = view(&host, |r| !r["detail"].is_null());
    assert_eq!(
        v["scriptResult"].get("token").unwrap(),
        old.get("token").unwrap()
    );
    assert!(actions::probe(&host, "read")["resultPanel"]["buttons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "Details ⌘D"));
    click(&host, "details");
    assert_eq!(
        view(&host, |r| r["detail_visible"] == true)["resultPanel"]["detail"],
        "New failure report"
    );
}
#[test]
fn empty_choices_and_blank_labels_keep_accessibility() {
    let host = special_host("empty");
    let v = open(&host);
    let r = &v["scriptResult"];
    assert_eq!(r["title"], "Execution Fixture");
    assert!(v["accessibilityText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "Search choices"));
    assert_eq!(r["rows"], json!([]));
    assert!(r.get("selection").unwrap().is_null());
    assert!(v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "No results"));
    let old = r.get("token").unwrap().clone();
    actions::key(&host, 36, 0, "\r", false);
    host.settled(|_| true);
    assert_eq!(
        view(&host, |_| true)["scriptResult"].get("token").unwrap(),
        &old
    );
    click(&host, "reload");
    assert_ne!(
        view(&host, |r| r.get("token").unwrap() != &old)["scriptResult"]
            .get("token")
            .unwrap(),
        &old
    );
    capture(&host, "blank-empty-labels");
}
#[test]
fn actual_process_failure_without_envelope_keeps_error_and_choices() {
    let host = special_host("process-failure");
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    click(&host, "choose");
    let v = view(&host, |r| {
        r["error"].as_str().is_some_and(|s| !s.is_empty())
    });
    assert_eq!(v["scriptResult"].get("token").unwrap(), &old);
    assert!(v["scriptResult"]["error"]
        .as_str()
        .unwrap()
        .contains("status 7"));
    assert_eq!(v["scriptResult"]["busy"], false);
    let logs = host.rpc(&json!({"op":"logs"}));
    assert!(logs["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["phase"] == "command.finish")
        .any(|e| e["exitStatus"] == 7 && e["success"] == false));
}
fn replace_args(host: &Bridge, args: Value, prompt: Option<&str>) {
    let mut config: Value = serde_json::from_slice(&fs::read(&host.config).unwrap()).unwrap();
    let command = config["commands"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == "script.fixture")
        .unwrap();
    command["arguments"] = args;
    if let Some(prompt) = prompt {
        command["inputPrompt"] = json!(prompt);
    }
    host.replace(&config, None);
    assert_eq!(host.rpc(&json!({"op":"reload"}))["ok"], true);
}
#[test]
fn failure_report_back_retains_ordinary_input_draft() {
    let host = host("close");
    replace_args(&host, json!(["rich-input-failure"]), Some("Message"));
    actions::show(&host, "runfixture");
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
    actions::key(&host, 36, 0, "\r", false);
    actions::view(&host, |v| v["runInputInline"] == true);
    actions::key(&host, 36, actions::CMD, "\r", false);
    let editor = actions::view(&host, |v| {
        v["formVisible"] == true
            && v["screenText"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s == "Run ⌘↵")
    });
    let token = editor.get("editorToken").unwrap().clone();
    host.probe("run-input-type", json!({"text":"PRIVATE input"}));
    actions::probe(&host, "save-editor");
    host.wait(|s| s["scriptsRunning"] == 0);
    let v = view(&host, |r| r["detail_visible"] == true);
    assert_eq!(v.get("editorToken").unwrap(), &token);
    capture(&host, "input-failure-report");
    click(&host, "close");
    host.settled(|_| true);
    let v = actions::probe(&host, "read");
    assert_eq!(v.get("editorToken").unwrap(), &token);
    assert!(v["screenText"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "PRIVATE input"));
    capture(&host, "input-restored");
    let state = actions::probe(&host, "run-input-read");
    assert_eq!(state["focused"], true);
    assert_eq!(state["canUndo"], true);
    host.probe("run-input-type", json!({"text":" continued"}));
    actions::key(&host, 6, actions::CMD, "z", false);
    assert_eq!(
        actions::probe(&host, "run-input-read")["text"],
        "PRIVATE input"
    );
}
fn native_event_boundary(host: &Bridge) -> Value {
    let before = actions::probe(host, "rich-query-read")
        .get("groupingLevel")
        .unwrap()
        .clone();
    actions::probe(host, "rich-query-native-event");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let after = actions::probe(host, "rich-query-read")
            .get("groupingLevel")
            .unwrap()
            .clone();
        if after == 0 {
            return json!({"before":before,"after":after});
        }
        assert!(
            Instant::now() < deadline,
            "AppKit event did not close undo group"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn native_query_undo_survives_detail_and_back() {
    use sha2::{Digest, Sha256};
    let host = host("close");
    open(&host);
    host.probe("rich-query-type", json!({"text":"cafe"}));
    view(&host, |r| r["query"] == "cafe");
    let first = native_event_boundary(&host);
    assert_eq!(actions::probe(&host, "rich-query-read")["canUndo"], true);
    actions::key(&host, 2, actions::CMD, "d", false);
    let details = view(&host, |r| r["detail_visible"] == true);
    actions::key(&host, 53, 0, "", false);
    view(&host, |r| r["detail_visible"] == false);
    assert_eq!(actions::probe(&host, "rich-query-read")["canUndo"], true);
    host.probe("rich-query-type", json!({"text":" PLAN"}));
    view(&host, |r| r["query"] == "cafe PLAN");
    let second = native_event_boundary(&host);
    actions::key(&host, 6, actions::CMD, "z", false);
    view(&host, |r| r["query"] == "cafe");
    actions::key(&host, 6, actions::CMD, "z", false);
    view(&host, |r| r["query"] == "");
    actions::key(&host, 6, actions::CMD | actions::SHIFT, "z", false);
    view(&host, |r| r["query"] == "cafe");
    assert!(!host.base.join("selected").exists());
    if let Some(output) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let folder = std::path::PathBuf::from(output)
            .parent()
            .unwrap()
            .join("rich");
        fs::create_dir_all(&folder).unwrap();
        let digest = format!(
            "{:x}",
            Sha256::digest(fs::read(&host.settings.host).unwrap())
        );
        let evidence = json!({"first":first,"second":second,"roleGroupBefore":details["resultPanel"].get("queryGroupBeforeRole").unwrap(),"roleGroupAfter":details["resultPanel"].get("queryGroupAfterRole").unwrap(),"undo1":"cafe","undo2":"","redo":"cafe","binarySHA256":digest});
        fs::write(
            folder.join("query-undo-event-boundaries.json"),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn retired_result_ingress_cannot_suppress_new_initial_job() {
    let host = GracefulBridge(host("close"));
    let old = open(&host)["scriptResult"].get("token").unwrap().clone();
    replace_args(&host, json!(["rich-new-job", host.base]), None);
    actions::show(&host, "runfixture");
    assert_eq!(actions::probe(&host, "read")["windowShown"], false);
    actions::key(&host, 36, 0, "\r", false);
    wait_exists(&host.base.join("new-ready"));
    let epoch = host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]
        .get("queryEpoch")
        .unwrap()
        .clone();
    for name in ["details", "close", "choose"] {
        host.probe(
            "result-event",
            json!({"token":old,"epoch":epoch,"name":name,"id":"reused"}),
        );
    }
    host.probe(
        "result-query-event",
        json!({"token":old,"query":"retired query"}),
    );
    assert_eq!(
        host.rpc(&json!({"op":"status"}))["result"]["nativeUI"]
            .get("queryEpoch")
            .unwrap(),
        &epoch
    );
    fs::write(host.base.join("new-release"), "go").unwrap();
    wait_exists(&host.base.join("new-effect"));
    host.wait(|s| s["scriptsRunning"] == 0);
    let v = view(&host, |r| r["detail"] == "NEW JOB REPORT");
    assert_eq!(v["scriptResult"]["detail_visible"], true);
    assert_eq!(
        host.rpc(&json!({"op":"usage"}))["result"]["script.fixture"],
        2
    );
}
