//! Owned native keyword filter/effects with the real Rust probe and SDK fixtures.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Owned directory and direct probe share bounded child support"
)]
#[path = "support/keyword_capture.rs"]
mod capture;
use capture::*;
use serde_json::{json, Value};
#[test]
fn complete_sdk_missing_fails_before_real_discovery() {
    for lib in [None, Some("missing")] {
        assert!(invoke(vec![], lib, false)["error"]
            .as_str()
            .unwrap()
            .contains("Complete keyword SDK fixture"));
    }
}
#[test]
fn actual_tap_plain_replacement_is_synchronous_and_tagged() {
    let v = run_keys("abc", config("HELLO", "abc"), vec![]);
    let consumed = v["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s.get("consumed"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(consumed, vec![json!(false), json!(false), json!(true)]);
    assert_eq!(inserted(&v["final"]), "HELLO");
    let out = v["final"]["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["key"].as_u64().unwrap(), e["down"].as_bool().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        out,
        vec![
            (51, true),
            (51, false),
            (51, true),
            (51, false),
            (0, true),
            (0, false)
        ]
    );
    assert_eq!(v["final"]["valueReads"], 0);
}
#[test]
fn scalar_chunks_and_cursor_steps() {
    let text = format!("{}éZ", "😀".repeat(9));
    let v = run_keys(
        "abc",
        config(&format!("{text}{{cursor}}END"), "abc"),
        vec![],
    );
    let out = v["final"]["output"].as_array().unwrap();
    assert!(out
        .iter()
        .all(|e| e["units"].as_array().unwrap().len() <= 16));
    assert_eq!(inserted(&v["final"]), format!("{text}END"));
    assert_eq!(
        out.iter()
            .filter(|e| e["key"] == 123 && e["down"] == true)
            .count(),
        3
    );
}
#[test]
fn empty_replacement_consumes_one_key_without_posts() {
    let v = run_keys("x", config("{cursor}", "x"), vec![]);
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap()["consumed"],
        true
    );
    assert_eq!(v["final"]["posts"], 0);
}
#[test]
fn clipboard_only_on_actual_matching_demand() {
    let v = run_keys(
        "ab",
        config("{clipboard}", "abc"),
        vec![sdk("clipboard", json!({"value":"OWN CLIP"}))],
    );
    assert_eq!(v["final"]["clipboardReads"], 0);
    let v = run_keys(
        "abc",
        config("{clipboard}", "abc"),
        vec![sdk("clipboard", json!({"value":"OWN CLIP"}))],
    );
    assert_eq!(v["final"]["clipboardReads"], 1);
    assert_eq!(inserted(&v["final"]), "OWN CLIP");
    let v = run_keys("abc", config("HELLO", "abc"), vec![]);
    assert_eq!(v["final"]["clipboardReads"], 0);
}
#[test]
fn modifiers_repeats_navigation_mouse_empty_and_oversized_unicode_reset() {
    for middle in [
        event("x", json!({"flags":1<<20})),
        event("x", json!({"flags":1<<18})),
        event("x", json!({"flags":1<<19})),
        event("x", json!({"repeat":1})),
        event("", json!({"key":123})),
        event("", json!({"type":1})),
        event("", json!({"type":3})),
        event("", json!({})),
        event(&"x".repeat(17), json!({})),
    ] {
        let mut ops = vec![
            json!({"op":"config","value":config("HELLO","abc")}),
            json!({"op":"refresh"}),
        ];
        ops.extend(keys("ab"));
        ops.push(middle.clone());
        ops.push(event("c", json!({})));
        let v = invoke(ops, Some("complete"), true);
        assert_eq!(
            v["steps"].as_array().unwrap().last().unwrap()["consumed"],
            false,
            "{middle}"
        );
        assert_eq!(v["final"]["posts"], 0);
    }
}
#[test]
fn backspace_and_generated_input() {
    for middle in [
        vec![event("z", json!({})), event("", json!({"key":51}))],
        vec![event("z", json!({"tag":0x5255535443415354u64}))],
    ] {
        let mut ops = vec![
            json!({"op":"config","value":config("HELLO","abc")}),
            json!({"op":"refresh"}),
        ];
        ops.extend(keys("ab"));
        ops.extend(middle);
        ops.push(event("c", json!({})));
        let v = invoke(ops, Some("complete"), true);
        assert_eq!(
            v["steps"].as_array().unwrap().last().unwrap()["consumed"],
            true
        );
    }
}
fn started() -> Vec<Value> {
    vec![
        json!({"op":"config","value":config("HELLO","abc")}),
        json!({"op":"refresh"}),
    ]
}
fn after_prefix(middle: Vec<Value>) -> Value {
    let mut ops = started();
    ops.extend(keys("ab"));
    ops.extend(middle);
    ops.push(event("c", json!({})));
    ops.push(event("", json!({})));
    ops.extend(keys("abc"));
    invoke(ops, Some("complete"), true)
}
#[test]
fn eventless_recording_and_hyper_episodes_invalidate_prefix() {
    for middle in [
        vec![
            json!({"op":"gate","blocked":true,"epoch":1}),
            json!({"op":"gate","blocked":false,"epoch":2}),
        ],
        vec![
            json!({"op":"gate","hyperHeld":true,"hyperEpoch":1}),
            json!({"op":"gate","hyperHeld":false,"hyperEpoch":2}),
        ],
    ] {
        let v = after_prefix(middle);
        let s = v["steps"].as_array().unwrap();
        assert_eq!(s[s.len() - 5]["consumed"], false);
        assert_eq!(s.last().unwrap()["consumed"], true);
    }
}
#[test]
fn native_target_instance_switch_resets_even_when_pid_reused() {
    let v = after_prefix(vec![sdk("environment", json!({"app":2}))]);
    let s = v["steps"].as_array().unwrap();
    assert_eq!(s[s.len() - 5]["consumed"], false);
    assert_eq!(s.last().unwrap()["consumed"], true);
}
#[test]
fn secure_permission_and_own_app_no_text_read_or_effect() {
    for env in [
        json!({"flags":8}),
        json!({"flags":16}),
        json!({"flags":4096}),
        json!({"pid":"self"}),
    ] {
        let v = run_keys("abc", config("HELLO", "abc"), vec![sdk("environment", env)]);
        assert_eq!(v["final"]["posts"], 0);
        assert_eq!(v["final"]["valueReads"], 0);
    }
}
#[test]
fn canonical_excluded_app_bundle() {
    let mut c = config("HELLO", "abc");
    c["clipboard"]["excludedApps"] = json!(["own.é"]);
    let v = run_keys("abc", c, vec![sdk("bundle", json!({"value":"own.é"}))]);
    assert_eq!(v["final"]["posts"], 0);
}
#[test]
fn generation_replacement_resets_and_uses_new_template() {
    let v = after_prefix(vec![
        json!({"op":"config","generation":2,"value":config("NEW","abc")}),
        json!({"op":"refresh"}),
    ]);
    let s = v["steps"].as_array().unwrap();
    assert_eq!(s[s.len() - 5]["consumed"], false);
    assert_eq!(s.last().unwrap()["consumed"], true);
    assert_eq!(inserted(&v["final"]), "NEW");
}
#[test]
fn catalog_adoption_keeps_tap_but_revokes_old_prefix() {
    let mut changed = config("NEW", "abc");
    changed["theme"] = json!("dark");
    let mut ops = started();
    ops.extend(keys("ab"));
    ops.extend([
        json!({"op":"config","generation":2,"value":changed}),
        sdk("state", json!({})),
        event("c", json!({})),
        json!({"op":"refresh"}),
        event("", json!({})),
    ]);
    ops.extend(keys("abc"));
    let v = invoke(ops, Some("complete"), true);
    assert_eq!(v["steps"][5]["tapValid"], true);
    assert_eq!(v["steps"][5]["tapEnabled"], true);
    assert_eq!(v["steps"][6]["consumed"], false);
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap()["consumed"],
        true
    );
    assert_eq!(inserted(&v["final"]), "NEW");
    assert_eq!(v["final"]["tapCreates"], 1);
}
#[test]
fn pending_generation_keeps_tap_but_no_effects() {
    let mut ops = started();
    ops.extend(keys("ab"));
    ops.extend([
        json!({"op":"gate","generation":0}),
        json!({"op":"refresh"}),
        sdk("state", json!({})),
    ]);
    ops.extend(keys("abc"));
    ops.extend([
        json!({"op":"gate","generation":1}),
        json!({"op":"refresh"}),
        event("c", json!({})),
        event("", json!({})),
    ]);
    ops.extend(keys("abc"));
    let v = invoke(ops, Some("complete"), true);
    assert_eq!(v["steps"][6]["tapValid"], true);
    assert_eq!(v["steps"][6]["tapEnabled"], true);
    assert!(!v["steps"].as_array().unwrap()[7..10]
        .iter()
        .any(|s| s["consumed"] == true));
    assert_eq!(v["steps"][12]["consumed"], false);
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap()["consumed"],
        true
    );
    assert_eq!(v["final"]["tapCreates"], 1);
}
#[test]
fn sampled_tap_loss_cannot_restore_old_prefix() {
    let mut ops = started();
    ops.extend(keys("ab"));
    ops.extend([
        sdk("failure", json!({"name":"tap_enabled"})),
        json!({"op":"refresh"}),
        sdk("failure", json!({})),
        json!({"op":"refresh"}),
        event("c", json!({})),
        event("", json!({})),
    ]);
    ops.extend(keys("abc"));
    let v = invoke(ops, Some("complete"), true);
    assert_eq!(v["steps"][8]["consumed"], false);
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap()["consumed"],
        true
    );
}
#[test]
fn stage_episode_not_seen_by_refresh_still_revokes_prefix() {
    let v = after_prefix(vec![json!({"op":"gate","stageEpoch":1})]);
    assert_eq!(v["steps"][5]["consumed"], false);
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap()["consumed"],
        true
    );
    assert_eq!(v["final"]["tapCreates"], 1);
}
#[test]
fn off_and_paused_short_circuit_permission_app_discovery() {
    let mut disabled = config("HELLO", "abc");
    disabled["snippetExpansionEnabled"] = json!(false);
    for mut ops in [
        vec![json!({"op":"config","value":disabled})],
        vec![
            json!({"op":"config","value":config("HELLO","abc")}),
            json!({"op":"gate","running":false}),
        ],
    ] {
        ops.extend([
            sdk("environment", json!({"flags":8192|4096})),
            json!({"op":"refresh"}),
        ]);
        let v = invoke(ops, Some("complete"), true);
        assert_eq!(v["final"]["calls"], 0);
        assert_eq!(v["final"]["tapCreates"], 0);
    }
}
#[test]
fn interactive_trigger_passes_without_synchronous_effects() {
    let v = run_keys("abc", config("{name options=\"One, Two\"}", "abc"), vec![]);
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap()["consumed"],
        false
    );
    assert_eq!(v["final"]["posts"], 0);
}
#[test]
fn allocation_failure_before_first_down_does_not_consume() {
    for name in ["event:down", "event:up"] {
        let v = run_keys(
            "abc",
            config("HELLO", "abc"),
            vec![sdk("failure", json!({"name":name}))],
        );
        assert_eq!(
            v["steps"].as_array().unwrap().last().unwrap()["consumed"],
            false
        );
        assert_eq!(v["final"]["posts"], 0);
    }
}
#[test]
fn secure_change_after_entered_down_balances_without_further_text() {
    for flags in [8, 16, 4096] {
        let v = run_keys(
            "abc",
            config("HELLO", "abc"),
            vec![sdk("change", json!({"at":"post:down","flags":flags}))],
        );
        assert_eq!(
            v["steps"].as_array().unwrap().last().unwrap()["consumed"],
            true
        );
        let out = v["final"]["output"].as_array().unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out.last().unwrap()["units"], json!([]));
        assert_eq!(out.last().unwrap()["down"], false);
    }
}
#[test]
fn last_policy_call_cannot_redirect_first_post_to_another_app() {
    let mut ops = started();
    ops.extend(keys("ab"));
    ops.push(sdk("change", json!({"at":"copy:1","nth":4,"app":2})));
    ops.push(event("c", json!({})));
    let v = invoke(ops, Some("complete"), true);
    assert_eq!(
        v["steps"].as_array().unwrap().last().unwrap()["consumed"],
        false
    );
    assert_eq!(v["final"]["posts"], 0);
}
#[test]
fn recording_or_reentry_after_entered_down_cannot_lose_balance() {
    for action in ["record", "reenter", "drop", "panic", "stop", "disable"] {
        let v = run_keys(
            "abc",
            config("HELLO", "abc"),
            vec![json!({"op":"hook","at":"post:down","action":action})],
        );
        assert_eq!(
            v["steps"].as_array().unwrap().last().unwrap()["consumed"],
            true,
            "{action}"
        );
        let out = v["final"]["output"].as_array().unwrap();
        assert_eq!(out.len(), 2, "{action}");
        assert_eq!(out.last().unwrap()["down"], false);
        assert_eq!(out.last().unwrap()["units"], json!([]));
    }
}
#[test]
fn reentrant_disabled_config_never_reauthorizes_setup() {
    for phase in [
        "raycast",
        "trusted",
        "runloop",
        "common",
        "tap_create",
        "source",
        "add_source",
        "tap_enable",
        "tap_enabled",
    ] {
        let v = invoke(
            vec![
                json!({"op":"config","value":config("HELLO","abc")}),
                json!({"op":"hook","at":phase,"action":"disable"}),
                json!({"op":"refresh"}),
                sdk("state", json!({})),
            ],
            Some("complete"),
            true,
        );
        let s = v["steps"].as_array().unwrap();
        assert_ne!(s[s.len() - 2].get("status").unwrap(), "On", "{phase}");
        assert_eq!(s.last().unwrap()["tapValid"], false, "{phase}");
    }
}
#[test]
fn callback_timeout_revokes_and_refresh_recovers() {
    let mut ops = started();
    ops.extend(keys("ab"));
    ops.extend([
        event("", json!({"type":0xffff_ffffu64})),
        event("c", json!({})),
        json!({"op":"refresh"}),
    ]);
    ops.extend(keys("abc"));
    let v = invoke(ops, Some("complete"), true);
    let s = v["steps"].as_array().unwrap();
    assert_eq!(s[s.len() - 5]["consumed"], false);
    assert_eq!(s.last().unwrap()["consumed"], true);
}
#[test]
fn failed_invalidation_retains_callback_until_recovery() {
    let v = invoke(
        vec![
            json!({"op":"config","value":config("HELLO","abc")}),
            json!({"op":"refresh"}),
            sdk("failure", json!({"name":"invalidate"})),
            json!({"op":"stop"}),
            json!({"op":"drop"}),
            sdk("state", json!({})),
            sdk("failure", json!({})),
            json!({"op":"recover"}),
            sdk("state", json!({})),
        ],
        Some("complete"),
        true,
    );
    assert_eq!(v["steps"][3]["retired"], false);
    assert_eq!(v["steps"][5]["tapValid"], true);
    let s = v["steps"].as_array().unwrap();
    assert_eq!(s[s.len() - 2]["retired"], true);
    assert_eq!(s.last().unwrap()["tapValid"], false);
}
