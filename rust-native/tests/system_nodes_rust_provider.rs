#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn run(mode: &str, effects: i32) {
    let target =
        PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("Explicit Cargo target"));
    assert!(target.is_absolute());
    let child = Command::new(target.join("release/examples/system-nodes-probe"))
        .arg(mode)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child_output::wait(child, Duration::from_secs(10));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["mode"], mode);
    assert_eq!(value["effects"], effects);
    assert_eq!(value["completeReady"], false);
    assert!(value["reads"].as_u64().unwrap() > 0);
}
#[test]
fn native_opaque_nodes_keep_attributes_arrays_geometry_and_retained_slices() {
    run("normal", 0);
}
#[test]
fn unsupported_denied_and_mode_changes_preserve_read_contract() {
    run("boundaries", 0);
}
#[test]
fn synthetic_frame_restore_raise_and_press_preserve_exact_readback_and_counts() {
    run("write-normal", 5);
}
#[test]
fn constrained_and_cannot_complete_keep_mutation_then_denied_read_has_no_effect() {
    run("write-faults", 0);
}
