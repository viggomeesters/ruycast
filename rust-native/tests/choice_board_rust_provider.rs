#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn run(loaded: bool) -> Value {
    let target =
        PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("Explicit Cargo target"));
    assert!(target.is_absolute());
    let mut command = Command::new(target.join("release/examples/choice-board-probe"));
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if loaded {
        command.env(
            "DYLD_INSERT_LIBRARIES",
            target.join("release/examples/libchoice_board_fixture.dylib"),
        );
    }
    let output = child_output::wait(command.spawn().unwrap(), Duration::from_secs(10));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn repeated_framework_requests_share_only_the_empty_private_board_then_cleanup() {
    assert_eq!(
        run(true),
        json!({"readyBeforeCleanup":true,"samePrivateObject":true,"initiallyEmpty":true,"privateContent":true,"readyAfterCleanup":false,"monitorAPIsAbsent":true,"applicationCreated":false,"desktopInputSent":false})
    );
}
#[test]
fn absent_provider_refuses_before_any_board_creation() {
    assert_eq!(run(false), json!({"ready":false,"boardCreated":false}));
}
