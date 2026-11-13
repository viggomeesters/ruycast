//! Native script failure details preserve the owned input draft.
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
use serde_json::json;
use std::fs;
#[test]
fn native_provider_and_process_failure_details_keep_the_same_input_draft() {
    for (ok, status) in [(false, 0), (false, 7), (true, 7)] {
        let mut h = Bridge::new(Config::current(), false);
        let payload =
            json!({"ok":ok,"error":"Provider asks attention","detail":"PRIVATE failure detail"});
        let mut cfg = default_config();
        cfg["commands"].as_array_mut().unwrap().push(json!({"id":"script.fixture","title":"Execution Fixture","aliases":["runfixture"],"kind":"script","target":env!("CARGO_BIN_EXE_private-results-fixture"),"arguments":["fake",if status==7 {"exit7"} else {"write"},h.base.join("result-marker"),format!("{payload}\n")],"timeout":3,"inputPrompt":"Message"}));
        h.replace(&cfg, None);
        h.start();
        h.settled(|_| true);
        show(&h, "runfixture");
        assert_eq!(probe(&h, "read")["windowShown"], false);
        key(&h, 36, 0, "\r", false);
        view(&h, |v| v["runInputInline"] == true);
        key(&h, 36, CMD, "\r", false);
        let token = view(&h, |v| {
            v["formVisible"] == true
                && v["screenText"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("Run ⌘↵"))
        })["editorToken"]
            .clone();
        h.probe("run-input-type", json!({"text":"PRIVATE keep input"}));
        probe(&h, "save-editor");
        h.wait(|s| s["scriptsRunning"] == 0);
        let report = view(&h, |v| !v["scriptResult"].is_null());
        assert_eq!(report["resultPanel"]["selectable"], true);
        assert_eq!(report["resultPanel"]["editable"], false);
        h.probe("result-control", json!({"name":"close"}));
        let expected = if ok {
            "status 7"
        } else {
            "Provider asks attention"
        };
        view(&h, |v| {
            v["formVisible"] == true && text_contains(v, expected)
        });
        assert_eq!(probe(&h, "read")["editorToken"], token);
        let field = probe(&h, "run-input-read");
        assert_eq!(field["text"], "PRIVATE keep input");
        assert_eq!(field["focused"], true);
        assert_eq!(field["editable"], true);
        assert_eq!(field["canUndo"], true);
        h.probe("run-input-type", json!({"text":" continued"}));
        assert_eq!(
            probe(&h, "run-input-read")["text"],
            "PRIVATE keep input continued"
        );
        assert_eq!(h.rpc(&json!({"op":"usage"}))["result"], json!({}));
        assert!(!fs::read_to_string(h.profile.join("command-log.json"))
            .unwrap()
            .contains("PRIVATE"));
        h.capture(&format!(
            "script-result-failure-{}-{status}",
            if ok { "True" } else { "False" }
        ));
    }
}
