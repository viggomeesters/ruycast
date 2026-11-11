//! Genuine frozen Swift template corpus and separate live Rust budget cases.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use objc2_foundation::{NSLocale, NSProcessInfo, NSTimeZone};
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
const GOLDEN: &str = "rust-native/tests/template_reference.json";
const SHA: &str = "5525ab96d374e81259e0b4b94cd6c85f62764da931868c5b57f132573283cb38";
const BATCHES: &[(&str, usize)] = &[
    (
        "application_data_is_not_bounded_by_socket_or_fixture_limits",
        2,
    ),
    ("canonical_fields_options_and_ordered_value_insertion", 4),
    (
        "clipboard_demand_matches_current_appstate_character_semantics",
        13,
    ),
    (
        "combining_character_after_date_prefix_is_not_an_automatic_token",
        12,
    ),
    (
        "current_locale_and_zone_defaults_without_global_mutation",
        1,
    ),
    ("field_order_defaults_options_and_prompt_ownership", 13),
    ("fixed_dates_locales_zones_dst_and_custom_patterns", 29),
    ("large_invalid_clipboard_demand_corpus", 2),
    ("malformed_nested_newline_and_combining_brace_grammar", 17),
    (
        "missing_blank_and_explicit_empty_values_preserve_reverse_error_order",
        7,
    ),
    ("repeated_large_values_and_cursor_parity", 2),
    ("seeded_mixed_token_corpus", 300),
    ("url_percent_encoding_is_only_for_user_fields", 3),
    ("utf16_cursor_multiple_marks_and_literal_replacements", 5),
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
fn context() -> Value {
    json!({"locale":NSLocale::currentLocale().localeIdentifier().to_string(),"timeZone":NSTimeZone::defaultTimeZone().name().to_string(),"os":NSProcessInfo::processInfo().operatingSystemVersionString().to_string()})
}
fn corpus() -> &'static Value {
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE.get_or_init(|| {
        let bytes = fs::read(root().join(GOLDEN)).unwrap();
        assert_eq!(hash(&bytes), SHA);
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema"], "ruycast.template-swift-goldens.v1");
        assert_eq!(value["differentialMethods"], 14);
        assert_eq!(value["differentialCases"], 410);
        assert_eq!(value["rustOnlyMethods"], 2);
        assert_eq!(value["rustOnlyCases"], 6);
        let batches = value["batches"].as_array().unwrap();
        assert_eq!(batches.len(), BATCHES.len());
        let mut total = 0;
        for (batch, (name, count)) in batches.iter().zip(BATCHES) {
            assert_eq!(batch["method"], *name);
            assert_eq!(batch["cases"].as_array().unwrap().len(), *count);
            assert_eq!(batch["expected"].as_array().unwrap().len(), *count);
            total += count;
            for case in batch["cases"].as_array().unwrap() {
                assert!(
                    case.get("limits").is_none(),
                    "Rust-only budgets are not Swift goldens"
                );
                assert!(case["values"].is_array(), "Keep ordered pairs, not a map");
            }
        }
        assert_eq!(total, 410);
        value
    })
}
fn probe() -> &'static Path {
    static PROBE: OnceLock<PathBuf> = OnceLock::new();
    PROBE
        .get_or_init(|| {
            let source = PathBuf::from(env!("CARGO_BIN_EXE_template-probe"));
            assert!(source.is_absolute() && source.is_file());
            if source.starts_with(root()) {
                return source;
            }
            let copy = root().join(format!(
                "rust-native/target/template-golden-probe-{}",
                std::process::id()
            ));
            assert!(!copy.exists());
            fs::create_dir_all(copy.parent().unwrap()).unwrap();
            fs::copy(&source, &copy).unwrap();
            assert_eq!(
                hash(&fs::read(source).unwrap()),
                hash(&fs::read(&copy).unwrap())
            );
            copy
        })
        .as_path()
}
fn invoke(cases: &Value, large: bool) -> Value {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("rust-native/target"));
    assert!(parent.is_absolute() && parent.is_dir());
    let dir = parent.join(format!(
        "template-{}-{}",
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
    let child = Command::new(probe())
        .env_clear()
        .stdin(fs::File::open(input).unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Original application-data output is 1,200,333 bytes before its envelope.
    // Only this real original case needs more than the ordinary 1 MiB bound.
    let output = if large {
        child_output::wait_with_limit(child, Duration::from_secs(15), 2 * 1024 * 1024)
    } else {
        child_output::wait(child, Duration::from_secs(15))
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["pureTemplateFixture"], true);
    value["cases"].clone()
}
fn receipt() {
    if let Some(directory) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        fs::create_dir_all(&directory).unwrap();
        let root = root();
        let sources = [
            GOLDEN,
            "rust-native/tests/template_process.rs",
            "rust-native/tests/support/child_output.rs",
            "rust-native/src/bin/template_probe.rs",
            "rust-native/src/templates.rs",
            "rust-native/src/native_text.rs",
            "rust-native/src/command_identity.rs",
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
        fs::write(PathBuf::from(directory).join("template-reference-bindings.json"),serde_json::to_vec_pretty(&json!({"sources":sources,"artifacts":{artifact:hash(&fs::read(probe()).unwrap())},"goldenSHA256":SHA,"differentialMethods":14,"differentialCases":410,"rustOnlyMethods":2,"rustOnlyCases":6,"defaultContext":context(),"scope":"Fixed genuine Swift template corpus; synthetic clipboard/date arguments only, context-sensitive default case refuses drift"})).unwrap()).unwrap();
    }
}
fn compare(method: &str) {
    let batch = corpus()["batches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["method"] == method)
        .unwrap();
    if method == "current_locale_and_zone_defaults_without_global_mutation" {
        assert!(batch["cases"][0]["locale"].is_null() && batch["cases"][0]["timeZone"].is_null());
        assert_eq!(context(),corpus()["defaultContext"],"Foundation context changed; recapture/review this actual default-context case, never silently skip or fill its nulls");
    }
    let actual = invoke(
        &batch["cases"],
        method == "application_data_is_not_bounded_by_socket_or_fixture_limits",
    );
    assert_eq!(
        actual.as_array().unwrap().len(),
        batch["cases"].as_array().unwrap().len()
    );
    for (i, (expected, actual)) in batch["expected"]
        .as_array()
        .unwrap()
        .iter()
        .zip(actual.as_array().unwrap())
        .enumerate()
    {
        assert_eq!(expected, actual, "{method} case{i}");
    }
    match method {
        "clipboard_demand_matches_current_appstate_character_semantics" => assert_eq!(
            actual.as_array().unwrap()[..5]
                .iter()
                .map(|c| c["needsClipboard"].as_bool().unwrap())
                .collect::<Vec<_>>(),
            [true, false, false, false, true]
        ),
        "large_invalid_clipboard_demand_corpus" => assert_eq!(
            actual
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["needsClipboard"].as_bool().unwrap())
                .collect::<Vec<_>>(),
            [false, true]
        ),
        "application_data_is_not_bounded_by_socket_or_fixture_limits" => {
            assert_eq!(actual[0]["render"]["text"], "https://fixture.invalid/");
            assert_eq!(actual[1]["render"]["text"].as_str().unwrap().len(), 1200024);
        }
        "utf16_cursor_multiple_marks_and_literal_replacements" => {
            assert_eq!(actual[1]["render"]["cursor"], 2)
        }
        _ => {}
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
method!(application_data_is_not_bounded_by_socket_or_fixture_limits);
method!(canonical_fields_options_and_ordered_value_insertion);
method!(clipboard_demand_matches_current_appstate_character_semantics);
method!(combining_character_after_date_prefix_is_not_an_automatic_token);
method!(current_locale_and_zone_defaults_without_global_mutation);
method!(field_order_defaults_options_and_prompt_ownership);
method!(fixed_dates_locales_zones_dst_and_custom_patterns);
method!(large_invalid_clipboard_demand_corpus);
method!(malformed_nested_newline_and_combining_brace_grammar);
method!(missing_blank_and_explicit_empty_values_preserve_reverse_error_order);
method!(repeated_large_values_and_cursor_parity);
method!(seeded_mixed_token_corpus);
method!(url_percent_encoding_is_only_for_user_fields);
method!(utf16_cursor_multiple_marks_and_literal_replacements);
fn fixture(target: &str, values: Value) -> Value {
    json!({"command":{"id":"fixture","title":"Fixture","kind":"snippet","target":target},"values":values,"now":1761411300,"locale":"en_GB","timeZone":"UTC"})
}
#[test]
fn explicit_budget_and_invalid_context_guards() {
    let mut cases = vec![
        fixture("12345", json!([])),
        fixture("{x}{y}", json!([])),
        fixture("{x}", json!([["x", "123456789"]])),
        fixture("{x}", json!([["x", "a".repeat(200)]])),
        fixture("plain", json!([])),
    ];
    for (case, limits) in cases.iter_mut().zip([
        json!({"sourceBytes":4,"tokens":4,"outputBytes":8}),
        json!({"sourceBytes":100,"tokens":1,"outputBytes":100}),
        json!({"sourceBytes":100,"tokens":4,"outputBytes":8}),
        json!({"sourceBytes":100,"tokens":4,"outputBytes":8}),
    ]) {
        case["limits"] = limits;
    }
    cases[4]["timeZone"] = json!("dev-fixture-invalid-zone");
    let result = invoke(&json!(cases), false);
    assert_eq!(result.as_array().unwrap().len(), 5);
    assert_eq!(result[0]["ok"], false);
    assert_eq!(result[1]["ok"], false);
    for case in &result.as_array().unwrap()[2..] {
        assert_eq!(case["ok"], true);
        assert_eq!(case["render"]["ok"], false);
    }
    receipt();
}
#[test]
fn output_bound_uses_final_size_not_intermediate_reverse_size() {
    let mut case = fixture("{cursor}{x}", json!([["x", "12345678"]]));
    case["limits"] = json!({"sourceBytes":100,"tokens":4,"outputBytes":8});
    let actual = invoke(&json!([case]), false);
    assert_eq!(actual.as_array().unwrap().len(), 1);
    assert_eq!(
        actual[0]["render"],
        json!({"ok":true,"text":"12345678","cursor":0})
    );
    receipt();
}
