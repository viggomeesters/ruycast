#[path = "child_output.rs"]
mod child_output;
use serde_json::Value;
use std::{
    process::{Command, Stdio},
    time::Duration,
};

pub fn invoke(ops: Value, library: Option<&str>, ok: bool) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keyword-native-probe"));
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(name) = library {
        // Same profile/target as this test, including explicit cross-target builds.
        let profile = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let artifact = profile.join("examples").join(name);
        assert!(
            artifact.is_absolute() && artifact.is_file(),
            "Build keyword-fixture and keyword-missing-value-type examples first: {}",
            artifact.display()
        );
        command.env("DYLD_INSERT_LIBRARIES", artifact);
    }
    let mut child = command.spawn().unwrap();
    let input = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || serde_json::to_writer(input, &ops));
    let output = child_output::wait(child, Duration::from_secs(10));
    assert!(writer.join().unwrap().is_ok());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], ok, "{value}");
    if ok {
        assert_eq!(value["final"]["allocated"], 0, "{value}");
        assert_eq!(value["final"]["apps"], 0, "{value}");
    }
    value
}
pub fn run(ops: Value) -> Value {
    invoke(ops, Some("libkeyword_fixture.dylib"), true)
}
