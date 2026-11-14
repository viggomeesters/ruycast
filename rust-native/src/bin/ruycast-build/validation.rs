//! The native validation matrix, with unchanged leaf validators. Compatibility
//! artifact copies are freshly hash-bound test inputs, never delivery inputs.
use super::*;
use std::time::{Duration, Instant};
const DIRECTORY: &str = "rust-native/target/verification";
fn check_host_sdk_paths<'a>(paths: impl Iterator<Item = &'a String>) -> Result<()> {
    let expected = ["GeneralKeyboard", "NativeChoice", "NativeHid", "NativeKeyboard", "NativeKeyword"]
        .map(|name| format!("{DIRECTORY}/native-profile/host-sdks/{name}.dylib"))
        .into_iter().collect::<std::collections::BTreeSet<_>>();
    if paths.cloned().collect::<std::collections::BTreeSet<_>>() != expected {
        return Err("Exactly the five required native host SDK roles and paths must be present.".into());
    }
    Ok(())
}
// Exact source roster emitted by the actual NativeKeyboard producer.
const KEYBOARD_SDK_SOURCES: &[&str] = &[
    "rust-native/Cargo.toml",
    "rust-native/Cargo.lock",
    "rust/Cargo.toml",
    "rust-toolchain.toml",
    "rust-native/tests/fixtures/keyboard_host.rs",
    "rust-native/tests/fixtures/caps_sdk.rs",
    "rust-native/tests/fixtures/hid_sdk.rs",
    "rust-native/tests/fixtures/keyboard_sdk.rs",
    "rust-native/tests/fixtures/keyword_core.rs",
    "rust-native/tests/fixtures/keyword_notification.rs",
    "rust-native/tests/fixtures/keyword_board.rs",
    "rust-native/tests/fixtures/keyword_ax.rs",
    "rust-native/tests/fixtures/keyword_events.rs",
    "rust-native/tests/fixtures/workspace_sdk.rs",
    "rust-native/tests/fixtures/keyboard_host_loader.json",
];
const SYSTEM_UI_SOURCES: &[&str] = &[
    "rust-native/tests/support/native_keyboard.rs",
    "rust-native/tests/system_search_sdk.rs",
    "rust-native/tests/support/owned_keyboard.rs",
];
// Ordered transcription of the former check.py matrix. `p` invokes an existing
// validator; `b` builds an explicit probe; `c` preserves a full Cargo command.
const MATRIX: &str = r#"
rust-only-policy|p|rust-native/scripts/check_rust_only.py
rust-only-policy-tests|p|rust-native/tests/rust_only_policy.py
core-unit|c|test --manifest-path rust/Cargo.toml --locked
unit|c|test --manifest-path rust-native/Cargo.toml --locked
clippy-default|c|clippy --manifest-path rust-native/Cargo.toml --locked --all-targets -- -D warnings
clippy|c|clippy --manifest-path rust-native/Cargo.toml --locked --all-targets --all-features -- -D warnings
host-build|b|ruycast-native default
headless-tools-build|b|ruycast-native headless
analytics-probe-build|b|analytics-probe tools
analytics-write-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example analytics-write-shim --locked
analytics-runtime-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test analytics_runtime_process --locked
analytics-cast-reference|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test analytics_cast_reference --locked
dashboard-presentation-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test dashboard_presentation_reference --locked
probe-build|b|foundation-probe tools
script-probe-build|b|script-probe tools
supervision-probe-build|b|supervision-probe tools
workspace-probe-build|b|workspace-probe tools
workspace-process|p|rust-native/tests/workspace_process.py
template-probe-build|b|template-probe tools
template-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test template_process --locked
snippet-probe-build|b|snippet-probe tools
snippet-faults-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example snippet-faults --locked
snippet-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test snippet_process --locked
snippet-html-import|c|run --manifest-path rust-native/Cargo.toml --features test-tools --bin snippet-probe --locked -- --check-html-import
snippet-board-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example snippet-board-fixture --locked
snippet-board-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test snippet_board_process --locked
snippet-choice-probe-build|b|snippet-choice-probe tools
snippet-choice-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test snippet_choice_process --locked
snippet-choice-request-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test snippet_choice_request_process --locked
native-choice-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example choice-fixture --locked
native-snippet-choice-probe-build|b|native-snippet-choice-probe tools
native-snippet-choice-process-rust|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test native_snippet_choice_process --locked
keyword-probe-build|b|keyword-probe tools
keyword-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test keyword_process --locked
keyword-native-probe-build|b|keyword-native-probe tools
keyword-native-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example keyword-fixture --example keyword-missing-value-type --locked
keyword-native-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test keyword_native_process --locked
workspace-activation-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test workspace_activation_process --locked
keyword-capture-probe-build|b|keyword-capture-probe tools
keyword-capture-process-rust|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test keyword_capture_process --locked
keyword-rich-process-rust|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test keyword_rich_process --locked
keyboard-probe-build|b|keyboard-probe tools
keyboard-process|p|rust-native/tests/keyboard_process.py
hid-state-probe-build|b|hid-state-probe tools
hid-state-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test hid_state_process --locked
caps-probe-build|b|caps-probe tools
caps-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example caps-fixture --example caps-missing-set --locked
caps-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test caps_process --locked
hid-probe-build|b|hid-probe tools
hid-process|p|rust-native/tests/hid_process.py
manager-probe-build|b|manager-probe tools
manager-workspace-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example workspace-fixture --locked
manager-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test manager_process --locked
cli-open-basic-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test cli_open_basic_process --locked
supervision-process|p|rust-native/tests/supervision_process.py
script-fault-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example script-fault-shim --locked
script-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test script_basic_process --locked
script-fault-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test script_fault_process --locked
runtime-writer-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example fcntl-sync-shim --locked
foundation-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test foundation_process --locked
runtime-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test runtime_process --locked
native-profile-process|p|rust-native/tests/native_process.py
bridge-tools-build|b|ruycast-native native
native-dashboard-dialog-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test native_dashboard_dialog_process --locked
dashboard-export-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test dashboard_export_process --locked
settings-preferences-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test settings_preferences_process --locked
native-keyboard-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example keyboard-host --locked
settings-keyboard-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test settings_keyboard_process --locked
clipboard-text-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test clipboard_text_process --locked
native-keyboard-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test native_keyboard_process --locked
native-choice-host-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test native_choice_host_process --locked
snippet-effects-process|p|rust-native/tests/snippet_effects_process.py
native-open-basic-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test native_open_basic_process --locked
app-picker-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test app_picker_identity_process --locked
bridge-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test bridge_process --locked
system-search-tools-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --bin ruycast-native --bin system-search-probe --locked
system-search-cli-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test system_search_cli_process --locked
clipboard-history-process|p|rust-native/tests/clipboard_history_process.py
snippet-links-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --snippet-links-only
snippet-hotkey-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --snippet-hotkey-only
feedback-actions-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --feedback-actions-only
actions-menu-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --actions-menu-only
item-binding-original-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --item-binding-original-only
settings-ui-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --settings-only
script-advanced-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --script-advanced-only
clipboard-rich-paste|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test clipboard_rich_paste --locked
clipboard-preview-original|c|test --manifest-path rust-native/Cargo.toml --features test-tools --lib clipboard_preview::tests::original_two_color_image_and_local_file_previews_preserve_pixels_and_refuse_bad_assets --locked -- --exact
clipboard-preview-cancel-original|c|test --manifest-path rust-native/Cargo.toml --features test-tools --lib clipboard_preview::tests::cancelled_same_id_reopen_cannot_publish_old_error_or_asset --locked -- --exact
system-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example system-fixture --locked
system-search-sdk|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test system_search_sdk --locked
remaining-macos-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test remaining_macos_process --locked
startup-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test startup_process --locked
private-results-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test private_results --locked
cli-contract-process|c|test --manifest-path rust/Cargo.toml --bin ruycast startup_discovery_tests::current_cli_ --locked -- --ignored
render-updates-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --render-updates-only
polish-bindings-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --polish-bindings-only
polish-original-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --polish-original-only
palette-lifecycle-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --palette-lifecycle-only
interaction-trace-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --interaction-trace-only
clipboard-pause-original|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test clipboard_pause_repair --locked -- real_host_preserves_invalid_pause_and_requires_confirmed_explicit_resume --exact
task-date-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --task-date-only
utility-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test utility_process --locked
applications-discovery-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test applications_discovery_process --locked
actions-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test actions_process --locked
editor-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test editor_process --locked
creation-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test creation_process --locked
library-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test library_lifecycle_process --locked
library-checkbox-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test library_checkbox_process --locked
library-drafts-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test library_drafts_process --locked
library-navigation-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test library_navigation_process --locked
inline-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test inline_sync_process --locked
inline-basic-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test inline_basic_process --locked
inline-composition-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test inline_composition_process --locked
inline-transition-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test inline_transition_process --locked
inline-focus-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test inline_focus_process --locked
input-process-rust|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test input_process --locked
execution-basic-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test execution_basic_process --locked
execution-native-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test execution_native_process --locked
script-result-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test script_result_basic_process --locked
script-result-native-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test script_result_native_process --locked
capture-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test capture_native_process --locked
capture-bytes-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test capture_bytes_process --locked
capture-cli-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test capture_cli_process --locked
rich-process-rust|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test rich_process --locked
clipboard-import-parser|p|Tests/test_clipboard_import.py
benchmark-contract-1|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test startup_process --locked benchmark_cli_contracts::real_rust_benchmark_ignores_external_configuration_with_500_commands -- --ignored --exact
benchmark-contract-2|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test startup_process --locked benchmark_cli_contracts::real_rust_benchmark_rejects_changed_cli_and_quits_gracefully -- --ignored --exact
temporal-contract-1|c|test --manifest-path rust-native/Cargo.toml --bin ruycast-build --locked compat::temporal_contract_tests::final_consumer_keeps_artifacts_leased_through_actual_receipt_publication -- --ignored --exact
temporal-contract-2|c|test --manifest-path rust-native/Cargo.toml --bin ruycast-build --locked compat::temporal_contract_tests::source_mutation_during_final_consumer_cannot_publish_pipeline_pass -- --ignored --exact
temporal-contract-3|c|test --manifest-path rust-native/Cargo.toml --bin ruycast-build --locked compat::temporal_contract_tests::artifact_mutation_during_final_consumer_cannot_publish_pipeline_pass -- --ignored --exact
temporal-contract-4|c|test --manifest-path rust-native/Cargo.toml --bin ruycast-build --locked compat::temporal_contract_tests::settings_writer_and_performance_authority_refusal_keep_the_pipeline_lease -- --ignored --exact
temporal-contract-5|c|test --manifest-path rust-native/Cargo.toml --bin ruycast-build --locked compat::temporal_contract_tests::shared_pipeline_lease_survives_successful_consumer_and_final_return -- --ignored --exact
release-build|b|ruycast-native release
"#;
const REFERENCES: &[(&str, &str)] = &[
    ("templateReference", "template-reference-bindings.json"),
    ("snippetReference", "snippet-reference-bindings.json"),
    ("snippetEffects", "snippet-effects-bindings.json"),
    ("keyboardReference", "keyboard-reference-bindings.json"),
    ("hidStateReference", "hid-state-reference-bindings.json"),
    ("keywordReference", "keyword-reference-bindings.json"),
    ("snippetBoard", "snippet-board-bindings.json"),
    (
        "clipboardTextReference",
        "clipboard-text-reference-bindings.json",
    ),
    (
        "snippetChoiceReference",
        "snippet-choice-reference-bindings.json",
    ),
    ("nativeSnippetChoice", "native-snippet-choice-bindings.json"),
    ("nativeChoiceHost", "native-choice-bindings.json"),
];
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    name: String,
    arguments: Vec<String>,
    copies: Vec<(String, String)>,
}
fn matrix(target: &str) -> Result<Vec<Step>> {
    artifact_paths(target)?;
    let mut steps = vec![];
    for line in MATRIX.lines().filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('|').collect();
        if fields.len() != 3 {
            return Err("Malformed native validation matrix.".into());
        }
        let mut copies = vec![];
        let arguments = match fields[1] {
            "p" => {
                let mut args=vec!["python3".into(), fields[2].into()];
                if fields[0]=="clipboard-import-parser" {args.push("-v".into());}
                args
            },
            "c" => {
                let mut args = vec!["cargo".to_owned()];
                args.extend(fields[2].split_whitespace().map(str::to_owned));
                let separator = args.iter().position(|v| v == "--").unwrap_or(args.len());
                args.splice(
                    separator..separator,
                    [
                        "--target".into(),
                        target.into(),
                        "--target-dir".into(),
                        "rust-native/target/validation-build".into(),
                        "--offline".into(),
                    ],
                );
                if args.get(1).is_some_and(|value| value == "test") {
                    if !args.iter().any(|value| value == "--") {
                        args.push("--".into());
                    }
                    args.push("--test-threads=1".into());
                }
                for pair in args.windows(2) {
                    if pair[0] == "--bin" && fields[0] != "cli-contract-process"
                        && !fields[0].starts_with("temporal-contract-") {
                        copies.push((
                            format!(
                                "rust-native/target/validation-build/{target}/debug/{}",
                                pair[1]
                            ),
                            format!("rust-native/target/debug/{}", pair[1]),
                        ));
                    }
                }
                args
            }
            "b" => {
                let spec: Vec<_> = fields[2].split_whitespace().collect();
                if spec.len() != 2 {
                    return Err("Malformed validation build descriptor.".into());
                }
                let (build_root, alias, configuration, tools) = match spec[1] {
                    "default" => (
                        "rust-native/target/validation-build",
                        "rust-native/target",
                        "debug",
                        false,
                    ),
                    "tools" => (
                        "rust-native/target/validation-build",
                        "rust-native/target",
                        "debug",
                        true,
                    ),
                    "headless" => (
                        "rust-native/target/validation-headless",
                        "rust-native/target/headless-tools",
                        "debug",
                        true,
                    ),
                    "native" => (
                        "rust-native/target/validation-native",
                        "rust-native/target/native-tools",
                        "debug",
                        true,
                    ),
                    "release" => (
                        "rust-native/target/validation-release",
                        "rust-native/target",
                        "release",
                        false,
                    ),
                    _ => return Err("Unknown validation build descriptor.".into()),
                };
                let mut args = vec![
                    "cargo",
                    "build",
                    "--manifest-path",
                    "rust-native/Cargo.toml",
                    "--bin",
                    spec[0],
                    "--target",
                    target,
                    "--target-dir",
                    build_root,
                    "--locked",
                    "--offline",
                ];
                if tools {
                    args.extend(["--features", "test-tools"]);
                }
                if configuration == "release" {
                    args.push("--release");
                }
                copies.push((
                    format!("{build_root}/{target}/{configuration}/{}", spec[0]),
                    format!("{alias}/{configuration}/{}", spec[0]),
                ));
                args.into_iter().map(str::to_owned).collect()
            }
            _ => return Err("Unknown validation matrix command.".into()),
        };
        steps.push(Step {
            name: fields[0].into(),
            arguments,
            copies,
        });
    }
    Ok(steps)
}
pub(super) fn source_folder(
    root: &Path,
    path: &Path,
    values: &mut BTreeMap<String, String>,
) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(error)?;
    if meta.file_type().is_symlink() {
        return Err("Linked validation source.".into());
    }
    if meta.is_dir() {
        for entry in fs::read_dir(path).map_err(error)? {
            let entry = entry.map_err(error)?;
            if entry.file_name() != "__pycache__" {
                source_folder(root, &entry.path(), values)?;
            }
        }
    } else if meta.is_file() {
        values.insert(
            path.strip_prefix(root)
                .map_err(error)?
                .to_str()
                .ok_or("Invalid validation source path.")?
                .into(),
            digest(path)?,
        );
    } else {
        return Err("Nonregular validation source.".into());
    }
    Ok(())
}
fn sources(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut rows = BTreeMap::new();
    for name in [
        "rust-native/Cargo.toml",
        "rust-native/Cargo.lock",
        "rust-native/README.md",
        "rust-native/swift-reference-lock.json",
        "rust/Cargo.toml",
        "rust/Cargo.lock",
        "config/default.json",
        "Sources/LauncherCore/Templates.swift",
        "Sources/LauncherModels/Models.swift",
        "Sources/RuycastApp/AppState.swift",
        "Sources/RuycastApp/CommandEditor.swift",
        "Sources/LauncherCore/SnippetOutput.swift",
        "Sources/LauncherCore/SnippetMatcher.swift",
        "Sources/RuycastApp/SnippetClipboard.swift",
        "Sources/RuycastApp/PaletteActions.swift",
        "Sources/RuycastApp/HyperKey.swift",
        "Sources/RuycastApp/Palette.swift",
        "Sources/RuycastApp/Hotkeys.swift",
        "Sources/LauncherCore/RustBridge.swift",
        "Sources/LauncherCore/KeyboardState.swift",
        "Sources/LauncherCore/Files.swift",
        "Sources/RuycastApp/SnippetExpander.swift",
        "Sources/RuycastApp/SnippetChoice.swift",
        "Sources/RuycastApp/KeyboardChecks.swift",
        "Sources/RuycastApp/SettingsChecks.swift",
        "Sources/RuycastApp/AnalyticsStore.swift",
        "rust-native/src",
        "rust-native/tests",
        "rust-native/scripts",
        "rust-native/examples/ui_completion.rs",
        "rust/src",
        "Tests/test_clipboard_import.py",
        "scripts/import-clipboard.py",
        "Tests/test_cli.py",
        "scripts/benchmark-host.py",
    ] {
        source_folder(root, &root.join(name), &mut rows)?;
    }
    Ok(rows)
}
fn log(path: &Path) -> Result<File> {
    ruycast_native::private_storage::write(path, b"", true)?;
    let file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(error)?;
    let meta = file.metadata().map_err(error)?;
    if !meta.is_file() || meta.nlink() != 1 || meta.uid() != unsafe { libc::getuid() } {
        return Err("Invalid validation log file.".into());
    }
    Ok(file)
}
fn sanitized(command: &mut Command, root: &Path, lease: &BuildLock, fixture: &Path) -> Result<()> {
    for (key, _) in std::env::vars_os() {
        let value = key.to_string_lossy();
        if value.starts_with("RUYCAST_")
            || value.starts_with("RUSTCAST_")
            || value.starts_with("DYLD_")
        {
            command.env_remove(key);
        }
    }
    if !fixture.is_absolute() {
        return Err("Validation fixture parent must be absolute.".into());
    }
    command
        .env("RUYCAST_NATIVE_TEST_FIXTURE_DIR", fixture)
        .env("RUYCAST_INSTALL_TEST_ROOT", fixture)
        .env("RUYCAST_TEST_CLI", root.join(".build/rust/release/ruycast"))
        .env("RUYCAST_REPAIR_CLI", root.join(".build/rust/release/ruycast"))
        .env("RUYCAST_RICH_PASTE_CLI", root.join(".build/rust/release/ruycast"))
        .env("TMPDIR", fixture)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env(
            "RUYCAST_NATIVE_EVIDENCE_DIR",
            root.join(DIRECTORY).join("native-profile"),
        );
    lease.apply_to_child(command)
}
fn step(
    root: &Path,
    lease: &BuildLock,
    fixture: &Path,
    entry: &Step,
    receipt: &Receipt,
    copies: &mut BTreeMap<String, serde_json::Value>,
) -> Result<serde_json::Value> {
    lease.validate()?;
    let start = Instant::now();
    let mut command = Command::new(&entry.arguments[0]);
    command.args(&entry.arguments[1..]).current_dir(root);
    sanitized(&mut command, root, lease, fixture)?;
    if entry.name == "startup-process" {
        let (artifact, sha) = keyboard_fixture(root)?;
        command
            .env("RUYCAST_TEST_KEYBOARD_SDK", artifact)
            .env("RUYCAST_TEST_KEYBOARD_SHA", sha);
    }
    // Keep this existing exclusive stage alive until the child has stopped.
    // Its short path is independent of any ambient fixture/evidence variables.
    let mut system_ui = if entry.name == "system-search-sdk" {
        let owned = preparation::Stage::in_parent(&root.join(".build"), "u")?;
        system_ui_socket_budget(owned.path())?;
        let inputs = system_ui_environment(root, entry, owned.path(), &mut command)?;
        Some((owned, inputs))
    } else {
        None
    };
    let example_ui = if matches!(entry.name.as_str(), "render-updates-process" | "polish-bindings-process" | "polish-original-process" | "task-date-process" | "actions-menu-process" | "item-binding-original-process" | "palette-lifecycle-process" | "interaction-trace-process" | "feedback-actions-process") {
        let owned = preparation::Stage::in_parent(&root.join(".build"), "u")?;
        example_ui_socket_budget(owned.path())?;
        let inputs = example_ui_environment(root, entry, owned.path(), &mut command)?;
        Some((owned, inputs))
    } else { None };
    let cli_contract = if entry.name == "cli-contract-process" {
        let owned = preparation::Stage::in_parent(&root.join(".build"), "u")?;
        cli_contract_socket_budget(owned.path())?;
        let inputs = cli_contract_environment(root, receipt, owned.path(), &mut command)?;
        Some((owned, inputs))
    } else { None };
    let benchmark = if entry.name.starts_with("benchmark-contract-") {
        let owned = preparation::Stage::in_parent(&root.join(".build"), "u")?;
        cli_contract_socket_budget(owned.path())?;
        let inputs = benchmark_environment(root, receipt, owned.path(), &mut command)?;
        Some((owned, inputs))
    } else { None };
    let temporal = if entry.name.starts_with("temporal-contract-") {
        let inputs = temporal_environment(root, receipt, &mut command)?;
        Some(inputs)
    } else { None };
    let _pause_fixture = if entry.name == "clipboard-pause-original" {
        let owned = preparation::Stage::in_parent(&root.join(".build"), "u")?;
        // The existing UI budget is stricter than rcpr-PID-directory/p/control.sock.
        example_ui_socket_budget(owned.path())?;
        command.env("RUYCAST_NATIVE_TEST_FIXTURE_DIR", owned.path()).env("TMPDIR", owned.path());
        Some(owned)
    } else { None };
    // Tool builds may replace the legacy alias. Restore only for the existing
    // normal-feature negative routes, from the already verified default host.
    let normal_host = if normal_host_negative(entry) {
        Some(prepare_normal_host(root, receipt, copies)?)
    } else { None };
    let captured = process::capture_logged(
        command,
        Duration::from_secs(180),
        64 * 1024 * 1024,
        Some((
            log(&root.join(DIRECTORY).join(format!("{}.stdout", entry.name)))?,
            log(&root.join(DIRECTORY).join(format!("{}.stderr", entry.name)))?,
        )),
    )?;
    if !captured.status.success() {
        return Err(format!(
            "{} failed ({}); inspect {DIRECTORY} logs. No pass published.",
            entry.name, captured.status
        ));
    }
    if let Some(inputs) = &normal_host { check_normal_host_inputs(root, receipt, inputs)?; }
    selected_step_output(&entry.name, &captured.stdout)?;
    if entry.name=="clipboard-import-parser" {
        let text=std::str::from_utf8(&captured.stderr).map_err(error)?;
        let cases=["test_text_stable_identity_dedup_and_date","test_files_are_references_and_missing_images_explicit",
            "test_malformed_oversize_and_unsupported","test_bounded_batches_and_retry_identity"];
        if cases.iter().any(|case|text.lines().filter(|line|line.starts_with(&format!("{case} ("))&&line.ends_with(") ... ok")).count()!=1)
            ||text.lines().filter(|line|line.contains(" ... ")).count()!=4
            ||!text.lines().any(|line|line.starts_with("Ran 4 tests in "))
            ||!text.lines().any(|line|line=="OK") {
            return Err("All four public clipboard parser cases must actually pass without skips.".into());
        }
    }
    for (source, alias) in &entry.copies {
        compatibility_copy(root, source, alias, copies)?;
    }
    if let Some((_, inputs)) = &mut system_ui {
        check_binding(root, inputs)?;
        let evidence = root.join(inputs["evidenceDirectory"].as_str().ok_or("Missing owned UI evidence directory.")?);
        let mut generated = BTreeMap::new();
        source_folder(root, &evidence, &mut generated)?;
        if generated.is_empty() {
            return Err("System UI tests produced no actual owned evidence.".into());
        }
        inputs["artifacts"].as_object_mut().ok_or("Missing system UI input bindings.")?
            .extend(serde_json::to_value(generated).map_err(error)?.as_object().ok_or("Invalid UI evidence bindings.")?.clone());
    }
    lease.validate()?;
    let mut result = serde_json::json!({"name":entry.name,"arguments":entry.arguments,"exitCode":captured.status.code(),"seconds":start.elapsed().as_secs_f64()});
    if let Some(inputs) = normal_host { result["normalHostInputs"] = inputs; }
    if let Some((_, inputs)) = system_ui {
        result["systemUiInputs"] = inputs;
    }
    if let Some((_, mut inputs)) = example_ui {
        check_binding(root, &inputs)?;
        let evidence = root.join(inputs["evidenceDirectory"].as_str().ok_or("Missing example UI evidence directory.")?);
        let mut generated = BTreeMap::new();
        source_folder(root, &evidence, &mut generated)?;
        if generated.is_empty() { return Err("No actual example UI evidence was produced.".into()); }
        inputs["artifacts"].as_object_mut().ok_or("Missing example UI artifact bindings.")?
            .extend(serde_json::to_value(generated).map_err(error)?.as_object().ok_or("Invalid example UI evidence bindings.")?.clone());
        result["exampleUiInputs"] = inputs;
    }
    if let Some((_, inputs)) = cli_contract {
        check_cli_contract_inputs(root, receipt, &inputs)?;
        result["cliContractInputs"] = inputs;
    }
    if let Some((_, inputs)) = benchmark {
        check_benchmark_inputs(root, receipt, &inputs)?;
        result["benchmarkInputs"] = inputs;
    }
    if let Some(inputs) = temporal {
        check_temporal_inputs(root, receipt, &inputs)?;
        result["temporalInputs"] = inputs;
    }
    Ok(result)
}
const BENCHMARK_CASES: [&str;2] = [
    "benchmark_cli_contracts::real_rust_benchmark_ignores_external_configuration_with_500_commands",
    "benchmark_cli_contracts::real_rust_benchmark_rejects_changed_cli_and_quits_gracefully",
];
const TEMPORAL_CASES: [&str;5] = [
    "compat::temporal_contract_tests::final_consumer_keeps_artifacts_leased_through_actual_receipt_publication",
    "compat::temporal_contract_tests::source_mutation_during_final_consumer_cannot_publish_pipeline_pass",
    "compat::temporal_contract_tests::artifact_mutation_during_final_consumer_cannot_publish_pipeline_pass",
    "compat::temporal_contract_tests::settings_writer_and_performance_authority_refusal_keep_the_pipeline_lease",
    "compat::temporal_contract_tests::shared_pipeline_lease_survives_successful_consumer_and_final_return",
];
fn benchmark_environment(root:&Path, receipt:&Receipt, fixture:&Path, command:&mut Command)->Result<serde_json::Value> {
    let contract=cli_contract_environment(root,receipt,fixture,command)?;
    let python=std::env::split_paths(&std::env::var_os("PATH").ok_or("Missing benchmark interpreter search path.")?)
        .map(|directory|directory.join("python3")).find(|path|path.is_file())
        .ok_or("Missing supported benchmark interpreter.")?.canonicalize().map_err(error)?;
    if python.parent().ok_or("Missing interpreter parent.")?.join("python3").canonicalize().map_err(error)?!=python {
        return Err("Benchmark shebang does not select the qualified interpreter.".into());
    }
    let python_sha=digest(&python)?;
    command.env("RUYCAST_BENCHMARK_PYTHON",&python).env("RUYCAST_BENCHMARK_PYTHON_SHA256",&python_sha);
    let mut sources=BTreeMap::new();
    for name in ["scripts/benchmark-host.py","Tests/test_cli.py"] {source_folder(root,&root.join(name),&mut sources)?;}
    Ok(serde_json::json!({"contract":contract,"python":python,"pythonSHA256":python_sha,"sources":sources}))
}
fn check_benchmark_inputs(root:&Path, receipt:&Receipt, inputs:&serde_json::Value)->Result<()> {
    check_cli_contract_inputs(root,receipt,&inputs["contract"])?;
    let python=Path::new(inputs["python"].as_str().ok_or("Missing qualified benchmark interpreter.")?);
    if !python.is_absolute()||python.canonicalize().map_err(error)?!=python
        ||python.parent().ok_or("Missing interpreter parent.")?.join("python3").canonicalize().map_err(error)?!=python
        ||inputs["pythonSHA256"]!=digest(python)? {return Err("Benchmark interpreter changed.".into());}
    let mut sources=BTreeMap::new();
    for name in ["scripts/benchmark-host.py","Tests/test_cli.py"] {source_folder(root,&root.join(name),&mut sources)?;}
    if inputs["sources"]!=serde_json::to_value(sources).map_err(error)? {return Err("Benchmark contract sources changed.".into());}
    Ok(())
}
fn temporal_environment(root:&Path, receipt:&Receipt, command:&mut Command)->Result<serde_json::Value> {
    // The disposable child uses its own HOME; avoid resolving a rustup shim
    // there. Select the same compiler installation as this actual build.
    let sysroot=PathBuf::from(output(root,"rustc",&["--print","sysroot"])?.trim()).canonicalize().map_err(error)?;
    let bin=sysroot.join("bin");
    let mut tools=BTreeMap::new();
    for name in ["rustc","cargo"] {
        let path=bin.join(name).canonicalize().map_err(error)?;
        if output(root,path.to_str().ok_or("Invalid temporal compiler path.")?,&["--version"])?.trim()
            !=receipt.toolchains.get(name).ok_or("Missing verified compiler version.")? {
            return Err("Temporal compiler installation differs from the verified build.".into());
        }
        tools.insert(path.to_string_lossy().into_owned(),digest(&path)?);
        command.env(name.to_uppercase(),&path);
    }
    let inherited=std::env::var_os("PATH").ok_or("Missing temporal tool path.")?;
    let path=std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited))).map_err(error)?;
    command.env("PATH",path);
    Ok(serde_json::json!({"tools":tools,"versions":receipt.toolchains}))
}
fn check_temporal_inputs(root:&Path, receipt:&Receipt, inputs:&serde_json::Value)->Result<()> {
    if inputs["versions"]!=serde_json::to_value(&receipt.toolchains).map_err(error)? {return Err("Temporal toolchain binding changed.".into());}
    let tools=inputs["tools"].as_object().ok_or("Missing temporal compiler bindings.")?;
    if tools.len()!=2 {return Err("Incomplete temporal compiler bindings.".into());}
    let names=tools.keys().filter_map(|path|Path::new(path).file_name().and_then(|name|name.to_str())).collect::<std::collections::BTreeSet<_>>();
    if names!=std::collections::BTreeSet::from(["cargo","rustc"]) {return Err("Missing exact temporal compiler pair.".into());}
    for (path,sha) in tools {
        let path=Path::new(path);
        let name=path.file_name().and_then(|name|name.to_str()).ok_or("Invalid compiler name.")?;
        if !["cargo","rustc"].contains(&name)||!path.is_absolute()||path.canonicalize().map_err(error)?!=path
            ||sha!=&serde_json::Value::String(digest(path)?)
            ||output(root,path.to_str().ok_or("Invalid compiler path.")?,&["--version"])?.trim()!=receipt.toolchains[name] {
            return Err("Temporal compiler input changed.".into());
        }
    }
    Ok(())
}
const CLI_CONTRACT_CASES: [&str;2] = [
    "startup_discovery_tests::current_cli_roundtrips_owned_headless_clipboard_files_and_json_argv_without_dry_run_effects",
    "startup_discovery_tests::current_cli_starts_one_real_headless_owner_and_quits_without_capture_or_input",
];
fn cli_contract_socket_budget(fixture:&Path)->Result<()> {
    use std::os::unix::ffi::OsStrExt;
    if !fixture.is_absolute()||fixture.join("cli-roundtrip-4294967295/p/control.sock").as_os_str().as_bytes().len()>104 {
        return Err("CLI contract validation requires a shorter owned socket parent.".into());
    }
    Ok(())
}
fn cli_contract_paths(root:&Path,target:&str)->Result<[(PathBuf,&'static str);3]> {
    Ok([
        (root.join(&artifact_paths(target)?[1].0),"RUYCAST_CLI_PARITY_BINARY"),
        (root.join("rust-native/target/validation-build").join(target).join("debug/private-results-fixture"),"RUYCAST_CLI_PARITY_HOST"),
        (root.join("rust-native/target/validation-native").join(target).join("debug/ruycast-native"),"RUYCAST_CLI_PARITY_NATIVE"),
    ])
}
fn cli_contract_inputs(root:&Path,receipt:&Receipt,fixture:&Path)->Result<serde_json::Value> {
    let cli=&artifact_paths(&receipt.target)?[1].0;
    let expected=receipt.artifacts.get(cli).ok_or("Missing CLI in verified build receipt.")?;
    if digest(&root.join(cli))?!=*expected {return Err("CLI contract binary differs from the verified build receipt.".into());}
    let mut sources=BTreeMap::new();
    for name in ["rust/Cargo.toml","rust/Cargo.lock","config/default.json","rust/src","rust-native/Cargo.toml","rust-native/Cargo.lock","rust-native/src"] {
        source_folder(root,&root.join(name),&mut sources)?;
    }
    let mut artifacts=BTreeMap::new();
    for (path,_) in cli_contract_paths(root,&receipt.target)? {
        owned_path(root,&path)?;
        artifacts.insert(path.strip_prefix(root).map_err(error)?.to_string_lossy().into_owned(),digest(&path)?);
    }
    Ok(serde_json::json!({"sources":sources,"artifacts":artifacts,"fixtureDirectory":fixture.strip_prefix(root).map_err(error)?.to_string_lossy()}))
}
fn cli_contract_environment(root:&Path,receipt:&Receipt,fixture:&Path,command:&mut Command)->Result<serde_json::Value> {
    let inputs=cli_contract_inputs(root,receipt,fixture)?;
    command.env("RUYCAST_NATIVE_TEST_FIXTURE_DIR",fixture).env("TMPDIR",fixture);
    for (path,key) in cli_contract_paths(root,&receipt.target)? {
        command.env(key,&path).env(format!("{key}_SHA256"),digest(&path)?);
    }
    Ok(inputs)
}
fn check_cli_contract_inputs(root:&Path,receipt:&Receipt,inputs:&serde_json::Value)->Result<()> {
    let relative=Path::new(inputs["fixtureDirectory"].as_str().ok_or("Missing owned CLI fixture directory.")?);
    if relative.parent()!=Some(Path::new(".build"))||!relative.file_name().is_some_and(|name|name.to_string_lossy().starts_with(".u-")) {
        return Err("CLI contract fixture escaped its exclusive stage.".into());
    }
    if inputs!=&cli_contract_inputs(root,receipt,&root.join(relative))? {
        return Err("Incomplete or changed CLI contract source or artifact bindings.".into());
    }
    Ok(())
}
fn system_ui_socket_budget(fixture: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    // Match the longest existing SDK-test child naming scheme, including a
    // full-width PID. Do not shorten or relax the native socket boundary.
    if !fixture.is_absolute() || fixture.join("su-4294967295-shortcuts-light-760/p/control.sock").as_os_str().as_bytes().len() > 104 {
        return Err("System UI validation requires a shorter repository path for its owned sockets.".into());
    }
    Ok(())
}
const EXAMPLE_UI_SOURCES: &[&str] = &[
    "rust-native/examples/ui_completion.rs",
    "rust-native/src/bin/private_results_fixture.rs",
    "rust-native/tests/fixtures/workspace_sdk.rs",
    "rust-native/tests/fixtures/snippet_effects_sdk.rs",
    "rust-native/tests/fixtures/clipboard_history_sdk.rs",
    "rust-native/tests/fixtures/clipboard_history_fixture.rs",
    "rust-native/Cargo.toml",
    "rust-native/Cargo.lock",
    "rust/Cargo.toml",
    "rust-toolchain.toml",
];
fn example_ui_socket_budget(fixture: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    if !fixture.is_absolute() || fixture.join("u4294967295-18446744073709551615/p/control.sock").as_os_str().as_bytes().len() > 104 {
        return Err("Example UI validation requires a shorter repository path for owned sockets.".into());
    }
    Ok(())
}
fn example_ui_inputs(root: &Path, target: &str, evidence: &Path, uses_fixture: bool, uses_keyboard: bool) -> Result<serde_json::Value> {
    artifact_paths(target)?;
    let mut sources = BTreeMap::new();
    for name in EXAMPLE_UI_SOURCES { sources.insert(*name, digest(&root.join(name))?); }
    let mut artifacts = BTreeMap::new();
    let mut paths = vec![
        root.join("rust-native/target/validation-native").join(target).join("debug/ruycast-native"),
        root.join("rust-native/target/clipboard-history-effects.dylib"),
    ];
    if uses_fixture { paths.push(root.join("rust-native/target/validation-build").join(target).join("debug/private-results-fixture")); }
    if uses_keyboard {
        paths.push(keyboard_fixture(root)?.0);
        paths.push(root.join(DIRECTORY).join("native-profile/host-sdks/NativeKeyboard.json"));
    }
    for path in paths {
        owned_path(root, &path)?;
        artifacts.insert(path.strip_prefix(root).map_err(error)?.to_string_lossy().into_owned(), digest(&path)?);
    }
    Ok(serde_json::json!({"sources":sources,"artifacts":artifacts,"evidenceDirectory":evidence.strip_prefix(root).map_err(error)?.to_string_lossy()}))
}
fn example_ui_environment(root: &Path, entry: &Step, fixture: &Path, command: &mut Command) -> Result<serde_json::Value> {
    use std::os::unix::fs::DirBuilderExt;
    let target = entry.arguments.windows(2).find(|pair|pair[0]=="--target").map(|pair|pair[1].as_str()).ok_or("Missing example UI target.")?;
    ruycast_native::private_storage::write(&fixture.join(".ruycast-ui-fixture-parent"), b"owned-ui-fixture-parent-v1", false)?;
    let evidence = root.join(DIRECTORY).join(fixture.file_name().ok_or("Missing exclusive example UI fixture name.")?);
    fs::DirBuilder::new().mode(0o700).create(&evidence).map_err(error)?;
    let frames = evidence.join("frames");
    fs::DirBuilder::new().mode(0o700).create(&frames).map_err(error)?;
    let uses_fixture = matches!(entry.name.as_str(), "task-date-process" | "polish-original-process");
    let uses_keyboard = entry.name == "item-binding-original-process";
    let inputs = example_ui_inputs(root, target, &evidence, uses_fixture, uses_keyboard)?;
    let native = root.join("rust-native/target/validation-native").join(target).join("debug/ruycast-native");
    command.env("RUYCAST_NATIVE_TEST_FIXTURE_DIR", fixture).env("TMPDIR", fixture)
        .env("RUYCAST_UI_VALIDATION_HOST", &native).env("RUYCAST_UI_VALIDATION_HOST_SHA", digest(&native)?)
        .env("RUYCAST_UI_VALIDATION_EVIDENCE", &frames);
    if uses_fixture {
        let binary = root.join("rust-native/target/validation-build").join(target).join("debug/private-results-fixture");
        command.env("RUYCAST_UI_VALIDATION_FIXTURE", &binary).env("RUYCAST_UI_VALIDATION_FIXTURE_SHA", digest(&binary)?);
    }
    if uses_keyboard {
        let (sdk, sha) = keyboard_fixture(root)?;
        command.env("RUYCAST_TEST_KEYBOARD_SDK", sdk).env("RUYCAST_TEST_KEYBOARD_SHA", sha);
    }
    Ok(inputs)
}
fn check_example_ui_inputs(root: &Path, target: &str, inputs: &serde_json::Value, uses_fixture: bool, uses_keyboard: bool) -> Result<()> {
    let relative = Path::new(inputs["evidenceDirectory"].as_str().ok_or("Missing example UI evidence directory.")?);
    if relative.parent() != Some(Path::new(DIRECTORY)) || !relative.file_name().is_some_and(|name|name.to_string_lossy().starts_with(".u-")) {
        return Err("Example UI evidence escaped its exclusive validation directory.".into());
    }
    let evidence = root.join(relative);
    let mut expected = example_ui_inputs(root, target, &evidence, uses_fixture, uses_keyboard)?;
    let mut generated = BTreeMap::new();
    source_folder(root, &evidence, &mut generated)?;
    if generated.is_empty() { return Err("Missing actual example UI evidence.".into()); }
    expected["artifacts"].as_object_mut().ok_or("Missing example UI input artifacts.")?
        .extend(serde_json::to_value(generated).map_err(error)?.as_object().ok_or("Invalid example UI evidence.")?.clone());
    if inputs != &expected { return Err("Incomplete or changed example UI sources, inputs or actual evidence.".into()); }
    Ok(())
}
fn system_ui_environment(root: &Path, entry: &Step, fixture: &Path, command: &mut Command) -> Result<serde_json::Value> {
    use std::os::unix::fs::DirBuilderExt;
    let target = entry.arguments.windows(2).find(|pair| pair[0] == "--target")
        .map(|pair| pair[1].as_str()).ok_or("Missing system UI Cargo target.")?;
    artifact_paths(target)?;
    let target_dir = entry.arguments.windows(2).find(|pair| pair[0] == "--target-dir")
        .map(|pair| pair[1].as_str()).ok_or("Missing system UI Cargo target directory.")?;
    if target_dir != "rust-native/target/validation-build" {
        return Err("Unexpected system UI Cargo target directory.".into());
    }
    let native = root.join(target_dir).join(target).join("debug/ruycast-native");
    let native_sha = digest(&native)?;
    // This qualifies byte bindings only. The unchanged SDK consumer still
    // independently refuses any non-qualified artifact (including a fresh
    // producer hash outside its admitted contract); this is not SDK admission.
    let (sdk, _) = keyboard_fixture(root)?;
    let manifest = root.join(DIRECTORY).join("native-profile/host-sdks/NativeKeyboard.json");
    let marker = fixture.join(".ruycast-ui-fixture-parent");
    ruycast_native::private_storage::write(&marker, b"owned-ui-fixture-parent-v1", false)?;
    let evidence = root.join(DIRECTORY).join(fixture.file_name().ok_or("Missing exclusive UI fixture name.")?);
    fs::DirBuilder::new().mode(0o700).create(&evidence).map_err(error)?;
    let system = evidence.join("system");
    fs::DirBuilder::new().mode(0o700).create(&system).map_err(error)?;
    command.env("RUYCAST_NATIVE_TEST_FIXTURE_DIR", fixture)
        .env("TMPDIR", fixture)
        .env("RUYCAST_BROWSER_NATIVE_SHA", native_sha)
        .env("RUYCAST_BROWSER_KEYBOARD_MANIFEST", &manifest)
        .env("RUYCAST_BROWSER_EVIDENCE", evidence.join("browser"))
        .env("RUYCAST_NATIVE_EVIDENCE_DIR", &evidence)
        .env("RUYCAST_SYSTEM_UI_EVIDENCE", system);
    let mut sources = BTreeMap::new();
    for name in KEYBOARD_SDK_SOURCES.iter().chain(SYSTEM_UI_SOURCES) {
        let path = root.join(name);
        sources.insert(path.strip_prefix(root).map_err(error)?.to_string_lossy().into_owned(), digest(&path)?);
    }
    let mut artifacts = BTreeMap::new();
    for path in [&native, &manifest, &sdk] {
        artifacts.insert(path.strip_prefix(root).map_err(error)?.to_string_lossy().into_owned(), digest(path)?);
    }
    Ok(serde_json::json!({"sources":sources,"artifacts":artifacts,"evidenceDirectory":evidence.strip_prefix(root).map_err(error)?.to_string_lossy()}))
}
fn check_system_ui_inputs(root: &Path, target: &str, inputs: &serde_json::Value) -> Result<()> {
    artifact_paths(target)?;
    let relative = Path::new(inputs["evidenceDirectory"].as_str().ok_or("Missing system UI evidence directory.")?);
    if relative.parent() != Some(Path::new(DIRECTORY)) || !relative.file_name().is_some_and(|name|name.to_string_lossy().starts_with(".u-")) {
        return Err("System UI evidence escaped its exclusive validation directory.".into());
    }
    let mut generated = BTreeMap::new();
    source_folder(root, &root.join(relative), &mut generated)?;
    if generated.is_empty() { return Err("Missing actual owned system UI evidence.".into()); }
    let (sdk, _) = keyboard_fixture(root)?;
    for path in [root.join("rust-native/target/validation-build").join(target).join("debug/ruycast-native"), root.join(DIRECTORY).join("native-profile/host-sdks/NativeKeyboard.json"), sdk] {
        generated.insert(path.strip_prefix(root).map_err(error)?.to_string_lossy().into_owned(), digest(&path)?);
    }
    let mut sources = BTreeMap::new();
    for name in KEYBOARD_SDK_SOURCES.iter().chain(SYSTEM_UI_SOURCES) {
        let path = root.join(name);
        sources.insert(path.strip_prefix(root).map_err(error)?.to_string_lossy().into_owned(), digest(&path)?);
    }
    if inputs["sources"] != serde_json::to_value(sources).map_err(error)? || inputs["artifacts"] != serde_json::to_value(generated).map_err(error)? {
        return Err("Incomplete or changed system UI source, input or actual evidence bindings.".into());
    }
    Ok(())
}
const DASHBOARD_EXPORT_CASES: [&str; 5] = [
            "positive_held_export_global_reset_order_and_parent_mode",
            "file_sync_and_rename_failure_preserve_prior_destination",
            "postrename_parent_sync_failure_reports_published_outcome",
            "close_before_commit_revokes_destination_without_feedback_to_reopened_window",
            "commit_entered_before_close_keeps_published_outcome_and_retires_feedback",
];
fn selected_step_output(name: &str, stdout: &[u8]) -> Result<()> {
    if name == "dashboard-presentation-process" {
        let text = std::str::from_utf8(stdout).map_err(error)?;
        for case in ["populated_top30_hash_ties_names_samples_errors_and_legacy", "elapsed_hours_retention_dst_and_calendar_year"] {
            if text.lines().filter(|line| *line == format!("test {case} ... ok")).count() != 1 {
                return Err("Both captured dashboard presentation scenarios must pass.".into());
            }
        }
        if !text.lines().any(|line| line.starts_with("test result: ok. 5 passed; 0 failed; 0 ignored;")) {
            return Err("Dashboard presentation tests must pass without skips.".into());
        }
    }

    if name == "dashboard-export-process" {
        let text = std::str::from_utf8(stdout).map_err(error)?;
        if DASHBOARD_EXPORT_CASES.iter().any(|case| text.lines().filter(|line| *line == format!("test {case} ... ok")).count() != 1)
            || text.lines().filter(|line| line.starts_with("test ") && line.contains(" ... ")).count() != 5
            || text.lines().filter(|line| line.starts_with("test result:")).count() != 1
            || !text.lines().any(|line| line.starts_with("test result: ok. 5 passed; 0 failed; 0 ignored;"))
        {
            return Err("All five original dashboard export workloads must actually pass without skips.".into());
        }
    }
    let selected=BENCHMARK_CASES.iter().enumerate().find(|(i,_)|name==format!("benchmark-contract-{}",i+1))
        .or_else(||TEMPORAL_CASES.iter().enumerate().find(|(i,_)|name==format!("temporal-contract-{}",i+1)));
    if let Some((_,case))=selected {
        let text=std::str::from_utf8(stdout).map_err(error)?;
        if text.lines().filter(|line|*line==format!("test {case} ... ok")).count()!=1
            ||text.lines().filter(|line|line.starts_with("test ")&&line.contains(" ... ")).count()!=1
            ||text.lines().filter(|line|line.starts_with("test result:")).count()!=1
            ||!text.lines().any(|line|line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;")) {
            return Err("The selected mandatory contract must actually pass exactly once without skips.".into());
        }
    }
    if name=="cli-contract-process" {
        let text=std::str::from_utf8(stdout).map_err(error)?;
        if CLI_CONTRACT_CASES.iter().any(|case|text.lines().filter(|line|*line==format!("test {case} ... ok")).count()!=1)
            || text.lines().filter(|line|line.starts_with("test ")&&line.contains(" ... ")).count()!=2
            || text.lines().filter(|line|line.starts_with("test result:")).count()!=1
            || !text.lines().any(|line|line.starts_with("test result: ok. 2 passed; 0 failed; 0 ignored;")) {
            return Err("Both mandatory CLI contract cases must actually pass without skips.".into());
        }
    }
    if name == "core-unit"
        && !std::str::from_utf8(stdout).is_ok_and(|text| {
            text.lines().any(|line| line == "test tests::command_sides_are_independent ... ok")
                && text.lines().any(|line| line.starts_with("test result: ok. ")
                    && !line.starts_with("test result: ok. 0 passed;")
                    && line.contains(" passed; 0 failed;"))
        })
    {
        return Err("RustCore units, including fresh and missing legacy Command-side settings, must actually pass.".into());
    }
    if name == "snippet-html-import" {
        let value: serde_json::Value = serde_json::from_slice(stdout).map_err(error)?;
        if value["richModes"] != 2 || value["blankParagraphSamples"] != 7
            || ["ok", "hyperlinkImport", "noTextTables"].iter().any(|key| value[*key] != true)
            || ["clipboardAccessed", "visibleWindowOpened", "desktopInputSent"].iter().any(|key| value[*key] != false)
        {
            return Err("The actual snippet HTML import and seven paragraph samples must pass without desktop effects.".into());
        }
    }
    if name == "settings-ui-process" {
        let value: serde_json::Value = serde_json::from_slice(stdout).map_err(error)?;
        if value["themes"] != 2 || value["tabsPerTheme"] != 7 || value["renders"] != 20
            || ["ok", "settings", "hyperLeftAndCommandK", "disabledSnippetListed", "independentManagementQuery"].iter().any(|key| value[*key] != true)
            || ["visibleWindowOpened", "physicalInput"].iter().any(|key| value[*key] != false)
        {
            return Err("All Settings tabs, original save keys and independent queries must actually pass in both themes.".into());
        }
    }
    if name == "actions-menu-process" {
        let value: serde_json::Value = serde_json::from_slice(stdout).map_err(error)?;
        if value["themes"] != 2 || value["physicalInput"] != false
            || ["ok", "actionsMenu", "nativeMenuFirstResponder", "fieldEditorFallback", "selectedRowFourPreserved", "clampedMenuArrows", "returnEditsOriginal", "escapeAndModeRestoreSearch", "commandKTogglesClosed", "removalQueryBoundaryPreservesIdentity", "newSnippetDraft", "clipboardDelete", "discoveredAppMaterializedWithTarget", "recordedPaletteShortcutPayloads", "repeatCancelConflictClear"].iter().any(|key|value[*key] != true)
        { return Err("The complete original Actions menu and post-save route must actually pass in both themes without physical input.".into()); }
    }
    if name == "item-binding-original-process" {
        let value: serde_json::Value = serde_json::from_slice(stdout).map_err(error)?;
        if value["themes"] != 2 || value["fieldCaptures"] != 8 || value["keyboardCopyCaptures"] != 4
            || ["ok", "itemBindingOriginal", "queuedBeforeMainApplyStopped", "plainCommandCommaNotConsumed", "genuineRecorderResponderAndAX", "clipboardHotkeyOriginal", "hotkeyFailureNotice", "normalQuit"].iter().any(|key| value[*key] != true)
            || ["physicalInput", "visibleWindowOpened"].iter().any(|key| value[*key] != false)
        { return Err("Both themes and all eight real binding fields, recorder accessibility, queued-hide acknowledgement and normal quit must pass without desktop input.".into()); }
    }
    if name == "palette-lifecycle-process" {
        let value: serde_json::Value = serde_json::from_slice(stdout).map_err(error)?;
        if value["callbackRoutes"] != 7 || value["themes"] != 2
            || ["ok", "paletteLifecycle", "nativeFloatingAndSpaces", "retiredCallbackRefused", "normalQuit"].iter().any(|key| value[*key] != true)
            || ["actualOSMonitorRegistration", "physicalInput", "visibleWindowOpened"].iter().any(|key| value[*key] != false)
        { return Err("Both themes must pass every genuine panel-property and owned lifecycle callback case without desktop monitoring or input.".into()); }
    }
    if name == "interaction-trace-process" {
        let value: serde_json::Value = serde_json::from_slice(stdout).map_err(error)?;
        if value["cycles"] != 100 || value["catalogCommands"] != 500
            || ["ok", "interactionTrace", "reconciliationBeforeCapture", "actualCaptureDraws", "closedFieldCancelled", "publicReset", "tracePayloadPrivate", "normalQuit"].iter().any(|key| value[*key] != true)
            || ["physicalInput", "visibleWindowOpened"].iter().any(|key| value[*key] != false)
        { return Err("All hundred real field/reconciliation/draw cycles, cancellation, reset and private payload checks must pass without desktop input.".into()); }
    }
    if name == "clipboard-pause-original" {
        let text = std::str::from_utf8(stdout).map_err(error)?;
        if text.lines().filter(|line| *line == "test real_host_preserves_invalid_pause_and_requires_confirmed_explicit_resume ... ok").count() != 1
            || text.lines().filter(|line| line.starts_with("test result:")).count() != 1
            || !text.lines().any(|line| line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;"))
        { return Err("The exact real-host pause-retention case must pass once with its required test-tools host and without skips.".into()); }
    }
    if matches!(name, "render-updates-process" | "polish-bindings-process" | "polish-original-process" | "task-date-process") {
        let value: serde_json::Value = serde_json::from_slice(stdout).map_err(error)?;
        let (count, themes, keys): (i64, i64, &[&str]) = match name {
            "render-updates-process" => (26, 2, &["renderUpdates", "watchedTheme", "pendingQuery", "selectionImageChanged", "independentQueries"]),
            "polish-bindings-process" => (4, 2, &["polishBindings", "aliasAndKeywordSearch", "arrowSelection"]),
            "polish-original-process" => (52, 4, &["polishOriginal", "doubleSubmitOnce", "clipboardClear", "geometry", "normalQuit"]),
            _ => (16, 2, &["taskDate", "nativeTyping", "invalidRefused", "doubleSubmitOnce", "normalQuit"]),
        };
        if value["ok"] != true || value["themes"] != themes || value["captures"] != count
            || keys.iter().any(|key|value[*key] != true)
            || ["visibleWindowOpened", "physicalInput"].iter().any(|key|value[*key] != false)
            || name == "render-updates-process" && value["originalFramesPerTheme"] != 12
            || name == "task-date-process" && value["frozenValue"] != "2000-01-02"
            || name == "polish-original-process" && (value["contextThemes"] != 2 || value["filledCaptureFrames"] != 4 || value["healthIconQualified"] != false)
        { return Err("The selected complete example UI route and actual case markers must pass without desktop effects.".into()); }
    }
    if name == "clipboard-preview-cancel-original" {
        let text = std::str::from_utf8(stdout).map_err(error)?;
        if text.lines().filter(|line| *line == "test clipboard_preview::tests::cancelled_same_id_reopen_cannot_publish_old_error_or_asset ... ok").count() != 1
            || text.lines().filter(|line|line.starts_with("test result:")).count() != 1
            || !text.lines().any(|line|line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;"))
        { return Err("The exact original cancelled preview case must actually pass once without skips.".into()); }
    }
    if name == "clipboard-preview-original"
        && !std::str::from_utf8(stdout).is_ok_and(|text| {
            text.lines()
                .any(|line| line.starts_with("test result: ok. 1 passed; 0 failed;"))
        })
    {
        return Err("The exact original clipboard preview test must actually pass once.".into());
    }
    if name == "snippet-hotkey-process" {
        let value: serde_json::Value = serde_json::from_slice(stdout).map_err(error)?;
        if value["snippetHotkey"] != true || value["themes"] != 2 {
            return Err("The snippet footer route must actually pass in both themes.".into());
        }
    }
    if name == "feedback-actions-process" {
        let value: serde_json::Value = serde_json::from_slice(stdout).map_err(error)?;
        if value["themes"] != 2 || [
            "ok", "feedbackActions", "allOriginalMenuControlsReachable", "emptyMainTabsRefresh",
            "sameBoardChangeCountAndRawPreserved", "useEitherCommand",
        ].iter().any(|key| value[*key] != true) {
            return Err("The feedback, same-board and recorder menu routes must actually pass in both themes.".into());
        }
    }
    Ok(())
}
fn keyboard_fixture(root: &Path) -> Result<(PathBuf, String)> {
    let directory = root.join(DIRECTORY).join("native-profile/host-sdks");
    let binding: serde_json::Value =
        serde_json::from_slice(&regular_bytes(&directory.join("NativeKeyboard.json"))?)
            .map_err(error)?;
    let mut sources = BTreeMap::new();
    for name in KEYBOARD_SDK_SOURCES {
        let path = root.join(name);
        owned_path(root, &path)?;
        let sha = digest(&path)?;
        sources.insert(path, sha);
    }
    if binding["sources"] != serde_json::to_value(sources).map_err(error)? {
        return Err("Current owned keyboard fixture source bindings are required.".into());
    }
    let artifact = directory.join("NativeKeyboard.dylib");
    owned_path(root, &artifact)?;
    let sha = digest(&artifact)?;
    if binding["artifact"].as_str() != artifact.to_str() || binding["sha256"] != sha {
        return Err("Current owned keyboard fixture artifact binding is required.".into());
    }
    Ok((artifact, sha))
}
fn check_host_sdk_sources(root: &Path, binding: &serde_json::Value) -> Result<()> {
    let sources = binding["sources"].as_object().ok_or("Missing SDK source binding.")?;
    if sources.len() != KEYBOARD_SDK_SOURCES.len() {
        return Err("Incomplete or unexpected native SDK source roster.".into());
    }
    for name in KEYBOARD_SDK_SOURCES {
        let path = root.join(name);
        let source = owned_path(root, &path)?;
        let key = source.to_str().ok_or("Invalid native SDK source path.")?;
        if sources.get(key).and_then(serde_json::Value::as_str) != Some(digest(source)?.as_str()) {
            return Err("Native SDK fixture source changed.".into());
        }
    }
    Ok(())
}
const NORMAL_HOST_ALIAS: &str = "rust-native/target/debug/ruycast-native";
fn normal_host_negative(entry: &Step) -> bool {
    matches!(entry.name.as_str(), "bridge-process")
}
fn normal_host_inputs(root: &Path, receipt: &Receipt) -> Result<serde_json::Value> {
    if receipt.configuration != "release" {
        return Err("Normal-host negative tests require the verified release-role receipt.".into());
    }
    let source = artifact_paths(&receipt.target)?[0].0.clone();
    let expected = receipt.artifacts.get(&source).ok_or("Missing default host in verified receipt.")?;
    if digest(&root.join(&source))? != *expected {
        return Err("Normal host differs from the verified default artifact.".into());
    }
    Ok(serde_json::json!({"source":source,"alias":NORMAL_HOST_ALIAS,"sha256":expected,
        "configuration":"release","purpose":"normal-feature refusal only; no debug or performance claim"}))
}
fn prepare_normal_host(root: &Path, receipt: &Receipt, copies: &mut BTreeMap<String, serde_json::Value>) -> Result<serde_json::Value> {
    let inputs = normal_host_inputs(root, receipt)?;
    compatibility_copy(root, inputs["source"].as_str().ok_or("Missing normal host source.")?, NORMAL_HOST_ALIAS, copies)?;
    check_normal_host_inputs(root, receipt, &inputs)?;
    Ok(inputs)
}
fn check_normal_host_inputs(root: &Path, receipt: &Receipt, inputs: &serde_json::Value) -> Result<()> {
    if inputs != &normal_host_inputs(root, receipt)?
        || inputs["sha256"].as_str() != Some(digest(&root.join(NORMAL_HOST_ALIAS))?.as_str()) {
        return Err("Normal-host negative test lost its verified default-role alias.".into());
    }
    Ok(())
}
fn compatibility_copy(
    root: &Path,
    source: &str,
    alias: &str,
    copies: &mut BTreeMap<String, serde_json::Value>,
) -> Result<()> {
    let before = digest(&root.join(source))?;
    let bytes = regular_bytes(&root.join(source))?;
    let path = root.join(alias);
    ruycast_native::private_storage::secure_directory(
        path.parent().ok_or("Missing test artifact parent.")?,
    )?;
    ruycast_native::private_storage::write(&path, &bytes, true)?;
    use std::os::unix::fs::PermissionsExt;
    regular_file(&path)?
        .set_permissions(fs::Permissions::from_mode(0o755))
        .map_err(error)?;
    if digest(&path)? != before || digest(&root.join(source))? != before {
        return Err("Test compatibility artifact changed during copy.".into());
    }
    copies.insert(alias.into(), serde_json::json!({"source":source,"sha256":before,"purpose":"test-compatibility-only","productionAuthority":false}));
    Ok(())
}
fn check_binding(root: &Path, value: &serde_json::Value) -> Result<()> {
    for group in ["sources", "artifacts"] {
        for (name, expected) in value[group]
            .as_object()
            .ok_or("Missing reference bindings.")?
        {
            let path = Path::new(name);
            if path.is_absolute()
                || path
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err("Reference escaped repository.".into());
            }
            if expected.as_str() != Some(digest(&root.join(path))?.as_str()) {
                return Err("Reference changed during verification.".into());
            }
        }
    }
    Ok(())
}
pub(super) fn plan(target: &str) -> Result<serde_json::Value> {
    Ok(
        serde_json::json!({"schema":"ruycast.native-validation-plan.v1","steps":matrix(target)?,"renders":[["light","600"],["light","760"],["dark","600"],["dark","760"]],"references":REFERENCES,"requiredNativeSDKs":5,"productionAccepted":false}),
    )
}
fn owned_path<'a>(root: &Path, path: &'a Path) -> Result<&'a Path> {
    let relative = path.strip_prefix(root).map_err(error)?;
    if relative
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("Evidence path escaped repository.".into());
    }
    Ok(path)
}
pub(super) fn verify(root: &Path, lease: &BuildLock) -> Result<serde_json::Value> {
    lease.validate()?;
    let value: serde_json::Value =
        serde_json::from_slice(&regular_bytes(&root.join(DIRECTORY).join("report.json"))?)
            .map_err(error)?;
    if value["schema"] != "ruycast.native-verification.v2"
        || value["ok"] != true
        || value["productionAccepted"] != false
        || value["sources"] != serde_json::to_value(sources(root)?).map_err(error)?
    {
        return Err("Native verification report is invalid or stale.".into());
    }
    if value["pureCoreOracle"] != oracles::verify_core(root, lease)? {
        return Err("Native verification lost its actual pure Core oracle binding.".into());
    }
    let receipt = verify_under_lease(root, "release", lease)?;
    let mut names = matrix(&receipt.target)?
        .into_iter()
        .map(|step| step.name)
        .collect::<Vec<_>>();
    for theme in ["light", "dark"] {
        for width in ["600", "760"] {
            names.push(format!("native-{theme}-{width}"));
        }
    }
    names.push("rust-only-policy-final".into());
    let actual = value["steps"]
        .as_array()
        .ok_or("Missing complete validation steps.")?;
    if actual.len() != names.len()
        || actual.iter().zip(&names).any(|(step, name)| {
            step["name"].as_str() != Some(name)
                || step["exitCode"] != 0
                || step["seconds"]
                    .as_f64()
                    .is_none_or(|seconds| !seconds.is_finite() || seconds < 0.0)
        })
    {
        return Err(
            "Native verification does not contain the entire successful ordered matrix.".into(),
        );
    }
    for step in actual {
        if matches!(step["name"].as_str(), Some("bridge-process")) {
            check_normal_host_inputs(root, &receipt, &step["normalHostInputs"])?;
        }
        if step["name"] == "system-search-sdk" {
            check_system_ui_inputs(root, &receipt.target, &step["systemUiInputs"])?;
        }
        if matches!(step["name"].as_str(), Some("render-updates-process" | "polish-bindings-process" | "polish-original-process" | "task-date-process" | "actions-menu-process" | "item-binding-original-process" | "palette-lifecycle-process" | "interaction-trace-process" | "feedback-actions-process")) {
            check_example_ui_inputs(root, &receipt.target, &step["exampleUiInputs"], matches!(step["name"].as_str(), Some("task-date-process" | "polish-original-process")), step["name"]=="item-binding-original-process")?;
        }
        if step["name"].as_str().is_some_and(|name|name.starts_with("benchmark-contract-")) {
            check_benchmark_inputs(root,&receipt,&step["benchmarkInputs"])?;
        }
        if step["name"].as_str().is_some_and(|name|name.starts_with("temporal-contract-")) {
            check_temporal_inputs(root,&receipt,&step["temporalInputs"])?;
        }
        if step["name"]=="cli-contract-process" {
            check_cli_contract_inputs(root,&receipt,&step["cliContractInputs"])?;
        }
    }
    let renders = value["renders"]
        .as_object()
        .ok_or("Missing native render evidence.")?;
    if renders.len() != 4 {
        return Err("Missing four native renders.".into());
    }
    for theme in ["light", "dark"] {
        for width in ["600", "760"] {
            let name = format!("native-{theme}-{width}");
            let row = &value["renders"][&name];
            if row["imageSHA256"].as_str()
                != Some(digest(&root.join(DIRECTORY).join(format!("{name}.png")))?.as_str())
                || row["resultSHA256"].as_str()
                    != Some(digest(&root.join(DIRECTORY).join(format!("{name}.stdout")))?.as_str())
            {
                return Err("Native render evidence changed.".into());
            }
        }
    }
    for (path, expected) in value["binaries"]
        .as_object()
        .ok_or("Missing verified binaries.")?
    {
        let absolute = root.join(path);
        owned_path(root, &absolute)?;
        if expected.as_str() != Some(digest(&absolute)?.as_str()) {
            return Err("Verified native test artifact changed.".into());
        }
    }
    for (field, _) in REFERENCES {
        if *field == "hidStateReference" {
            let raw = regular_bytes(&root.join(DIRECTORY).join("native-profile/hid-state-reference-bindings.json"))?;
            if value[*field] != oracles::qualify_hid_reference(root, &value["binaries"], &raw)? {
                return Err("HID reference qualification changed.".into());
            }
        }
        check_binding(root, &value[*field])?;
    }
    let sdk = value["nativeHostSDKs"]
        .as_object()
        .ok_or("Missing native SDK bindings.")?;
    check_host_sdk_paths(sdk.keys())?;
    for (path, expected) in sdk {
        let absolute = root.join(path);
        owned_path(root, &absolute)?;
        if expected.as_str() != Some(digest(&absolute)?.as_str()) {
            return Err("Verified native SDK changed.".into());
        }
    }
    lease.validate()?;
    Ok(value)
}
pub(super) fn run(root: &Path, lease: &BuildLock) -> Result<serde_json::Value> {
    lease.validate()?;
    let pure_core_oracle = oracles::core(root, lease)?;
    let directory = root.join(DIRECTORY);
    ruycast_native::private_storage::secure_directory(&directory)?;
    let owned_fixture = preparation::Stage::new(root, "validation-fixtures")?;
    let fixture = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| owned_fixture.path().to_path_buf());
    if !fixture.is_absolute() {
        return Err("Validation fixture parent must be absolute.".into());
    }
    ruycast_native::private_storage::secure_directory(&fixture)?;
    let ui_marker = fixture.join(".ruycast-ui-fixture-parent");
    let marker_bytes = b"owned-ui-fixture-parent-v1";
    match fs::symlink_metadata(&ui_marker) {
        Ok(meta) if meta.is_file() && regular_bytes(&ui_marker)? == marker_bytes => (),
        Ok(_) => {
            return Err("Unrecognized UI fixture marker; preserve its existing contents.".into())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            ruycast_native::private_storage::write(&ui_marker, marker_bytes, false)?;
        }
        Err(e) => return Err(error(e)),
    }
    let report = directory.join("report.json");
    match fs::symlink_metadata(&report) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            fs::remove_file(&report).map_err(error)?
        }
        Ok(_) => return Err("Invalid existing validation report.".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(error(e)),
    }
    let before = sources(root)?;
    let receipt: Receipt = serde_json::from_slice(&regular_bytes(
        &root.join(OUTPUT).join("development-build.json"),
    )?)
    .map_err(error)?;
    verify_value(root, &receipt)?;
    let target = receipt.target.clone();
    let cli = artifact_paths(&target)?[1].0.clone();
    let cli_hash = digest(&root.join(&cli))?;
    let mut copies = BTreeMap::new();
    compatibility_copy(root, &cli, ".build/rust/release/ruycast", &mut copies)?;
    compatibility_copy(root, &cli, "rust/target/debug/ruycast", &mut copies)?;
    let mut steps = vec![];
    let mut renders = BTreeMap::new();
    for command in matrix(&target)? {
        steps.push(step(root, lease, &fixture, &command, &receipt, &mut copies)?);
    }
    for theme in ["light", "dark"] {
        for width in ["600", "760"] {
            let name = format!("native-{theme}-{width}");
            let command = Step {
                name: name.clone(),
                arguments: vec![
                    root.join("rust-native/target/release/ruycast-native")
                        .to_str()
                        .ok_or("Invalid native host path.")?
                        .into(),
                    "--check-native".into(),
                    directory
                        .join(format!("{name}.png"))
                        .to_str()
                        .ok_or("Invalid render path.")?
                        .into(),
                    theme.into(),
                    width.into(),
                ],
                copies: vec![],
            };
            steps.push(step(root, lease, &fixture, &command, &receipt, &mut copies)?);
            let value: serde_json::Value =
                serde_json::from_slice(&regular_bytes(&directory.join(format!("{name}.stdout")))?)
                    .map_err(error)?;
            if value["ok"] != true
                || value["windowShown"] != false
                || value["externalExecution"] != false
            {
                return Err("Native render violated hidden/no-execution contract.".into());
            }
            renders.insert(name.clone(),serde_json::json!({"imageSHA256":digest(&directory.join(format!("{name}.png")))?,"resultSHA256":digest(&directory.join(format!("{name}.stdout")))?}));
        }
    }
    steps.push(step(
        root,
        lease,
        &fixture,
        &Step {
            name: "rust-only-policy-final".into(),
            arguments: vec![
                "python3".into(),
                "rust-native/scripts/check_rust_only.py".into(),
            ],
            copies: vec![],
        },
        &receipt,
        &mut copies,
    )?);
    if sources(root)? != before || digest(&root.join(&cli))? != cli_hash {
        return Err("Source or CLI changed during verification; no pass published.".into());
    }
    let mut value = serde_json::json!({"ok":true,"schema":"ruycast.native-verification.v2","scope":"Rust development validation; complete product and delivery acceptance remain separate","sources":before,"steps":steps,"renders":renders,"pureCoreOracle":pure_core_oracle,"testCompatibilityCopies":copies,"productionAccepted":false});
    let mut binaries = BTreeMap::new();
    for alias in copies.keys() {
        binaries.insert(alias.clone(), digest(&root.join(alias))?);
    }
    value["binaries"] = serde_json::to_value(binaries).map_err(error)?;
    if pure_core_oracle != oracles::verify_core(root, lease)? {
        return Err("Pure Core oracle changed during native verification.".into());
    }
    for (field, filename) in REFERENCES {
        let raw = regular_bytes(&directory.join("native-profile").join(filename))?;
        let binding: serde_json::Value = if *field == "hidStateReference" {
            oracles::qualify_hid_reference(root, &value["binaries"], &raw)?
        } else {
            serde_json::from_slice(&raw).map_err(error)?
        };
        check_binding(root, &binding)?;
        value[*field] = binding;
    }
    let mut sdks = BTreeMap::new();
    for entry in fs::read_dir(directory.join("native-profile/host-sdks")).map_err(error)? {
        let path = entry.map_err(error)?.path();
        if path.extension().is_none_or(|value| value != "json") {
            continue;
        }
        let binding: serde_json::Value =
            serde_json::from_slice(&regular_bytes(&path)?).map_err(error)?;
        check_host_sdk_sources(root, &binding)?;
        let artifact = Path::new(
            binding["artifact"]
                .as_str()
                .ok_or("Missing SDK artifact.")?,
        );
        owned_path(root, artifact)?;
        if artifact.parent() != Some(directory.join("native-profile/host-sdks").as_path()) {
            return Err("Unexpected native SDK artifact location.".into());
        }
        let sha = digest(artifact)?;
        if binding["sha256"] != sha {
            return Err("Native SDK artifact changed.".into());
        }
        sdks.insert(
            artifact
                .strip_prefix(root)
                .map_err(error)?
                .to_str()
                .ok_or("Invalid SDK artifact path.")?
                .to_owned(),
            sha,
        );
    }
    check_host_sdk_paths(sdks.keys())?;
    value["nativeHostSDKs"] = serde_json::to_value(sdks).map_err(error)?;
    lease.validate()?;
    verify_value(root, &receipt)?;
    ruycast_native::private_storage::write(
        &report,
        &serde_json::to_vec_pretty(&value).map_err(error)?,
        true,
    )?;
    Ok(
        serde_json::json!({"ok":true,"steps":value["steps"].as_array().ok_or("Missing steps.")?.len(),"report":report,"productionAccepted":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_receipt_sources_match_both_consumers_and_reject_drift() {
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        let mut sources = BTreeMap::new();
        for name in KEYBOARD_SDK_SOURCES {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"owned nonexecutable source").unwrap();
            sources.insert(path.clone(), digest(&path).unwrap());
        }
        let directory = root.join(DIRECTORY).join("native-profile/host-sdks");
        fs::create_dir_all(&directory).unwrap();
        let artifact = directory.join("NativeKeyboard.dylib");
        fs::write(&artifact, b"owned nonexecutable fixture").unwrap();
        let binding = serde_json::json!({"sources":sources,"artifact":artifact,"sha256":digest(&artifact).unwrap()});
        fs::write(directory.join("NativeKeyboard.json"), serde_json::to_vec(&binding).unwrap()).unwrap();
        assert!(keyboard_fixture(root).is_ok());
        assert!(check_host_sdk_sources(root, &binding).is_ok());
        let mut omitted = binding.clone();
        omitted["sources"].as_object_mut().unwrap().remove(root.join(KEYBOARD_SDK_SOURCES[0]).to_str().unwrap());
        assert!(check_host_sdk_sources(root, &omitted).is_err());
        let mut extra = binding.clone();
        extra["sources"].as_object_mut().unwrap().insert(root.join("rust-native/tests/extra.rs").to_str().unwrap().into(), serde_json::json!("irrelevant"));
        assert!(check_host_sdk_sources(root, &extra).is_err());
        let mut escaped = binding.clone();
        escaped["sources"].as_object_mut().unwrap().remove(root.join(KEYBOARD_SDK_SOURCES[0]).to_str().unwrap());
        escaped["sources"].as_object_mut().unwrap().insert(root.join("../outside").to_str().unwrap().into(), serde_json::json!("irrelevant"));
        assert!(check_host_sdk_sources(root, &escaped).is_err());
        fs::write(&artifact, b"changed artifact").unwrap();
        assert!(keyboard_fixture(root).is_err());
        fs::write(&artifact, b"owned nonexecutable fixture").unwrap();
        fs::write(root.join(KEYBOARD_SDK_SOURCES[0]), b"changed source").unwrap();
        assert!(check_host_sdk_sources(root, &binding).is_err());
        assert!(keyboard_fixture(root).is_err());
    }
    #[test]
    fn host_sdk_roster_requires_the_general_recorder_and_refuses_substitutes_or_extras() {
        assert_eq!(plan("aarch64-apple-darwin").unwrap()["requiredNativeSDKs"], 5);
        let paths = ["GeneralKeyboard", "NativeChoice", "NativeHid", "NativeKeyboard", "NativeKeyword"]
            .map(|name| format!("rust-native/target/verification/native-profile/host-sdks/{name}.dylib"));
        assert!(check_host_sdk_paths(paths.iter()).is_ok());
        assert!(check_host_sdk_paths(paths[1..].iter()).is_err());
        let mut substitute = paths.to_vec();
        substitute[0] = "rust-native/target/verification/native-profile/host-sdks/Unrelated.dylib".into();
        assert!(check_host_sdk_paths(substitute.iter()).is_err());
        let mut extra = paths.to_vec();
        extra.push("rust-native/target/verification/native-profile/host-sdks/Extra.dylib".into());
        assert!(check_host_sdk_paths(extra.iter()).is_err());
        let escaped = paths.map(|path| path.replace("native-profile/host-sdks/", "native-profile/host-sdks/../"));
        assert!(check_host_sdk_paths(escaped.iter()).is_err());
    }
    #[test]
    fn cli_contract_route_binds_receipt_cli_and_fresh_hosts_without_running_them() {
        let fixture=super::super::tests::Fixture::new();let root=&fixture.0;
        let target="aarch64-apple-darwin";
        let paths=cli_contract_paths(root,target).unwrap();
        for (path,_) in &paths {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path,b"owned nonexecutable metadata fixture, never a loader grant").unwrap();
        }
        let cli=artifact_paths(target).unwrap()[1].0.clone();
        let receipt=Receipt{
            schema:SCHEMA.into(),product_version:rustcast_core::product_version::CURRENT.into(),configuration:"release".into(),release_preparation:None,target:target.into(),
            sources:BTreeMap::new(),artifacts:BTreeMap::from([(cli.clone(),digest(&root.join(&cli)).unwrap())]),linked_libraries:BTreeMap::new(),toolchains:BTreeMap::new(),commands:vec![],environment_overrides:BTreeMap::new(),production_accepted:false,installed_or_published:false,
        };
        // The outer run verifies a real Receipt. This unit never admits these
        // deliberately nonexecutable bytes or calls product-version helpers.
        let parent=root.join(".build/.u-cli-contract");fs::create_dir_all(&parent).unwrap();
        let mut command=Command::new("unexecuted-cli-carry");
        command.env("RUYCAST_CLI_PARITY_BINARY","/untrusted").env("RUYCAST_CLI_PARITY_BINARY_SHA256","untrusted");
        let inputs=cli_contract_environment(root,&receipt,&parent,&mut command).unwrap();
        let env=command.get_envs().map(|(k,v)|(k.to_string_lossy().into_owned(),v.map(|s|s.to_string_lossy().into_owned()))).collect::<BTreeMap<_,_>>();
        for (path,key) in &paths {
            assert_eq!(env[*key],Some(path.to_string_lossy().into_owned()));
            assert_eq!(env[&format!("{key}_SHA256")],Some(digest(path).unwrap()));
        }
        assert_eq!(env["RUYCAST_NATIVE_TEST_FIXTURE_DIR"],Some(parent.to_string_lossy().into_owned()));
        check_cli_contract_inputs(root,&receipt,&inputs).unwrap();
        let mut omitted=inputs.clone();omitted["artifacts"].as_object_mut().unwrap().remove(&cli);
        assert!(check_cli_contract_inputs(root,&receipt,&omitted).is_err());
        let mut escaped=inputs.clone();escaped["fixtureDirectory"]=serde_json::json!(".build/../escape");
        assert!(check_cli_contract_inputs(root,&receipt,&escaped).is_err());
        for (path,_) in &paths {
            let before=fs::read(path).unwrap();fs::write(path,b"changed actual artifact").unwrap();
            assert!(check_cli_contract_inputs(root,&receipt,&inputs).is_err());fs::write(path,before).unwrap();
        }
        let source=root.join("rust/src/lib.rs");fs::write(&source,b"changed actual source").unwrap();
        assert!(check_cli_contract_inputs(root,&receipt,&inputs).is_err());
        assert!(cli_contract_socket_budget(Path::new("relative")).is_err());
        assert!(cli_contract_socket_budget(Path::new("/a/path/that/is/too/long/for/owned/sockets/because/it/exceeds/the/actual/platform/budget/without/shortening/the/test")).is_err());
        let steps=matrix(target).unwrap();
        let index=steps.iter().position(|s|s.name=="private-results-process").unwrap();
        let entry=&steps[index+1];assert_eq!(entry.name,"cli-contract-process");assert!(entry.copies.is_empty());
        assert!(entry.arguments.windows(2).any(|p|p==["--manifest-path","rust/Cargo.toml"]));
        assert!(entry.arguments.windows(2).any(|p|p==["--bin","ruycast"]));
        assert!(entry.arguments.contains(&"startup_discovery_tests::current_cli_".into()));
        assert!(entry.arguments.contains(&"--ignored".into()));
    }
    #[test]
    fn example_ui_routes_bind_current_builds_and_actual_frames_without_loading_code() {
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        for name in EXAMPLE_UI_SOURCES {
            let file = root.join(name);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"owned nonexecutable source").unwrap();
        }
        let sdk_dir=root.join(DIRECTORY).join("native-profile/host-sdks");
        fs::create_dir_all(&sdk_dir).unwrap();
        let mut keyboard_sources=BTreeMap::new();
        for name in KEYBOARD_SDK_SOURCES {
            let path=root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();fs::write(&path,b"owned nonexecutable keyboard source").unwrap();
            keyboard_sources.insert(path.clone(),digest(&path).unwrap());
        }
        let sdk=sdk_dir.join("NativeKeyboard.dylib");fs::write(&sdk,b"owned nonexecutable keyboard fixture").unwrap();
        fs::write(sdk_dir.join("NativeKeyboard.json"),serde_json::to_vec(&serde_json::json!({"sources":keyboard_sources,"artifact":sdk,"sha256":digest(&sdk).unwrap()})).unwrap()).unwrap();
        let native = root.join("rust-native/target/validation-native/aarch64-apple-darwin/debug/ruycast-native");
        let child = root.join("rust-native/target/validation-build/aarch64-apple-darwin/debug/private-results-fixture");
        let shim = root.join("rust-native/target/clipboard-history-effects.dylib");
        for file in [&native, &child, &shim] {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"owned nonexecutable input, never a loader grant").unwrap();
        }
        fs::create_dir_all(root.join(DIRECTORY)).unwrap();
        let steps = matrix("aarch64-apple-darwin").unwrap();
        assert!(example_ui_socket_budget(Path::new("relative")).is_err());
        assert!(example_ui_socket_budget(Path::new("/a/repository/path/that/is/far/too/long/to/fit/any/owned/native/unix/socket/under/its/own/fixture/root")).is_err());
        assert!(steps.iter().position(|s|s.name=="private-results-process").unwrap() < steps.iter().position(|s|s.name=="task-date-process").unwrap());
        for name in ["render-updates-process", "polish-bindings-process", "polish-original-process", "task-date-process", "actions-menu-process", "item-binding-original-process", "palette-lifecycle-process", "interaction-trace-process", "feedback-actions-process"] {
            let entry = steps.iter().find(|step|step.name==name).unwrap();
            let parent = root.join(format!(".u-{}", entry.name));
            fs::create_dir(&parent).unwrap();
            let mut command = Command::new("unexecuted-owned-example-fixture");
            let mut inputs = example_ui_environment(root, entry, &parent, &mut command).unwrap();
            let env = command.get_envs().map(|(k,v)|(k.to_string_lossy().into_owned(), v.map(|s|s.to_string_lossy().into_owned()))).collect::<BTreeMap<_,_>>();
            assert_eq!(env["RUYCAST_UI_VALIDATION_HOST"], Some(native.to_string_lossy().into_owned()));
            assert_eq!(env["RUYCAST_UI_VALIDATION_HOST_SHA"], Some(digest(&native).unwrap()));
            let uses_fixture = matches!(name, "task-date-process" | "polish-original-process");
            let uses_keyboard = name == "item-binding-original-process";
            if uses_keyboard {
                assert_eq!(env["RUYCAST_TEST_KEYBOARD_SDK"],Some(sdk.to_string_lossy().into_owned()));
                assert_eq!(env["RUYCAST_TEST_KEYBOARD_SHA"],Some(digest(&sdk).unwrap()));
            } else { assert!(!env.contains_key("RUYCAST_TEST_KEYBOARD_SDK")); }
            if uses_fixture {
                assert_eq!(env["RUYCAST_UI_VALIDATION_FIXTURE"], Some(child.to_string_lossy().into_owned()));
                assert_eq!(env["RUYCAST_UI_VALIDATION_FIXTURE_SHA"], Some(digest(&child).unwrap()));
            }
            let evidence = root.join(inputs["evidenceDirectory"].as_str().unwrap());
            assert!(check_example_ui_inputs(root, "aarch64-apple-darwin", &inputs, uses_fixture, uses_keyboard).is_err());
            let frame = evidence.join("frames/owned-frame.json");
            fs::write(&frame, b"owned nonexecutable evidence").unwrap();
            let mut generated = BTreeMap::new();
            source_folder(root, &evidence, &mut generated).unwrap();
            inputs["artifacts"].as_object_mut().unwrap().extend(serde_json::to_value(generated).unwrap().as_object().unwrap().clone());
            check_example_ui_inputs(root, "aarch64-apple-darwin", &inputs, uses_fixture, uses_keyboard).unwrap();
            if uses_keyboard {
                assert!(check_example_ui_inputs(root,"aarch64-apple-darwin",&inputs,uses_fixture,false).is_err(),"Keyboard evidence cannot be omitted");
                let old=fs::read(&sdk).unwrap();fs::write(&sdk,b"changed owned keyboard fixture").unwrap();
                assert!(check_example_ui_inputs(root,"aarch64-apple-darwin",&inputs,uses_fixture,true).is_err());fs::write(&sdk,old).unwrap();
            }
            if uses_fixture {
                assert!(check_example_ui_inputs(root, "aarch64-apple-darwin", &inputs, false, uses_keyboard).is_err(), "The required child fixture cannot be omitted");
                let original = fs::read(&child).unwrap();
                fs::write(&child, b"changed actual child fixture").unwrap();
                assert!(check_example_ui_inputs(root, "aarch64-apple-darwin", &inputs, true, uses_keyboard).is_err());
                fs::write(&child, original).unwrap();
            }
            fs::write(&frame, b"changed actual-frame binding").unwrap();
            assert!(check_example_ui_inputs(root, "aarch64-apple-darwin", &inputs, uses_fixture, uses_keyboard).is_err());
            assert!(example_ui_environment(root, entry, &parent, &mut command).is_err(), "Never reuse prior fixture or output directory");
        }
    }
    #[test]
    fn system_ui_plumbing_binds_fresh_inputs_and_isolates_owned_outputs_without_loading_code() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        let sdk_dir = root.join(DIRECTORY).join("native-profile/host-sdks");
        fs::create_dir_all(&sdk_dir).unwrap();
        let mut sources = BTreeMap::new();
        for name in KEYBOARD_SDK_SOURCES {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"owned nonexecutable input").unwrap();
            sources.insert(path.clone(), digest(&path).unwrap());
        }
        for name in SYSTEM_UI_SOURCES {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"owned nonexecutable input").unwrap();
        }
        let sdk = sdk_dir.join("NativeKeyboard.dylib");
        fs::write(&sdk, b"owned nonexecutable artifact; NOT qualified SDK code").unwrap();
        let manifest = sdk_dir.join("NativeKeyboard.json");
        fs::write(&manifest, serde_json::to_vec(&serde_json::json!({"sources":sources,"artifact":sdk,"sha256":digest(&sdk).unwrap()})).unwrap()).unwrap();
        let entry = matrix("aarch64-apple-darwin").unwrap().into_iter().find(|s| s.name == "system-search-sdk").unwrap();
        let native = root.join("rust-native/target/validation-build/aarch64-apple-darwin/debug/ruycast-native");
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, b"owned nonexecutable host fixture").unwrap();
        let parent = root.join(".u-owned-fixture");
        fs::create_dir(&parent).unwrap();
        let mut command = Command::new("unexecuted-owned-plumbing-fixture");
        command.env("RUYCAST_BROWSER_NATIVE_SHA", "untrusted ambient value")
            .env("RUYCAST_BROWSER_EVIDENCE", "/untrusted/output")
            .env("RUYCAST_NATIVE_EVIDENCE_DIR", "/untrusted/output")
            .env("RUYCAST_SYSTEM_UI_EVIDENCE", "/untrusted/output");
        let inputs = system_ui_environment(root, &entry, &parent, &mut command).unwrap();
        let env = command.get_envs().map(|(key,value)|(key.to_string_lossy().into_owned(),value.map(|v|v.to_string_lossy().into_owned()))).collect::<BTreeMap<_,_>>();
        assert_eq!(env["RUYCAST_BROWSER_NATIVE_SHA"], Some(digest(&native).unwrap()));
        assert_eq!(env["RUYCAST_BROWSER_KEYBOARD_MANIFEST"], Some(manifest.to_string_lossy().into_owned()));
        assert_eq!(env["RUYCAST_NATIVE_TEST_FIXTURE_DIR"], Some(parent.to_string_lossy().into_owned()));
        let output = root.join(inputs["evidenceDirectory"].as_str().unwrap());
        assert_eq!(env["RUYCAST_BROWSER_EVIDENCE"], Some(output.join("browser").to_string_lossy().into_owned()));
        assert_eq!(env["RUYCAST_NATIVE_EVIDENCE_DIR"], Some(output.to_string_lossy().into_owned()));
        assert_eq!(env["RUYCAST_SYSTEM_UI_EVIDENCE"], Some(output.join("system").to_string_lossy().into_owned()));
        assert!(!output.join("browser").exists(), "Existing strict consumer must exclusively create its output");
        assert!(output.join("system").is_dir());
        assert_eq!(fs::metadata(&output).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::read(parent.join(".ruycast-ui-fixture-parent")).unwrap(), b"owned-ui-fixture-parent-v1");
        check_binding(root, &inputs).unwrap();
        fs::write(output.join("system/owned-nonexecutable-receipt.json"), b"owned plumbing fixture; not a host result").unwrap();
        let producer = output.join("keyboard-owned");
        fs::create_dir(&producer).unwrap();
        fs::write(producer.join("fixture.dylib"), b"owned nonexecutable producer-output binding fixture").unwrap();
        fs::write(producer.join("receipt.json"), b"owned nonexecutable producer receipt").unwrap();
        let mut complete = inputs.clone();
        let mut generated = BTreeMap::new();
        source_folder(root, &output, &mut generated).unwrap();
        complete["artifacts"].as_object_mut().unwrap().extend(serde_json::to_value(generated).unwrap().as_object().unwrap().clone());
        check_system_ui_inputs(root, "aarch64-apple-darwin", &complete).unwrap();
        let mut omitted = complete.clone();
        omitted["artifacts"] = serde_json::json!({});
        assert!(check_system_ui_inputs(root, "aarch64-apple-darwin", &omitted).is_err());
        fs::write(producer.join("fixture.dylib"), b"late producer SDK byte mutation").unwrap();
        assert!(check_system_ui_inputs(root, "aarch64-apple-darwin", &complete).is_err());
        fs::write(producer.join("fixture.dylib"), b"owned nonexecutable producer-output binding fixture").unwrap();
        fs::write(output.join("system/owned-nonexecutable-receipt.json"), b"late receipt mutation").unwrap();
        assert!(check_system_ui_inputs(root, "aarch64-apple-darwin", &complete).is_err());
        assert!(system_ui_environment(root, &entry, &parent, &mut command).is_err(), "Never reuse an existing evidence directory");
        fs::write(&native, b"late native image replacement").unwrap();
        assert!(check_binding(root, &inputs).is_err());
        assert!(system_ui_socket_budget(Path::new("/owned/u")).is_ok());
        assert!(system_ui_socket_budget(&root.join("this-owned-path-is-deliberately-too-long-for-a-Darwin-socket")).is_err());
        // No child is spawned, no SDK or host bytes above are executable, and
        // none of these binding checks substitutes for SDK consumer admission.
    }
    #[test]
    fn dashboard_reference_output_requires_both_cases_without_skips() {
        let output = b"test populated_top30_hash_ties_names_samples_errors_and_legacy ... ok\ntest elapsed_hours_retention_dst_and_calendar_year ... ok\ntest result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n";
        assert!(selected_step_output("dashboard-presentation-process", output).is_ok());
        assert!(selected_step_output("dashboard-presentation-process", &output[..40]).is_err());
        let skipped=String::from_utf8_lossy(output).replace("5 passed; 0 failed; 0 ignored;", "4 passed; 0 failed; 1 ignored;");
        assert!(selected_step_output("dashboard-presentation-process", skipped.as_bytes()).is_err());
    }
    #[test]
    fn keyboard_fixture_requires_current_sources_and_exact_owned_artifact() {
        let fixture = super::super::tests::Fixture::new();
        let directory = fixture.0.join(DIRECTORY).join("native-profile/host-sdks");
        fs::create_dir_all(&directory).unwrap();
        let mut sources = BTreeMap::new();
        for name in KEYBOARD_SDK_SOURCES {
            let path = fixture.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"owned nonexecutable fixture input").unwrap();
            sources.insert(path.clone(), digest(&path).unwrap());
        }
        let artifact = directory.join("NativeKeyboard.dylib");
        fs::write(&artifact, b"owned nonexecutable fixture artifact").unwrap();
        let sha = digest(&artifact).unwrap();
        let original = serde_json::json!({"sources":sources,"artifact":artifact,"sha256":sha});
        let receipt = directory.join("NativeKeyboard.json");
        let write = |value: &serde_json::Value| {
            fs::write(&receipt, serde_json::to_vec(value).unwrap()).unwrap()
        };
        write(&original);
        assert_eq!(
            keyboard_fixture(&fixture.0).unwrap(),
            (artifact.clone(), sha)
        );
        let mut changed = original.clone();
        changed["artifact"] = serde_json::json!(fixture.0.join("other.dylib"));
        write(&changed);
        assert!(keyboard_fixture(&fixture.0).is_err());
        write(&original);
        fs::write(&artifact, b"late artifact mutation").unwrap();
        assert!(keyboard_fixture(&fixture.0).is_err());
        fs::write(&artifact, b"owned nonexecutable fixture artifact").unwrap();
        let input = fixture.0.join("rust-native/tests/fixtures/keyboard_sdk.rs");
        fs::write(&input, b"late source mutation").unwrap();
        assert!(keyboard_fixture(&fixture.0).is_err());
        fs::write(&input, b"owned nonexecutable fixture input").unwrap();
        let mut changed = original.clone();
        changed["sources"]
            .as_object_mut()
            .unwrap()
            .insert("unreviewed".into(), serde_json::json!("ignored"));
        write(&changed);
        assert!(keyboard_fixture(&fixture.0).is_err());
        write(&original);
        assert!(keyboard_fixture(&fixture.0).is_ok());
    }
    #[test]
    fn compatibility_cli_copies_replace_stale_aliases_and_bind_both_outputs() {
        let fixture = super::super::tests::Fixture::new();
        let source = "verified-cli-fixture";
        fs::write(fixture.0.join(source), b"owned verified CLI fixture bytes").unwrap();
        let expected = digest(&fixture.0.join(source)).unwrap();
        let mut copies = BTreeMap::new();
        for alias in [".build/rust/release/ruycast", "rust/target/debug/ruycast"] {
            let path = fixture.0.join(alias);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"stale owned CLI fixture").unwrap();
            assert_ne!(digest(&path).unwrap(), expected);
            compatibility_copy(&fixture.0, source, alias, &mut copies).unwrap();
            assert_eq!(digest(&path).unwrap(), expected);
            assert_eq!(copies[alias]["source"], source);
            assert_eq!(copies[alias]["sha256"], expected);
            assert_eq!(copies[alias]["productionAuthority"], false);
            assert_eq!(copies[alias]["purpose"], "test-compatibility-only");
        }
        assert_eq!(copies.len(), 2);
        assert_eq!(digest(&fixture.0.join(source)).unwrap(), expected);
        fs::write(
            fixture.0.join("rust/target/debug/ruycast"),
            b"late owned write",
        )
        .unwrap();
        assert_ne!(
            digest(&fixture.0.join("rust/target/debug/ruycast")).unwrap(),
            copies["rust/target/debug/ruycast"]["sha256"]
                .as_str()
                .unwrap()
        );
    }
    #[test]
    fn normal_host_negative_alias_uses_receipt_default_and_refuses_mutation() {
        let fixture = super::super::tests::Fixture::new();
        let source = artifact_paths("aarch64-apple-darwin").unwrap()[0].0.clone();
        fs::create_dir_all(fixture.0.join(&source).parent().unwrap()).unwrap();
        fs::write(fixture.0.join(&source), b"owned default-role fixture").unwrap();
        let expected = digest(&fixture.0.join(&source)).unwrap();
        let receipt = Receipt {schema:String::new(),product_version:String::new(),configuration:"release".into(),release_preparation:None,
            target:"aarch64-apple-darwin".into(),sources:BTreeMap::new(),artifacts:BTreeMap::from([(source.clone(),expected.clone())]),
            linked_libraries:BTreeMap::new(),toolchains:BTreeMap::new(),commands:vec![],environment_overrides:BTreeMap::new(),production_accepted:false,installed_or_published:false};
        fs::create_dir_all(fixture.0.join(NORMAL_HOST_ALIAS).parent().unwrap()).unwrap();
        fs::write(fixture.0.join(NORMAL_HOST_ALIAS), b"owned tools-role alias").unwrap();
        let mut copies = BTreeMap::new();
        let inputs = prepare_normal_host(&fixture.0, &receipt, &mut copies).unwrap();
        assert_eq!(digest(&fixture.0.join(NORMAL_HOST_ALIAS)).unwrap(), expected);
        assert_eq!(copies[NORMAL_HOST_ALIAS]["source"], source);
        check_normal_host_inputs(&fixture.0, &receipt, &inputs).unwrap();
        fs::write(fixture.0.join(NORMAL_HOST_ALIAS), b"late tools overwrite").unwrap();
        assert!(check_normal_host_inputs(&fixture.0, &receipt, &inputs).is_err());
        prepare_normal_host(&fixture.0, &receipt, &mut copies).unwrap();
        fs::write(fixture.0.join(&source), b"changed receipt source").unwrap();
        assert!(prepare_normal_host(&fixture.0, &receipt, &mut copies).is_err());
        let selected = matrix("aarch64-apple-darwin").unwrap().into_iter()
            .filter(normal_host_negative).map(|entry|entry.name).collect::<Vec<_>>();
        assert_eq!(selected, ["bridge-process"]);
    }
    #[test]
    fn complete_ordered_matrix_keeps_all_oracles_and_explicit_targets() {
        let steps = matrix("aarch64-apple-darwin").unwrap();
        assert_eq!(steps.first().unwrap().name, "rust-only-policy");
        assert_eq!(steps.last().unwrap().name, "release-build");
        assert_eq!(steps.len(), 133);
        let choice_sdk = steps.iter().position(|step|step.name=="native-choice-sdk-build").unwrap();
        let snippet_choice = steps.iter().position(|step|step.name=="native-snippet-choice-process-rust").unwrap();
        assert!(choice_sdk < snippet_choice);
        let manager_sdk = steps.iter().position(|step|step.name=="manager-workspace-sdk-build").unwrap();
        let native_open = steps.iter().position(|step|step.name=="native-open-basic-process").unwrap();
        assert!(manager_sdk < native_open);
        assert!(steps[native_open].arguments.windows(2).any(|pair|pair==["--test","native_open_basic_process"]));
        let manager_test = steps.iter().position(|step|step.name=="manager-process").unwrap();
        assert!(manager_sdk < manager_test);
        assert!(steps[manager_sdk].arguments.windows(2).any(|pair|pair==["--example","workspace-fixture"]));
        assert!(steps[manager_test].arguments.windows(2).any(|pair|pair==["--test","manager_process"]));
        assert!(!steps[manager_test].arguments.iter().any(|arg|arg.ends_with(".py")));
        let keyboard_sdk = steps.iter().position(|step|step.name=="native-keyboard-sdk-build").unwrap();
        let keyboard_test = steps.iter().position(|step|step.name=="native-keyboard-process").unwrap();
        assert!(keyboard_sdk < keyboard_test);
        let settings_keyboard = steps.iter().position(|step|step.name=="settings-keyboard-process").unwrap();
        assert!(keyboard_sdk < settings_keyboard);
        assert!(steps[settings_keyboard].arguments.windows(2).any(|pair|pair==["--test","settings_keyboard_process"]));
        assert!(!steps[settings_keyboard].arguments.iter().any(|arg|arg.ends_with(".py")));
        let settings_preferences = steps.iter().find(|step|step.name=="settings-preferences-process").unwrap();
        assert!(settings_preferences.arguments.windows(2).any(|pair|pair==["--test","settings_preferences_process"]));
        assert!(!settings_preferences.arguments.iter().any(|arg|arg.ends_with(".py")));
        assert!(steps[keyboard_sdk].arguments.windows(2).any(|pair|pair==["--example","keyboard-host"]));
        assert!(steps[keyboard_test].arguments.windows(2).any(|pair|pair==["--test","native_keyboard_process"]));
        assert!(!steps[keyboard_test].arguments.iter().any(|arg|arg.ends_with(".py")));
        let caps_sdk = steps.iter().position(|step|step.name=="caps-sdk-build").unwrap();
        let caps_test = steps.iter().position(|step|step.name=="caps-process").unwrap();
        assert!(caps_sdk < caps_test);
        for example in ["caps-fixture", "caps-missing-set"] {
            assert!(steps[caps_sdk].arguments.windows(2).any(|pair|pair==["--example",example]));
        }
        assert!(steps[caps_test].arguments.windows(2).any(|pair|pair==["--test","caps_process"]));
        assert!(!steps[caps_test].arguments.iter().any(|arg|arg.ends_with(".py")));
        let sdk_build = steps.iter().position(|step|step.name=="system-sdk-build").unwrap();
        let sdk_test = steps.iter().position(|step|step.name=="system-search-sdk").unwrap();
        assert!(sdk_build < sdk_test);
        assert!(steps[sdk_build].arguments.windows(2).any(|pair|pair==["--example","system-fixture"]));
        let clipboard_text = steps.iter().position(|step|step.name=="clipboard-text-process").unwrap();
        let board_build = steps.iter().position(|step|step.name=="snippet-board-sdk-build").unwrap();
        assert!(board_build < clipboard_text);
        assert!(steps[clipboard_text].arguments.windows(2).any(|pair|pair==["--test","clipboard_text_process"]));
        assert!(!steps[clipboard_text].arguments.iter().any(|arg|arg.ends_with(".py") || arg.contains("swiftc")));
        let hid_state = steps.iter().find(|step|step.name=="hid-state-process").unwrap();
        assert!(hid_state.arguments.windows(2).any(|pair|pair==["--test","hid_state_process"]));
        assert!(!hid_state.arguments.iter().any(|arg|arg.ends_with(".py") || arg.contains("swiftc")));
        let keyword = steps.iter().find(|step|step.name=="keyword-process").unwrap();
        assert!(keyword.arguments.windows(2).any(|pair|pair==["--test","keyword_process"]));
        assert!(!keyword.arguments.iter().any(|arg|arg.ends_with(".py") || arg.contains("swiftc")));
        let choice = steps.iter().find(|step|step.name=="snippet-choice-process").unwrap();
        assert!(choice.arguments.windows(2).any(|pair|pair==["--test","snippet_choice_process"]));
        assert!(!choice.arguments.iter().any(|arg|arg.ends_with(".py") || arg.contains("swiftc")));
        let snippet = steps.iter().find(|step|step.name=="snippet-process").unwrap();
        assert!(snippet.arguments.windows(2).any(|pair|pair==["--test","snippet_process"]));
        assert!(!snippet.arguments.iter().any(|arg|arg.ends_with(".py") || arg.contains("swiftc")));
        let snippet_sdk = steps.iter().position(|step|step.name=="snippet-faults-sdk-build").unwrap();
        let snippet_test = steps.iter().position(|step|step.name=="snippet-process").unwrap();
        assert!(snippet_sdk < snippet_test);
        assert!(steps[snippet_sdk].arguments.windows(2).any(|pair|pair==["--example","snippet-faults"]));
        let template = steps.iter().find(|step|step.name=="template-process").unwrap();
        assert!(template.arguments.windows(2).any(|pair|pair==["--test","template_process"]));
        assert!(!template.arguments.iter().any(|arg|arg.ends_with(".py") || arg.contains("swiftc")));
        let runtime_sdk = steps.iter().position(|step|step.name=="runtime-writer-sdk-build").unwrap();
        let runtime_test = steps.iter().position(|step|step.name=="runtime-process").unwrap();
        let foundation_test = steps.iter().position(|step|step.name=="foundation-process").unwrap();
        assert!(runtime_sdk < foundation_test);
        assert!(steps[foundation_test].arguments.windows(2).any(|pair|pair==["--test","foundation_process"]));
        assert!(!steps[foundation_test].arguments.iter().any(|arg|arg.ends_with(".py")));
        assert!(runtime_sdk < runtime_test);
        let inline_sync = steps.iter().position(|step|step.name=="inline-process").unwrap();
        assert!(runtime_sdk < inline_sync);
        assert!(steps[inline_sync].arguments.windows(2).any(|pair|pair==["--test","inline_sync_process"]));
        assert!(steps[runtime_sdk].arguments.windows(2).any(|pair|pair==["--example","fcntl-sync-shim"]));
        assert!(steps[runtime_test].arguments.windows(2).any(|pair|pair==["--test","runtime_process"]));
        assert!(!steps[runtime_test].arguments.iter().any(|arg|arg.ends_with(".py")));
        let bridge = steps.iter().find(|step|step.name=="bridge-process").unwrap();
        assert!(bridge.arguments.windows(2).any(|pair|pair==["--test","bridge_process"]));
        assert!(normal_host_negative(bridge));
        assert!(!bridge.arguments.iter().any(|arg|arg.ends_with(".py")));
        let actions = steps.iter().find(|step|step.name=="actions-process").unwrap();
        assert!(actions.arguments.windows(2).any(|pair|pair==["--test","actions_process"]));
        assert!(!actions.arguments.iter().any(|arg|arg.ends_with(".py")));
        let creation = steps.iter().find(|step|step.name=="creation-process").unwrap();
        assert!(creation.arguments.windows(2).any(|pair|pair==["--test","creation_process"]));
        assert!(!creation.arguments.iter().any(|arg|arg.ends_with(".py")));
        let editor = steps.iter().find(|step|step.name=="editor-process").unwrap();
        assert!(editor.arguments.windows(2).any(|pair|pair==["--test","editor_process"]));
        assert!(!editor.arguments.iter().any(|arg|arg.ends_with(".py")));
        let library_checkbox = steps.iter().find(|step|step.name=="library-checkbox-process").unwrap();
        assert!(library_checkbox.arguments.windows(2).any(|pair|pair==["--test","library_checkbox_process"]));
        assert!(steps.iter().any(|step|step.name=="library-process" && step.arguments.windows(2).any(|pair|pair==["--test","library_lifecycle_process"])));
        let library_drafts = steps.iter().find(|step|step.name=="library-drafts-process").unwrap();
        assert!(library_drafts.arguments.windows(2).any(|pair|pair==["--test","library_drafts_process"]));
        let library_navigation = steps.iter().find(|step|step.name=="library-navigation-process").unwrap();
        assert!(library_navigation.arguments.windows(2).any(|pair|pair==["--test","library_navigation_process"]));
        let inline_basic = steps.iter().find(|step|step.name=="inline-basic-process").unwrap();
        assert!(inline_basic.arguments.windows(2).any(|pair|pair==["--test","inline_basic_process"]));
        let inline_composition = steps.iter().find(|step|step.name=="inline-composition-process").unwrap();
        assert!(inline_composition.arguments.windows(2).any(|pair|pair==["--test","inline_composition_process"]));
        let inline_transition = steps.iter().find(|step|step.name=="inline-transition-process").unwrap();
        assert!(inline_transition.arguments.windows(2).any(|pair|pair==["--test","inline_transition_process"]));
        let inline_focus = steps.iter().find(|step|step.name=="inline-focus-process").unwrap();
        assert!(inline_focus.arguments.windows(2).any(|pair|pair==["--test","inline_focus_process"]));
        let script_result_basic = steps.iter().find(|step|step.name=="script-result-process").unwrap();
        assert!(script_result_basic.arguments.windows(2).any(|pair|pair==["--test","script_result_basic_process"]));
        let script_result_native = steps.iter().find(|step|step.name=="script-result-native-process").unwrap();
        assert!(script_result_native.arguments.windows(2).any(|pair|pair==["--test","script_result_native_process"]));
        let script_basic = steps.iter().find(|step|step.name=="script-process").unwrap();
        assert!(script_basic.arguments.windows(2).any(|pair|pair==["--test","script_basic_process"]));
        let script_fault_sdk = steps.iter().position(|step|step.name=="script-fault-sdk-build").unwrap();
        let script_fault = steps.iter().position(|step|step.name=="script-fault-process").unwrap();
        assert!(script_fault_sdk < script_fault);
        assert!(steps[script_fault_sdk].arguments.windows(2).any(|pair|pair==["--example","script-fault-shim"]));
        assert!(steps[script_fault].arguments.windows(2).any(|pair|pair==["--test","script_fault_process"]));
        let capture_bytes = steps.iter().find(|step|step.name=="capture-bytes-process").unwrap();
        assert!(capture_bytes.arguments.windows(2).any(|pair|pair==["--test","capture_bytes_process"]));
        let capture_cli = steps.iter().find(|step|step.name=="capture-cli-process").unwrap();
        assert!(capture_cli.arguments.windows(2).any(|pair|pair==["--test","capture_cli_process"]));
        let capture_native = steps.iter().find(|step|step.name=="capture-process").unwrap();
        assert!(capture_native.arguments.windows(2).any(|pair|pair==["--test","capture_native_process"]));
        let app_picker_identity = steps.iter().find(|step|step.name=="app-picker-process").unwrap();
        assert!(app_picker_identity.arguments.windows(2).any(|pair|pair==["--test","app_picker_identity_process"]));
        let applications_discovery = steps.iter().find(|step|step.name=="applications-discovery-process").unwrap();
        assert!(applications_discovery.arguments.windows(2).any(|pair|pair==["--test","applications_discovery_process"]));
        let names = steps
            .iter()
            .map(|step| step.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), steps.len());
        for required in [
            "clipboard-import-parser",
            "benchmark-contract-1", "benchmark-contract-2",
            "temporal-contract-1", "temporal-contract-2", "temporal-contract-3",
            "temporal-contract-4", "temporal-contract-5",
            "remaining-macos-process",
            "startup-process",
            "private-results-process",
            "cli-contract-process",
            "utility-process",
            "clipboard-history-process",
            "clipboard-rich-paste",
            "clipboard-preview-original",
            "clipboard-preview-cancel-original",
            "snippet-links-process",
            "snippet-hotkey-process",
            "feedback-actions-process",
            "actions-menu-process",
            "item-binding-original-process",
            "settings-ui-process",
            "render-updates-process",
            "polish-bindings-process",
            "polish-original-process",
            "palette-lifecycle-process",
            "interaction-trace-process",
            "clipboard-pause-original",
            "task-date-process",
            "snippet-html-import",
            "script-advanced-process",
            "system-search-sdk",
            "native-choice-host-process",
            "capture-process",
            "rich-process-rust",
            "unit",
            "core-unit",
            "clippy",
            "clippy-default",
        ] {
            assert!(names.contains(required));
        }
        for step in &steps {
            if step.arguments[0] == "cargo" {
                assert!(step
                    .arguments
                    .windows(2)
                    .any(|pair| pair == ["--target", "aarch64-apple-darwin"]));
                assert!(step.arguments.contains(&"--offline".into()));
                if step.arguments[1] == "test" {
                    assert_eq!(step.arguments.iter().filter(|arg| *arg == "--").count(), 1);
                    assert_eq!(step.arguments.last().unwrap(), "--test-threads=1");
                }
            }
        }
        let preview = steps.iter().position(|step|step.name=="clipboard-preview-original").unwrap();
        let cancelled = &steps[preview+1];
        assert_eq!(cancelled.name,"clipboard-preview-cancel-original");
        assert!(cancelled.arguments.contains(&"clipboard_preview::tests::cancelled_same_id_reopen_cannot_publish_old_error_or_asset".into()));
        assert!(cancelled.arguments.contains(&"--exact".into()));
        let original = steps.iter().position(|step|step.name=="polish-original-process").unwrap();
        assert!(steps.iter().position(|step|step.name=="private-results-process").unwrap()<original);
        assert_eq!(steps[original].arguments.last().unwrap(),"--polish-original-only");
        let actions = steps.iter().find(|step|step.name=="actions-menu-process").unwrap();
        assert_eq!(actions.arguments.last().unwrap(),"--actions-menu-only");
        assert_eq!(steps.iter().find(|step|step.name=="item-binding-original-process").unwrap().arguments.last().unwrap(),"--item-binding-original-only");
        assert_eq!(REFERENCES.len(), 11);
        assert!(matrix("unsupported").is_err());
    }
    #[test]
    fn selected_routes_reject_zero_tests_and_a_different_example_branch() {
        let dashboard = format!("{}test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n",
            DASHBOARD_EXPORT_CASES.iter().map(|case| format!("test {case} ... ok\n")).collect::<String>());
        assert!(selected_step_output("dashboard-export-process", dashboard.as_bytes()).is_ok());
        for invalid in [
            dashboard.replace(DASHBOARD_EXPORT_CASES[0], "unrelated_case"),
            dashboard.replace("5 passed; 0 failed; 0 ignored;", "0 passed; 0 failed; 5 ignored;"),
            dashboard.replace(" ... ok", " ... ignored"),
            format!("{dashboard}{dashboard}"),
            String::new(),
        ] { assert!(selected_step_output("dashboard-export-process", invalid.as_bytes()).is_err()); }
        let cli=format!("test {} ... ok\ntest {} ... ok\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.1s\n",CLI_CONTRACT_CASES[0],CLI_CONTRACT_CASES[1]);
        assert!(selected_step_output("cli-contract-process",cli.as_bytes()).is_ok());
        for invalid in [
            cli.replace(CLI_CONTRACT_CASES[0],"unrelated_case"),
            cli.replace(CLI_CONTRACT_CASES[1],"unrelated_case"),
            cli.replace("2 passed; 0 failed; 0 ignored;","0 passed; 0 failed; 2 ignored;"),
            cli.replace("2 passed; 0 failed; 0 ignored;","1 passed; 1 failed; 0 ignored;"),
            cli.replace(" ... ok"," ... ignored"),
            format!("test unrelated ... ok\n{cli}"),
            format!("test {} ... ok\n{cli}",CLI_CONTRACT_CASES[0]),
            format!("{cli}test result: ok. 0 passed; 0 failed; 0 ignored;\n"),
        ] {assert!(selected_step_output("cli-contract-process",invalid.as_bytes()).is_err());}
        assert!(selected_step_output("core-unit", b"test result: ok. 0 passed; 0 failed;").is_err());
        assert!(selected_step_output("core-unit", b"test tests::other ... ok\ntest result: ok. 1 passed; 0 failed;").is_err());
        assert!(selected_step_output("core-unit", b"test tests::command_sides_are_independent ... ok\ntest result: ok. 1 passed; 0 failed;").is_ok());
        let settings = serde_json::json!({"ok":true,"settings":true,"themes":2,"tabsPerTheme":7,"renders":20,"hyperLeftAndCommandK":true,"disabledSnippetListed":true,"independentManagementQuery":true,"visibleWindowOpened":false,"physicalInput":false});
        let html = serde_json::json!({"ok":true,"richModes":2,"blankParagraphSamples":7,"hyperlinkImport":true,"noTextTables":true,"clipboardAccessed":false,"visibleWindowOpened":false,"desktopInputSent":false});
        let render = serde_json::json!({"ok":true,"renderUpdates":true,"themes":2,"originalFramesPerTheme":12,"captures":26,"watchedTheme":true,"pendingQuery":true,"selectionImageChanged":true,"independentQueries":true,"visibleWindowOpened":false,"physicalInput":false});
        let bindings = serde_json::json!({"ok":true,"polishBindings":true,"themes":2,"captures":4,"aliasAndKeywordSearch":true,"arrowSelection":true,"visibleWindowOpened":false,"physicalInput":false});
        let actions = serde_json::json!({"ok":true,"actionsMenu":true,"themes":2,"nativeMenuFirstResponder":true,"fieldEditorFallback":true,"selectedRowFourPreserved":true,"clampedMenuArrows":true,"returnEditsOriginal":true,"escapeAndModeRestoreSearch":true,"commandKTogglesClosed":true,"removalQueryBoundaryPreservesIdentity":true,"newSnippetDraft":true,"clipboardDelete":true,"discoveredAppMaterializedWithTarget":true,"recordedPaletteShortcutPayloads":true,"repeatCancelConflictClear":true,"physicalInput":false});
        let original = serde_json::json!({"ok":true,"polishOriginal":true,"themes":4,"contextThemes":2,"captures":52,"filledCaptureFrames":4,"doubleSubmitOnce":true,"clipboardClear":true,"geometry":true,"normalQuit":true,"healthIconQualified":false,"physicalInput":false,"visibleWindowOpened":false});
        let date = serde_json::json!({"ok":true,"taskDate":true,"themes":2,"captures":16,"nativeTyping":true,"invalidRefused":true,"frozenValue":"2000-01-02","doubleSubmitOnce":true,"normalQuit":true,"visibleWindowOpened":false,"physicalInput":false});
        let lifecycle = serde_json::json!({"ok":true,"paletteLifecycle":true,"themes":2,"callbackRoutes":7,"nativeFloatingAndSpaces":true,"retiredCallbackRefused":true,"normalQuit":true,"actualOSMonitorRegistration":false,"physicalInput":false,"visibleWindowOpened":false});
        let item_binding = serde_json::json!({"ok":true,"itemBindingOriginal":true,"themes":2,"fieldCaptures":8,"queuedBeforeMainApplyStopped":true,"plainCommandCommaNotConsumed":true,"genuineRecorderResponderAndAX":true,"clipboardHotkeyOriginal":true,"hotkeyFailureNotice":true,"keyboardCopyCaptures":4,"normalQuit":true,"physicalInput":false,"visibleWindowOpened":false});
        let trace = serde_json::json!({"ok":true,"interactionTrace":true,"cycles":100,"catalogCommands":500,"reconciliationBeforeCapture":true,"actualCaptureDraws":true,"closedFieldCancelled":true,"publicReset":true,"tracePayloadPrivate":true,"normalQuit":true,"physicalInput":false,"visibleWindowOpened":false});
        for (name, value) in [("settings-ui-process", settings), ("snippet-html-import", html), ("render-updates-process", render), ("polish-bindings-process", bindings), ("polish-original-process", original.clone()), ("task-date-process", date), ("actions-menu-process", actions), ("item-binding-original-process", item_binding), ("palette-lifecycle-process", lifecycle), ("interaction-trace-process", trace)] {
            assert!(selected_step_output(name, &serde_json::to_vec(&value).unwrap()).is_ok());
            for key in value.as_object().unwrap().keys() {
                let mut missing = value.clone();
                missing.as_object_mut().unwrap().remove(key);
                assert!(selected_step_output(name, &serde_json::to_vec(&missing).unwrap()).is_err(), "{name} missing {key}");
            }
            let mut wrong = value.clone();
            if name == "settings-ui-process" { wrong["renders"] = 19.into(); }
            else if name == "snippet-html-import" { wrong["blankParagraphSamples"] = 6.into(); }
            else if name == "actions-menu-process" { wrong["themes"] = 1.into(); }
            else if name == "item-binding-original-process" { wrong["fieldCaptures"] = 7.into(); }
            else if name == "palette-lifecycle-process" { wrong["callbackRoutes"] = 6.into(); }
            else if name == "interaction-trace-process" { wrong["cycles"] = 99.into(); }
            else { wrong["captures"] = 0.into(); }
            assert!(selected_step_output(name, &serde_json::to_vec(&wrong).unwrap()).is_err());
            assert!(selected_step_output(name, br#"{"ok":true}"#).is_err());
        }
        for (key, invalid) in [("themes", serde_json::json!(2)), ("contextThemes", serde_json::json!(0)), ("filledCaptureFrames", serde_json::json!(0)), ("healthIconQualified", serde_json::json!(true))] {
            let mut wrong = original.clone(); wrong[key] = invalid;
            assert!(selected_step_output("polish-original-process", &serde_json::to_vec(&wrong).unwrap()).is_err());
        }
        let cancel="test clipboard_preview::tests::cancelled_same_id_reopen_cannot_publish_old_error_or_asset ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.01s\n";
        assert!(selected_step_output("clipboard-preview-cancel-original",cancel.as_bytes()).is_ok());
        let pause="test real_host_preserves_invalid_pause_and_requires_confirmed_explicit_resume ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s\n";
        assert!(selected_step_output("clipboard-pause-original",pause.as_bytes()).is_ok());
        for invalid in [pause.replace("real_host_preserves_invalid_pause_and_requires_confirmed_explicit_resume","unrelated"),pause.replace("1 passed; 0 failed; 0 ignored;","0 passed; 0 failed; 1 ignored;"),pause.replace(" ... ok"," ... ignored"),format!("{pause}test result: ok. 0 passed; 0 failed; 0 ignored;\n")] {
            assert!(selected_step_output("clipboard-pause-original",invalid.as_bytes()).is_err());
        }
        for invalid in [cancel.replace("cancelled_same_id_reopen_cannot_publish_old_error_or_asset","unrelated"),cancel.replace("1 passed; 0 failed; 0 ignored;","0 passed; 0 failed; 1 ignored;"),cancel.replace(" ... ok"," ... ignored"),format!("{cancel}test result: ok. 0 passed; 0 failed; 0 ignored;\n")] {
            assert!(selected_step_output("clipboard-preview-cancel-original",invalid.as_bytes()).is_err());
        }
        assert!(selected_step_output(
            "clipboard-preview-original",
            b"test result: ok. 0 passed; 0 failed; 0 ignored;"
        )
        .is_err());
        assert!(selected_step_output(
            "clipboard-preview-original",
            b"test result: ok. 1 passed; 0 failed; 0 ignored;"
        )
        .is_ok());
        for output in [
            br#"{"ok":true}"#.as_slice(),
            br#"{"snippetHotkey":true,"themes":1}"#,
            b"not JSON",
        ] {
            assert!(selected_step_output("snippet-hotkey-process", output).is_err());
        }
        assert!(selected_step_output(
            "snippet-hotkey-process",
            br#"{"snippetHotkey":true,"themes":2}"#
        )
        .is_ok());
        let complete = serde_json::json!({
            "ok":true,"themes":2,"feedbackActions":true,
            "allOriginalMenuControlsReachable":true,"emptyMainTabsRefresh":true,
            "sameBoardChangeCountAndRawPreserved":true,"useEitherCommand":true,
        });
        assert!(selected_step_output("feedback-actions-process", &serde_json::to_vec(&complete).unwrap()).is_ok());
        for key in ["ok", "feedbackActions", "allOriginalMenuControlsReachable", "emptyMainTabsRefresh", "sameBoardChangeCountAndRawPreserved", "useEitherCommand"] {
            let mut partial = complete.clone();
            partial.as_object_mut().unwrap().remove(key);
            assert!(selected_step_output("feedback-actions-process", &serde_json::to_vec(&partial).unwrap()).is_err());
        }
        let mut one_theme = complete;
        one_theme["themes"] = serde_json::json!(1);
        assert!(selected_step_output("feedback-actions-process", &serde_json::to_vec(&one_theme).unwrap()).is_err());
    }
}
