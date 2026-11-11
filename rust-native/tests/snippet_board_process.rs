//! Original PreparedItem cases through the actual Rust named-board provider.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn case(mode: u8, full: bool, partial: bool) {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_snippet-probe"));
    let artifact = executable
        .parent()
        .unwrap()
        .join("examples/libsnippet_board_fixture.dylib");
    assert!(
        artifact.is_absolute() && artifact.is_file(),
        "Build the same-profile snippet-board-fixture example first"
    );
    // The normal matrix target is inside this repository. Standalone callers
    // may use an external shared Cargo cache; bind an exact owned copy there.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let artifact = if artifact.starts_with(&root) {
        artifact
    } else {
        static COPY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        COPY.get_or_init(|| {
            let copy = root.join(format!(
                "rust-native/target/snippet-board-rust-{}.dylib",
                std::process::id()
            ));
            assert!(!copy.exists(), "Never reuse an earlier owned artifact");
            std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
            std::fs::copy(&artifact, &copy).unwrap();
            assert_eq!(
                Sha256::digest(std::fs::read(&artifact).unwrap()),
                Sha256::digest(std::fs::read(&copy).unwrap())
            );
            copy
        })
        .clone()
    };
    let child = Command::new(executable)
        .args(["owned-board", &mode.to_string()])
        .env_clear()
        .env("DYLD_INSERT_LIBRARIES", &artifact)
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
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        json!({"original":true,"full":full,"partial":partial,"restored":true,"generalBoardAbortGuard":true})
    );
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned();
        let sources = [
            "rust-native/tests/fixtures/snippet_board_fixture.rs",
            "rust-native/tests/snippet_board_process.rs",
            "rust-native/tests/support/child_output.rs",
            "rust-native/src/bin/snippet_probe.rs",
            "rust-native/Cargo.toml",
            "rust-native/Cargo.lock",
            "rust/Cargo.toml",
            "rust-toolchain.toml",
        ]
        .into_iter()
        .map(|name| {
            (
                name,
                format!(
                    "{:x}",
                    Sha256::digest(std::fs::read(root.join(name)).unwrap())
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
        let bindings = json!({"sources":sources,"artifacts":{artifact.strip_prefix(&root).unwrap().to_str().unwrap():format!("{:x}",Sha256::digest(std::fs::read(&artifact).unwrap()))}});
        std::fs::write(
            PathBuf::from(directory).join("snippet-board-bindings.json"),
            serde_json::to_vec_pretty(&bindings).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn equal_native_text_html_and_unordered_types() {
    case(0, true, true);
}
#[test]
fn lone_high_surrogate_text() {
    case(1, false, false);
}
#[test]
fn lone_low_surrogate_html() {
    case(2, false, false);
}
#[test]
fn lone_surrogate_type() {
    case(3, false, false);
}
#[test]
fn partial_text_with_missing_html() {
    case(4, false, true);
}
#[test]
fn foreign_html_is_not_owned() {
    case(5, false, false);
}
#[test]
fn extra_type_is_not_owned() {
    case(6, false, false);
}
#[test]
fn difference_after_embedded_nul() {
    case(7, false, false);
}
#[test]
fn difference_after_utf16_chunk_boundary() {
    case(8, false, false);
}
#[test]
fn equal_multiple_utf16_chunks() {
    case(9, true, true);
}
