//! Original eight pure keyword methods against genuine frozen Swift results.
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
const GOLDEN: &str = "rust-native/tests/keyword_reference.json";
const SHA256: &str = "b3ad168109a4712a792756f6227a730cec365fae7399824803034131ce956119";
const BATCHES: &[(&str, u64, usize, usize)] = &[
    ("all_valid_unicode_scalars_word_boundary_union", 0, 1, 1),
    (
        "boundaries_canonical_equivalence_and_graphemes",
        0,
        194,
        391,
    ),
    (
        "choice_single_use_retries_cancel_and_target_guards",
        0,
        7,
        21,
    ),
    ("focus_secure_precedence", 0, 12, 12),
    ("longest_ties_eligibility_and_catalog_changes", 0, 1, 15),
    ("longest_ties_eligibility_and_catalog_changes", 1, 2, 9),
    (
        "replacement_utf16_chunking_cursor_and_erase_counts",
        0,
        500,
        500,
    ),
    (
        "replacement_utf16_chunking_cursor_and_erase_counts",
        1,
        72,
        72,
    ),
    ("seeded_mixed_transitions_and_catalog_scale", 0, 40, 2840),
    ("seeded_mixed_transitions_and_catalog_scale", 1, 1, 5),
    ("suffix128_backspace_reset_and_segment_merges", 0, 58, 288),
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
        assert_eq!(value["schema"], "ruycast.keyword-swift-goldens.v1");
        assert_eq!(value["caseCount"], 888);
        assert_eq!(value["steps"], 4154);
        let batches = value["batches"].as_array().unwrap();
        assert_eq!(batches.len(), BATCHES.len());
        for (batch, (method, index, cases, steps)) in batches.iter().zip(BATCHES) {
            assert_eq!(batch["method"], *method);
            assert_eq!(batch["batch"], *index);
            assert_eq!(batch["caseCount"], *cases);
            assert_eq!(batch["steps"], *steps);
            let input = batch["cases"].as_array().unwrap();
            let expected = batch["expected"].as_array().unwrap();
            assert_eq!(input.len(), *cases);
            assert_eq!(expected.len(), *cases);
            assert_eq!(
                input
                    .iter()
                    .map(|c| c.as_array().unwrap().len())
                    .sum::<usize>(),
                *steps
            );
            for (input, expected) in input.iter().zip(expected) {
                assert_eq!(
                    input.as_array().unwrap().len(),
                    expected.as_array().unwrap().len()
                );
            }
        }
        value
    })
}
fn probe() -> &'static Path {
    static PROBE: OnceLock<PathBuf> = OnceLock::new();
    PROBE
        .get_or_init(|| {
            let source = PathBuf::from(env!("CARGO_BIN_EXE_keyword-probe"));
            assert!(source.is_absolute() && source.is_file());
            if source.starts_with(root()) {
                return source;
            }
            let path = root().join(format!(
                "rust-native/target/keyword-golden-probe-{}",
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
        "keyword-state-{}-{}",
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
    let output = child_output::wait(child, Duration::from_secs(30));
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
            "rust-native/tests/keyword_process.rs",
            "rust-native/tests/support/child_output.rs",
            "rust-native/src/bin/keyword_probe.rs",
            "rust-native/src/keyword_matcher.rs",
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
        fs::write(PathBuf::from(directory).join("keyword-reference-bindings.json"),serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":{artifact:hash(&fs::read(probe()).unwrap())},"goldenSHA256":SHA256,"methods":8,"batches":11,"cases":888,"steps":4154,"scope":"Frozen genuine Swift pure keyword corpus, not general current-platform/native-effects acceptance"})).unwrap()).unwrap();
    }
}
fn bits(value: &Value) -> Vec<u8> {
    let text = value["bits"].as_str().unwrap();
    assert_eq!(text.len(), 139264 * 2);
    text.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn compare(method: &str) {
    let batches = corpus()["batches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["method"] == method)
        .collect::<Vec<_>>();
    assert_eq!(
        batches.len(),
        if [
            "longest_ties_eligibility_and_catalog_changes",
            "replacement_utf16_chunking_cursor_and_erase_counts",
            "seeded_mixed_transitions_and_catalog_scale"
        ]
        .contains(&method)
        {
            2
        } else {
            1
        }
    );
    for batch in batches {
        let actual = invoke(&batch["cases"]);
        let before = batch["expected"].as_array().unwrap();
        let after = actual.as_array().unwrap();
        let cases = batch["cases"].as_array().unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(after.len(), cases.len());
        for (i, ((input, a), b)) in cases.iter().zip(before).zip(after).enumerate() {
            assert_eq!(a.as_array().unwrap().len(), input.as_array().unwrap().len());
            assert_eq!(b.as_array().unwrap().len(), a.as_array().unwrap().len());
            for (j, (a, b)) in a
                .as_array()
                .unwrap()
                .iter()
                .zip(b.as_array().unwrap())
                .enumerate()
            {
                assert_eq!(a, b, "{method} batch{} case{i} step{j}", batch["batch"]);
            }
        }
        if method == "longest_ties_eligibility_and_catalog_changes" && batch["batch"] == 0 {
            assert_eq!(
                (2..15)
                    .step_by(2)
                    .map(|i| actual[0][i]["match"].clone())
                    .collect::<Vec<_>>(),
                vec![
                    json!("default"),
                    json!("option"),
                    json!("long"),
                    json!("same-first"),
                    Value::Null,
                    Value::Null,
                    Value::Null
                ]
            );
        }
        if method == "all_valid_unicode_scalars_word_boundary_union" {
            let expected = bits(&batch["expected"][0][0]);
            let actual_bits = bits(&actual[0][0]);
            assert_eq!(actual_bits, expected);
            assert_eq!(
                hash(&expected),
                "f671386063e70274b2501501ddf5a3d2b0ce03baa2aa1a5ace7f736103accc2d"
            );
            assert_eq!(actual[0][0]["count"], 149141);
            assert_eq!(
                actual_bits
                    .iter()
                    .map(|b| b.count_ones() as usize)
                    .sum::<usize>(),
                149141
            );
            for scalar in 0xd800..0xe000 {
                assert_eq!(actual_bits[scalar / 8] & (1 << (scalar % 8)), 0);
            }
        }
    }
    receipt();
}
#[test]
fn all_valid_unicode_scalars_word_boundary_union() {
    compare("all_valid_unicode_scalars_word_boundary_union");
}
#[test]
fn longest_ties_eligibility_and_catalog_changes() {
    compare("longest_ties_eligibility_and_catalog_changes");
}
#[test]
fn boundaries_canonical_equivalence_and_graphemes() {
    compare("boundaries_canonical_equivalence_and_graphemes");
}
#[test]
fn suffix128_backspace_reset_and_segment_merges() {
    compare("suffix128_backspace_reset_and_segment_merges");
}
#[test]
fn replacement_utf16_chunking_cursor_and_erase_counts() {
    compare("replacement_utf16_chunking_cursor_and_erase_counts");
}
#[test]
fn focus_secure_precedence() {
    compare("focus_secure_precedence");
}
#[test]
fn choice_single_use_retries_cancel_and_target_guards() {
    compare("choice_single_use_retries_cancel_and_target_guards");
}
#[test]
fn seeded_mixed_transitions_and_catalog_scale() {
    compare("seeded_mixed_transitions_and_catalog_scale");
}
