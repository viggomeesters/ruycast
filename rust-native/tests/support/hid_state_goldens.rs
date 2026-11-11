//! Qualification of the fixed, genuinely captured pure-state Swift corpus.
use serde_json::Value;
use sha2::{Digest, Sha256};
pub const PATH: &str = "rust-native/tests/hid_state_reference.json";
pub const SHA256: &str = "2e7977aa2445b4bed784b9d2d53277b4f475c42d4c83d35d58188abf9d070bfc";
pub const ERROR: &str = "Use Ctrl+Left-Control or Alt+Left-Option for a single modifier tap.";
pub const SOURCES: &[&str] = &[
    PATH,
    "rust-native/tests/hid_state_process.rs",
    "rust-native/tests/support/hid_state_goldens.rs",
    "rust-native/tests/support/child_output.rs",
    "rust-native/src/bin/hid_state_probe.rs",
    "rust-native/src/keyboard_state.rs",
    "rust-native/src/recorder.rs",
    "rust/src/lib.rs",
    "rust-native/Cargo.toml",
    "rust-native/Cargo.lock",
    "rust/Cargo.toml",
    "rust/Cargo.lock",
    "rust-toolchain.toml",
];
pub const BATCHES: &[(&str, u64, usize, usize)] = &[
    (
        "generated_flags_rcmd_unknown_bits_and_all_modifier_combinations",
        0,
        4096,
        4096,
    ),
    (
        "generated_flags_rcmd_unknown_bits_and_all_modifier_combinations",
        1,
        4,
        4,
    ),
    (
        "multiple_devices_chords_cancel_remove_and_modifier_masks",
        0,
        29,
        159,
    ),
    (
        "permission_and_recovery_reason_precedence_is_not_activation",
        0,
        256,
        256,
    ),
    (
        "physical_edges_duplicates_timing_nonfinite_and_stray_events",
        0,
        52,
        251,
    ),
    (
        "sequence_oracle_fuzz_and_first_duplicate_binding_wins",
        0,
        48,
        4656,
    ),
    (
        "sided_binding_keycodes_exact_modifiers_and_swallowed_release",
        0,
        142,
        1562,
    ),
];
pub fn parse(bytes: &[u8]) -> Result<Value, String> {
    if format!("{:x}", Sha256::digest(bytes)) != SHA256 {
        return Err("Frozen Swift HID corpus changed".into());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let batches = value["batches"].as_array().ok_or("Missing HID batches")?;
    if value["schema"] != "ruycast.hid-state-swift-goldens.v1"
        || batches.len() != BATCHES.len()
        || value["caseCount"] != 4627
        || value["positiveCases"] != 4623
        || value["observedSnapshots"] != 10940
        || value["firstOperationRefusals"] != 4
    {
        return Err("Incomplete Swift HID corpus".into());
    }
    for (batch, (method, number, count, steps)) in batches.iter().zip(BATCHES) {
        let cases = batch["cases"].as_array().ok_or("Missing HID cases")?;
        let indices = batch["positiveIndices"]
            .as_array()
            .ok_or("Missing HID positive indices")?;
        let expected = batch["expected"]
            .as_array()
            .ok_or("Missing HID snapshots")?;
        let refusals = batch["refusals"].as_array().ok_or("Missing HID refusals")?;
        let step_count = cases
            .iter()
            .map(|c| c.as_array().map(Vec::len).ok_or("Invalid HID case"))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum::<usize>();
        if batch["method"] != *method
            || batch["batch"] != *number
            || cases.len() != *count
            || step_count != *steps
        {
            return Err("HID batch identity/count changed".into());
        }
        let negative_indices = if method.starts_with("sided_") {
            vec![90, 91, 92, 93]
        } else {
            vec![]
        };
        let positives = (0..*count)
            .filter(|i| !negative_indices.contains(i))
            .collect::<Vec<_>>();
        if indices
            != &positives
                .iter()
                .map(|i| Value::from(*i))
                .collect::<Vec<_>>()
            || expected.len() != positives.len()
            || refusals.len() != negative_indices.len()
        {
            return Err("HID positive/refusal partition changed".into());
        }
        for (snapshots, index) in expected.iter().zip(positives) {
            if snapshots
                .as_array()
                .ok_or("Invalid HID snapshot case")?
                .len()
                != cases[index].as_array().unwrap().len()
            {
                return Err("Missing ordered HID snapshot".into());
            }
        }
        for (refusal, index) in refusals.iter().zip(negative_indices) {
            if refusal["index"] != index
                || refusal["swiftExit"] != -5
                || refusal["swiftErrorContains"] != ERROR
                || refusal["rustExit"] != 1
                || refusal["executedInputOperations"] != 1
                || refusal["rustResponse"] != serde_json::json!({"ok":false,"error":ERROR})
                || cases[index].as_array().unwrap().len() != 11
            {
                return Err("HID parser refusal changed".into());
            }
        }
    }
    Ok(value)
}
