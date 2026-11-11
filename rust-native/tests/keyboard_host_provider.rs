//! Combined host gate through the actual Carbon owner; no global registration.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::Value;
use std::{
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
#[test]
fn combined_marker_authority_precedes_any_keyboard_sdk_entry() {
    let parent = PathBuf::from(
        std::env::var_os("RUYCAST_KEYBOARD_HOST_TEST_PARENT").expect("Own absolute parent"),
    );
    assert!(parent.is_absolute());
    let target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("Own target"));
    assert!(target.is_absolute());
    let library = target.join("release/examples/libkeyboard_host.dylib");
    assert!(library.is_file());
    for (tag, workspace, keyboard, expected) in [
        ("complete", true, true, true),
        ("workspace-missing", false, true, false),
        ("keyboard-missing", true, false, false),
    ] {
        let home = parent.join(format!("{}-{tag}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&home)
            .unwrap();
        if workspace {
            std::fs::write(
                home.join(".ruycast-test-fixture"),
                "own-disposable-fixture-v1",
            )
            .unwrap();
        }
        if keyboard {
            std::fs::write(
                home.join(".ruycast-keyboard-fixture"),
                "own-keyboard-fixture-v1",
            )
            .unwrap();
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_keyboard-probe"))
            .env_clear()
            .env("HOME", &home)
            .env("RUYCAST_KEYBOARD_FIXTURE_HOME", &home)
            .env("RUYCAST_WORKSPACE_FIXTURE_HOME", &home)
            .env("DYLD_INSERT_LIBRARIES", &library)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let written = std::io::Write::write_all(&mut child.stdin.take().unwrap(), b"[]");
        let output = child_output::wait(child, Duration::from_secs(8));
        assert!(
            written.is_ok() && output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["initialization"]["ok"], expected, "{value}");
        let trace = std::fs::read_to_string(home.join("keyboard-trace.jsonl")).unwrap_or_default();
        if expected {
            assert!(trace
                .lines()
                .any(|line| serde_json::from_str::<Value>(line).unwrap()["phase"] == "remove"));
        } else {
            assert!(
                trace.is_empty(),
                "Refuse before target/install/registration"
            );
        }
        std::fs::remove_dir_all(home).unwrap();
    }
}
