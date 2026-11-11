//! Original keyword-native process scenarios, with the actual Rust SDK/probe.
#![cfg(feature = "test-tools")]
#[path = "support/keyword_native.rs"]
mod keyword_native;
use keyword_native::{invoke, run};
use serde_json::{json, Value};
fn last(value: &Value) -> &Value {
    value["steps"].as_array().unwrap().last().unwrap()
}

#[test]
fn complete_table_required_before_any_real_discovery() {
    for lib in [None, Some("libkeyword_missing_value_type.dylib")] {
        let v = invoke(json!([{"op":"front"}]), lib, false);
        assert!(v["error"]
            .as_str()
            .unwrap()
            .contains("Complete keyword SDK fixture"));
    }
}
#[test]
fn mixed_backend_refuses_anchor_and_effects_before_sdk_calls() {
    let v = run(json!([{"op":"front"},{"op":"state"},{"op":"mixed-backend"},{"op":"state"}]));
    assert_eq!(v["steps"][1], v["steps"][3]);
    assert_eq!(
        v["steps"][2],
        json!({"plain":false,"rich":false,"cursor":false,"keyboardPossible":false,"clipboardPossible":false,"anchorRejected":true})
    );
}
#[test]
fn retained_instance_not_pid_or_bundle_replacement() {
    let v = run(
        json!([{"op":"front"},{"op":"current"},{"op":"env","app":2},{"op":"current"},{"op":"env","pid":43},{"op":"current"},{"op":"env","flags":4},{"op":"current"},{"op":"activate"}]),
    );
    assert_eq!(v["steps"][1]["current"], true);
    for i in [3, 5, 7] {
        assert_eq!(v["steps"][i]["current"], false);
    }
    assert_eq!(v["steps"][8]["activated"], false);
}
#[test]
fn missing_target_and_bundle() {
    let v =
        run(json!([{"op":"env","flags":1},{"op":"front"},{"op":"env","flags":2},{"op":"front"}]));
    assert!(v["steps"][1]["target"].is_null());
    assert!(v["steps"][3]["target"]["bundle"].is_null());
}
#[test]
fn plain_secure_checks_never_read_editor_value() {
    for (flags, safe) in [
        (0, true),
        (8, false),
        (16, false),
        (4096, false),
        (32, true),
        (64, true),
        (128, true),
    ] {
        let v = run(json!([{"op":"env","flags":flags},{"op":"safe"}]));
        assert_eq!(last(&v), &json!({"safe":safe}));
        assert_eq!(v["final"]["valueReads"], 0);
    }
}
#[test]
fn explicit_timeout_failure_stops_before_copy() {
    for (name, count) in [("timeout:system", 0), ("timeout:element", 1)] {
        let v = run(json!([{"op":"failure","name":name},{"op":"safe"}]));
        assert_eq!(last(&v), &json!({"error":"TimeoutBudget"}));
        assert_eq!(v["final"]["copies"], count);
    }
}
#[test]
fn anchor_identity_value_and_range_cannot_degrade() {
    for change in [
        json!({"op":"env","element":2}),
        json!({"op":"env","flags":32}),
        json!({"op":"env","flags":64}),
        json!({"op":"env","flags":256}),
        json!({"op":"env","flags":512}),
        json!({"op":"env","flags":1024}),
        json!({"op":"text","value":"abd"}),
        json!({"op":"text","value":"abc","at":2}),
    ] {
        let v = run(
            json!([{"op":"text","value":"abc"},{"op":"front"},{"op":"anchor"},{"op":"verify"},change,{"op":"verify"}]),
        );
        assert_eq!(v["steps"][2], json!({"ok":true}));
        assert_eq!(v["steps"][3], json!({"continuity":"pinned"}));
        assert_eq!(last(&v), &json!({"continuity":"changed"}), "{change}");
    }
}
#[test]
fn initial_missing_element_requires_event_guards() {
    let v = run(json!([{"op":"env","flags":32},{"op":"front"},{"op":"anchor"},{"op":"verify"}]));
    assert_eq!(last(&v), &json!({"continuity":"event-guards-required"}));
}
#[test]
fn initial_partial_metadata_is_preserved_without_new_requirement() {
    for flags in [256, 512, 1024, 256 | 1024] {
        let v = run(
            json!([{"op":"text","value":"abc"},{"op":"env","flags":flags},{"op":"front"},{"op":"anchor"},{"op":"env"},{"op":"verify"}]),
        );
        assert_eq!(last(&v), &json!({"continuity":"pinned"}), "{flags}");
    }
}
#[test]
fn range_validation_and_exact_utf16_scalar_boundary() {
    for (at, length) in [(-1, 0), (4, 0), (2, 1)] {
        let v = run(
            json!([{"op":"text","value":"abc","at":at,"length":length},{"op":"front"},{"op":"anchor"}]),
        );
        assert_eq!(last(&v), &json!({"error":"TargetChanged"}));
    }
    let v = run(
        json!([{"op":"text","units":[0xd83d,0xde00],"at":1},{"op":"front"},{"op":"anchor","keyword":"\u{fffd}"}]),
    );
    assert_eq!(last(&v), &json!({"error":"TargetChanged"}));
}
#[test]
fn canonical_value_equality_but_original_utf16_range() {
    let v = run(
        json!([{"op":"text","value":"abce\u{301}","at":3},{"op":"front"},{"op":"anchor"},{"op":"text","value":"abcé","at":3},{"op":"verify"}]),
    );
    assert_eq!(v["steps"][2], json!({"ok":true}));
    assert_eq!(last(&v), &json!({"continuity":"pinned"}));
}
#[test]
fn canonical_editor_value_cannot_move_keyword_behind_same_utf16_caret() {
    let v = run(
        json!([{"op":"text","value":"e\u{301}abcZZ","at":5},{"op":"front"},{"op":"anchor"},{"op":"text","value":"éabcZZ","at":5},{"op":"verify"}]),
    );
    assert_eq!(v["steps"][2], json!({"ok":true}));
    assert_eq!(last(&v), &json!({"continuity":"changed"}));
}
#[test]
fn ax_factory_failures_do_not_become_metadata_fallback() {
    let v = run(json!([{"op":"failure","name":"system"},{"op":"safe"}]));
    assert_eq!(last(&v), &json!({"error":"InvalidData"}));
    let v = run(
        json!([{"op":"text","value":"abc"},{"op":"front"},{"op":"failure","name":"ax_app"},{"op":"anchor"}]),
    );
    assert_eq!(last(&v), &json!({"error":"InvalidData"}));
}
#[test]
fn error_outvalues_retire_and_known_value_never_becomes_unknown() {
    let v = run(
        json!([{"op":"text","value":"abc"},{"op":"front"},{"op":"anchor"},{"op":"env","flags":2048},{"op":"failure","name":"copy:3"},{"op":"verify"}]),
    );
    assert_eq!(last(&v), &json!({"continuity":"changed"}));
}
#[test]
fn reentrant_revocation_releases_every_owned_intermediate() {
    for phase in [
        "front",
        "app_pid",
        "app_bundle",
        "type_id",
        "string_type",
        "string_len",
        "string_units",
        "release",
    ] {
        let v = run(json!([{"op":"revoke","at":phase},{"op":"front"}]));
        assert_eq!(last(&v), &json!({"error":"Revoked"}), "{phase}");
    }
    for phase in ["app_equal", "app_release", "app_terminated"] {
        let v = run(json!([{"op":"front"},{"op":"revoke","at":phase},{"op":"current"}]));
        assert_eq!(last(&v), &json!({"error":"Revoked"}), "{phase}");
    }
    for phase in [
        "system",
        "timeout:system",
        "copy:0",
        "ax_type",
        "timeout:element",
        "copy:1",
        "string_units",
        "release",
        "trusted",
        "secure",
    ] {
        let v = run(json!([{"op":"revoke","at":phase},{"op":"safe"}]));
        assert_eq!(last(&v), &json!({"error":"Revoked"}), "{phase}");
    }
}
#[test]
fn native_permission_change_during_ax_copy_is_rechecked() {
    for flags in [8, 4096] {
        for phase in ["copy:0", "copy:1", "string_units", "release"] {
            let v = run(json!([{"op":"change","at":phase,"flags":flags},{"op":"safe"}]));
            assert_eq!(last(&v), &json!({"safe":false}), "{flags} {phase}");
        }
        let v = run(
            json!([{"op":"text","value":"abc"},{"op":"front"},{"op":"anchor"},{"op":"change","at":"copy:3","flags":flags},{"op":"verify"}]),
        );
        assert_eq!(last(&v), &json!({"error":"UnsafeInput"}));
    }
}
#[test]
fn sensitive_value_is_not_read_after_observed_native_permission_change() {
    for flags in [8, 16, 4096] {
        for phase in ["copy:2", "value_range"] {
            let v = run(
                json!([{"op":"text","value":"abc"},{"op":"front"},{"op":"change","at":phase,"flags":flags},{"op":"anchor"}]),
            );
            assert_eq!(last(&v), &json!({"error":"UnsafeInput"}));
            assert_eq!(v["final"]["valueReads"], 0, "{flags} {phase} {v}");
            let v = run(
                json!([{"op":"text","value":"abc"},{"op":"front"},{"op":"anchor"},{"op":"change","at":phase,"flags":flags},{"op":"verify"}]),
            );
            assert_eq!(last(&v), &json!({"error":"UnsafeInput"}));
            assert_eq!(v["final"]["valueReads"], 1, "{flags} {phase} {v}");
        }
    }
}
#[test]
fn native_app_switch_during_anchor_value_read_is_rejected() {
    let v = run(
        json!([{"op":"text","value":"abc"},{"op":"front"},{"op":"change","at":"copy:3","app":2},{"op":"anchor"}]),
    );
    assert_eq!(last(&v), &json!({"error":"TargetChanged"}));
    let v = run(
        json!([{"op":"text","value":"abc"},{"op":"front"},{"op":"anchor"},{"op":"change","at":"copy:3","app":2},{"op":"verify"}]),
    );
    assert_eq!(last(&v), &json!({"continuity":"changed"}));
}
#[test]
fn negative_native_string_length_fails_closed() {
    let v = run(json!([{"op":"failure","name":"string_len"},{"op":"front"}]));
    assert_eq!(last(&v), &json!({"error":"InvalidData"}));
}
#[test]
fn anchor_contents_never_enter_status_or_errors() {
    let secret = "OWN-AX-VALUE-NEVER-LEAVES-PROBE abc";
    let v =
        run(json!([{"op":"text","value":secret},{"op":"front"},{"op":"anchor"},{"op":"verify"}]));
    assert!(!serde_json::to_string(&v).unwrap().contains(secret));
    assert_eq!(last(&v), &json!({"continuity":"pinned"}));
}
#[test]
fn malformed_known_ax_values_cannot_collapse_to_same_certificate() {
    for (units, at) in [
        (vec![0xd800, 97, 98, 99], 4),
        (vec![97, 98, 99, 0xd800], 3),
        (vec![0xdc00, 97, 98, 99], 4),
        (vec![97, 98, 99, 0xdc00], 3),
    ] {
        let v = run(json!([{"op":"text","units":units,"at":at},{"op":"front"},{"op":"anchor"}]));
        assert_eq!(last(&v), &json!({"error":"InvalidData"}));
    }
    for (text, units, at) in [
        ("Xabc", vec![0xd801, 97, 98, 99], 4),
        ("abcX", vec![97, 98, 99, 0xd801], 3),
    ] {
        let v = run(
            json!([{"op":"text","value":text,"at":at},{"op":"front"},{"op":"anchor"},{"op":"text","units":units,"at":at},{"op":"verify"}]),
        );
        assert_eq!(last(&v), &json!({"error":"InvalidData"}));
    }
}
#[test]
fn valid_pair_across_stack_chunks_and_malformed_edges() {
    let text = "a".repeat(4095) + "😀abc";
    let v = run(json!([{"op":"text","value":text},{"op":"front"},{"op":"anchor"},{"op":"verify"}]));
    assert_eq!(last(&v), &json!({"continuity":"pinned"}));
    for units in [
        [vec![97; 4095], vec![0xd800, 98, 97, 98, 99]].concat(),
        [vec![97; 4096], vec![0xdc00, 97, 98, 99]].concat(),
        [vec![97, 98, 99], vec![97; 4092], vec![0xd800]].concat(),
    ] {
        let v = run(json!([{"op":"text","units":units},{"op":"front"},{"op":"anchor"}]));
        assert_eq!(last(&v), &json!({"error":"InvalidData"}));
    }
}
#[test]
fn large_valid_ax_snapshot_and_revoke_during_value_chunk() {
    let value = "a".repeat(1_000_000) + "abc";
    let v =
        run(json!([{"op":"text","value":value},{"op":"front"},{"op":"anchor"},{"op":"verify"}]));
    assert_eq!(last(&v), &json!({"continuity":"pinned"}));
    assert!(v["final"]["valueUnitReads"].as_u64().unwrap() > 400);
    let v = run(
        json!([{"op":"text","value":value},{"op":"front"},{"op":"revoke","at":"value_units"},{"op":"anchor"}]),
    );
    assert_eq!(last(&v), &json!({"error":"Revoked"}));
    assert_eq!(v["final"]["valueUnitReads"], 1);
}
#[test]
fn general_text_keeps_lossy_semantics_across_stack_chunks() {
    for units in [
        vec![0xd800],
        vec![0xdc00],
        vec![0xd800, 97, 0xdc00],
        vec![0xd800, 0xd83d, 0xde00],
        [vec![97; 4095], vec![0xd83d, 0xde00, 0xdc00]].concat(),
    ] {
        let expected = String::from_utf16_lossy(&units);
        let v = run(json!([{"op":"bundle-units","units":units},{"op":"front"}]));
        assert_eq!(last(&v)["target"]["bundle"], expected);
    }
}
#[test]
fn unaddressable_known_value_length_refuses_before_value_units() {
    let v = run(
        json!([{"op":"front"},{"op":"text","value":"abc"},{"op":"failure","name":"value_len_huge"},{"op":"anchor"}]),
    );
    assert_eq!(last(&v), &json!({"error":"InvalidData"}));
    assert_eq!(v["final"]["valueUnitReads"], 0);
}
