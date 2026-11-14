//! Rich keyword clipboard and delayed cursor with the owned native SDK.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Owned directory and direct probe share bounded child support"
)]
#[path = "support/keyword_capture.rs"]
mod capture;
use capture::{config, invoke, keys, sdk};
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
