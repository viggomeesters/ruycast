//! Rich keyword clipboard and delayed cursor with the owned native SDK.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Owned directory and direct probe share bounded child support"
)]
#[path = "support/keyword_capture.rs"]
mod capture;
use capture::{config, event, invoke, keys, sdk};
use serde_json::{json, Value};
fn rich(text: &str, mode: &str, keyword: &str, before: Vec<Value>, after: Vec<Value>) -> Value {
    let mut c = config(text, keyword);
    c["commands"][0]["snippetOutput"] = json!(mode);
    let mut ops = vec![json!({"op":"config","value":c}), json!({"op":"refresh"})];
    ops.extend(before);
    ops.extend(keys(keyword));
    ops.extend(after);
    ops.push(json!({"op":"status"}));
    invoke(ops, Some("complete"), true)
}
fn text(value: &Value) -> String {
    if value.is_null() {
        return String::new();
    }
    let bytes = value
        .as_array()
        .unwrap()
        .iter()
        .map(|x| u8::try_from(x.as_u64().unwrap()).unwrap())
        .collect();
    String::from_utf8(bytes).unwrap()
}
#[test]
fn plain_leaves_existing_clipboard_intact() {
    let mut ops = vec![
        json!({"op":"config","value":config("HELLO","abc")}),
        json!({"op":"refresh"}),
        sdk("clipboard", json!({"value":"ORIGINAL"})),
    ];
    ops.extend(keys("abc"));
    let v = invoke(ops, Some("complete"), true);
    assert_eq!(text(&v["final"]["boardText"]), "ORIGINAL");
    assert_eq!(v["final"]["boardClears"], 0);
    assert_eq!(v["final"]["boardWrites"], 0);
}
#[test]
fn html_and_teams_prepare_item_then_erase_and_command_v() {
    for mode in ["html", "teams"] {
        let v = rich(
            "A😀[label](https://example.test)\nEnd",
            mode,
            "abc",
            vec![],
            vec![],
        );
        let state = &v["final"];
        assert_eq!(text(&state["boardText"]), "A😀label\nEnd");
        assert!(text(&state["boardHTML"]).contains("href=\"https://example.test\""));
        assert_eq!(state["pasteIssues"], 1);
        assert_eq!(text(&state["pasteText"]), text(&state["boardText"]));
        let out = state["output"].as_array().unwrap();
        assert_eq!(
            out.iter()
                .map(|e| e["key"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![51, 51, 51, 51, 9, 9]
        );
        assert_eq!(out[out.len() - 2]["flags"], 1 << 20);
        assert_eq!(out[out.len() - 1]["flags"], 1 << 20);
        assert_eq!(out[out.len() - 2]["units"], json!([118]));
        let last = v["steps"].as_array().unwrap().last().unwrap();
        assert_eq!(
            last.get("ownChange").unwrap(),
            state.get("boardCount").unwrap()
        );
        assert_eq!(last["ownActive"], false);
    }
}
#[test]
fn item_preparation_failure_never_clears_or_consumes() {
    for failure in ["item_create", "item_text", "item_html"] {
        let v = rich(
            "HELLO",
            "html",
            "abc",
            vec![
                sdk("clipboard", json!({"value":"OLD"})),
                sdk("failure", json!({"name":failure})),
            ],
            vec![],
        );
        let s = v["steps"].as_array().unwrap();
        assert_eq!(s[s.len() - 2]["consumed"], false);
        assert_eq!(v["final"]["boardClears"], 0);
        assert_eq!(v["final"]["posts"], 0);
        assert_eq!(text(&v["final"]["boardText"]), "OLD");
        assert_eq!(s.last().unwrap()["clipboardPartials"], 0);
    }
}
#[test]
fn clear_write_and_partial_failures_pass_trigger() {
    for failure in ["board_clear", "board_write", "board_partial"] {
        let v = rich(
            "HELLO",
            "html",
            "abc",
            vec![
                sdk("clipboard", json!({"value":"OLD"})),
                sdk("failure", json!({"name":failure})),
            ],
            vec![],
        );
        let s = v["steps"].as_array().unwrap();
        assert_eq!(s[s.len() - 2]["consumed"], false);
        assert_eq!(v["final"]["posts"], 0);
        assert_eq!(s.last().unwrap()["clipboardPartials"], 1);
        assert_eq!(s.last().unwrap()["ownActive"], false);
        if failure == "board_partial" {
            assert_eq!(text(&v["final"]["boardText"]), "HELLO");
            assert!(v["final"].get("boardHTML").unwrap().is_null());
            assert_eq!(
                s.last().unwrap().get("ownChange").unwrap(),
                v["final"].get("boardCount").unwrap()
            );
        }
    }
}
#[test]
fn external_replacement_is_not_owned_or_pasted() {
    for phase in [
        "board_cleared",
        "board_written",
        "board_matches",
        "board_ours",
    ] {
        let mut before = vec![json!({"op":"hook","at":phase,"action":"external"})];
        if phase == "board_ours" {
            before.insert(0, sdk("failure", json!({"name":"board_matches"})));
        }
        let v = rich("HELLO", "html", "abc", before, vec![]);
        let s = v["steps"].as_array().unwrap();
        assert_eq!(s[s.len() - 2]["consumed"], false, "{phase}");
        assert_eq!(text(&v["final"]["boardText"]), "EXTERNAL");
        assert_eq!(v["final"]["posts"], 0);
        assert_ne!(
            s.last().unwrap().get("ownChange").unwrap(),
            v["final"].get("boardCount").unwrap()
        );
    }
}
#[test]
fn foreign_html_with_same_plain_text_is_not_suppressed() {
    let v = rich(
        "HELLO",
        "html",
        "abc",
        vec![json!({"op":"hook","at":"board_written","action":"external-html"})],
        vec![],
    );
    let s = v["steps"].as_array().unwrap();
    assert_eq!(s[s.len() - 2]["consumed"], false);
    assert_eq!(text(&v["final"]["boardText"]), "HELLO");
    assert_eq!(text(&v["final"]["boardHTML"]), "EXTERNAL HTML");
    assert_ne!(
        s.last().unwrap().get("ownChange").unwrap(),
        v["final"].get("boardCount").unwrap()
    );
}
#[test]
fn clipboard_only_cancel_drop_or_panic_never_swallows_trigger() {
    for phase in [
        "item_create",
        "board_clear",
        "board_cleared",
        "board_write",
        "board_written",
        "board_matches",
    ] {
        for action in ["stop", "record", "drop", "disable", "panic"] {
            let v = rich(
                "HELLO",
                "html",
                "abc",
                vec![json!({"op":"hook","at":phase,"action":action})],
                vec![],
            );
            let s = v["steps"].as_array().unwrap();
            assert_eq!(s[s.len() - 2]["consumed"], false, "{phase} {action}");
            assert_eq!(v["final"]["posts"], 0, "{phase} {action}");
            assert_eq!(s.last().unwrap()["ownActive"], false);
        }
    }
}
#[test]
fn external_board_change_after_first_erase_stops_before_paste() {
    let v = rich(
        "HELLO",
        "html",
        "abc",
        vec![json!({"op":"hook","at":"post:down","action":"external"})],
        vec![],
    );
    let s = v["steps"].as_array().unwrap();
    assert_eq!(s[s.len() - 2]["consumed"], true);
    assert_eq!(v["final"]["pasteIssues"], 0);
    let out = v["final"]["output"].as_array().unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out.last().unwrap()["down"], false);
}
#[test]
fn entered_paste_down_retains_cleared_up_on_revoke_and_panic() {
    for action in ["stop", "record", "drop", "disable", "panic", "reenter"] {
        let v = rich(
            "HELLO",
            "html",
            "x",
            vec![json!({"op":"hook","at":"post:down","action":action})],
            vec![],
        );
        let s = v["steps"].as_array().unwrap();
        assert_eq!(s[s.len() - 2]["consumed"], true);
        let out = v["final"]["output"].as_array().unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["key"], 9);
        assert_eq!(out[0]["units"], json!([118]));
        assert_eq!(out.last().unwrap()["units"], json!([]));
        assert_eq!(out.last().unwrap()["flags"], 0);
    }
}
fn cursor_down(v: &Value) -> usize {
    v["final"]["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["key"] == 123 && e["down"] == true)
        .count()
}
#[test]
fn rich_cursor_is_delayed_and_not_sent_with_paste() {
    let v = rich(
        "A{cursor}BC",
        "html",
        "abc",
        vec![],
        vec![
            json!({"op":"status"}),
            json!({"op":"wait","ms":100}),
            json!({"op":"pump"}),
        ],
    );
    let s = v["steps"].as_array().unwrap();
    assert_eq!(s[s.len() - 4]["cursor"], true);
    assert_eq!(s[s.len() - 2]["pendingCursor"], false);
    assert_eq!(cursor_down(&v), 2);
}
#[test]
fn cursor_cancelled_by_input_record_hyper_activation_generation_and_target() {
    for change in [
        event("z", json!({})),
        json!({"op":"gate","epoch":1}),
        json!({"op":"gate","hyperEpoch":1}),
        json!({"op":"gate","activationEpoch":1}),
        json!({"op":"gate","generation":2}),
        json!({"op":"gate","stageEpoch":1}),
        sdk("environment", json!({"app":2})),
        sdk("environment", json!({"flags":8})),
        sdk("environment", json!({"flags":16})),
        sdk("environment", json!({"flags":4096})),
        json!({"op":"stop"}),
    ] {
        let v = rich(
            "A{cursor}BC",
            "html",
            "abc",
            vec![],
            vec![
                change.clone(),
                json!({"op":"wait","ms":100}),
                json!({"op":"pump"}),
            ],
        );
        assert_eq!(cursor_down(&v), 0, "{change}");
    }
}
#[test]
fn sampled_loss_or_pending_rollback_never_revives_old_cursor() {
    for mut changes in [
        vec![
            sdk("failure", json!({"name":"tap_enabled"})),
            json!({"op":"refresh"}),
            sdk("failure", json!({})),
            json!({"op":"refresh"}),
        ],
        vec![
            json!({"op":"gate","generation":0}),
            json!({"op":"refresh"}),
            json!({"op":"gate","generation":1}),
            json!({"op":"refresh"}),
        ],
        vec![json!({"op":"gate","stageEpoch":1})],
    ] {
        changes.extend([json!({"op":"wait","ms":100}), json!({"op":"pump"})]);
        let v = rich("A{cursor}BC", "html", "abc", vec![], changes);
        assert_eq!(cursor_down(&v), 0);
    }
}
#[test]
fn own_generated_input_and_unrelated_copy_do_not_cancel_paste_cursor() {
    for change in [
        event("z", json!({"tag":0x5255535443415354u64})),
        sdk("board-external", json!({"value":"LATER COPY"})),
    ] {
        let v = rich(
            "A{cursor}BC",
            "html",
            "abc",
            vec![],
            vec![
                change.clone(),
                json!({"op":"wait","ms":100}),
                json!({"op":"pump"}),
            ],
        );
        assert_eq!(cursor_down(&v), 2, "{change}");
    }
}
#[test]
fn large_cursor_runs_bounded_batches_and_allows_cancellation() {
    let v = rich(
        &format!("{{cursor}}{}", "x".repeat(20000)),
        "html",
        "x",
        vec![],
        vec![
            json!({"op":"wait","ms":100}),
            json!({"op":"pump"}),
            event("z", json!({})),
            json!({"op":"pump"}),
        ],
    );
    let arrows = cursor_down(&v);
    assert!(arrows > 0 && arrows <= 32);
    let s = v["steps"].as_array().unwrap();
    assert_eq!(s[s.len() - 2]["pendingCursor"], false);
}
#[test]
fn cursor_panic_balances_keyup_and_poison_only_keyword_component() {
    let v = rich(
        "A{cursor}BC",
        "html",
        "abc",
        vec![],
        vec![
            json!({"op":"hook","at":"post:down","action":"panic"}),
            json!({"op":"wait","ms":100}),
            json!({"op":"pump"}),
        ],
    );
    let out = v["final"]["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["key"] == 123)
        .collect::<Vec<_>>();
    assert_eq!(out.len(), 2);
    assert_eq!(out.last().unwrap()["down"], false);
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap()["status"],
        "Keyword callback failed"
    );
}
