//! Main-only snippet choice transition policy through the actual Rust probe.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};
fn command() -> Value {
    json!({"id":"own.é","title":"own.é","kind":"snippet","target":"{field name=\"name\" options=\"One,Two\"}","keyword":";own","aliases":[],"arguments":[]})
}
fn visible() -> Vec<Value> {
    vec![
        json!({"op":"advance"}),
        json!({"op":"capture"}),
        json!({"op":"present"}),
        json!({"op":"activation","sequence":1,"actor":"own"}),
        json!({"op":"visible"}),
    ]
}
fn run(steps: Vec<Value>) -> Vec<Value> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snippet-choice-probe"));
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars_os() {
        let s = key.to_string_lossy();
        if s.starts_with("RUYCAST_") || s.starts_with("RUSTCAST_") || s == "DYLD_INSERT_LIBRARIES" {
            cmd.env_remove(key);
        }
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            &serde_json::to_vec(&json!([[{"op":"policy","command":command(),"steps":steps}]]))
                .unwrap(),
        )
        .unwrap();
    let output = child_output::wait(child, Duration::from_secs(10));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    v["cases"][0][0].as_array().unwrap().clone()
}
fn extend(mut left: Vec<Value>, right: Vec<Value>) -> Vec<Value> {
    left.extend(right);
    left
}
#[test]
fn trigger_delay_and_single_use_apply() {
    let v = run(vec![
        json!({"op":"capture"}),
        json!({"op":"advance","ms":29}),
        json!({"op":"capture"}),
        json!({"op":"advance","ms":1}),
        json!({"op":"capture"}),
        json!({"op":"present"}),
        json!({"op":"visible"}),
        json!({"op":"return"}),
        json!({"op":"check"}),
        json!({"op":"apply"}),
        json!({"op":"advance"}),
        json!({"op":"check"}),
        json!({"op":"apply"}),
        json!({"op":"apply"}),
        json!({"op":"finish"}),
        json!({"op":"return"}),
    ]);
    for i in [0, 2, 9, 13] {
        assert_eq!(v[i]["accepted"], false)
    }
    for i in [4, 12] {
        assert_eq!(v[i]["accepted"], true)
    }
    assert_eq!(v.last().unwrap()["phase"], "Finished");
}
#[test]
fn every_phase_sticky_foreign_away_back() {
    let phases = vec![
        vec![],
        vec![json!({"op":"advance"}), json!({"op":"capture"})],
        vec![
            json!({"op":"advance"}),
            json!({"op":"capture"}),
            json!({"op":"present"}),
        ],
        visible(),
        extend(visible(), vec![json!({"op":"return"})]),
        extend(
            visible(),
            vec![
                json!({"op":"return"}),
                json!({"op":"advance"}),
                json!({"op":"check"}),
                json!({"op":"apply"}),
            ],
        ),
    ];
    for prefix in phases {
        for actor in ["foreign", "unknown"] {
            let count = prefix.len();
            let v = run(extend(
                prefix.clone(),
                vec![
                    json!({"op":"activation","sequence":2,"actor":actor}),
                    json!({"op":"activation","sequence":3,"actor":"original"}),
                    json!({"op":"visible"}),
                    json!({"op":"return"}),
                    json!({"op":"advance"}),
                    json!({"op":"check"}),
                    json!({"op":"apply"}),
                ],
            ));
            assert!(v[count..].iter().all(|s| s["phase"] == "Finished"));
        }
    }
}
#[test]
fn input_during_pending_capture_present_return_cancels() {
    for prefix in [
        vec![],
        vec![json!({"op":"advance"}), json!({"op":"capture"})],
        vec![
            json!({"op":"advance"}),
            json!({"op":"capture"}),
            json!({"op":"present"}),
        ],
        extend(visible(), vec![json!({"op":"return"})]),
    ] {
        for kind in ["form", "mouse", "other"] {
            let v = run(extend(
                prefix.clone(),
                vec![json!({"op":"input","sequence":1,"kind":kind})],
            ));
            assert_eq!(v.last().unwrap()["phase"], "Finished");
        }
    }
}
#[test]
fn visible_own_editing_survives_but_outside_click_does_not() {
    let v = run(extend(
        visible(),
        vec![
            json!({"op":"input","sequence":1,"kind":"form"}),
            json!({"op":"input","sequence":2,"kind":"mouse"}),
            json!({"op":"outside"}),
            json!({"op":"visible"}),
        ],
    ));
    assert_eq!(v[v.len() - 3]["phase"], "Visible");
    assert_eq!(v.last().unwrap()["phase"], "Finished");
}
#[test]
fn activation_and_input_sequence_replay_fail_closed() {
    for suffix in [
        vec![json!({"op":"activation","sequence":1,"actor":"own"})],
        vec![
            json!({"op":"input","sequence":1,"kind":"form"}),
            json!({"op":"input","sequence":1,"kind":"form"}),
        ],
    ] {
        assert_eq!(
            run(extend(visible(), suffix)).last().unwrap()["phase"],
            "Finished"
        );
    }
}
#[test]
fn pending_config_preserves_draft_without_allowing_return() {
    let mut equivalent = command();
    equivalent["id"] = json!("own.é");
    let v = run(extend(
        visible(),
        vec![
            json!({"op":"pending"}),
            json!({"op":"input","sequence":1,"kind":"form"}),
            json!({"op":"return"}),
            json!({"op":"config","command":equivalent}),
            json!({"op":"return"}),
            json!({"op":"activation","sequence":2,"actor":"original"}),
            json!({"op":"advance"}),
            json!({"op":"check"}),
            json!({"op":"apply"}),
        ],
    ));
    assert_eq!(v[7]["phase"], "Visible");
    assert_eq!(v[7]["accepted"], false);
    assert_eq!(v[8]["accepted"], true);
    assert_eq!(v.last().unwrap()["accepted"], true);
}
#[test]
fn all_stored_command_fields_and_removed_command_revoke() {
    let mut candidates = vec![Value::Null];
    for (field, value) in [
        ("id", json!("other")),
        ("title", json!("Changed")),
        ("kind", json!("url")),
        ("target", json!("new")),
        ("aliases", json!(["new"])),
        ("arguments", json!(["new"])),
        ("hotkey", json!("cmd+a")),
        ("timeout", json!(2)),
        ("inputPrompt", json!("new")),
        ("keyword", json!("other")),
        ("applicationID", json!("new")),
        ("snippetOutput", json!("html")),
        ("enabled", json!(false)),
    ] {
        let mut c = command();
        c[field] = value;
        candidates.push(c);
    }
    for candidate in candidates {
        let v = run(extend(
            visible(),
            vec![
                json!({"op":"pending"}),
                json!({"op":"config","command":candidate}),
                json!({"op":"pending"}),
                json!({"op":"config","command":command()}),
            ],
        ));
        assert_eq!(v.last().unwrap()["phase"], "Finished");
    }
}
#[test]
fn stale_config_ticket_cannot_accept_or_cancel_newer_pending() {
    let mut stale = command();
    stale["title"] = json!("stale");
    let v = run(extend(
        visible(),
        vec![
            json!({"op":"pending"}),
            json!({"op":"pending"}),
            json!({"op":"config","ticket":0,"command":stale}),
            json!({"op":"return"}),
            json!({"op":"config","ticket":1,"command":command(),"generation":3}),
            json!({"op":"config","ticket":1,"command":stale}),
            json!({"op":"return"}),
        ],
    ));
    assert_eq!(v[7]["accepted"], false);
    assert_eq!(v[7]["phase"], "Visible");
    assert_eq!(v[8]["accepted"], false);
    assert_eq!(v[9]["accepted"], true);
    assert_eq!(v[10]["accepted"], false);
    assert_eq!(v[11]["accepted"], true);
}
#[test]
fn cross_request_capture_and_config_tickets_do_not_authorize_new_request() {
    let v = run(vec![
        json!({"op":"advance"}),
        json!({"op":"capture"}),
        json!({"op":"replace"}),
        json!({"op":"advance"}),
        json!({"op":"capture","keep":true}),
        json!({"op":"present"}),
    ]);
    assert_eq!(v.last().unwrap()["accepted"], false);
    assert_eq!(v.last().unwrap()["phase"], "Capturing");
    let v = run(vec![
        json!({"op":"pending"}),
        json!({"op":"replace"}),
        json!({"op":"pending"}),
        json!({"op":"config","ticket":0,"command":command()}),
        json!({"op":"config","ticket":1,"command":command()}),
    ]);
    assert_eq!(v[v.len() - 2]["accepted"], false);
    assert_eq!(v.last().unwrap()["accepted"], true);
    let steps = extend(
        extend(
            extend(
                visible(),
                vec![
                    json!({"op":"return"}),
                    json!({"op":"advance"}),
                    json!({"op":"check"}),
                    json!({"op":"replace"}),
                ],
            ),
            visible(),
        ),
        vec![
            json!({"op":"return"}),
            json!({"op":"advance"}),
            json!({"op":"apply"}),
        ],
    );
    let v = run(steps);
    assert_eq!(v.last().unwrap()["accepted"], false);
    assert_eq!(v.last().unwrap()["phase"], "Returning");
}
#[test]
fn sequence_gaps_are_not_silently_coalesced() {
    for suffix in [
        json!({"op":"activation","sequence":3,"actor":"own"}),
        json!({"op":"input","sequence":2,"kind":"form"}),
    ] {
        assert_eq!(
            run(extend(visible(), vec![suffix])).last().unwrap()["phase"],
            "Finished"
        );
    }
}
#[test]
fn confirmed_older_generation_cannot_replace_current_certificate() {
    let v = run(extend(
        visible(),
        vec![
            json!({"op":"pending"}),
            json!({"op":"config","command":command(),"generation":3}),
            json!({"op":"pending"}),
            json!({"op":"config","command":command(),"generation":2}),
        ],
    ));
    assert_eq!(v.last().unwrap()["phase"], "Finished");
}
#[test]
fn captured_stamp_is_not_refreshed_by_compatible_config() {
    let v = run(vec![
        json!({"op":"advance"}),
        json!({"op":"capture"}),
        json!({"op":"pending"}),
        json!({"op":"config","command":command()}),
        json!({"op":"present"}),
    ]);
    assert_eq!(v.last().unwrap()["accepted"], false);
    assert_eq!(v.last().unwrap()["phase"], "Capturing");
}
#[test]
fn return_original_and_own_allowed_but_later_input_revokes() {
    let v = run(extend(
        visible(),
        vec![
            json!({"op":"return"}),
            json!({"op":"activation","sequence":2,"actor":"own"}),
            json!({"op":"activation","sequence":3,"actor":"original"}),
            json!({"op":"advance"}),
            json!({"op":"check"}),
            json!({"op":"input","sequence":1,"kind":"other"}),
            json!({"op":"apply"}),
        ],
    ));
    assert_eq!(v.last().unwrap()["phase"], "Finished");
    assert_eq!(v.last().unwrap()["accepted"], false);
}
#[test]
fn return_has_initial_delay_and_fifteen_retries() {
    let mut steps = extend(
        visible(),
        vec![json!({"op":"return"}), json!({"op":"retry"})],
    );
    for _ in 0..16 {
        steps.extend([json!({"op":"advance"}), json!({"op":"retry"})]);
    }
    let v = run(steps);
    assert_eq!(v[6]["accepted"], false);
    assert_eq!(v[7..].iter().filter(|s| s["accepted"] == true).count(), 15);
    assert_eq!(v.last().unwrap()["phase"], "Finished");
}
#[test]
fn config_pending_revokes_an_earlier_return_check() {
    let v = run(extend(
        visible(),
        vec![
            json!({"op":"return"}),
            json!({"op":"advance"}),
            json!({"op":"check"}),
            json!({"op":"pending"}),
            json!({"op":"apply"}),
            json!({"op":"config","command":command()}),
            json!({"op":"apply"}),
            json!({"op":"check"}),
            json!({"op":"apply"}),
        ],
    ));
    assert_eq!(v[9]["accepted"], false);
    assert_eq!(v[11]["accepted"], false);
    assert_eq!(v[13]["accepted"], true);
}
#[test]
fn first_return_poll_starts_after_single_activation_completes() {
    let v = run(extend(
        visible(),
        vec![
            json!({"op":"return"}),
            json!({"op":"advance","ms":100}),
            json!({"op":"activated-return"}),
            json!({"op":"activated-return"}),
            json!({"op":"advance","ms":29}),
            json!({"op":"retry"}),
            json!({"op":"advance","ms":1}),
            json!({"op":"check"}),
            json!({"op":"apply"}),
        ],
    ));
    assert_eq!(v[7]["accepted"], true);
    assert_eq!(v[7]["returnDue"], false);
    assert_eq!(v[8]["accepted"], false);
    assert_eq!(v[10]["accepted"], false);
    assert_eq!(v[11]["returnDue"], true);
    assert_eq!(v[13]["accepted"], true);
}
