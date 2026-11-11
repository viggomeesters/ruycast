//! Original pure-state corpus, captured from genuine current-core Swift output.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
#[path = "support/hid_state_goldens.rs"]
mod goldens;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
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
fn corpus() -> &'static Value {
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE.get_or_init(|| goldens::parse(&fs::read(root().join(goldens::PATH)).unwrap()).unwrap())
}
fn probe() -> &'static Path {
    static PROBE: OnceLock<PathBuf> = OnceLock::new();
    PROBE
        .get_or_init(|| {
            let source = PathBuf::from(env!("CARGO_BIN_EXE_hid-state-probe"));
            assert!(source.is_absolute() && source.is_file());
            if source.starts_with(root()) {
                return source;
            }
            let path = root().join(format!(
                "rust-native/target/hid-state-golden-probe-{}",
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
fn invoke(input: &Value) -> std::process::Output {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("rust-native/target"));
    assert!(parent.is_absolute() && parent.is_dir());
    let directory = parent.join(format!(
        "hid-state-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    struct Owned(PathBuf);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let owned = Owned(directory);
    let path = owned.0.join("input.json");
    fs::write(&path, serde_json::to_vec(input).unwrap()).unwrap();
    let child = Command::new(probe())
        .env_clear()
        .stdin(fs::File::open(&path).unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child_output::wait(child, Duration::from_secs(15))
}
fn receipt() {
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        fs::create_dir_all(&directory).unwrap();
        let root = root();
        let sources = goldens::SOURCES
            .iter()
            .copied()
            .map(|p| (p, hash(&fs::read(root.join(p)).unwrap())))
            .collect::<BTreeMap<_, _>>();
        let artifact = probe().strip_prefix(&root).unwrap().to_str().unwrap();
        let binding = json!({"schema":"ruycast.hid-state-golden-binding.v1","sources":sources,"artifacts":{artifact:hash(&fs::read(probe()).unwrap())},"probe":artifact,"goldenSHA256":goldens::SHA256,"positiveCases":4623,"observedSnapshots":10940,"firstOperationRefusals":4});
        fs::write(
            PathBuf::from(directory).join("hid-state-reference-bindings.json"),
            serde_json::to_vec_pretty(&binding).unwrap(),
        )
        .unwrap();
    }
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
        if method.starts_with("generated_") {
            2
        } else {
            1
        }
    );
    for batch in batches {
        let cases = batch["cases"].as_array().unwrap();
        let positives = batch["positiveIndices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| cases[v.as_u64().unwrap() as usize].clone())
            .collect::<Vec<_>>();
        let output = invoke(&json!(positives));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["ok"], true);
        let actual = value["cases"].as_array().unwrap();
        let expected = batch["expected"].as_array().unwrap();
        assert_eq!(actual.len(), positives.len());
        assert_eq!(expected.len(), positives.len());
        for (i, (before, after)) in expected.iter().zip(actual).enumerate() {
            assert_eq!(
                before.as_array().unwrap().len(),
                positives[i].as_array().unwrap().len()
            );
            assert_eq!(
                after.as_array().unwrap().len(),
                before.as_array().unwrap().len()
            );
            for (j, (a, b)) in before
                .as_array()
                .unwrap()
                .iter()
                .zip(after.as_array().unwrap())
                .enumerate()
            {
                assert_eq!(a, b, "{method} case{i} step{j} input={}", positives[i][j]);
            }
        }
        if method.starts_with("multiple_") {
            assert_eq!(actual[0][3]["caps"], true);
            assert_eq!(
                actual[0].as_array().unwrap().last().unwrap()["held"],
                json!([])
            );
        }
        for refusal in batch["refusals"].as_array().unwrap() {
            let index = refusal["index"].as_u64().unwrap() as usize;
            let output = invoke(&json!([cases[index]]));
            assert_eq!(output.status.code(), Some(1));
            assert_eq!(
                serde_json::from_slice::<Value>(&output.stdout).unwrap(),
                json!({"ok":false,"error":goldens::ERROR})
            );
        }
    }
    receipt();
}
#[test]
fn physical_edges_duplicates_timing_nonfinite_and_stray_events() {
    compare("physical_edges_duplicates_timing_nonfinite_and_stray_events");
}
#[test]
fn multiple_devices_chords_cancel_remove_and_modifier_masks() {
    compare("multiple_devices_chords_cancel_remove_and_modifier_masks");
}
#[test]
fn generated_flags_rcmd_unknown_bits_and_all_modifier_combinations() {
    compare("generated_flags_rcmd_unknown_bits_and_all_modifier_combinations");
}
#[test]
fn sided_binding_keycodes_exact_modifiers_and_swallowed_release() {
    compare("sided_binding_keycodes_exact_modifiers_and_swallowed_release");
}
#[test]
fn permission_and_recovery_reason_precedence_is_not_activation() {
    compare("permission_and_recovery_reason_precedence_is_not_activation");
}
#[test]
fn sequence_oracle_fuzz_and_first_duplicate_binding_wins() {
    compare("sequence_oracle_fuzz_and_first_duplicate_binding_wins");
}
#[test]
fn oracle_contains_no_native_hook_sdk_imports() {
    let child = Command::new("/usr/bin/nm")
        .arg("-u")
        .arg(probe())
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child_output::wait(child, Duration::from_secs(5));
    assert!(output.status.success());
    let symbols = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| l.split_whitespace().last())
        .collect::<BTreeSet<_>>();
    for forbidden in [
        "_CGEventTapCreate",
        "_CGEventPost",
        "_IOServiceOpen",
        "_IOHIDManagerCreate",
        "_IOHIDSetModifierLockState",
        "_CGPreflightListenEventAccess",
        "_AXIsProcessTrusted",
        "_IsSecureEventInputEnabled",
        "_OBJC_CLASS_$_NSWorkspace",
        "_OBJC_CLASS_$_NSRunningApplication",
    ] {
        assert!(!symbols.contains(forbidden), "{forbidden}");
    }
    receipt();
}
