//! Original four clipboard-text methods against frozen, genuine Swift outputs.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn goldens() -> &'static Value {
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE.get_or_init(|| {
        let bytes =
            fs::read(root().join("rust-native/tests/clipboard_text_reference.json")).unwrap();
        assert_eq!(
            hash(&bytes),
            "1648bd57fadd592df1d09d711280c4d075ffe30ec931a4f65e21afe9012db9a2"
        );
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema"], "ruycast.clipboard-text-swift-goldens.v1");
        let expected = [1, 0, 10, 11, 12, 13, 14, 15, 16, 17]
            .into_iter()
            .flat_map(|mode| [format!("read:{mode}"), format!("prefill:{mode}")])
            .collect::<BTreeSet<_>>();
        assert_eq!(
            value["cases"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            expected
        );
        value
    })
}
fn artifacts() -> &'static (PathBuf, PathBuf) {
    static ARTIFACTS: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        let host = std::env::var_os("RUYCAST_TEST_APP_HOST")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_ruycast-native")));
        let board = std::env::var_os("RUYCAST_TEST_BOARD_SDK")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_BIN_EXE_ruycast-native"))
                    .parent()
                    .unwrap()
                    .join("examples/libsnippet_board_fixture.dylib")
            });
        let copy = |artifact: PathBuf, label: &str| {
            assert!(
                artifact.is_absolute() && artifact.is_file(),
                "Build same-profile Rust host and snippet-board-fixture first"
            );
            if artifact.starts_with(root()) {
                return artifact;
            }
            let path = root().join(format!(
                "rust-native/target/clipboard-text-{label}-{}",
                std::process::id()
            ));
            assert!(!path.exists(), "Never reuse an earlier artifact copy");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::copy(&artifact, &path).unwrap();
            assert_eq!(
                hash(&fs::read(&artifact).unwrap()),
                hash(&fs::read(&path).unwrap())
            );
            path
        };
        (copy(host, "host"), copy(board, "sdk.dylib"))
    })
}
fn run(reader: &str, mode: u8, provider: bool) -> std::process::Output {
    let (host, board) = artifacts();
    let mut command = Command::new(host);
    command.args(["--check-owned-clipboard", reader, &mode.to_string()]);
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("RUYCAST_")
            || name.starts_with("RUSTCAST_")
            || name.starts_with("DYLD_")
        {
            command.env_remove(key);
        }
    }
    if provider {
        command.env("DYLD_INSERT_LIBRARIES", board);
    }
    child_output::wait(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        Duration::from_secs(if provider { 15 } else { 5 }),
    )
}
fn receipt() {
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        fs::create_dir_all(&directory).unwrap();
        let root = root();
        let sources = [
            "rust-native/tests/clipboard_text_process.rs",
            "rust-native/tests/clipboard_text_reference.json",
            "rust-native/tests/fixtures/snippet_board_fixture.rs",
            "rust-native/tests/support/child_output.rs",
            "rust-native/src/main.rs",
            "rust-native/src/native_services.rs",
            "rust-native/Cargo.toml",
            "rust-native/Cargo.lock",
            "rust/Cargo.toml",
            "rust-toolchain.toml",
        ]
        .into_iter()
        .map(|p| (p, hash(&fs::read(root.join(p)).unwrap())))
        .collect::<BTreeMap<_, _>>();
        let (host, board) = artifacts();
        let artifacts = [host, board]
            .into_iter()
            .map(|p| {
                (
                    p.strip_prefix(&root).unwrap().to_str().unwrap(),
                    hash(&fs::read(p).unwrap()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        fs::write(
            PathBuf::from(directory).join("clipboard-text-reference-bindings.json"),
            serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":artifacts})).unwrap(),
        )
        .unwrap();
    }
}
fn case(reader: &str, mode: u8, expected: Option<String>) {
    let output = run(reader, mode, true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    // Preserve the original explicit expected-value assertion independently of goldens.
    assert_eq!(
        actual,
        json!({"ok":true,"value":expected,"restored":true,"generalBoardAbortGuard":true})
    );
    let mut observed = actual.clone();
    observed["value"] = actual["value"]
        .as_str()
        .map(|s| json!({"byteLength":s.len(),"sha256":hash(s.as_bytes())}))
        .unwrap_or(Value::Null);
    assert_eq!(
        observed,
        goldens()["cases"][format!("{reader}:{mode}")],
        "captured Swift {reader}/{mode}"
    );
    receipt();
}
#[test]
fn native_read_lone_high_surrogate() {
    case("read", 1, Some("�".into()));
}
#[test]
fn native_prefill_lone_high_surrogate() {
    case("prefill", 1, Some("�".into()));
}
#[test]
fn provider_is_required_before_any_native_board_creation() {
    let output = run("read", 1, false);
    assert_eq!(output.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("Complete owned clipboard provider required"));
}
#[test]
fn all_owned_native_read_and_prefill_cases() {
    for (mode, read, prefill) in [
        (0, Some("A\0B😀".into()), Some("A\0B😀".into())),
        (10, Some("�".into()), Some("�".into())),
        (11, Some(" �𐀀�\0\n".into()), Some("�𐀀�\0".into())),
        (
            12,
            Some(format!("{}😀", "x".repeat(4095))),
            Some(format!("{}😀", "x".repeat(4095))),
        ),
        (
            13,
            Some(format!("{}�A", "x".repeat(4095))),
            Some(format!("{}�A", "x".repeat(4095))),
        ),
        (
            14,
            Some(format!(" \n{}\n ", "x".repeat(1_000_001))),
            Some("x".repeat(1_000_001)),
        ),
        (15, Some(" \n\u{200b}".into()), None),
        (16, Some(String::new()), None),
        (17, None, None),
    ] {
        case("read", mode, read);
        case("prefill", mode, prefill);
    }
}
