//! Original snippet output methods against genuine frozen Swift results.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::Duration,
};
const GOLDEN: &str = "rust-native/tests/snippet_reference.json";
const SHA: &str = "b19d5abc788642df64402065797d255db3fa399b0b763eb2cf66544746f1f746";
const BATCHES: &[(&str, &[usize])] = &[
    ("generated_unicode_and_markup_corpus", &[120]),
    (
        "grapheme_suffix_count_across_scripts_controls_and_joiners",
        &[115],
    ),
    (
        "large_source_is_complete_without_an_application_quota",
        &[1],
    ),
    (
        "links_balanced_parentheses_escape_labels_and_invalid_syntax",
        &[20],
    ),
    (
        "native_named_board_contains_one_complete_plain_or_rich_item",
        &[4],
    ),
    ("plain_unknown_modes_and_complete_literal_text", &[5]),
    (
        "real_sdk_item_failure_keeps_previous_board_but_write_failure_is_partial",
        &[4, 1, 1, 1],
    ),
    ("teams_paragraphs_html_breaks_and_safe_escaping", &[2, 2]),
    (
        "url_validation_and_link_source_preserve_actual_bridge_behavior",
        &[42],
    ),
    (
        "utf16_cursor_mapping_at_every_position_and_grapheme_boundary",
        &[93, 93, 174, 90, 93, 48, 81, 12],
    ),
];
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn corpus() -> &'static Value {
    static CORPUS: OnceLock<Value> = OnceLock::new();
    CORPUS.get_or_init(|| {
        let bytes = fs::read(root().join(GOLDEN)).unwrap();
        assert_eq!(hash(&bytes), SHA);
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema"], "ruycast.snippet-output-swift-goldens.v1");
        assert_eq!(value["methods"], 11);
        assert_eq!(value["caseCount"], 1002);
        assert_eq!(value["batchCount"], 21);
        let batches = value["batches"].as_array().unwrap();
        assert_eq!(batches.len(), 21);
        let mut cursor = 0;
        let mut total = 0;
        for (method, counts) in BATCHES {
            for (index, count) in counts.iter().enumerate() {
                let b = &batches[cursor];
                cursor += 1;
                total += count;
                assert_eq!(b["method"], *method);
                assert_eq!(b["batch"], index);
                assert_eq!(b["cases"].as_array().unwrap().len(), *count);
                assert_eq!(b["expected"].as_array().unwrap().len(), *count);
                let fault = if method.starts_with("real_sdk_") {
                    [
                        Some("pass-with-general-access-forbidden"),
                        Some("string"),
                        Some("html"),
                        Some("write"),
                    ][index]
                } else {
                    None
                };
                assert_eq!(b["fault"].as_str(), fault);
            }
        }
        assert_eq!(cursor, 21);
        assert_eq!(total, 1002);
        let pathological = value["pathological"].as_array().unwrap();
        assert_eq!(pathological.len(), 1);
        // This is a real historical timeout, not a synthetic expected Swift payload.
        assert_eq!(pathological[0]["differentialPass"], false);
        assert_eq!(
            pathological[0]["actualSwiftOutcome"],
            "No response before 1-second fixture deadline; process killed/reaped."
        );
        assert!(pathological[0].get("expected").is_none());
        value
    })
}
fn artifacts() -> &'static (PathBuf, PathBuf) {
    static ARTIFACTS: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        let probe = PathBuf::from(env!("CARGO_BIN_EXE_snippet-probe"));
        let sdk = probe
            .parent()
            .unwrap()
            .join("examples/libsnippet_faults.dylib");
        assert!(probe.is_absolute() && probe.is_file());
        assert!(
            sdk.is_file(),
            "Build the same-profile snippet-faults example first"
        );
        fn bind(source: PathBuf, label: &str) -> PathBuf {
            if source.starts_with(root()) {
                return source;
            }
            let path = root().join(format!(
                "rust-native/target/snippet-output-{label}-{}",
                std::process::id()
            ));
            assert!(!path.exists());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::copy(&source, &path).unwrap();
            assert_eq!(
                hash(&fs::read(source).unwrap()),
                hash(&fs::read(&path).unwrap())
            );
            path
        }
        (bind(probe, "probe"), bind(sdk, "faults.dylib"))
    })
}
fn invoke(cases: &Value, fault: Option<&str>) -> Value {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("rust-native/target"));
    assert!(parent.is_absolute() && parent.is_dir());
    let dir = parent.join(format!(
        "snippet-output-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    struct Owned(PathBuf);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let owned = Owned(dir);
    let input = owned.0.join("input.json");
    fs::write(&input, serde_json::to_vec(cases).unwrap()).unwrap();
    let (probe, sdk) = artifacts();
    let mut command = Command::new(probe);
    command.env_clear();
    if let Some(fault) = fault {
        command
            .env("DYLD_INSERT_LIBRARIES", sdk)
            .env("RUYCAST_SNIPPET_FIXTURE", "own-named-board")
            .env("RUYCAST_SNIPPET_FAULT", fault);
    }
    let child = command
        .stdin(fs::File::open(input).unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child_output::wait(child, Duration::from_secs(20));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    value["cases"].clone()
}
fn receipt() {
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        fs::create_dir_all(&directory).unwrap();
        let root = root();
        let sources = [
            GOLDEN,
            "rust-native/tests/snippet_process.rs",
            "rust-native/tests/support/child_output.rs",
            "rust-native/tests/fixtures/snippet_faults.rs",
            "rust-native/src/bin/snippet_probe.rs",
            "rust-native/src/snippet_output.rs",
            "rust-native/src/snippet_clipboard.rs",
            "rust-native/Cargo.toml",
            "rust-native/Cargo.lock",
            "rust/Cargo.toml",
            "rust/Cargo.lock",
            "rust-toolchain.toml",
        ]
        .into_iter()
        .map(|p| (p, hash(&fs::read(root.join(p)).unwrap())))
        .collect::<BTreeMap<_, _>>();
        let (probe, sdk) = artifacts();
        let artifacts = [probe, sdk]
            .into_iter()
            .map(|p| {
                (
                    p.strip_prefix(&root).unwrap().to_str().unwrap(),
                    hash(&fs::read(p).unwrap()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        fs::write(PathBuf::from(directory).join("snippet-reference-bindings.json"),serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":artifacts,"goldenSHA256":SHA,"methods":11,"comparedCases":1002,"batches":21,"historicalSwiftTimeouts":1,"timeoutDifferentialPass":false,"scope":"Frozen genuine Swift output corpus plus live owned named-board/fault assertions; no general clipboard, activation, paste or input"})).unwrap()).unwrap();
    }
}
fn compare(method: &str) {
    let batches = corpus()["batches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["method"] == method)
        .collect::<Vec<_>>();
    let counts = BATCHES.iter().find(|(m, _)| *m == method).unwrap().1;
    assert_eq!(batches.len(), counts.len());
    for (index, batch) in batches.into_iter().enumerate() {
        let fault = batch["fault"].as_str().map(|f| {
            if f == "pass-with-general-access-forbidden" {
                "pass"
            } else {
                f
            }
        });
        let actual = invoke(&batch["cases"], fault);
        assert_eq!(actual.as_array().unwrap().len(), counts[index]);
        for (i, (expected, actual)) in batch["expected"]
            .as_array()
            .unwrap()
            .iter()
            .zip(actual.as_array().unwrap())
            .enumerate()
        {
            assert_eq!(expected, actual, "{method} batch{index} case{i}");
        }
        if method == "teams_paragraphs_html_breaks_and_safe_escaping" && index == 0 {
            let html = actual[0]["payload"]["html"].as_str().unwrap();
            assert!(!html.contains("<table"));
            assert!(html.contains("<p style=\"margin:0\"><br></p>"));
            assert!(html.contains("Guide &amp; tips</a>"));
        }
        if method == "large_source_is_complete_without_an_application_quota" {
            assert_eq!(
                actual[0]["payload"]["text"],
                "Guide\n".repeat(4000) + "Ω & < end"
            );
        }
        if method == "native_named_board_contains_one_complete_plain_or_rich_item"
            || fault == Some("pass")
        {
            for case in actual.as_array().unwrap() {
                assert_eq!(case["clipboard"]["success"], true);
                assert_eq!(case["clipboard"]["items"], 1);
            }
        }
        if matches!(fault, Some("string" | "html" | "write")) {
            let value = &actual[0]["clipboard"];
            assert_eq!(value["success"], false);
            if fault == Some("write") {
                assert!(value["text"].is_null() && value["html"].is_null());
                assert_eq!(value["items"], 0);
                assert!(value["changes"].as_i64().unwrap() > 0);
            } else {
                assert_eq!(value["text"], "Own previous text");
                assert_eq!(value["html"], "<b>Own previous HTML</b>");
                assert_eq!(value["changes"], 0);
            }
        }
    }
    receipt();
}
macro_rules! method {
    ($name:ident) => {
        #[test]
        fn $name() {
            compare(stringify!($name));
        }
    };
}
method!(plain_unknown_modes_and_complete_literal_text);
method!(teams_paragraphs_html_breaks_and_safe_escaping);
method!(links_balanced_parentheses_escape_labels_and_invalid_syntax);
method!(url_validation_and_link_source_preserve_actual_bridge_behavior);
method!(utf16_cursor_mapping_at_every_position_and_grapheme_boundary);
method!(native_named_board_contains_one_complete_plain_or_rich_item);
method!(grapheme_suffix_count_across_scripts_controls_and_joiners);
method!(real_sdk_item_failure_keeps_previous_board_but_write_failure_is_partial);
method!(large_source_is_complete_without_an_application_quota);
method!(generated_unicode_and_markup_corpus);
#[test]
fn invalid_long_url_does_not_repeat_legacy_exponential_match() {
    let historical = &corpus()["pathological"][0];
    let actual = invoke(&json!([historical["case"]]), None);
    assert_eq!(actual[0]["links"], json!([]));
    assert_eq!(actual[0]["payload"]["text"], historical["case"]["source"]);
    // Retain the observed native result without attributing it to timed-out Swift.
    assert_eq!(actual[0], historical["actual"]);
    receipt();
}
