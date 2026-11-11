//! Original private notification-center scenarios; no focus or input effects.
#![cfg(feature = "test-tools")]
#[path = "support/keyword_native.rs"]
mod keyword_native;
use serde_json::{json, Value};
fn run_case(case: Value) -> Value {
    keyword_native::run(json!([{"op":"activation","case":case}]))["steps"][0].clone()
}
fn field(result: &Value, name: &str) -> Vec<Value> {
    result["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event[name].clone())
        .collect()
}
#[test]
fn each_notification_delivered_with_original_typed_instance() {
    let v = run_case(json!({"events":["typed","missing","malformed","typed"]}));
    assert_eq!(
        field(&v, "sequence"),
        vec![json!(1), json!(2), json!(3), json!(4)]
    );
    assert_eq!(
        field(&v, "typed"),
        vec![json!(true), json!(false), json!(false), json!(true)]
    );
    assert_eq!(
        field(&v, "ownInstance"),
        vec![json!(true), json!(false), json!(false), json!(true)]
    );
    assert_eq!(v["failed"], 0);
    assert_eq!(v["beforeClose"], v["afterClose"]);
}
#[test]
fn nested_sink_preserves_each_sequence() {
    let v = run_case(json!({"events":["typed","typed"],"sink":"nested"}));
    assert_eq!(field(&v, "sequence"), vec![json!(1), json!(2), json!(3)]);
    assert_eq!(
        field(&v, "typed"),
        vec![json!(true), json!(false), json!(true)]
    );
}
#[test]
fn ingress_reentry_does_not_mint_new_sequence_for_old_notification() {
    let v = run_case(json!({"events":["typed","typed"],"ingress":"nested"}));
    // Preserve ingress order, rather than resampling the latest sequence.
    assert_eq!(field(&v, "sequence"), vec![json!(2), json!(1), json!(3)]);
    assert_eq!(
        field(&v, "typed"),
        vec![json!(false), json!(true), json!(true)]
    );
}
#[test]
fn close_during_ingress_revokes_before_native_payload_read_or_sink() {
    let v = run_case(json!({"events":["typed","typed"],"ingress":"drop"}));
    assert_eq!(v["events"], json!([]));
    assert_eq!(v["sequence"], 1);
}
#[test]
fn close_from_sink_keeps_receiver_alive_but_no_further_delivery() {
    let v = run_case(json!({"events":["typed","typed"],"sink":"drop"}));
    assert_eq!(v["events"].as_array().unwrap().len(), 1);
    assert_eq!(v["beforeClose"], v["afterClose"]);
}
#[test]
fn ingress_and_sink_panics_are_contained_and_disable_delivery() {
    for kind in ["ingress", "sink"] {
        let mut case = json!({"events":["typed","typed"]});
        case[kind] = json!("panic");
        let v = run_case(case);
        assert_eq!(v["failed"], 1);
        assert_eq!(v["sequence"], 1);
        assert_eq!(
            v["events"].as_array().unwrap().len(),
            usize::from(kind == "sink")
        );
    }
}
#[test]
fn sequence_exhaustion_never_wraps() {
    let v = run_case(json!({"events":["typed","typed"],"sequence":u64::MAX-1}));
    assert_eq!(field(&v, "sequence"), vec![json!(u64::MAX), Value::Null]);
    assert_eq!(v["sequence"], json!(u64::MAX));
}
