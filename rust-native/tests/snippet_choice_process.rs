//! Original five pure choice-model methods against frozen genuine Swift output.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::Value;
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
const GOLDEN: &str = "rust-native/tests/snippet_choice_reference.json";
const SHA256: &str = "c92b919a07cb680209092ddcea14d59e4c603520221d7574ceda21a7189fab40";
const BATCHES: &[(&str, usize, usize)] = &[
    ("complete_command_equality_matches_actual_swift", 42, 42),
    ("defaults_canonical_options_labels_and_empty_values", 9, 9),
    (
        "keyboard_modifiers_repeat_composition_and_field_editing",
        192,
        768,
    ),
    (
        "options_clamp_first_canonical_match_and_active_field",
        1,
        16,
    ),
    ("trim_validation_correct_retry_and_single_use", 6, 36),
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
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE.get_or_init(|| {
        let bytes = fs::read(root().join(GOLDEN)).unwrap();
        assert_eq!(hash(&bytes), SHA256);
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema"], "ruycast.snippet-choice-swift-goldens.v1");
        assert_eq!(value["methods"], 5);
        assert_eq!(value["caseCount"], 250);
        assert_eq!(value["steps"], 871);
        let batches = value["batches"].as_array().unwrap();
        assert_eq!(batches.len(), BATCHES.len());
        for (batch, (method, count, steps)) in batches.iter().zip(BATCHES) {
            assert_eq!(batch["method"], *method);
            assert_eq!(batch["batch"], 0);
            assert_eq!(batch["caseCount"], *count);
            assert_eq!(batch["steps"], *steps);
            let cases = batch["cases"].as_array().unwrap();
            let expected = batch["expected"].as_array().unwrap();
            assert_eq!(cases.len(), *count);
            assert_eq!(expected.len(), *count);
            assert_eq!(
                cases
                    .iter()
                    .map(|c| c.as_array().unwrap().len())
                    .sum::<usize>(),
                *steps
            );
            for (case, expected) in cases.iter().zip(expected) {
                assert_eq!(
                    case.as_array().unwrap().len(),
                    expected.as_array().unwrap().len()
                );
                for op in case.as_array().unwrap() {
                    if op["op"] == "start" {
                        assert!(!op["command"]["target"]
                            .as_str()
                            .unwrap()
                            .contains("{clipboard}"));
                    }
                }
            }
            if method.starts_with("complete_") {
                assert!(!cases[40][0]["left"]["timeout"]
                    .as_f64()
                    .unwrap()
                    .is_sign_negative());
                assert!(cases[40][0]["right"]["timeout"]
                    .as_f64()
                    .unwrap()
                    .is_sign_negative());
                for field in ["aliases", "arguments"] {
                    assert!(cases[41][0]["left"].get(field).is_some());
                    assert!(cases[41][0]["right"].get(field).is_none());
                }
            }
        }
        value
    })
}
fn probe() -> &'static Path {
    static PROBE: OnceLock<PathBuf> = OnceLock::new();
    PROBE
        .get_or_init(|| {
            let source = PathBuf::from(env!("CARGO_BIN_EXE_snippet-choice-probe"));
            assert!(source.is_absolute() && source.is_file());
            if source.starts_with(root()) {
                return source;
            }
            let path = root().join(format!(
                "rust-native/target/choice-model-golden-probe-{}",
                std::process::id()
            ));
            assert!(!path.exists());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::copy(&source, &path).unwrap();
            assert_eq!(
                hash(&fs::read(&source).unwrap()),
                hash(&fs::read(&path).unwrap())
            );
            path
        })
        .as_path()
}
fn invoke(cases: &Value) -> Value {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("rust-native/target"));
    assert!(parent.is_absolute() && parent.is_dir());
    let path = parent.join(format!(
        "choice-model-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    struct Owned(PathBuf);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let owned = Owned(path);
    let input = owned.0.join("input.json");
    fs::write(&input, serde_json::to_vec(cases).unwrap()).unwrap();
    let child = Command::new(probe())
        .env_clear()
        .stdin(fs::File::open(&input).unwrap())
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
            "rust-native/tests/snippet_choice_process.rs",
            "rust-native/tests/support/child_output.rs",
            "rust-native/src/bin/snippet_choice_probe.rs",
            "rust-native/src/snippet_choice.rs",
            "rust-native/src/command_identity.rs",
            "rust-native/src/templates.rs",
            "rust-native/src/snippet_output.rs",
            "rust/src/lib.rs",
            "rust-native/Cargo.toml",
            "rust-native/Cargo.lock",
            "rust/Cargo.toml",
            "rust/Cargo.lock",
            "rust-toolchain.toml",
        ]
        .into_iter()
        .map(|p| (p, hash(&fs::read(root.join(p)).unwrap())))
        .collect::<BTreeMap<_, _>>();
        let artifact = probe().strip_prefix(&root).unwrap().to_str().unwrap();
        fs::write(PathBuf::from(directory).join("snippet-choice-reference-bindings.json"),serde_json::to_vec_pretty(&serde_json::json!({"sources":sources,"artifacts":{artifact:hash(&fs::read(probe()).unwrap())},"goldenSHA256":SHA256,"methods":5,"cases":250,"steps":871,"scope":"Frozen genuine Swift model-route results, not current-platform or native UI/controller acceptance"})).unwrap()).unwrap();
    }
}
fn compare(method: &str) {
    let batches = corpus()["batches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["method"] == method)
        .collect::<Vec<_>>();
    assert_eq!(batches.len(), 1);
    let batch = batches[0];
    let actual = invoke(&batch["cases"]);
    let expected = batch["expected"].as_array().unwrap();
    let actual = actual.as_array().unwrap();
    let cases = batch["cases"].as_array().unwrap();
    assert_eq!(expected.len(), actual.len());
    assert_eq!(actual.len(), cases.len());
    for (i, ((case, expected), actual)) in cases.iter().zip(expected).zip(actual).enumerate() {
        assert_eq!(
            expected.as_array().unwrap().len(),
            case.as_array().unwrap().len()
        );
        assert_eq!(
            actual.as_array().unwrap().len(),
            expected.as_array().unwrap().len()
        );
        for (j, (expected, actual)) in expected
            .as_array()
            .unwrap()
            .iter()
            .zip(actual.as_array().unwrap())
            .enumerate()
        {
            assert_eq!(expected, actual, "{method} case{i} step{j}");
        }
    }
    receipt();
}
#[test]
fn complete_command_equality_matches_actual_swift() {
    compare("complete_command_equality_matches_actual_swift");
}
#[test]
fn defaults_canonical_options_labels_and_empty_values() {
    compare("defaults_canonical_options_labels_and_empty_values");
}
#[test]
fn keyboard_modifiers_repeat_composition_and_field_editing() {
    compare("keyboard_modifiers_repeat_composition_and_field_editing");
}
#[test]
fn trim_validation_correct_retry_and_single_use() {
    compare("trim_validation_correct_retry_and_single_use");
}
#[test]
fn options_clamp_first_canonical_match_and_active_field() {
    compare("options_clamp_first_canonical_match_and_active_field");
}
