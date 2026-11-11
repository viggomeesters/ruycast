#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn run(mode: &str, success: bool) -> Option<Value> {
    let target =
        PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("Explicit Cargo target"));
    assert!(target.is_absolute());
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_CHOICE_TEST_PARENT").expect("Owned absolute parent"),
    );
    assert!(parent.is_absolute() && parent.is_dir());
    let home = parent.join(format!("{}-{mode}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&home)
        .unwrap();
    let child = Command::new(target.join("release/examples/choice-monitor-probe"))
        .arg(mode)
        .env_clear()
        .env("HOME", &home)
        .env("RUYCAST_KEYWORD_FIXTURE_HOME", &home)
        .env(
            "DYLD_INSERT_LIBRARIES",
            target.join("release/examples/libchoice_fixture.dylib"),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child_output::wait(child, Duration::from_secs(10));
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("owned-choice-boundary:{mode}")), "The actual loaded provider must reach the selected boundary before any refusal is accepted");
    std::fs::remove_dir_all(home).unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    success.then(|| {
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["ownApplicationActive"], false);
        assert_eq!(value["ownWindowCount"], 0);
        value
    })
}
#[test]
fn actual_registration_callback_and_block_release_match_original_counts() {
    assert_eq!(run("basic", true).unwrap()["counts"], json!([1, 1, 1, 0]));
}
#[test]
fn native_local_tokens_and_nil_removal_keep_process_local_semantics() {
    assert_eq!(run("local", true).unwrap()["counts"], json!([0, 0, 0, 0]));
}
#[test]
fn callback_snapshot_observes_reentrant_removal_without_stale_delivery() {
    assert_eq!(
        run("snapshot", true).unwrap()["counts"],
        json!([2, 2, 1, 0])
    );
}
#[test]
fn held_removal_pumps_scheduled_callback_before_retiring_block() {
    assert_eq!(run("held", true).unwrap()["counts"], json!([1, 1, 1, 0]));
}
#[test]
fn invalid_mask_handler_token_and_live_cleanup_refuse() {
    for mode in ["bad-mask", "null-handler", "unknown-token", "live-exit"] {
        run(mode, false);
    }
}
