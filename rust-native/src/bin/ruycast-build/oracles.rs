//! Explicit historical gate inventory. Only source-qualified, data-only
//! oracles execute here; unqualified old-host branches must never run raw.
use super::*;
#[path = "../../../tests/support/hid_state_goldens.rs"]
mod hid_state_goldens;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CoreReceipt {
    schema: String,
    scope: String,
    sources: BTreeMap<String, String>,
    artifacts: BTreeMap<String, String>,
    commands: Vec<Vec<String>>,
    outcome: String,
    production_accepted: bool,
    swift_compatibility: SwiftCompatibility,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SwiftCompatibility {
    modulemap: String,
    architecture: String,
    alias_target: Option<String>,
}
fn swift_alias(root: &Path, architecture: &str) -> Result<Option<String>> {
    if !matches!(architecture, "arm64-apple-macosx" | "x86_64-apple-macosx") {
        return Err("Unexpected Swift oracle architecture.".into());
    }
    let alias = root.join(".build/swift/debug");
    let expected = PathBuf::from(architecture).join("debug");
    match fs::symlink_metadata(&alias) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if fs::read_link(&alias).map_err(error)?.as_os_str() != expected.as_os_str() {
                return Err("Unexpected SwiftPM debug alias; preserve existing target.".into());
            }
            Ok(Some(expected.to_string_lossy().into_owned()))
        }
        Ok(meta) if meta.is_dir() => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(_) => Err("SwiftPM debug alias must be the expected relative link.".into()),
        Err(e) => Err(error(e)),
    }
}
fn swift_compatibility(root: &Path, scratch: &Path, bin: &Path, target: &str) -> Result<SwiftCompatibility> {
    let architecture = match target {
        "aarch64-apple-darwin" => "arm64-apple-macosx",
        "x86_64-apple-darwin" => "x86_64-apple-macosx",
        _ => return Err("Unsupported Swift oracle target.".into()),
    };
    let suffix = bin.strip_prefix(scratch).map_err(error)?;
    if suffix.as_os_str() != PathBuf::from(architecture).join("debug").as_os_str() {
        return Err("Swift oracle reported an unexpected physical debug suffix.".into());
    }
    let alias_target = swift_alias(root, architecture)?;
    let compatibility_bin = if alias_target.is_some() { suffix } else { Path::new("debug") };
    let modulemap = PathBuf::from(".build/swift").join(compatibility_bin).join("CRuycastCore.build/module.modulemap");
    Ok(SwiftCompatibility { modulemap: modulemap.to_string_lossy().into_owned(), architecture: architecture.into(), alias_target })
}
fn verify_swift_compatibility(root: &Path, binding: &SwiftCompatibility, artifacts: &BTreeMap<String, String>) -> Result<()> {
    let compatibility_bin = if binding.alias_target.is_some() { PathBuf::from(&binding.architecture).join("debug") } else { PathBuf::from("debug") };
    let expected = PathBuf::from(".build/swift").join(compatibility_bin).join("CRuycastCore.build/module.modulemap");
    if Path::new(&binding.modulemap) != expected
        || swift_alias(root, &binding.architecture)? != binding.alias_target
        || artifacts.get(&binding.modulemap) != Some(&digest(&root.join(&expected))?)
    {
        return Err("Swift oracle compatibility binding changed.".into());
    }
    Ok(())
}
fn core_sources(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for name in [
        "Package.swift",
        "Sources/LauncherModels",
        "Sources/LauncherCore",
        "Sources/CRuycastCore",
        "Tests/LauncherCoreTests",
        "rust/Cargo.toml",
        "rust/Cargo.lock",
        "rust/src",
    ] {
        validation::source_folder(root, &root.join(name), &mut values)?;
    }
    Ok(values)
}
fn tool(root: &Path, lease: &BuildLock, args: &[String]) -> Result<String> {
    lease.validate()?;
    let mut command = Command::new(&args[0]);
    command.args(&args[1..]).current_dir(root);
    for (name, _) in std::env::vars_os() {
        let name_text = name.to_string_lossy();
        if name_text.starts_with("RUYCAST_")
            || name_text.starts_with("RUSTCAST_")
            || name_text.starts_with("DYLD_")
        {
            command.env_remove(name);
        }
    }
    lease.apply_to_child(&mut command)?;
    let value = process::capture(
        command,
        std::time::Duration::from_secs(180),
        16 * 1024 * 1024,
    )?;
    if !value.status.success() {
        return Err(format!(
            "Historical pure oracle tool failed: {}",
            String::from_utf8_lossy(&value.stderr)
        ));
    }
    String::from_utf8(value.stdout).map_err(error)
}
fn data_copy(root: &Path, source: &Path, destination: &Path) -> Result<()> {
    for path in [source, destination] {
        if !path.starts_with(root) || path.components().any(|part| !matches!(part, std::path::Component::RootDir | std::path::Component::Normal(_))) {
            return Err("Oracle artifact escaped repository.".into());
        }
    }
    // The destination may not exist yet; check its existing prefix before the
    // private writer creates anything, retaining the final descriptor checks.
    let mut prefix = PathBuf::new();
    for component in destination.components() {
        prefix.push(component);
        match fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(format!("Linked oracle copy destination: {}", prefix.display())),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(error(e)),
        }
    }
    let before = digest(source)?;
    let bytes = regular_bytes(source)?;
    ruycast_native::private_storage::write(destination, &bytes, true)?;
    use std::os::unix::fs::PermissionsExt;
    regular_file(destination)?
        .set_permissions(fs::Permissions::from_mode(0o644))
        .map_err(error)?;
    if digest(source)? != before || digest(destination)? != before {
        return Err("Test-only oracle library changed during copy.".into());
    }
    Ok(())
}
pub(super) fn core(root: &Path, lease: &BuildLock) -> Result<serde_json::Value> {
    let before = core_sources(root)?;
    let target = host_target(root)?;
    tool(
        root,
        lease,
        &[
            "python3".into(),
            "rust-native/scripts/check_rust_only.py".into(),
        ],
    )?;
    let mut commands = vec![
        vec![
            "cargo",
            "build",
            "--manifest-path",
            "rust/Cargo.toml",
            "--target-dir",
            ".build/oracle-core",
            "--target",
            &target,
            "--lib",
            "--locked",
            "--offline",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>(),
        vec![
            "swift",
            "build",
            "--scratch-path",
            ".build/oracle-swift",
            "--configuration",
            "debug",
            "--product",
            "LauncherCoreChecks",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>(),
    ];
    tool(root, lease, &commands[0])?;
    let archive = root.join(format!(
        ".build/oracle-core/{target}/debug/librustcast_core.a"
    ));
    let compatibility_archive = root.join(".build/rust/release/librustcast_core.a");
    data_copy(root, &archive, &compatibility_archive)?;
    // SwiftPM does not track the externally linked Rust archive. A distinct
    // content-addressed scratch tree makes stale linkage impossible without
    // deleting or adopting another cache.
    commands[1][3] = format!(".build/oracle-swift/{}", digest(&archive)?);
    tool(root, lease, &commands[1])?;
    let bin_path = tool(
        root,
        lease,
        &[
            "swift".into(),
            "build".into(),
            "--scratch-path".into(),
            commands[1][3].clone(),
            "--configuration".into(),
            "debug".into(),
            "--show-bin-path".into(),
        ],
    )?;
    let bin_path = PathBuf::from(bin_path.trim());
    if !bin_path.is_absolute()
        || !bin_path.starts_with(root.join(".build/oracle-swift"))
        || bin_path.components().any(|part| {
            !matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
    {
        return Err("Swift oracle build reported an unexpected artifact path.".into());
    }
    let modulemap = bin_path.join("CRuycastCore.build/module.modulemap");
    let swift_compatibility = swift_compatibility(root, &root.join(&commands[1][3]), &bin_path, &target)?;
    let compatibility_modulemap = root.join(&swift_compatibility.modulemap);
    data_copy(root, &modulemap, &compatibility_modulemap)?;
    let binary = bin_path.join("LauncherCoreChecks");
    regular_file(&binary)?;
    tool(
        root,
        lease,
        &[
            "python3".into(),
            "rust-native/scripts/check_rust_only.py".into(),
        ],
    )?;
    let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".build/oracle-fixtures"));
    if !parent.is_absolute() {
        return Err("Oracle fixture parent must be absolute.".into());
    }
    let fixture = preparation::Stage::in_parent(&parent, "core-oracle")?;
    let mut command = Command::new(&binary);
    command
        .current_dir(root)
        .env_clear()
        .env("HOME", fixture.path())
        .env("TMPDIR", fixture.path());
    lease.apply_to_child(&mut command)?;
    let value = process::capture(command, std::time::Duration::from_secs(60), 1024 * 1024)?;
    ruycast_native::private_storage::write(
        &root.join(".build/receipts/pure-core.stdout"),
        &value.stdout,
        true,
    )?;
    ruycast_native::private_storage::write(
        &root.join(".build/receipts/pure-core.stderr"),
        &value.stderr,
        true,
    )?;
    if !value.status.success()
        || String::from_utf8(value.stdout)
            .map_err(error)?
            .lines()
            .collect::<Vec<_>>()
            != [
                "Script choice search and validation checks passed",
                "Snippet output checks passed",
                "Natural date checks passed",
                "Core behavior checks passed",
            ]
    {
        return Err("Historical data-only CoreChecks failed.".into());
    }
    if core_sources(root)? != before {
        return Err("Pure oracle source changed; no receipt published.".into());
    }
    let mut artifacts = BTreeMap::new();
    for path in [
        &archive,
        &compatibility_archive,
        &modulemap,
        &compatibility_modulemap,
        &binary,
    ] {
        artifacts.insert(
            path.strip_prefix(root)
                .map_err(error)?
                .to_str()
                .ok_or("Invalid oracle artifact path.")?
                .into(),
            digest(path)?,
        );
    }
    verify_swift_compatibility(root, &swift_compatibility, &artifacts)?;
    let receipt=CoreReceipt{schema:"ruycast.pure-core-oracle.v2".into(),scope:"Frozen LauncherCoreChecks: Foundation data/owned filesystem and actual linked Rust ABI; no RuycastApp/AppKit/startup/desktop claim".into(),sources:before,artifacts,commands,outcome:"Core behavior checks passed".into(),production_accepted:false,swift_compatibility};
    lease.validate()?;
    let path = root.join(".build/receipts/pure-core-oracle.json");
    ruycast_native::private_storage::write(
        &path,
        &serde_json::to_vec_pretty(&receipt).map_err(error)?,
        true,
    )?;
    verify_core(root, lease)
}
pub(super) fn qualify_hid_reference(
    root: &Path,
    binaries: &serde_json::Value,
    raw: &[u8],
) -> Result<serde_json::Value> {
    const ALIAS: &str = "rust-native/target/debug/hid-state-probe";
    hid_state_goldens::parse(&regular_bytes(&root.join(hid_state_goldens::PATH))?)?;
    let mut value: serde_json::Value = serde_json::from_slice(raw).map_err(error)?;
    if value["schema"] != "ruycast.hid-state-golden-binding.v1"
        || value["goldenSHA256"] != hid_state_goldens::SHA256
        || value["positiveCases"] != 4623
        || value["observedSnapshots"] != 10940
        || value["firstOperationRefusals"] != 4
        || value.get("hidGoldenQualification").is_some()
    {
        return Err("Incomplete or already-qualified HID golden receipt.".into());
    }
    let sources = value["sources"]
        .as_object()
        .ok_or("Missing HID source bindings.")?;
    if sources.len() != hid_state_goldens::SOURCES.len() {
        return Err("HID source roster changed.".into());
    }
    for name in hid_state_goldens::SOURCES {
        if sources.get(*name).and_then(serde_json::Value::as_str)
            != Some(digest(&root.join(name))?.as_str())
        {
            return Err("HID golden/producer source changed.".into());
        }
    }
    let expected = binaries
        .get(ALIAS)
        .and_then(serde_json::Value::as_str)
        .ok_or("Missing actual HID probe producer.")?;
    let probe = value["probe"]
        .as_str()
        .ok_or("Missing live HID probe binding.")?;
    if Path::new(probe).is_absolute()
        || Path::new(probe)
            .components()
            .any(|p| !matches!(p, std::path::Component::Normal(_)))
    {
        return Err("HID probe escaped repository.".into());
    }
    let artifacts = value["artifacts"]
        .as_object()
        .ok_or("Missing HID probe artifact.")?;
    if artifacts.len() != 1
        || artifacts.get(probe).and_then(serde_json::Value::as_str) != Some(expected)
        || digest(&root.join(ALIAS))? != expected
        || digest(&root.join(probe))? != expected
    {
        return Err("HID probe copy differs from its actual producer.".into());
    }
    value["hidGoldenQualification"] = serde_json::json!({
        "goldenSHA256":hid_state_goldens::SHA256,
        "probeProducer":ALIAS,
        "probeSHA256":expected,
        "rawBindingSHA256":format!("{:x}",Sha256::digest(raw))
    });
    Ok(value)
}
pub(super) fn verify_core(root: &Path, lease: &BuildLock) -> Result<serde_json::Value> {
    lease.validate()?;
    let receipt: CoreReceipt = serde_json::from_slice(&regular_bytes(
        &root.join(".build/receipts/pure-core-oracle.json"),
    )?)
    .map_err(error)?;
    if receipt.schema != "ruycast.pure-core-oracle.v2"
        || receipt.production_accepted
        || receipt.sources != core_sources(root)?
        || receipt.artifacts.len() != 5
        || receipt.outcome != "Core behavior checks passed"
    {
        return Err("Pure CoreChecks oracle receipt is invalid or stale.".into());
    }
    verify_swift_compatibility(root, &receipt.swift_compatibility, &receipt.artifacts)?;
    for (name, expected) in &receipt.artifacts {
        if Path::new(name).is_absolute()
            || Path::new(name)
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || digest(&root.join(name))? != *expected
        {
            return Err("Pure oracle artifact changed.".into());
        }
    }
    serde_json::to_value(receipt).map_err(error)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsGate {
    id: &'static str,
    original_source: &'static str,
    required_native_steps: &'static [&'static str],
    native_cases: &'static [&'static str],
    unproven_invariants: &'static [&'static str],
}
// These are case mappings, not pass receipts. The normal pipeline still runs
// and verifies the complete source-bound native matrix after qualification.
const SETTINGS: &[SettingsGate] = &[
    SettingsGate { id:"--check-edit-commands", original_source:"Sources/RuycastApp/EditCommandChecks.swift", required_native_steps:&["native-light-600","native-light-760","native-dark-600","native-dark-760"], native_cases:&["native_edit::check"], unproven_invariants:&[] },
    SettingsGate { id:"--check-settings", original_source:"Sources/RuycastApp/SettingsChecks.swift", required_native_steps:&[
        "core-unit", "unit", "settings-preferences-process", "settings-keyboard-process",
        "library-process", "library-checkbox-process", "library-drafts-process", "library-navigation-process", "actions-process", "native-keyboard-process", "hid-process",
        "keyword-process", "applications-discovery-process", "app-picker-process", "capture-process", "bridge-process", "clipboard-history-process", "startup-process",
        "feedback-actions-process", "snippet-links-process", "snippet-process", "snippet-board-process",
        "keyword-rich-process", "clipboard-rich-paste", "settings-ui-process", "snippet-html-import",
    ], native_cases:&[
        "ui_completion::settings_parity", "ui_completion::use_either_command",
        "snippet_probe::check_html_import", "rustcast_core::tests::command_sides_are_independent",
        "hotkeys::tests::plan_preserves_carbon_flags_sided_policy_modifier_taps_and_disabled_items",
        "recorder::tests::physical_keys_sides_hyper_and_clear_match_shipped_contract",
        "runtime_health::tests::actual_hook_and_keyword_state_are_required_without_fabricating_permissions",
        "profile::tests::post_retention_snapshot_conflicts_refuse_reload_save_and_reconcile_commit",
        "startup_process::settings_rejected_registration_preserves_four_clips_bytes_and_old_callback",
        "startup_process::settings_clipboard_write_refusal_preserves_old_config_and_active_callback",
        "startup_process::settings_unchanged_reload_retries_repaired_clipboard_cleanup_immediately",
        "startup_process::settings_rejected_image_retention_preserves_asset_then_disabled_paused_acceptance_prunes_it",
        "startup_process::settings_original_sided_filter_preserves_both_routes_repeat_release_and_modifier_boundaries",
        "startup_process::settings_disabled_dryrun_and_discovered_identity_cannot_bypass_saved_policy",
        "startup_process::settings_reenable_conflict_then_alias_preservation_and_retention_fourteen_reload",
        "settings_preferences_process::general_actual_controls_write_config_preserve_unknown_fields_and_themes",
        "settings_preferences_process::record_both_shortcuts_local_events_cancel_and_navigation_release",
        "settings_preferences_process::advanced_reload_open_permissions_and_fake_folder_cancel_stale_guard",
        "settings_preferences_process::menu_bar_hidden_substitution_live_visibility_and_retired_callbacks",
        "settings_keyboard_process::general_recorder_uses_shared_lease_and_cannot_resume_peer_recorder",
        "settings_keyboard_process::general_recorded_hotkey_replaces_physical_plan_through_config_staging",
        "library_lifecycle_process::toolbar_add_is_private_until_save_then_shared_with_palette",
        "library_lifecycle_process::invalid_config_keeps_both_last_good_views_and_refuses_draft_save",
        "library_lifecycle_process::theme_and_application_resign_update_both_windows",
        "library_lifecycle_process::hidden_settings_callbacks_do_not_change_navigation_or_cancel_palette",
        "library_lifecycle_process::normal_quit_with_both_windows_and_drafts_releases_socket",
        "library_checkbox_process::inline_enabled_checkbox_targets_unselected_row_and_updates_palette",
        "library_checkbox_process::first_checkbox_click_after_same_results_query_and_reopen_uses_current_binding",
        "library_checkbox_process::old_checkbox_cannot_write_after_query_reload_close_or_editor_open",
        "library_checkbox_process::inline_checkbox_failure_keeps_published_value_and_allows_retry",
        "library_checkbox_process::inline_enable_does_not_discard_a_palette_draft",
        "library_drafts_process::different_window_drafts_merge_and_keep_each_others_focus",
        "library_drafts_process::same_field_conflict_retains_second_draft",
        "library_drafts_process::cmd_w_closes_only_settings_and_discards_only_its_draft",
        "library_drafts_process::repeated_show_preserves_actual_alias_and_name_editor_focus",
        "library_drafts_process::cmd_w_is_recorded_before_window_close_and_repeated_show_keeps_recording",
        "library_navigation_process::cli_settings_is_separate_and_reusable_without_showing_a_window",
        "library_navigation_process::disabled_items_can_be_found_and_enabled_without_execution",
        "library_navigation_process::tabs_groups_filters_and_query_reset_keep_all_saved_items",
        "library_navigation_process::library_has_no_launcher_hundred_item_cap",
        "actions_process::actions_keyboard_badges_and_escape_keeps_palette",
        "actions_process::alias_save_clear_and_cli_resolution", "actions_process::alias_conflict_keeps_draft_and_allows_repair",
        "actions_process::failed_async_save_restores_native_field_focus_and_undo", "actions_process::other_external_fields_merge_but_same_alias_change_conflicts",
        "actions_process::deleted_item_and_invalid_schema_are_not_overwritten", "actions_process::hotkey_sides_hyper_delete_and_repeat",
        "actions_process::hotkey_conflict_and_solo_modifiers", "actions_process::recorder_stops_on_window_app_resign_unfocus_hide_and_cancel",
        "actions_process::alias_native_composition_does_not_save_or_cancel", "actions_process::remove_confirmation_cancel_then_commit_and_disable",
        "actions_process::visible_selection_is_bound_and_unseen_selection_is_refused", "actions_process::busy_config_lock_keeps_draft_and_retries",
        "actions_process::queued_save_cancel_and_hide_prevent_dispatch_and_zombie_editor", "actions_process::queued_menu_return_cancel_does_not_open_editor",
        "actions_process::forms_and_actions_render_with_long_titles_in_light_and_dark",
        "native_keyboard_process::native_callback_toggles_launcher_and_two_queued_presses_preserve_toggle_order",
        "native_keyboard_process::item_callback_uses_exact_command_not_selected_row_and_logs_hotkey_without_showing_palette",
        "native_keyboard_process::hotkey_snippet_failure_keeps_exact_analytics_source_before_missing_sdk_refusal",
        "native_keyboard_process::reload_retires_old_ids_and_disabled_bindings",
        "native_keyboard_process::actual_recording_waits_for_physical_unregistration_and_restores_accepted_plan",
        "native_keyboard_process::two_window_recorders_do_not_resume_on_other_window_cancel",
        "native_keyboard_process::headless_hidden_does_not_enable_keyboard_without_explicit_optin",
        "native_keyboard_process::missing_owned_sdk_stamp_fails_before_any_global_target_or_handler",
        "native_keyboard_process::positive_held_main_registration_keeps_owner_responsive_and_profile_locked_until_retirement",
        "native_keyboard_process::launcher_from_hotkey_report_returns_to_commands_before_toggle_hides",
        "native_keyboard_process::failed_registration_does_not_publish_editor_save_and_old_routes_survive",
        "native_keyboard_process::pending_recorder_consumes_save_keys_and_cancels_during_positive_physical_hold",
        "native_keyboard_process::callback_during_commit_before_main_ack_is_deferred_until_exact_store_adoption",
        "native_keyboard_process::recorder_begin_reenters_registration_blocks_immediately_and_retires_new_handle",
        "native_keyboard_process::current_native_copy_and_recorder_error_renders_in_both_themes",
        "snippet_process::plain_unknown_modes_and_complete_literal_text",
        "snippet_process::teams_paragraphs_html_breaks_and_safe_escaping",
        "snippet_process::links_balanced_parentheses_escape_labels_and_invalid_syntax",
        "snippet_process::url_validation_and_link_source_preserve_actual_bridge_behavior",
        "snippet_process::utf16_cursor_mapping_at_every_position_and_grapheme_boundary",
        "snippet_process::native_named_board_contains_one_complete_plain_or_rich_item",
        "snippet_process::grapheme_suffix_count_across_scripts_controls_and_joiners",
        "snippet_process::real_sdk_item_failure_keeps_previous_board_but_write_failure_is_partial",
        "snippet_process::large_source_is_complete_without_an_application_quota",
        "snippet_process::generated_unicode_and_markup_corpus",
        "snippet_process::invalid_long_url_does_not_repeat_legacy_exponential_match",
        "keyword_rich_process.KeywordRich",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-interactions", original_source:"Sources/RuycastApp/InteractionChecks.swift", required_native_steps:&["bridge-process","native-profile-process","input-process-rust","native-choice-host-process","native-open-basic-process","creation-process","library-process","settings-preferences-process","feedback-actions-process","native-snippet-choice-process","native-light-600","native-light-760","native-dark-600","native-dark-760"], native_cases:&[
        "bridge_process::show_search_and_local_native_query_share_the_owner",
        "bridge_process::rapid_arrows_and_click_on_other_row_keep_intended_identity",
        "input_process::expand_preserves_actual_field_editor_text_and_token_without_execution",
        "input_process::marked_text_blocks_run_and_escape_without_effects",
        "native_open_basic_process::saved_app_hides_exact_panel_before_sdk_and_records_success",
        "creation_process::external_changes_survive_and_identity_collision_never_overwrites",
        "library_lifecycle_process::toolbar_add_is_private_until_save_then_shared_with_palette",
        "settings_preferences_process::general_actual_controls_write_config_preserve_unknown_fields_and_themes",
        "native_choice_host_process::live_trigger_uses_original_anchor_and_single_use_replacement",
        "native_choice_host_process::changed_anchor_and_secure_policy_fail_closed",
        "ui_completion::feedback_actions",
        "native_edit::check_snippet_replacement",
        "native_snippet_choice_process.NativeSnippetChoice.test_option_selection_and_arrow_clamping_without_opening_os_menu",
        "native_snippet_choice_process.NativeSnippetChoice.test_repeat_modifiers_and_marked_text_do_not_submit",
        "native_snippet_choice_process.NativeSnippetChoice.test_cancel_and_external_hide_have_distinct_restore_policy",
        "native_open_basic_process::new_show_after_sdk_entry_cannot_be_hidden_or_receive_old_failure",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-actions-menu", original_source:"Sources/RuycastApp/ActionMenuChecks.swift", required_native_steps:&["actions-process","actions-menu-process","input-process-rust","feedback-actions-process","snippet-hotkey-process","native-keyboard-process","polish-original-process","item-binding-original-process"], native_cases:&[
        "ui_completion::actions_menu",
        "ui_completion::feedback_actions",
        "ui_completion::snippet_hotkey",
        "ui_completion::polish_original",
        "actions_process::alias_save_clear_and_cli_resolution",
        "actions_process::hotkey_sides_hyper_delete_and_repeat",
        "actions_process::hotkey_conflict_and_solo_modifiers",
        "actions_process::recorder_stops_on_window_app_resign_unfocus_hide_and_cancel",
        "input_process::alias_and_hotkey_saves_resume_same_input",
        "native_keyboard_process::pending_recorder_consumes_save_keys_and_cancels_during_positive_physical_hold",
        "ui_completion::item_binding_original",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-browser-tabs", original_source:"Sources/RuycastApp/BrowserTabChecks.swift", required_native_steps:&["unit","system-search-cli-process","system-search-sdk"], native_cases:&[
        "system_search::tests::safari_position_snapshot_never_falls_back_to_matching_title_or_url",
        "system_search::tests::safari_original_snapshot_mutations_fail_before_the_next_effect",
        "system_search::tests::safari_selection_mismatch_stops_before_foreground_effect",
        "system_search::tests::safari_short_cache_reuses_tokens_and_case_insensitive_url_filter_without_reads",
        "system_search::tests::slow_safari_discovery_does_not_extend_snapshot_cache_lifetime",
        "system_search::tests::safari_missing_empty_and_original_collection_and_metadata_bounds",
        "system_search::providers::gate_tests::safari_thirty_second_target_expiry_refuses_before_any_native_read_or_effect",
        "system_search::tests::root_and_dedicated_tokens_cannot_cross_stores_or_retarget_old_results",
        "system_search::tests::optional_main_tab_caps_preserve_metadata_and_dedicated_results",
        "system_search::tests::safari_cancelled_consent_never_reaches_the_owned_permission_effect",
        "runtime::system::shortcut_budget_tests::browser_expired_queued_activation_never_reaches_main_queue_eligible_or_effect",
        "runtime::system::shortcut_budget_tests::browser_busy_main_cli_preserves_local_results_and_exactly_one_response",
        "runtime::system::shortcut_budget_tests::browser_dedicated_busy_and_expired_cli_requests_remain_errors",
        "runtime::system::shortcut_budget_tests::browser_eight_pending_cli_requests_do_not_block_live_local_root_results",
        "runtime::system::shortcut_budget_tests::browser_late_main_provider_preserves_navigation_without_retargeting_actions",
        "runtime::system::shortcut_budget_tests::browser_late_main_provider_never_crosses_hide_or_mode_change",
        "runtime::system::shortcut_budget_tests::browser_superseded_query_and_old_deadline_cannot_cancel_or_republish_latest_rows",
        "runtime::system::shortcut_budget_tests::browser_main_missing_denied_throttle_cache_and_explicit_refresh_never_prompt",
        "runtime::system::shortcut_budget_tests::browser_safari_send_crossing_the_real_deadline_stops_before_foreground_effects",
        "runtime::system::shortcut_budget_tests::held_consent_navigation_refreshes_without_resetting_the_selected_row",
        "runtime::system::shortcut_budget_tests::held_consent_default_and_closed_actions_choose_the_new_default_tab",
        "runtime::system::shortcut_budget_tests::held_consent_down_up_and_direct_selection_preserve_user_navigation",
        "runtime::system::shortcut_budget_tests::held_consent_current_query_is_refreshed_once_without_old_query_rows",
        "runtime::system::shortcut_budget_tests::held_consent_rejects_current_interaction_hide_mode_and_configuration_changes",
        "runtime::system::shortcut_budget_tests::held_denied_consent_refreshes_local_results_once_without_granting_access",
        "system_search_sdk::browser_original_native_return_and_forty_eight_hidden_variants_are_owned",
        "system_search_sdk::direct_sdk_transport_is_fully_owned",
        "system_search_cli_process::cli_real_provider_routes_metadata_order_and_exact_effects",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-apple-shortcuts", original_source:"Sources/RuycastApp/AppleShortcutChecks.swift", required_native_steps:&["unit","system-search-cli-process","system-search-sdk","private-results-process","script-process","script-result-process","script-result-native-process"], native_cases:&[
        "system_search::tests::shortcut_uuid_parser_overrides_and_execution_tuple_are_exact",
        "system_search::tests::owned_shortcut_children_drain_both_pipes_and_never_wait_for_inherited_eof",
        "system_search::tests::generated_uuid_and_builtin_bindings_persist_without_resurrecting_deleted_saved_rows",
        "system_search::providers::gate_tests::shortcut_discovery_cache_hit_thirty_one_second_expiry_and_explicit_refresh",
        "system_search::apple_shortcuts::tests::workflow_output_and_private_failures_never_become_ui_or_public_error_data",
        "system_search::apple_shortcuts::tests::elapsed_capture_deadline_reports_unknown_result_without_reinterpreting_output",
        "system_search_sdk::shortcuts_original_native_return_and_twenty_four_hidden_states_keep_uuid_identity",
        "system_search_cli_process::cli_shortcut_uuid_duplicate_titles_bindings_and_private_output",
        "system_search_cli_process::native_shortcut_alias_override_and_reopen",
        "script_result_basic_process::cli_preserves_choices_detail_unknown_fields_and_success_usage",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-window-search", original_source:"Sources/RuycastApp/WindowSearchChecks.swift", required_native_steps:&["unit","system-search-cli-process","system-search-sdk"], native_cases:&[
        "system_search::tests::window_identity_duplicate_titles_and_minimized_restore_are_exact",
        "system_search::tests::window_post_activation_pid_role_title_membership_and_restore_state_are_rechecked",
        "system_search::tests::window_unknown_minimized_untitled_and_old_snapshot_metadata_are_preserved",
        "system_search::tests::window_invalid_identity_and_unreadable_membership_refuse_before_activation",
        "system_search::tests::window_permission_and_app_exit_during_activation_stop_before_raise",
        "system_search::tests::window_restore_status_readback_and_membership_are_verified_before_raise",
        "system_search::tests::window_raise_error_is_not_reported_as_success",
        "system_search::tests::window_scanner_caps_two_hundred_apps_fifty_windows_and_one_hundred_results",
        "system_search::tests::window_internal_two_second_budget_returns_partial_while_request_is_live",
        "system_search::tests::window_internal_two_second_budget_can_return_empty_without_cancelling_request",
        "system_search::tests::window_original_application_and_title_filters_preserve_distinct_duplicate_targets",
        "system_search::tests::absolute_deadline_and_cancellation_stop_traversal_and_every_effect",
        "system_search::tests::result_caps_and_ax_permission_recovery_are_bounded",
        "system_search::tests::scan_platform_error_is_not_reclassified_as_internal_budget_exhaustion",
        "runtime::system::shortcut_budget_tests::ax_request_budget_windows_delivers_original_internal_partial_results",
        "system_search_sdk::window_original_native_return_and_twenty_four_hidden_states_preserve_durable_choices",
        "system_search_sdk::direct_sdk_transport_is_fully_owned",
        "system_search_cli_process::cli_real_provider_routes_metadata_order_and_exact_effects",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-menu-actions", original_source:"Sources/RuycastApp/MenuActionChecks.swift", required_native_steps:&["unit","system-search-cli-process","system-search-sdk"], native_cases:&[
        "system_search::tests::app_shortcuts_decode_without_registering_launcher_hotkeys",
        "system_search::tests::menus_disabled_unknown_original_app_and_shortcut_rebuild_fail_closed",
        "system_search::tests::detached_menu_path_is_never_replaced_by_a_same_title_item",
        "system_search::tests::menu_unknown_enabled_and_unreadable_submenu_never_offer_an_executable_leaf",
        "system_search::tests::menu_original_identity_must_match_the_complete_captured_identity",
        "system_search::tests::menu_post_activation_item_ancestor_identity_and_trust_are_revalidated",
        "system_search::tests::menu_scanner_caps_results_nodes_and_depth_without_visiting_hidden_tail",
        "system_search::tests::menu_nonmatching_scan_preserves_original_attribute_read_budget",
        "system_search::tests::menu_internal_two_second_budget_stops_while_request_is_live",
        "system_search::tests::menu_node_cap_must_not_turn_a_submenu_into_a_leaf",
        "system_search::tests::scan_platform_error_is_not_reclassified_as_internal_budget_exhaustion",
        "system_search::tests::menu_initial_identity_validation_precedes_the_internal_scan_budget",
        "system_search::tests::menu_original_four_items_duplicate_targets_queries_and_immutable_metadata_are_exact",
        "system_search::tests::menu_internal_two_second_budget_returns_empty_after_entered_reads_while_request_is_live",
        "system_search::tests::absolute_deadline_and_cancellation_stop_traversal_and_every_effect",
        "system_search::tests::result_caps_and_ax_permission_recovery_are_bounded",
        "system_search::native::tests::owned_cf_menu_attribute_consumers_refuse_wrong_types_and_preserve_values",
        "runtime::system::shortcut_budget_tests::ax_request_budget_menus_delivers_original_internal_partial_results",
        "runtime::system::shortcut_budget_tests::ax_request_budget_menu_identity_does_not_spend_private_traversal_clock",
        "system_search_sdk::menu_original_native_return_and_twenty_four_hidden_states_preserve_captured_actions",
        "system_search_sdk::direct_sdk_transport_is_fully_owned",
        "system_search_cli_process::cli_real_provider_routes_metadata_order_and_exact_effects",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-render-updates", original_source:"Sources/RuycastApp/InteractionChecks.swift", required_native_steps:&["render-updates-process"], native_cases:&[
        "ui_completion::render_updates",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-feedback", original_source:"Sources/RuycastApp/FeedbackChecks.swift", required_native_steps:&["unit","creation-process","input-process-rust","actions-process","feedback-actions-process","utility-process","polish-original-process","system-search-cli-process","remaining-macos-process"], native_cases:&[
        "ui_completion::feedback_actions",
        "ui_completion::polish_original",
        "utility_process::quick_capture_ids_use_native_return_shift_return_ime_and_single_submission_in_both_entries",
        "actions::tests::binding_save_preserves_external_execution_definition",
        "actions_process::remove_confirmation_cancel_then_commit_and_disable",
        "input_process::alias_and_hotkey_saves_resume_same_input",
        "processes::tests::matching_self_and_foreign_uid_identities_refuse_before_termination",
        "system_search::providers::gate_tests::cross_kind_and_missing_opaque_tokens_refuse_before_native_reads_or_effects",
        "ui_completion::closed_creation",
        "creation_process::empty_results_keep_all_three_creation_actions_available",
        "processes::tests::owned_table_sort_and_immutable_kill_identity",
        "system_search::tests::window_identity_duplicate_titles_and_minimized_restore_are_exact",
        "system_search::tests::menus_disabled_unknown_original_app_and_shortcut_rebuild_fail_closed",
        "system_search::tests::menu_original_identity_must_match_the_complete_captured_identity",
        "system_actions::tests::complete_owned_keyboard_sdk_pairs_and_tags_lock_shortcut",
        "system_actions::tests::denied_and_failed_construction_never_post",
        "file_search::tests::literal_predicate_and_normalization",
        "system_search::tests::main_service_queued_effect_is_revoked_before_native_adapter",
        "system_search_cli_process::superseded_discovery_config_change_and_cli_disconnect_cannot_publish",
        "remaining_macos_process::real_cli_window_files_processes_and_monitor_data_use_owned_backends",
        "remaining_macos_process::native_process_actions_remain_ephemeral_and_direct_kill_is_once_in_both_themes",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-rich-resources", original_source:"Sources/RuycastApp/RichResourceChecks.swift", required_native_steps:&["unit","clipboard-preview-original","remaining-macos-process","clipboard-history-process","clipboard-rich-paste"], native_cases:&[
        "remaining_macos_process::real_cli_window_files_processes_and_monitor_data_use_owned_backends",
        "remaining_macos_process::native_process_actions_remain_ephemeral_and_direct_kill_is_once_in_both_themes",
        "remaining_macos_process::native_process_sort_refresh_keeps_identity_and_kill_keeps_palette_open",
        "remaining_macos_process::vanished_native_process_selection_cannot_kill_the_remaining_owned_row",
        "remaining_macos_process::native_clipboard_two_color_image_file_pixels_and_idle_text_are_preserved_in_both_themes",
        "remaining_macos_process::native_monitor_delegate_pause_resume_close_and_reopen_use_actual_callback_route",
        "remaining_macos_process::real_read_only_resource_sample_satisfies_original_bounds_without_serializing_metadata",
        "remaining_macos_process::native_monitor_six_tabs_light_dark_minimum_wide_and_no_ordering",
        "remaining_macos_process::actual_cli_monitor_and_process_query_use_the_owned_running_host",
        "clipboard_preview::tests::original_two_color_image_and_local_file_previews_preserve_pixels_and_refuse_bad_assets",
        "system_search::providers::gate_tests::concurrent_process_queries_keep_each_result_and_metadata_snapshot",
        "system_search::providers::gate_tests::process_result_cap_bounds_opaque_target_registration",
        "processes::tests::owned_table_sort_and_immutable_kill_identity",
        "monitor::lifecycle_tests::visible_only_two_second_sampling_history_bound_and_reopen_preserve_state",
        "clipboard_history_process.ClipboardHistory.test_native_text_image_empty_error_busy_light_dark_narrow_wide",
        "clipboard_rich_paste::file_and_image_paste_complete_owned_ticket_without_recapture",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-analytics", original_source:"Sources/RuycastApp/AnalyticsChecks.swift", required_native_steps:&["unit","analytics-runtime-process","analytics-cast-reference","dashboard-presentation-process","native-dashboard-dialog-process","dashboard-export-process","startup-process"], native_cases:&[
        "analytics::tests::concurrent_original_hourly_counts_bounds_and_aged_retention",
        "analytics::tests::completion_survives_pause_but_reset_retires_old_ticket_and_private_export",
        "analytics::tests::invalid_known_file_is_not_overwritten_until_explicit_reset",
        "analytics::tests::populated_schema_one_upgrade_preserves_every_legacy_field_and_reload",
        "analytics::tests::repeated_dst_hour_keeps_attempt_completion_and_timezone_reload_distinct",
        "analytics::tests::hourly_retention_accepts_2161_prunes_on_advance_and_never_resurrects",
        "startup_process::analytics_cli_pause_privacy_confirmation_reset_and_inflight_epoch",
        "startup_process::analytics_same_command_tracks_actual_cli_launcher_and_owned_carbon_callback",
        "analytics_cast_reference::sample_caps_error_whitelist_and_hashed_raw_utf8_identity",
        "analytics_cast_reference::existing_document_rejection_and_date_codec_match_foundation",
        "dashboard_presentation_reference::populated_top30_hash_ties_names_samples_errors_and_legacy",
        "dashboard_presentation_reference::elapsed_hours_retention_dst_and_calendar_year",
        "native_dashboard_dialog_process::populated_paused_errors_confirmation_roles_values_and_local_key_loop",
        "native_dashboard_dialog_process::owned_save_dialog_takes_current_data_and_cancel_writes_nothing",
        "native_dashboard_dialog_process::actual_dashboard_renders_themes_widths_and_preserves_scroll",
        "native_dashboard_dialog_process::standard_builtin_cli_native_sources_usage_logs_and_override",
        "native_dashboard_dialog_process::builtin_input_validation_and_native_prompt_do_not_log_private_values",
        "native_dashboard_dialog_process::management_routing_controls_custom_retention_and_reset",
        "native_dashboard_dialog_process::repeated_confirmation_retires_old_queued_reset_callback",
        "native_dashboard_dialog_process::retired_controls_and_dialog_cannot_mutate_a_reopened_dashboard",
        "dashboard_export_process::positive_held_export_global_reset_order_and_parent_mode",
        "dashboard_export_process::file_sync_and_rename_failure_preserve_prior_destination",
        "dashboard_export_process::postrename_parent_sync_failure_reports_published_outcome",
        "dashboard_export_process::close_before_commit_revokes_destination_without_feedback_to_reopened_window",
        "dashboard_export_process::commit_entered_before_close_keeps_published_outcome_and_retires_feedback",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-performance-pipeline", original_source:"Sources/RuycastApp/PerformancePipelineChecks.swift", required_native_steps:&["unit","clipboard-history-process","clipboard-rich-paste"], native_cases:&[
        "clipboard_history::tests::capture_admission_is_single_flight_and_lease_drop_reopens_it",
        "configuration_jobs::tests::capture_retention_handshake_preserves_both_orders_and_stops_without_owner_lock",
        "native_services::check_owned_poll",
        "clipboard_rich_paste::file_and_image_paste_complete_owned_ticket_without_recapture",
        "clipboard_history_process.ClipboardHistory.test_capture_text_files_image_and_private_own_activation_filters",
        "clipboard_history_process.ClipboardHistory.test_pause_disable_and_source_exclusion",
        "runtime::inline_tests::clipboard_pipeline_admitted_maintenance_waits_for_held_capture",
        "runtime::inline_tests::clipboard_pipeline_unicode_latest_identity_hidden_mutation_and_reentry_use_real_owner",
        "runtime::inline_tests::clipboard_hidden_publication_prunes_ids_and_refreshes_payload_without_searching",
        "runtime::inline_tests::clipboard_query_and_data_change_reset_selection_before_the_same_publication",
        "runtime::inline_tests::clipboard_pipeline_observed_atomic_inplace_invalid_and_identical_recovery_reach_owner_views",
        "runtime::inline_tests::clipboard_pipeline_twenty_unrelated_atomic_writes_do_not_notify_configuration_owner",
        "clipboard_history::tests::accepted_image_reopens_with_identical_metadata_disk_and_decodable_asset",
        "clipboard_history::tests::capture_cleanup_failure_keeps_published_text_and_reports_error",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-visual-assets", original_source:"Sources/RuycastApp/VisualAssetChecks.swift", required_native_steps:&["unit","native-profile-process"], native_cases:&[
        "presentation::icon_tests::original_compact_keycaps_and_sided_accessibility_labels",
        "website_icons::tests::origin_and_candidate_privacy",
        "website_icons::tests::actual_transport_stream_bounds_status_cookies_and_redirects",
        "website_icons::tests::four_workers_cache_publication_and_cancel_join",
        "website_icons::tests::two_thousand_origins_bound_admission_and_deduplicate_while_four_workers_wait",
        "website_icons::tests::cache_special_files_and_close_are_bounded",
        "website_icons::tests::active_download_close_cancels_and_joins_without_cache_publication",
        "native_rows::check_cache",
        "native_process.NativeProfiles.test_loaded_config_alias_unicode_disabled_and_native_labels",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-command-log", original_source:"Sources/RuycastApp/CommandLogChecks.swift", required_native_steps:&["unit","private-results-process"], native_cases:&[
        "activity::tests::credential_patterns_input_and_home_are_redacted",
        "activity::tests::load_prunes_retention_and_whitelists_unknown_payloads",
        "activity::tests::malformed_optional_types_preserve_existing_log_until_clear",
        "activity::tests::existing_log_is_restricted_before_decoding_and_unqualified_files_stay_untouched",
        "activity::tests::original_log_bounds_filters_reload_and_clear_contract",
        "activity::tests::insecure_or_linked_profile_refuses_log_read_and_never_repairs_its_mode",
        "activity::tests::response_metadata_redacts_inputs_credentials_urls_home_and_unlisted_details",
        "activity::tests::response_metadata_respects_utf8_limits_and_success_never_records_stdout_or_error",
        "activity::tests::complete_metadata_budget_survives_save_reopen_and_retains_newest_entries",
        "private_results::provider_failure_keeps_sanitized_code_message_exit_and_queryable_trace",
        "private_results::synchronous_management_failure_log_keeps_message_and_trace",
        "private_results::synchronous_history_failure_log_keeps_message_and_trace",
        "runtime::system::shortcut_budget_tests::system_action_error_log_retains_worker_message_and_response_trace",
    ], unproven_invariants:&[] },
    SettingsGate { id:"--check-polish", original_source:"Sources/RuycastApp/PolishChecks.swift", required_native_steps:&["unit","polish-original-process","script-advanced-process","task-date-process","polish-bindings-process","remaining-macos-process","clipboard-preview-cancel-original"], native_cases:&[
        "ui_completion::polish_original",
        "ui_completion::script_advanced",
        "ui_completion::task_date",
        "ui_completion::polish_bindings",
        "native_menu_bar::tests::genuine_health_images_preserve_template_state_and_accessible_description",
        "runtime_health::tests::actual_hook_and_keyword_state_are_required_without_fabricating_permissions",
        "management_commands::catalog_tests::original_nine_defaults_keep_source_order_and_saved_overrides",
        "clipboard_preview::tests::cancelled_same_id_reopen_cannot_publish_old_error_or_asset",
        "remaining_macos_process::native_process_actions_remain_ephemeral_and_direct_kill_is_once_in_both_themes",
        "task_date::tests::original_natural_date_vectors_and_labels_match",
    ], unproven_invariants:&[] },
];
const LOG_SOURCE_SHA256: &str = "e8b4b609655d7946bbb27f287aaa739ed1530133f22a7cfdca2d5003655aa462";
const LOG_ORACLE_SHA256: &str = "9dd2ff68552ebcae0979530623309cafeb7a145a193444dc55a717e94a953455";
fn settings_sources(id: &str) -> &'static [(&'static str, &'static [u8], &'static str)] {
    match id {
        "--check-polish" => &[
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "a0e361eccac5eb167669841d4945deacf9803a1d87e06147ae2bc5a5512a4909"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "0cf0eb9765919ea4142fe4436df347a9ed7215663dfa699d218a970e39c1569c"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "c00bfde67fef22f13fc2f1aebf80617ffaf4609d9d788dd9f14520fcd579a84a"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "b1cb059ffc82fe259c1866957f98042272fc09bb877fe8154ff4637c474feccc"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "174d38c6cb1a3b6b96a45405eea6dd805b0c74899ae1230c902b3d85e7ae3e7e"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "7d93fd7492c92ff2c031a0879e3e5f265849981b70480fa8be4dc49be6217407"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "456a2d80b8a29c7902b023211b31bd9eddc56311e8468a177f0044060a44b895"),
            ("rust-native/src/native_clipboard.rs", include_bytes!("../../native_clipboard.rs"), "6a3db88c4184426dae72431087af17177caa1f7bb3f2cff1bc2c4d7ed621c62f"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "81562c61db965a8d14b14272973b4670fceef588834642d17e6861c99498a801"),
            ("rust-native/src/native_settings.rs", include_bytes!("../../native_settings.rs"), "5d662f5ad70e3be4407acf74babbf97345ac0b05079fcdd88069181d63a23041"),
            ("rust-native/src/native_snippet_editor.rs", include_bytes!("../../native_snippet_editor.rs"), "554be26da4356ff39640a2d4af5b6b4758062dcc782d5da129bfb6b85fb12671"),
            ("rust-native/src/script_result.rs", include_bytes!("../../script_result.rs"), "86cb6b6479e795e577f8dbd2b3e67884c4ab2ac03db1f2e615d0af08bdeb50a1"),
            ("rust-native/src/task_date.rs", include_bytes!("../../task_date.rs"), "ebc3df3a84c149af7f6fd6bcabea58073ea1209204e01b3a14571c3f393db856"),
            ("rust-native/src/runtime_health.rs", include_bytes!("../../runtime_health.rs"), "8c9964bcad7bda0bd6f52a2921b1e7d646c39545aa6a48bc7383138e1b33bdfb"),
            ("rust-native/src/native_menu_bar.rs", include_bytes!("../../native_menu_bar.rs"), "6c87e3f883b6f0abfbd96f3df1733a0e889fe347465f27fc55b3b81d3263d9d3"),
            ("rust-native/src/management_commands.rs", include_bytes!("../../management_commands.rs"), "14f68e0678ba748a65fc88a200f3cea59b9c9de8950462c2369ce5c526c75a31"),
            ("rust-native/src/clipboard_preview.rs", include_bytes!("../../clipboard_preview.rs"), "a7d1506387dc9fb6bc9c987d1d2449680ba6b5785bfcd8fdef644b172aada772"),
            ("rust-native/src/clipboard_image.rs", include_bytes!("../../clipboard_image.rs"), "37bc43506344b20c0334bc4737e29e5769d10384a908abd891134293ca1625db"),
            ("rust-native/src/clipboard_history.rs", include_bytes!("../../clipboard_history.rs"), "2b426ebe9c5b49d43de120859277f468c8285de16dc989b7179fc2de56a5862a"),
            ("rust-native/src/processes.rs", include_bytes!("../../processes.rs"), "3dd132e3eaaf7c74c5e73c1cbeb11fc32bee276f88d9fe5764149b8fd3800de5"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "fd42e39f7784f426e34806f43a8a2d7cef46413d26684d5cd6d5cd444c1759b7"),
            ("rust-native/src/catalog.rs", include_bytes!("../../catalog.rs"), "f74230be3234e4c219e3f26a20e3d92f9d8de6c76235e8b45ac7f8b3c5a98cfc"),
            ("rust-native/src/performance_trace.rs", include_bytes!("../../performance_trace.rs"), "f00bf9774d869674d3680ac57c58d7e429b436640f2a5fad61acae5b5e441781"),
            ("rust-native/src/performance_signposts.rs", include_bytes!("../../performance_signposts.rs"), "c0dcaaaf7b427d9363f536d2bc81ff097efc8042e1778e09fc6eac3302df1dcd"),
            ("rust-native/src/control.rs", include_bytes!("../../control.rs"), "3ed188dcb94c9578ed6dc499ec2737be8ed7ad019d4924fabe2d8f0edd6a06ef"),
            ("rust-native/src/lib.rs", include_bytes!("../../lib.rs"), "f6f059088e1000c1828df47924f5748f35d1bb362a74640f4796dd93b1c1a28f"),
            ("rust-native/tests/remaining_macos_process.rs", include_bytes!("../../../tests/remaining_macos_process.rs"), "3fd6aa6c6a3c8321dde7a9f4b85a01045e18aad1e0bfeea89d2a9d5d0f935409"),
            ("rust-native/src/bin/private_results_fixture.rs", include_bytes!("../private_results_fixture.rs"), "eccf15f44528f6449f76a83b04ae39c223e4d852051d606f7db4f64bae68d531"),
            ("Sources/RuycastApp/PolishChecks.swift", include_bytes!("../../../../Sources/RuycastApp/PolishChecks.swift"), "87377916433de1fd22fea0ee92b586e741f635213d2b2472368f1a8ae62111a2"),
        ],
        "--check-feedback" => &[
            ("rust-native/tests/system_search_cli_process.rs", include_bytes!("../../../tests/system_search_cli_process.rs"), "e31a670e462a90ab454e9fd5a4ae8d1568022ef061342d459ffae40464ce42c1"),
            ("rust-native/tests/actions_process.rs", include_bytes!("../../../tests/actions_process.rs"), "3ed819ce552c2e63ac4dc2e5f0001cd69a40f465e40bc003a13c6151c4b4d481"),
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "c9f43c7bfa10aaf3400dbb24bc4dd1c3383343e0569776950abc8b0a20a209f5"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "6cda15594c28aade5b415de9f1f22992febcc1973183053e5e045cfdfcd38e92"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "f003aad7e18fced24ad358e138042bface5d5212a53dee32cb9adacfc3db80f9"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            // Full Feedback plus its nested SystemChecks have scoped case
            // evidence; these source pins are not a host or matrix grant.
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "c00bfde67fef22f13fc2f1aebf80617ffaf4609d9d788dd9f14520fcd579a84a"),
            ("rust-native/src/processes.rs", include_bytes!("../../processes.rs"), "3dd132e3eaaf7c74c5e73c1cbeb11fc32bee276f88d9fe5764149b8fd3800de5"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "a0e361eccac5eb167669841d4945deacf9803a1d87e06147ae2bc5a5512a4909"),
            ("rust-native/tests/creation_process.rs", include_bytes!("../../../tests/creation_process.rs"), "0163bdfdaae3a99825d9a680dca56d9af7cdad280af50b61e5bed89b62859b6f"),
            ("rust-native/tests/input_process.py", include_bytes!("../../../tests/input_process.py"), "470741c414e43fae6bf68e5a2880c9d61c7483d47cfdc562564ed5d8ffec7f56"),
            ("rust-native/tests/input_process.rs", include_bytes!("../../../tests/input_process.rs"), "5669c69dbab3032e16d6e5788fed43e4e930e77b71706c436878f27547c993d5"),
            ("rust-native/tests/library_checkbox_process.rs", include_bytes!("../../../tests/library_checkbox_process.rs"), "5f9c6a687290d3f516d4b08f7c733652e582ee9c10a3dd222c4abdb2e4e80557"),
            ("rust-native/tests/library_drafts_process.rs", include_bytes!("../../../tests/library_drafts_process.rs"), "27676806949e31bb4679db9f2acfcf068fe7229d6deb51d2d6b6f3af2b4ec7b2"),
            ("rust-native/tests/library_navigation_process.rs", include_bytes!("../../../tests/library_navigation_process.rs"), "5c9ae7c66d226e7cb0178c047d36cfabee111b6f4dfd0097f9d39e06b1635186"),
            ("rust-native/tests/library_lifecycle_process.rs", include_bytes!("../../../tests/library_lifecycle_process.rs"), "4f3d417117826cd22f64e233a5c27609402a4a41f5029b1e54ce6de6907c92ad"),
            ("rust-native/tests/settings_preferences_process.rs", include_bytes!("../../../tests/settings_preferences_process.rs"), "94f965cbaa9ac8270d1775d4d1695806e1155e7328be9481602021536510de34"),
            ("rust-native/tests/utility_process.rs", include_bytes!("../../../tests/utility_process.rs"), "eef887ae1eade8bf817d9035ac3ab8baa7faec84eb818b2816bb8b58eada51aa"),
            ("rust-native/tests/bridge_process.py", include_bytes!("../../../tests/bridge_process.py"), "1d6c5f6f8143f9e6f4a642cd8d32e14b4d243baa24f3079fa62b4723fdaffac4"),
            ("rust-native/tests/native_choice_host_process.rs", include_bytes!("../../../tests/native_choice_host_process.rs"), "1eae948c8766cd6c7c006b24aedd3366bdc053209cecfd4ca257f9b8f287be87"),
            ("Sources/RuycastApp/FeedbackChecks.swift", include_bytes!("../../../../Sources/RuycastApp/FeedbackChecks.swift"), "1f91b690d51f82c2c2d6ea653c0f0ec4c06747dc21603472302c30c098cfaa04"),
            ("Sources/RuycastApp/ActionMenuChecks.swift", include_bytes!("../../../../Sources/RuycastApp/ActionMenuChecks.swift"), "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f"),
            ("Sources/RuycastApp/InteractionChecks.swift", include_bytes!("../../../../Sources/RuycastApp/InteractionChecks.swift"), "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858"),
            ("rust-native/src/system_actions.rs", include_bytes!("../../system_actions.rs"), "b97944f7f49e0fee4442084d7ef51dc7788e3f583cdf062283f9d141d5ed1833"),
            ("rust-native/src/file_search.rs", include_bytes!("../../file_search.rs"), "991b29f39c2da3608c17ad8de243a17eb1e7b32b16436b430cdc863e95ad5f28"),
            ("rust-native/src/os_command.rs", include_bytes!("../../os_command.rs"), "1faa996a7c7257702cabcd00bc2ecdb8c6b1e2951e0d24b563693ba062501da2"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/tests.rs", include_bytes!("../../system_search/tests.rs"), "d35042be9f1f88dc038a19e14d0e7c66f685fd0c2069afdfad5c85d1036f6bab"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "47b7823c045903339702225fdf009eb7f0913d4ccde98e5f208263fceea8bec7"),
            ("rust-native/src/system_search/main_bridge.rs", include_bytes!("../../system_search/main_bridge.rs"), "d0267d4a14fbefc08a7c17a50c4e1f9872abb11fe414fe32f7507274552c908e"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "0cf0eb9765919ea4142fe4436df347a9ed7215663dfa699d218a970e39c1569c"),
            ("rust-native/tests/remaining_macos_process.rs", include_bytes!("../../../tests/remaining_macos_process.rs"), "3fd6aa6c6a3c8321dde7a9f4b85a01045e18aad1e0bfeea89d2a9d5d0f935409"),
            ("Sources/RuycastApp/SystemChecks.swift", include_bytes!("../../../../Sources/RuycastApp/SystemChecks.swift"), "c70cc5149a16a43102cc8a853bfb5a38bc6a9b3b542b4f6b7388339f1737ffa4"),
            ("rust-native/tests/support/native_keyboard.rs", include_bytes!("../../../tests/support/native_keyboard.rs"), "76ff8a46006068ede9b84376c3ee0f96293c5efbae60c8a82032dcc6fd89ada3"),
            ("rust-native/tests/native_keyboard_process.rs", include_bytes!("../../../tests/native_keyboard_process.rs"), "651a992fc77ba45e15b55623e0d0e8eaeff964fcfccbf070fdc241e218cce6a2"),
        ],

        "--check-actions-menu" => &[
            ("rust-native/tests/native_keyboard_process.rs", include_bytes!("../../../tests/native_keyboard_process.rs"), "651a992fc77ba45e15b55623e0d0e8eaeff964fcfccbf070fdc241e218cce6a2"),
            ("rust-native/tests/support/native_keyboard.rs", include_bytes!("../../../tests/support/native_keyboard.rs"), "76ff8a46006068ede9b84376c3ee0f96293c5efbae60c8a82032dcc6fd89ada3"),
            ("rust-native/src/bin/private_results_fixture.rs", include_bytes!("../private_results_fixture.rs"), "eccf15f44528f6449f76a83b04ae39c223e4d852051d606f7db4f64bae68d531"),
            ("rust-native/tests/actions_process.rs", include_bytes!("../../../tests/actions_process.rs"), "3ed819ce552c2e63ac4dc2e5f0001cd69a40f465e40bc003a13c6151c4b4d481"),
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "c9f43c7bfa10aaf3400dbb24bc4dd1c3383343e0569776950abc8b0a20a209f5"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "6cda15594c28aade5b415de9f1f22992febcc1973183053e5e045cfdfcd38e92"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "f003aad7e18fced24ad358e138042bface5d5212a53dee32cb9adacfc3db80f9"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            // The final eight binding fields and real queued-recorder ACK are
            // carried by item-binding-original-process, never by Polish alone.
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "c00bfde67fef22f13fc2f1aebf80617ffaf4609d9d788dd9f14520fcd579a84a"),
            ("rust-native/src/processes.rs", include_bytes!("../../processes.rs"), "3dd132e3eaaf7c74c5e73c1cbeb11fc32bee276f88d9fe5764149b8fd3800de5"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "a0e361eccac5eb167669841d4945deacf9803a1d87e06147ae2bc5a5512a4909"),
            ("rust-native/tests/creation_process.rs", include_bytes!("../../../tests/creation_process.rs"), "0163bdfdaae3a99825d9a680dca56d9af7cdad280af50b61e5bed89b62859b6f"),
            ("rust-native/tests/input_process.py", include_bytes!("../../../tests/input_process.py"), "470741c414e43fae6bf68e5a2880c9d61c7483d47cfdc562564ed5d8ffec7f56"),
            ("rust-native/tests/input_process.rs", include_bytes!("../../../tests/input_process.rs"), "5669c69dbab3032e16d6e5788fed43e4e930e77b71706c436878f27547c993d5"),
            ("rust-native/tests/library_checkbox_process.rs", include_bytes!("../../../tests/library_checkbox_process.rs"), "5f9c6a687290d3f516d4b08f7c733652e582ee9c10a3dd222c4abdb2e4e80557"),
            ("rust-native/tests/library_drafts_process.rs", include_bytes!("../../../tests/library_drafts_process.rs"), "27676806949e31bb4679db9f2acfcf068fe7229d6deb51d2d6b6f3af2b4ec7b2"),
            ("rust-native/tests/library_navigation_process.rs", include_bytes!("../../../tests/library_navigation_process.rs"), "5c9ae7c66d226e7cb0178c047d36cfabee111b6f4dfd0097f9d39e06b1635186"),
            ("rust-native/tests/library_lifecycle_process.rs", include_bytes!("../../../tests/library_lifecycle_process.rs"), "4f3d417117826cd22f64e233a5c27609402a4a41f5029b1e54ce6de6907c92ad"),
            ("rust-native/tests/settings_preferences_process.rs", include_bytes!("../../../tests/settings_preferences_process.rs"), "94f965cbaa9ac8270d1775d4d1695806e1155e7328be9481602021536510de34"),
            ("rust-native/tests/utility_process.rs", include_bytes!("../../../tests/utility_process.rs"), "eef887ae1eade8bf817d9035ac3ab8baa7faec84eb818b2816bb8b58eada51aa"),
            ("rust-native/tests/bridge_process.py", include_bytes!("../../../tests/bridge_process.py"), "1d6c5f6f8143f9e6f4a642cd8d32e14b4d243baa24f3079fa62b4723fdaffac4"),
            ("rust-native/tests/native_choice_host_process.rs", include_bytes!("../../../tests/native_choice_host_process.rs"), "1eae948c8766cd6c7c006b24aedd3366bdc053209cecfd4ca257f9b8f287be87"),
            ("Sources/RuycastApp/FeedbackChecks.swift", include_bytes!("../../../../Sources/RuycastApp/FeedbackChecks.swift"), "1f91b690d51f82c2c2d6ea653c0f0ec4c06747dc21603472302c30c098cfaa04"),
            ("Sources/RuycastApp/ActionMenuChecks.swift", include_bytes!("../../../../Sources/RuycastApp/ActionMenuChecks.swift"), "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f"),
            ("Sources/RuycastApp/InteractionChecks.swift", include_bytes!("../../../../Sources/RuycastApp/InteractionChecks.swift"), "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858"),
        ],

        "--check-interactions" => &[
            ("rust-native/tests/native_open_basic_process.rs", include_bytes!("../../../tests/native_open_basic_process.rs"), "629c4befb66bc921ed3b645ee44cc25052374a9a131c53e53ddeab20a72c4c32"),
            ("rust-native/tests/fixtures/workspace_fixture.rs", include_bytes!("../../../tests/fixtures/workspace_fixture.rs"), "893eaba2a1a537958085554a78d7cd69131003c0c705d0a6c5952b16a9f5cb06"),
            ("rust-native/tests/fixtures/workspace_sdk.rs", include_bytes!("../../../tests/fixtures/workspace_sdk.rs"), "750f2ab8558a4ee887d64ad21c4022527019af91ca5390ccf89f23a6c125ef05"),
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "c9f43c7bfa10aaf3400dbb24bc4dd1c3383343e0569776950abc8b0a20a209f5"),
            // Original local-data conjunctions and retained native-choice/open
            // components are qualified separately from full matrix execution.
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "c00bfde67fef22f13fc2f1aebf80617ffaf4609d9d788dd9f14520fcd579a84a"),
            ("rust-native/src/processes.rs", include_bytes!("../../processes.rs"), "3dd132e3eaaf7c74c5e73c1cbeb11fc32bee276f88d9fe5764149b8fd3800de5"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "a0e361eccac5eb167669841d4945deacf9803a1d87e06147ae2bc5a5512a4909"),
            ("rust-native/tests/creation_process.rs", include_bytes!("../../../tests/creation_process.rs"), "0163bdfdaae3a99825d9a680dca56d9af7cdad280af50b61e5bed89b62859b6f"),
            ("rust-native/tests/input_process.py", include_bytes!("../../../tests/input_process.py"), "470741c414e43fae6bf68e5a2880c9d61c7483d47cfdc562564ed5d8ffec7f56"),
            ("rust-native/tests/input_process.rs", include_bytes!("../../../tests/input_process.rs"), "5669c69dbab3032e16d6e5788fed43e4e930e77b71706c436878f27547c993d5"),
            ("rust-native/tests/library_checkbox_process.rs", include_bytes!("../../../tests/library_checkbox_process.rs"), "5f9c6a687290d3f516d4b08f7c733652e582ee9c10a3dd222c4abdb2e4e80557"),
            ("rust-native/tests/library_drafts_process.rs", include_bytes!("../../../tests/library_drafts_process.rs"), "27676806949e31bb4679db9f2acfcf068fe7229d6deb51d2d6b6f3af2b4ec7b2"),
            ("rust-native/tests/library_navigation_process.rs", include_bytes!("../../../tests/library_navigation_process.rs"), "5c9ae7c66d226e7cb0178c047d36cfabee111b6f4dfd0097f9d39e06b1635186"),
            ("rust-native/tests/library_lifecycle_process.rs", include_bytes!("../../../tests/library_lifecycle_process.rs"), "4f3d417117826cd22f64e233a5c27609402a4a41f5029b1e54ce6de6907c92ad"),
            ("rust-native/tests/settings_preferences_process.rs", include_bytes!("../../../tests/settings_preferences_process.rs"), "94f965cbaa9ac8270d1775d4d1695806e1155e7328be9481602021536510de34"),
            ("rust-native/tests/utility_process.rs", include_bytes!("../../../tests/utility_process.rs"), "eef887ae1eade8bf817d9035ac3ab8baa7faec84eb818b2816bb8b58eada51aa"),
            ("rust-native/tests/bridge_process.py", include_bytes!("../../../tests/bridge_process.py"), "1d6c5f6f8143f9e6f4a642cd8d32e14b4d243baa24f3079fa62b4723fdaffac4"),
            ("rust-native/tests/bridge_process.rs", include_bytes!("../../../tests/bridge_process.rs"), "5e281d85d5122f4758e648ce78133657377df963c3fedf827a17b609e778ec3d"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "6cda15594c28aade5b415de9f1f22992febcc1973183053e5e045cfdfcd38e92"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "f003aad7e18fced24ad358e138042bface5d5212a53dee32cb9adacfc3db80f9"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),

            ("Sources/RuycastApp/FeedbackChecks.swift", include_bytes!("../../../../Sources/RuycastApp/FeedbackChecks.swift"), "1f91b690d51f82c2c2d6ea653c0f0ec4c06747dc21603472302c30c098cfaa04"),
            ("Sources/RuycastApp/ActionMenuChecks.swift", include_bytes!("../../../../Sources/RuycastApp/ActionMenuChecks.swift"), "7aa714222112b9d350949dc96992d9ac34f8afe21082ecacc2d60c62d0c6bb9f"),
            ("Sources/RuycastApp/InteractionChecks.swift", include_bytes!("../../../../Sources/RuycastApp/InteractionChecks.swift"), "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858"),
            ("rust-native/src/native_edit.rs", include_bytes!("../../native_edit.rs"), "9ebb1ecff6de79f76d0f69542e250a6d041b88afe0ca1ec5beb079a08b4f042c"),
            ("rust-native/src/keyword_matcher.rs", include_bytes!("../../keyword_matcher.rs"), "14ce406b97d0a9bf0e7497a81f102425a73c6ac181f3d98f1b766b0bd899c49d"),
            ("rust-native/src/templates.rs", include_bytes!("../../templates.rs"), "0c2c9f768bd6a911414ffea94335610eedebf8feea55eb6210fce6f5d72a7bdd"),
            ("rust-native/src/snippet_output.rs", include_bytes!("../../snippet_output.rs"), "c2fefbe888a3c3d1e9f940baaf783b6bf6a44faa19f83fdddc975aa3d2e53858"),
            ("rust-native/src/native_snippet_choice.rs", include_bytes!("../../native_snippet_choice.rs"), "caa9be3b6d84594d8fabc13f797a99a5042515ba94760cae86506773139a09a9"),
            ("rust-native/src/snippet_choice.rs", include_bytes!("../../snippet_choice.rs"), "d340d341185ef6000347355f8b88f68da6b34458730ecf646c5eb5541e2b95aa"),
            ("rust-native/src/snippet_choice_request.rs", include_bytes!("../../snippet_choice_request.rs"), "9f42eb8c27f93929e66a631de7dda8cc39630d2225459fa758d71195f391d6eb"),
            ("rust-native/src/bin/native_snippet_choice_probe.rs", include_bytes!("../native_snippet_choice_probe.rs"), "66f534623863df99f352a304bbb0a09669be6e609bfabb5db58699356f8ada03"),
            ("rust-native/tests/native_snippet_choice_process.py", include_bytes!("../../../tests/native_snippet_choice_process.py"), "4423b225ce9ba56a68fe096dc31b6650d805db26f36d3a90dc730b42b8b0bd3a"),
            ("rust-native/tests/fixtures/choice_board_sdk.rs", include_bytes!("../../../tests/fixtures/choice_board_sdk.rs"), "b2cf3257801da488997df127933a9f29a09d3b93abec66f2dd14ea6808a34450"),
            ("rust-native/tests/fixtures/choice_monitor_sdk.rs", include_bytes!("../../../tests/fixtures/choice_monitor_sdk.rs"), "7d83558c04fb39fcac4369de5dac244c5387962b896bb4653817d55327325e74"),
            ("rust-native/tests/fixtures/choice_fixture.rs", include_bytes!("../../../tests/fixtures/choice_fixture.rs"), "cf80241e31020b09f45243c22bd29df3682cb8e83bfe1b796a5c26b054f976d8"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "b08a088712b4344e9792ca3219fa93039eda94411f25e40169af7cd05704d8a3"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("rust-native/tests/native_choice_host_process.rs", include_bytes!("../../../tests/native_choice_host_process.rs"), "1eae948c8766cd6c7c006b24aedd3366bdc053209cecfd4ca257f9b8f287be87"),
            ("rust-native/tests/support/native_keyboard.rs", include_bytes!("../../../tests/support/native_keyboard.rs"), "76ff8a46006068ede9b84376c3ee0f96293c5efbae60c8a82032dcc6fd89ada3"),
            ("rust-native/tests/actions_process.rs", include_bytes!("../../../tests/actions_process.rs"), "3ed819ce552c2e63ac4dc2e5f0001cd69a40f465e40bc003a13c6151c4b4d481"),
            ("rust-native/tests/native_keyboard_process.rs", include_bytes!("../../../tests/native_keyboard_process.rs"), "651a992fc77ba45e15b55623e0d0e8eaeff964fcfccbf070fdc241e218cce6a2"),
        ],
        "--check-rich-resources" => &[
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "0cf0eb9765919ea4142fe4436df347a9ed7215663dfa699d218a970e39c1569c"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "fd42e39f7784f426e34806f43a8a2d7cef46413d26684d5cd6d5cd444c1759b7"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "b1cb059ffc82fe259c1866957f98042272fc09bb877fe8154ff4637c474feccc"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "174d38c6cb1a3b6b96a45405eea6dd805b0c74899ae1230c902b3d85e7ae3e7e"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "456a2d80b8a29c7902b023211b31bd9eddc56311e8468a177f0044060a44b895"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "81562c61db965a8d14b14272973b4670fceef588834642d17e6861c99498a801"),
            ("rust-native/src/native_monitor.rs", include_bytes!("../../native_monitor.rs"), "54716cd3c8d6d03f0c5da68ec83d723621854d9647d5dbc62e38d0eb2ffd89c2"),
            ("rust-native/src/native_clipboard.rs", include_bytes!("../../native_clipboard.rs"), "6a3db88c4184426dae72431087af17177caa1f7bb3f2cff1bc2c4d7ed621c62f"),
            ("rust-native/src/processes.rs", include_bytes!("../../processes.rs"), "3dd132e3eaaf7c74c5e73c1cbeb11fc32bee276f88d9fe5764149b8fd3800de5"),
            ("rust-native/src/monitor.rs", include_bytes!("../../monitor.rs"), "a0332045b3c8cb7f2ca8c2acf0a4c76c52acd5efff0b0a2b1d50f7822d25cf91"),
            ("rust-native/src/resource_sampler.rs", include_bytes!("../../resource_sampler.rs"), "14e577615f4d9807aed14da0f103e9338193cb50d1ccec6c75b6f667af755a98"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "47b7823c045903339702225fdf009eb7f0913d4ccde98e5f208263fceea8bec7"),
            ("rust-native/src/system_search/native.rs", include_bytes!("../../system_search/native.rs"), "d4be9915ab842a74652001bed1c7f7b363c72d5b94ca1dcac3e1db41652ea750"),
            ("rust-native/src/clipboard_history.rs", include_bytes!("../../clipboard_history.rs"), "2b426ebe9c5b49d43de120859277f468c8285de16dc989b7179fc2de56a5862a"),
            ("rust-native/src/clipboard_preview.rs", include_bytes!("../../clipboard_preview.rs"), "a7d1506387dc9fb6bc9c987d1d2449680ba6b5785bfcd8fdef644b172aada772"),
            ("rust-native/src/clipboard_image.rs", include_bytes!("../../clipboard_image.rs"), "37bc43506344b20c0334bc4737e29e5769d10384a908abd891134293ca1625db"),
            ("rust-native/src/clipboard_ownership.rs", include_bytes!("../../clipboard_ownership.rs"), "b38329314cae144ece5ca7063135db60c9deea8e3df6d3728e0b631c1f0e68b4"),
            ("rust-native/src/activity.rs", include_bytes!("../../activity.rs"), "e8b4b609655d7946bbb27f287aaa739ed1530133f22a7cfdca2d5003655aa462"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "c00bfde67fef22f13fc2f1aebf80617ffaf4609d9d788dd9f14520fcd579a84a"),
            ("rust/src/private_storage.rs", include_bytes!("../../../../rust/src/private_storage.rs"), "94fe1298f7a96fe03b2eef17d067921fa043ad21bfcf7568b63aeb9d81df4c1a"),
            ("rust-native/tests/remaining_macos_process.rs", include_bytes!("../../../tests/remaining_macos_process.rs"), "3fd6aa6c6a3c8321dde7a9f4b85a01045e18aad1e0bfeea89d2a9d5d0f935409"),
            ("rust-native/tests/clipboard_history_process.py", include_bytes!("../../../tests/clipboard_history_process.py"), "72798c56f799ebc73893de1e53345ead4d2bc82125e1a76be47bd0b529752066"),
            ("rust-native/tests/clipboard_rich_paste.rs", include_bytes!("../../../tests/clipboard_rich_paste.rs"), "263551b7339fbf2e7e714fc277af9840262f796fe9c9851ca972cd9f8cc014d8"),
            ("rust-native/tests/fixtures/workspace_sdk.rs", include_bytes!("../../../tests/fixtures/workspace_sdk.rs"), "750f2ab8558a4ee887d64ad21c4022527019af91ca5390ccf89f23a6c125ef05"),
            ("rust-native/tests/fixtures/snippet_effects_sdk.rs", include_bytes!("../../../tests/fixtures/snippet_effects_sdk.rs"), "4070c904b8d6cd406487561c108d03ea7870da176b64e66b14de718bfcadb4af"),
            ("rust-native/tests/fixtures/clipboard_history_sdk.rs", include_bytes!("../../../tests/fixtures/clipboard_history_sdk.rs"), "712c823e91e7670f738c52aff5781dbeac77314983434e19ec4c0b2e7984aae3"),
            ("rust-native/tests/fixtures/clipboard_history_fixture.rs", include_bytes!("../../../tests/fixtures/clipboard_history_fixture.rs"), "94473b2d9d0000dfd436c4acdcec048e6ae771d4f4a745d8686359709549ce92"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "3e0a03535dd190da1f79f459b512f5d00d79f7bfafacfecdcd8b336fbe2e7763"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("rust-native/tests/fixtures/workspace_fixture.rs", include_bytes!("../../../tests/fixtures/workspace_fixture.rs"), "893eaba2a1a537958085554a78d7cd69131003c0c705d0a6c5952b16a9f5cb06"),
            ("Sources/RuycastApp/RichResourceChecks.swift", include_bytes!("../../../../Sources/RuycastApp/RichResourceChecks.swift"), "5c2322b47bc436f51103e022a9878a53f4cb1565952d165e83aa63cd03e31eda"),
        ],
        "--check-menu-actions" => &[
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "c9f43c7bfa10aaf3400dbb24bc4dd1c3383343e0569776950abc8b0a20a209f5"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "6cda15594c28aade5b415de9f1f22992febcc1973183053e5e045cfdfcd38e92"),
            ("rust-native/tests/system_search_cli_process.rs", include_bytes!("../../../tests/system_search_cli_process.rs"), "e31a670e462a90ab454e9fd5a4ae8d1568022ef061342d459ffae40464ce42c1"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "f003aad7e18fced24ad358e138042bface5d5212a53dee32cb9adacfc3db80f9"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "0cf0eb9765919ea4142fe4436df347a9ed7215663dfa699d218a970e39c1569c"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "fd42e39f7784f426e34806f43a8a2d7cef46413d26684d5cd6d5cd444c1759b7"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "b1cb059ffc82fe259c1866957f98042272fc09bb877fe8154ff4637c474feccc"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "174d38c6cb1a3b6b96a45405eea6dd805b0c74899ae1230c902b3d85e7ae3e7e"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "7d93fd7492c92ff2c031a0879e3e5f265849981b70480fa8be4dc49be6217407"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "456a2d80b8a29c7902b023211b31bd9eddc56311e8468a177f0044060a44b895"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "81562c61db965a8d14b14272973b4670fceef588834642d17e6861c99498a801"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "47b7823c045903339702225fdf009eb7f0913d4ccde98e5f208263fceea8bec7"),
            ("rust-native/src/system_search/tests.rs", include_bytes!("../../system_search/tests.rs"), "d35042be9f1f88dc038a19e14d0e7c66f685fd0c2069afdfad5c85d1036f6bab"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/system_search/native.rs", include_bytes!("../../system_search/native.rs"), "d4be9915ab842a74652001bed1c7f7b363c72d5b94ca1dcac3e1db41652ea750"),
            ("rust-native/src/system_search/main_bridge.rs", include_bytes!("../../system_search/main_bridge.rs"), "d0267d4a14fbefc08a7c17a50c4e1f9872abb11fe414fe32f7507274552c908e"),
            ("rust-native/src/bin/system_search_probe.rs", include_bytes!("../../bin/system_search_probe.rs"), "091b06e8231011aab0b057b5c9f347bbbb531ac7e2b87e50ba92cabf1292c585"),
            ("rust-native/tests/system_search_sdk.rs", include_bytes!("../../../tests/system_search_sdk.rs"), "4232a21e7d0cd38ba0f43bf58fa289a336475985820644448b0dbdfc909afd55"),
            ("rust-native/tests/fixtures/system_nodes.rs", include_bytes!("../../../tests/fixtures/system_nodes.rs"), "a47bb7e0a8dea4524b09cc867655e1eb9c41791144f2a2c0172fbc3416252900"),
            ("rust-native/tests/fixtures/system_effects.rs", include_bytes!("../../../tests/fixtures/system_effects.rs"), "146408327fa184d10129d0a3f1fc6032079cf09d8e98fe313444150f72090f7d"),
            ("rust-native/tests/fixtures/system_apps.rs", include_bytes!("../../../tests/fixtures/system_apps.rs"), "6b32b83ad723be97e1d237dcc29bbfcc49a6d7bc533d3847dbafcbe70a48965e"),
            ("rust-native/tests/fixtures/system_events.rs", include_bytes!("../../../tests/fixtures/system_events.rs"), "7507bb74ca8392e0b7e60ec092eedb1a3820c1e00657b1e6a8c9cd7d4b1e4394"),
            ("rust-native/tests/fixtures/system_fixture.rs", include_bytes!("../../../tests/fixtures/system_fixture.rs"), "01482b9077efd5d418c533f9879d6fc31c7b7359259f3d2d5f45fff310e44e98"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "adc7afde131edc529fa38e65c37250970c93ba1860e73d8225816b02534c2e8b"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("rust-native/src/actions.rs", include_bytes!("../../actions.rs"), "c00bfde67fef22f13fc2f1aebf80617ffaf4609d9d788dd9f14520fcd579a84a"),
            ("rust-native/src/activity.rs", include_bytes!("../../activity.rs"), "e8b4b609655d7946bbb27f287aaa739ed1530133f22a7cfdca2d5003655aa462"),
            ("rust-native/src/analytics.rs", include_bytes!("../../analytics.rs"), "920a995c14978bcd86ff22a3cdd6ba9dcd1f83d3540748f14d7f79e5c2761852"),
            ("config/default.json", include_bytes!("../../../../config/default.json"), "a13a90a2ccfd5c92cc39dba19e3792a55c04700c26118721636ad036eb7ebd7c"),
            ("Sources/RuycastApp/MenuActionChecks.swift", include_bytes!("../../../../Sources/RuycastApp/MenuActionChecks.swift"), "41d224340e00d1970fb953aad90c3c4a539e6af84634b1aa04b90972c166f1ed"),
        ],
        "--check-window-search" => &[
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "c9f43c7bfa10aaf3400dbb24bc4dd1c3383343e0569776950abc8b0a20a209f5"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "6cda15594c28aade5b415de9f1f22992febcc1973183053e5e045cfdfcd38e92"),
            ("rust-native/tests/system_search_cli_process.rs", include_bytes!("../../../tests/system_search_cli_process.rs"), "e31a670e462a90ab454e9fd5a4ae8d1568022ef061342d459ffae40464ce42c1"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "f003aad7e18fced24ad358e138042bface5d5212a53dee32cb9adacfc3db80f9"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "0cf0eb9765919ea4142fe4436df347a9ed7215663dfa699d218a970e39c1569c"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "fd42e39f7784f426e34806f43a8a2d7cef46413d26684d5cd6d5cd444c1759b7"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "b1cb059ffc82fe259c1866957f98042272fc09bb877fe8154ff4637c474feccc"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "174d38c6cb1a3b6b96a45405eea6dd805b0c74899ae1230c902b3d85e7ae3e7e"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "7d93fd7492c92ff2c031a0879e3e5f265849981b70480fa8be4dc49be6217407"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "456a2d80b8a29c7902b023211b31bd9eddc56311e8468a177f0044060a44b895"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "81562c61db965a8d14b14272973b4670fceef588834642d17e6861c99498a801"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "47b7823c045903339702225fdf009eb7f0913d4ccde98e5f208263fceea8bec7"),
            ("rust-native/src/system_search/tests.rs", include_bytes!("../../system_search/tests.rs"), "d35042be9f1f88dc038a19e14d0e7c66f685fd0c2069afdfad5c85d1036f6bab"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/system_search/native.rs", include_bytes!("../../system_search/native.rs"), "d4be9915ab842a74652001bed1c7f7b363c72d5b94ca1dcac3e1db41652ea750"),
            ("rust-native/src/system_search/main_bridge.rs", include_bytes!("../../system_search/main_bridge.rs"), "d0267d4a14fbefc08a7c17a50c4e1f9872abb11fe414fe32f7507274552c908e"),
            ("rust-native/src/bin/system_search_probe.rs", include_bytes!("../../bin/system_search_probe.rs"), "091b06e8231011aab0b057b5c9f347bbbb531ac7e2b87e50ba92cabf1292c585"),
            ("rust-native/tests/system_search_sdk.rs", include_bytes!("../../../tests/system_search_sdk.rs"), "4232a21e7d0cd38ba0f43bf58fa289a336475985820644448b0dbdfc909afd55"),
            ("rust-native/tests/fixtures/system_nodes.rs", include_bytes!("../../../tests/fixtures/system_nodes.rs"), "a47bb7e0a8dea4524b09cc867655e1eb9c41791144f2a2c0172fbc3416252900"),
            ("rust-native/tests/fixtures/system_effects.rs", include_bytes!("../../../tests/fixtures/system_effects.rs"), "146408327fa184d10129d0a3f1fc6032079cf09d8e98fe313444150f72090f7d"),
            ("rust-native/tests/fixtures/system_apps.rs", include_bytes!("../../../tests/fixtures/system_apps.rs"), "6b32b83ad723be97e1d237dcc29bbfcc49a6d7bc533d3847dbafcbe70a48965e"),
            ("rust-native/tests/fixtures/system_events.rs", include_bytes!("../../../tests/fixtures/system_events.rs"), "7507bb74ca8392e0b7e60ec092eedb1a3820c1e00657b1e6a8c9cd7d4b1e4394"),
            ("rust-native/tests/fixtures/system_fixture.rs", include_bytes!("../../../tests/fixtures/system_fixture.rs"), "01482b9077efd5d418c533f9879d6fc31c7b7359259f3d2d5f45fff310e44e98"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "adc7afde131edc529fa38e65c37250970c93ba1860e73d8225816b02534c2e8b"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("Sources/RuycastApp/WindowSearchChecks.swift", include_bytes!("../../../../Sources/RuycastApp/WindowSearchChecks.swift"), "cae0f60d635d6aa5af41d8fe8ecc9bc8102c28e49e2aec87a7e53462829ddf7f"),
        ],
        "--check-apple-shortcuts" => &[
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "c9f43c7bfa10aaf3400dbb24bc4dd1c3383343e0569776950abc8b0a20a209f5"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "6cda15594c28aade5b415de9f1f22992febcc1973183053e5e045cfdfcd38e92"),
            ("rust-native/tests/system_search_cli_process.rs", include_bytes!("../../../tests/system_search_cli_process.rs"), "e31a670e462a90ab454e9fd5a4ae8d1568022ef061342d459ffae40464ce42c1"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "f003aad7e18fced24ad358e138042bface5d5212a53dee32cb9adacfc3db80f9"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "0cf0eb9765919ea4142fe4436df347a9ed7215663dfa699d218a970e39c1569c"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "fd42e39f7784f426e34806f43a8a2d7cef46413d26684d5cd6d5cd444c1759b7"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "b1cb059ffc82fe259c1866957f98042272fc09bb877fe8154ff4637c474feccc"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "174d38c6cb1a3b6b96a45405eea6dd805b0c74899ae1230c902b3d85e7ae3e7e"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "7d93fd7492c92ff2c031a0879e3e5f265849981b70480fa8be4dc49be6217407"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "456a2d80b8a29c7902b023211b31bd9eddc56311e8468a177f0044060a44b895"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "81562c61db965a8d14b14272973b4670fceef588834642d17e6861c99498a801"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "47b7823c045903339702225fdf009eb7f0913d4ccde98e5f208263fceea8bec7"),
            ("rust-native/src/system_search/tests.rs", include_bytes!("../../system_search/tests.rs"), "d35042be9f1f88dc038a19e14d0e7c66f685fd0c2069afdfad5c85d1036f6bab"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/system_search/native.rs", include_bytes!("../../system_search/native.rs"), "d4be9915ab842a74652001bed1c7f7b363c72d5b94ca1dcac3e1db41652ea750"),
            ("rust-native/src/system_search/main_bridge.rs", include_bytes!("../../system_search/main_bridge.rs"), "d0267d4a14fbefc08a7c17a50c4e1f9872abb11fe414fe32f7507274552c908e"),
            ("rust-native/src/bin/system_search_probe.rs", include_bytes!("../../bin/system_search_probe.rs"), "091b06e8231011aab0b057b5c9f347bbbb531ac7e2b87e50ba92cabf1292c585"),
            ("rust-native/tests/system_search_sdk.rs", include_bytes!("../../../tests/system_search_sdk.rs"), "4232a21e7d0cd38ba0f43bf58fa289a336475985820644448b0dbdfc909afd55"),
            ("rust-native/tests/fixtures/system_nodes.rs", include_bytes!("../../../tests/fixtures/system_nodes.rs"), "a47bb7e0a8dea4524b09cc867655e1eb9c41791144f2a2c0172fbc3416252900"),
            ("rust-native/tests/fixtures/system_effects.rs", include_bytes!("../../../tests/fixtures/system_effects.rs"), "146408327fa184d10129d0a3f1fc6032079cf09d8e98fe313444150f72090f7d"),
            ("rust-native/tests/fixtures/system_apps.rs", include_bytes!("../../../tests/fixtures/system_apps.rs"), "6b32b83ad723be97e1d237dcc29bbfcc49a6d7bc533d3847dbafcbe70a48965e"),
            ("rust-native/tests/fixtures/system_events.rs", include_bytes!("../../../tests/fixtures/system_events.rs"), "7507bb74ca8392e0b7e60ec092eedb1a3820c1e00657b1e6a8c9cd7d4b1e4394"),
            ("rust-native/tests/fixtures/system_fixture.rs", include_bytes!("../../../tests/fixtures/system_fixture.rs"), "01482b9077efd5d418c533f9879d6fc31c7b7359259f3d2d5f45fff310e44e98"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "adc7afde131edc529fa38e65c37250970c93ba1860e73d8225816b02534c2e8b"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("rust-native/src/system_search/apple_shortcuts.rs", include_bytes!("../../system_search/apple_shortcuts.rs"), "f2642015ad57c2d7b08af1f23c1e5125fbe841f9658714f7cc0e5adc56d21581"),
            ("rust-native/src/runtime_private.rs", include_bytes!("../../runtime_private.rs"), "88c10d0c32c7fa0e163ee6c3a320bde9ee81828852c0d204effaf1d6a151219b"),
            ("rust-native/src/script_result.rs", include_bytes!("../../script_result.rs"), "86cb6b6479e795e577f8dbd2b3e67884c4ab2ac03db1f2e615d0af08bdeb50a1"),
            ("rust-native/src/script_session.rs", include_bytes!("../../script_session.rs"), "d669d09ed8056ae0182bb4b3aa0b22ae2e4abe584592192839f69677fd5fa9a6"),
            ("rust-native/src/execution.rs", include_bytes!("../../execution.rs"), "a211e57ffabbb0cb0cbb8ce19ad894939ca310c36a34a9fea055a0df3e023c8f"),
            ("rust-native/src/activity.rs", include_bytes!("../../activity.rs"), "e8b4b609655d7946bbb27f287aaa739ed1530133f22a7cfdca2d5003655aa462"),
            ("rust/src/script.rs", include_bytes!("../../../../rust/src/script.rs"), "16974362c399c6c5a92596d7563d501eb0492df9f5e2463d8147eeb70108bfb0"),
            ("rust-native/tests/script_result_basic_process.rs", include_bytes!("../../../tests/script_result_basic_process.rs"), "70e1a2de21b80f8702438acb49b03fac13576c84cc90e3857af8fc3aa48eb498"),
            ("rust-native/tests/script_result_native_process.rs", include_bytes!("../../../tests/script_result_native_process.rs"), "6791270af079e8a05e679dc86de1528dd4dfb7f881b7a291dc8b9878a8786410"),
            ("rust-native/src/bin/private_results_fixture.rs", include_bytes!("../private_results_fixture.rs"), "eccf15f44528f6449f76a83b04ae39c223e4d852051d606f7db4f64bae68d531"),
            ("rust-native/tests/private_results.rs", include_bytes!("../../../tests/private_results.rs"), "9c734610fe76959e5b497ac0092b9c1d19ffbcd393e5ca8bd41aa055c2484db1"),
            ("config/default.json", include_bytes!("../../../../config/default.json"), "a13a90a2ccfd5c92cc39dba19e3792a55c04700c26118721636ad036eb7ebd7c"),
            ("Sources/RuycastApp/AppleShortcutChecks.swift", include_bytes!("../../../../Sources/RuycastApp/AppleShortcutChecks.swift"), "a5e0806bf33b5a4cbe365b4861e7c6143c5f7ca5a6c10ce347239af8da4fce74"),
        ],
        "--check-render-updates" => &[
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "fd42e39f7784f426e34806f43a8a2d7cef46413d26684d5cd6d5cd444c1759b7"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "b1cb059ffc82fe259c1866957f98042272fc09bb877fe8154ff4637c474feccc"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "174d38c6cb1a3b6b96a45405eea6dd805b0c74899ae1230c902b3d85e7ae3e7e"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "7d93fd7492c92ff2c031a0879e3e5f265849981b70480fa8be4dc49be6217407"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "456a2d80b8a29c7902b023211b31bd9eddc56311e8468a177f0044060a44b895"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "81562c61db965a8d14b14272973b4670fceef588834642d17e6861c99498a801"),
            ("rust-native/src/configuration_jobs.rs", include_bytes!("../../configuration_jobs.rs"), "8b82be49a8a8a424939eb726bc41b2389d6f7306070ef0c46e55fc2fae59d1cd"),
            ("rust-native/src/config_effects.rs", include_bytes!("../../config_effects.rs"), "7eb5168b123f15def09d4b22d8ce15a31e7e8cc7ead68cbdc95789af869e169e"),
            ("rust-native/src/observe.rs", include_bytes!("../../observe.rs"), "d74120c1ffbb0f2ad76238c95744fef8813ce1ae8a21eeb104a7f566ed3e2a5e"),
            ("rust-native/src/profile.rs", include_bytes!("../../profile.rs"), "6dff51a543b8895c04b5c5cc64cec33750ff3062a1326bbd6d49f8348637aea1"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "a0e361eccac5eb167669841d4945deacf9803a1d87e06147ae2bc5a5512a4909"),
            ("Sources/RuycastApp/InteractionChecks.swift", include_bytes!("../../../../Sources/RuycastApp/InteractionChecks.swift"), "7f629804da223ea12989179e5a7f87c0fa374e1b0b237c659d3d0196c0b57858"),
        ],
        "--check-browser-tabs" => &[
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "c9f43c7bfa10aaf3400dbb24bc4dd1c3383343e0569776950abc8b0a20a209f5"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "6cda15594c28aade5b415de9f1f22992febcc1973183053e5e045cfdfcd38e92"),
            ("rust-native/tests/system_search_cli_process.rs", include_bytes!("../../../tests/system_search_cli_process.rs"), "e31a670e462a90ab454e9fd5a4ae8d1568022ef061342d459ffae40464ce42c1"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "f003aad7e18fced24ad358e138042bface5d5212a53dee32cb9adacfc3db80f9"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "0cf0eb9765919ea4142fe4436df347a9ed7215663dfa699d218a970e39c1569c"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "fd42e39f7784f426e34806f43a8a2d7cef46413d26684d5cd6d5cd444c1759b7"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/native_model.rs", include_bytes!("../../native_model.rs"), "b1cb059ffc82fe259c1866957f98042272fc09bb877fe8154ff4637c474feccc"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "174d38c6cb1a3b6b96a45405eea6dd805b0c74899ae1230c902b3d85e7ae3e7e"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "81562c61db965a8d14b14272973b4670fceef588834642d17e6861c99498a801"),
            ("rust-native/src/system_search/mod.rs", include_bytes!("../../system_search/mod.rs"), "358221b1edab478fa8eda8dbf3d6cc74050f948f483cbbf251ab824a920c41e1"),
            ("rust-native/src/system_search/fixture.rs", include_bytes!("../../system_search/fixture.rs"), "47b7823c045903339702225fdf009eb7f0913d4ccde98e5f208263fceea8bec7"),
            ("rust-native/src/system_search/tests.rs", include_bytes!("../../system_search/tests.rs"), "d35042be9f1f88dc038a19e14d0e7c66f685fd0c2069afdfad5c85d1036f6bab"),
            ("rust-native/src/system_search/service.rs", include_bytes!("../../system_search/service.rs"), "da7a087cdd593525e6d4314655ea245bd9cc6954b89efcf87e43b8b3cb17595a"),
            ("rust-native/src/system_search/providers.rs", include_bytes!("../../system_search/providers.rs"), "254065ccb0e2529da82b85792f16965f051b7a101aa74fea78d6641905881280"),
            ("rust-native/src/system_search/native.rs", include_bytes!("../../system_search/native.rs"), "d4be9915ab842a74652001bed1c7f7b363c72d5b94ca1dcac3e1db41652ea750"),
            ("rust-native/src/bin/system_search_probe.rs", include_bytes!("../../bin/system_search_probe.rs"), "091b06e8231011aab0b057b5c9f347bbbb531ac7e2b87e50ba92cabf1292c585"),
            ("rust-native/tests/system_search_sdk.rs", include_bytes!("../../../tests/system_search_sdk.rs"), "4232a21e7d0cd38ba0f43bf58fa289a336475985820644448b0dbdfc909afd55"),
            ("rust-native/tests/fixtures/system_nodes.rs", include_bytes!("../../../tests/fixtures/system_nodes.rs"), "a47bb7e0a8dea4524b09cc867655e1eb9c41791144f2a2c0172fbc3416252900"),
            ("rust-native/tests/fixtures/system_effects.rs", include_bytes!("../../../tests/fixtures/system_effects.rs"), "146408327fa184d10129d0a3f1fc6032079cf09d8e98fe313444150f72090f7d"),
            ("rust-native/tests/fixtures/system_apps.rs", include_bytes!("../../../tests/fixtures/system_apps.rs"), "6b32b83ad723be97e1d237dcc29bbfcc49a6d7bc533d3847dbafcbe70a48965e"),
            ("rust-native/tests/fixtures/system_events.rs", include_bytes!("../../../tests/fixtures/system_events.rs"), "7507bb74ca8392e0b7e60ec092eedb1a3820c1e00657b1e6a8c9cd7d4b1e4394"),
            ("rust-native/tests/fixtures/system_fixture.rs", include_bytes!("../../../tests/fixtures/system_fixture.rs"), "01482b9077efd5d418c533f9879d6fc31c7b7359259f3d2d5f45fff310e44e98"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "adc7afde131edc529fa38e65c37250970c93ba1860e73d8225816b02534c2e8b"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("Sources/RuycastApp/BrowserTabChecks.swift", include_bytes!("../../../../Sources/RuycastApp/BrowserTabChecks.swift"), "82189e7c80a7baab3e38a8b34619de06f343c21ddf3b023df2bf0cafd6f0ebb9"),
        ],
        "--check-performance-pipeline" => &[
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/runtime_private_tests.rs", include_bytes!("../../runtime_private_tests.rs"), "ac3a8a55284b182fe6d56031a10c1bba66d265338e3f3540976b8832fd6fa22d"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/connected.rs", include_bytes!("../../connected.rs"), "fd42e39f7784f426e34806f43a8a2d7cef46413d26684d5cd6d5cd444c1759b7"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "81562c61db965a8d14b14272973b4670fceef588834642d17e6861c99498a801"),
            ("rust-native/src/clipboard_history.rs", include_bytes!("../../clipboard_history.rs"), "2b426ebe9c5b49d43de120859277f468c8285de16dc989b7179fc2de56a5862a"),
            ("rust-native/src/configuration_jobs.rs", include_bytes!("../../configuration_jobs.rs"), "8b82be49a8a8a424939eb726bc41b2389d6f7306070ef0c46e55fc2fae59d1cd"),
            ("rust-native/src/config_effects.rs", include_bytes!("../../config_effects.rs"), "7eb5168b123f15def09d4b22d8ce15a31e7e8cc7ead68cbdc95789af869e169e"),
            ("rust-native/src/profile.rs", include_bytes!("../../profile.rs"), "6dff51a543b8895c04b5c5cc64cec33750ff3062a1326bbd6d49f8348637aea1"),
            ("rust-native/src/observe.rs", include_bytes!("../../observe.rs"), "d74120c1ffbb0f2ad76238c95744fef8813ce1ae8a21eeb104a7f566ed3e2a5e"),
            ("rust-native/src/clipboard_preview.rs", include_bytes!("../../clipboard_preview.rs"), "a7d1506387dc9fb6bc9c987d1d2449680ba6b5785bfcd8fdef644b172aada772"),
            ("rust-native/src/native_clipboard.rs", include_bytes!("../../native_clipboard.rs"), "6a3db88c4184426dae72431087af17177caa1f7bb3f2cff1bc2c4d7ed621c62f"),
            ("rust-native/src/clipboard_image.rs", include_bytes!("../../clipboard_image.rs"), "37bc43506344b20c0334bc4737e29e5769d10384a908abd891134293ca1625db"),
            ("rust-native/src/clipboard_ownership.rs", include_bytes!("../../clipboard_ownership.rs"), "b38329314cae144ece5ca7063135db60c9deea8e3df6d3728e0b631c1f0e68b4"),
            ("rust/src/private_storage.rs", include_bytes!("../../../../rust/src/private_storage.rs"), "94fe1298f7a96fe03b2eef17d067921fa043ad21bfcf7568b63aeb9d81df4c1a"),
            ("rust/src/lib.rs", include_bytes!("../../../../rust/src/lib.rs"), "0dfcbb5c3d0d5d15149af8fd128067832235819923bd813fda83e4c894a86333"),
            ("rust-native/tests/clipboard_rich_paste.rs", include_bytes!("../../../tests/clipboard_rich_paste.rs"), "263551b7339fbf2e7e714fc277af9840262f796fe9c9851ca972cd9f8cc014d8"),
            ("rust-native/tests/clipboard_history_process.py", include_bytes!("../../../tests/clipboard_history_process.py"), "72798c56f799ebc73893de1e53345ead4d2bc82125e1a76be47bd0b529752066"),
            ("rust-native/tests/fixtures/workspace_sdk.rs", include_bytes!("../../../tests/fixtures/workspace_sdk.rs"), "750f2ab8558a4ee887d64ad21c4022527019af91ca5390ccf89f23a6c125ef05"),
            ("rust-native/tests/fixtures/snippet_effects_sdk.rs", include_bytes!("../../../tests/fixtures/snippet_effects_sdk.rs"), "4070c904b8d6cd406487561c108d03ea7870da176b64e66b14de718bfcadb4af"),
            ("rust-native/tests/fixtures/clipboard_history_sdk.rs", include_bytes!("../../../tests/fixtures/clipboard_history_sdk.rs"), "712c823e91e7670f738c52aff5781dbeac77314983434e19ec4c0b2e7984aae3"),
            ("rust-native/tests/fixtures/clipboard_history_fixture.rs", include_bytes!("../../../tests/fixtures/clipboard_history_fixture.rs"), "94473b2d9d0000dfd436c4acdcec048e6ae771d4f4a745d8686359709549ce92"),
            ("rust-native/Cargo.toml", include_bytes!("../../../Cargo.toml"), "3e0a03535dd190da1f79f459b512f5d00d79f7bfafacfecdcd8b336fbe2e7763"),
            ("rust-native/Cargo.lock", include_bytes!("../../../Cargo.lock"), "3ea0eb855d24f0b7b143746b49fa4dd1f311116e46c6561c7b5984e2383094e5"),
            ("rust/Cargo.toml", include_bytes!("../../../../rust/Cargo.toml"), "35f9cfa76ee0df1d4c8596264d5daf64c5683395330cc5db35be1a6e70138949"),
            ("rust-toolchain.toml", include_bytes!("../../../../rust-toolchain.toml"), "d0c46cd3a68028a2d8bad683c5016ddafb0218922f8b2bb09e7fbae0ee73cf82"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("Sources/RuycastApp/PerformancePipelineChecks.swift", include_bytes!("../../../../Sources/RuycastApp/PerformancePipelineChecks.swift"), "429130589dd29b1c12fea7854b9e868b9d365f85ec8334a80782da999fa90659"),
        ],
        "--check-settings" => &[
            ("rust-native/tests/native_keyboard_process.rs", include_bytes!("../../../tests/native_keyboard_process.rs"), "651a992fc77ba45e15b55623e0d0e8eaeff964fcfccbf070fdc241e218cce6a2"),
            ("rust-native/tests/support/native_keyboard.rs", include_bytes!("../../../tests/support/native_keyboard.rs"), "76ff8a46006068ede9b84376c3ee0f96293c5efbae60c8a82032dcc6fd89ada3"),
            ("rust-native/src/bin/private_results_fixture.rs", include_bytes!("../private_results_fixture.rs"), "eccf15f44528f6449f76a83b04ae39c223e4d852051d606f7db4f64bae68d531"),
            ("rust-native/tests/actions_process.rs", include_bytes!("../../../tests/actions_process.rs"), "3ed819ce552c2e63ac4dc2e5f0001cd69a40f465e40bc003a13c6151c4b4d481"),
            ("rust-native/tests/support/native_actions.rs", include_bytes!("../../../tests/support/native_actions.rs"), "c9f43c7bfa10aaf3400dbb24bc4dd1c3383343e0569776950abc8b0a20a209f5"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "6cda15594c28aade5b415de9f1f22992febcc1973183053e5e045cfdfcd38e92"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "f003aad7e18fced24ad358e138042bface5d5212a53dee32cb9adacfc3db80f9"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            ("rust-native/examples/ui_completion.rs", include_bytes!("../../../examples/ui_completion.rs"), "a0e361eccac5eb167669841d4945deacf9803a1d87e06147ae2bc5a5512a4909"),
            ("rust-native/src/bin/snippet_probe.rs", include_bytes!("../../bin/snippet_probe.rs"), "e8322e4076b5ddfe71b0e79c3039f34c7fe1dcfcf3f66f42f512e0207387361a"),
            ("rust-native/src/clipboard_history.rs", include_bytes!("../../clipboard_history.rs"), "2b426ebe9c5b49d43de120859277f468c8285de16dc989b7179fc2de56a5862a"),
            ("rust-native/src/config_effects.rs", include_bytes!("../../config_effects.rs"), "7eb5168b123f15def09d4b22d8ce15a31e7e8cc7ead68cbdc95789af869e169e"),
            ("rust-native/src/configuration_jobs.rs", include_bytes!("../../configuration_jobs.rs"), "8b82be49a8a8a424939eb726bc41b2389d6f7306070ef0c46e55fc2fae59d1cd"),
            ("rust-native/src/hotkeys.rs", include_bytes!("../../hotkeys.rs"), "b5e3579c365aed3164baabfba3914cc66146c6a7ee6e7557386a6d9e26b9f3cd"),
            ("rust-native/src/keyboard_state.rs", include_bytes!("../../keyboard_state.rs"), "d55e9cf045e87fc77b4906b72afaee2e8445daed26ddf4c7c66e9a683756e5fd"),
            ("rust-native/src/library.rs", include_bytes!("../../library.rs"), "4788c64fe77873c6f71ceff312f248421c8255c0e0a64387713b280964815525"),
            ("rust-native/src/native_editor.rs", include_bytes!("../../native_editor.rs"), "7d93fd7492c92ff2c031a0879e3e5f265849981b70480fa8be4dc49be6217407"),
            ("rust-native/src/native_library.rs", include_bytes!("../../native_library.rs"), "2da5d1b8b410bd90ae4f63855a0ff06be6b0677038b33acd0a2a69cde1542ecb"),
            ("rust-native/src/native_paste.rs", include_bytes!("../../native_paste.rs"), "aa76bdf2224e9ed4a249b91d9a41712ae067b4f6a47a265bb9bcb6365da3d626"),
            ("rust-native/src/native_rows.rs", include_bytes!("../../native_rows.rs"), "456a2d80b8a29c7902b023211b31bd9eddc56311e8468a177f0044060a44b895"),
            ("rust-native/src/native_services.rs", include_bytes!("../../native_services.rs"), "81562c61db965a8d14b14272973b4670fceef588834642d17e6861c99498a801"),
            ("rust-native/src/native_settings.rs", include_bytes!("../../native_settings.rs"), "5d662f5ad70e3be4407acf74babbf97345ac0b05079fcdd88069181d63a23041"),
            ("rust-native/src/native_snippet_editor.rs", include_bytes!("../../native_snippet_editor.rs"), "554be26da4356ff39640a2d4af5b6b4758062dcc782d5da129bfb6b85fb12671"),
            ("rust-native/src/native_surface.rs", include_bytes!("../../native_surface.rs"), "174d38c6cb1a3b6b96a45405eea6dd805b0c74899ae1230c902b3d85e7ae3e7e"),
            ("rust-native/src/native.rs", include_bytes!("../../native.rs"), "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10"),
            ("rust-native/src/profile.rs", include_bytes!("../../profile.rs"), "6dff51a543b8895c04b5c5cc64cec33750ff3062a1326bbd6d49f8348637aea1"),
            ("rust-native/src/recorder.rs", include_bytes!("../../recorder.rs"), "9225bfda7d0a27993cfeee5ac0a42870efd6ef89929305a2390e63a4ad6950b6"),
            ("rust-native/src/runtime_health.rs", include_bytes!("../../runtime_health.rs"), "8c9964bcad7bda0bd6f52a2921b1e7d646c39545aa6a48bc7383138e1b33bdfb"),
            ("rust-native/src/settings_surface.rs", include_bytes!("../../settings_surface.rs"), "0578d288549d01d76d3168cc8607ab56bd46df03d8583a49ab266ce95b5e5b37"),
            ("rust-native/src/snippet_output.rs", include_bytes!("../../snippet_output.rs"), "c2fefbe888a3c3d1e9f940baaf783b6bf6a44faa19f83fdddc975aa3d2e53858"),
            ("rust-native/tests/clipboard_rich_paste.rs", include_bytes!("../../../tests/clipboard_rich_paste.rs"), "263551b7339fbf2e7e714fc277af9840262f796fe9c9851ca972cd9f8cc014d8"),
            ("rust-native/tests/keyword_process.py", include_bytes!("../../../tests/keyword_process.py"), "f7aada122ee3c2da48d2390b60d5c4be5520b51a584e60e07a15fc5f3a3c09c2"),
            ("rust-native/tests/keyword_process.rs", include_bytes!("../../../tests/keyword_process.rs"), "435348c83048fe5a34a6ea4c121ee38089d493d2692732d563ed157b8f7c00eb"),
            ("rust-native/tests/keyword_reference.json", include_bytes!("../../../tests/keyword_reference.json"), "b3ad168109a4712a792756f6227a730cec365fae7399824803034131ce956119"),
            ("rust-native/tests/keyword_rich_process.py", include_bytes!("../../../tests/keyword_rich_process.py"), "48bc3d8bbcb3e617c1fb10671aea305c22329c44a37cd51c8cb06d9cc4e24965"),
            ("rust-native/tests/library_checkbox_process.rs", include_bytes!("../../../tests/library_checkbox_process.rs"), "5f9c6a687290d3f516d4b08f7c733652e582ee9c10a3dd222c4abdb2e4e80557"),
            ("rust-native/tests/library_drafts_process.rs", include_bytes!("../../../tests/library_drafts_process.rs"), "27676806949e31bb4679db9f2acfcf068fe7229d6deb51d2d6b6f3af2b4ec7b2"),
            ("rust-native/tests/library_navigation_process.rs", include_bytes!("../../../tests/library_navigation_process.rs"), "5c9ae7c66d226e7cb0178c047d36cfabee111b6f4dfd0097f9d39e06b1635186"),
            ("rust-native/tests/library_lifecycle_process.rs", include_bytes!("../../../tests/library_lifecycle_process.rs"), "4f3d417117826cd22f64e233a5c27609402a4a41f5029b1e54ce6de6907c92ad"),
            ("rust-native/tests/settings_keyboard_process.rs", include_bytes!("../../../tests/settings_keyboard_process.rs"), "0c4326feb11b031e79f30926410dc44719347b0988628b813c1286052df9e762"),
            ("rust-native/tests/settings_preferences_process.rs", include_bytes!("../../../tests/settings_preferences_process.rs"), "94f965cbaa9ac8270d1775d4d1695806e1155e7328be9481602021536510de34"),
            ("rust-native/tests/snippet_board_process.rs", include_bytes!("../../../tests/snippet_board_process.rs"), "8f5841d6591e369d7b120534b1bb2f78f87cfafc098efe4066992d7f0fef6d3a"),
            ("rust-native/tests/snippet_process.rs", include_bytes!("../../../tests/snippet_process.rs"), "cfe1f20e499e4da81bb7c8508f1a5c951a0114da28a6b7feabd4ba58dee673c0"),
            ("rust-native/tests/snippet_reference.json", include_bytes!("../../../tests/snippet_reference.json"), "b19d5abc788642df64402065797d255db3fa399b0b763eb2cf66544746f1f746"),
            ("rust-native/tests/startup_process.rs", include_bytes!("../../../tests/startup_process.rs"), "922179b5495bd479c70eccece85735bfafaaec0cbd1accc4590ca28a75f98a07"),
            ("rust/src/lib.rs", include_bytes!("../../../../rust/src/lib.rs"), "0dfcbb5c3d0d5d15149af8fd128067832235819923bd813fda83e4c894a86333"),
            ("Sources/RuycastApp/SettingsChecks.swift", include_bytes!("../../../../Sources/RuycastApp/SettingsChecks.swift"), "cb2b26a91a94e7537d1db308b272945ea7002946100245e84c70954b18ce9e1d"),
            ("Sources/RuycastApp/SnippetEditingChecks.swift", include_bytes!("../../../../Sources/RuycastApp/SnippetEditingChecks.swift"), "adfae471c755454375ff7af2b1c99108ab08bd4a9c2a50556809e2dad5f0b99e"),
        ],
        "--check-command-log" => &[
            ("rust-native/src/runtime.rs", include_bytes!("../../runtime.rs"), "a685a1539d71c086ce3f3f2b3123f1147c3e7ce1d8564b89d61dd1ac3d0884ce"),
            ("rust-native/src/runtime_system.rs", include_bytes!("../../runtime_system.rs"), "0cf0eb9765919ea4142fe4436df347a9ed7215663dfa699d218a970e39c1569c"),
            ("rust-native/src/runtime_utilities.rs", include_bytes!("../../runtime_utilities.rs"), "209d90ba65f4fb8fd9a1341d419fddf5ada441dcebb641914cd2ac1e6692e8bd"),
            ("rust-native/src/runtime_builtins.rs", include_bytes!("../../runtime_builtins.rs"), "43bc0ddfcfde25438465f7dc2f9c245bd7084d8906f44baf5f229be43e6f991c"),
            ("rust-native/tests/private_results.rs", include_bytes!("../../../tests/private_results.rs"), "9c734610fe76959e5b497ac0092b9c1d19ffbcd393e5ca8bd41aa055c2484db1"),
            ("rust-native/src/bin/private_results_fixture.rs", include_bytes!("../private_results_fixture.rs"), "eccf15f44528f6449f76a83b04ae39c223e4d852051d606f7db4f64bae68d531"),
            (
                "rust-native/src/activity.rs",
                include_bytes!("../../activity.rs"),
                LOG_SOURCE_SHA256,
            ),
            (
                "Sources/RuycastApp/CommandLogChecks.swift",
                include_bytes!("../../../../Sources/RuycastApp/CommandLogChecks.swift"),
                LOG_ORACLE_SHA256,
            ),
        ],
        "--check-edit-commands" => &[
            (
                "rust-native/src/native_edit.rs",
                include_bytes!("../../native_edit.rs"),
                "9ebb1ecff6de79f76d0f69542e250a6d041b88afe0ca1ec5beb079a08b4f042c",
            ),
            (
                "rust-native/src/native_edit_menu.rs",
                include_bytes!("../../native_edit_menu.rs"),
                "8213290295ec55cd13adf37a5fd30f6648854fadc471272be9ed49e3385c438b",
            ),
            (
                "Sources/RuycastApp/EditCommandChecks.swift",
                include_bytes!("../../../../Sources/RuycastApp/EditCommandChecks.swift"),
                "a91248845b9256c30f2906adc818e5e706034b6277a52519df2f8758203b1145",
            ),
        ],
        "--check-analytics" => &[
            ("rust-native/tests/native_dashboard_dialog_process.rs", include_bytes!("../../../tests/native_dashboard_dialog_process.rs"), "9acf5206263ce816d73c47170d37ab1f0b21c4fb7664ec53d74b8a2e9c8b5c76"),
            ("rust-native/tests/support/native_bridge.rs", include_bytes!("../../../tests/support/native_bridge.rs"), "6cda15594c28aade5b415de9f1f22992febcc1973183053e5e045cfdfcd38e92"),
            ("rust-native/tests/support/runtime_host.rs", include_bytes!("../../../tests/support/runtime_host.rs"), "f003aad7e18fced24ad358e138042bface5d5212a53dee32cb9adacfc3db80f9"),
            ("rust-native/tests/support/status_rpc.rs", include_bytes!("../../../tests/support/status_rpc.rs"), "d31e794e77f6a3c9a0e1747cf9d256426cb70009f74dadceb612dbc733829582"),
            ("rust-native/tests/support/child_output.rs", include_bytes!("../../../tests/support/child_output.rs"), "66d6a8792a00d8fb802014bae60a7c6fa227f029883eb2b0882f2a0e374dcd18"),
            (
                "rust-native/src/analytics.rs",
                include_bytes!("../../analytics.rs"),
                "920a995c14978bcd86ff22a3cdd6ba9dcd1f83d3540748f14d7f79e5c2761852",
            ),
            (
                "rust-native/src/analytics_dashboard.rs",
                include_bytes!("../../analytics_dashboard.rs"),
                "df90b83a49b965a6eeea0ee7c7810eace54a825d226a35f892d4d518d57e6e28",
            ),
            (
                "rust-native/src/native_dashboard.rs",
                include_bytes!("../../native_dashboard.rs"),
                "44eadd8b53ea51c8f8617de4db3c5eac18464d0b966b24ec465162f86b3c2485",
            ),
            (
                "rust-native/src/management_commands.rs",
                include_bytes!("../../management_commands.rs"),
                "14f68e0678ba748a65fc88a200f3cea59b9c9de8950462c2369ce5c526c75a31",
            ),
            (
                "rust-native/tests/startup_process.rs",
                include_bytes!("../../../tests/startup_process.rs"),
                "922179b5495bd479c70eccece85735bfafaaec0cbd1accc4590ca28a75f98a07",
            ),
            (
                "rust-native/tests/dashboard_export_process.rs",
                include_bytes!("../../../tests/dashboard_export_process.rs"),
                "710c9f5b5010acef290e19af2a9f14d3f1cb8851cf4d596f97952dea33644a0b",
            ),
            (
                "rust-native/tests/fixtures/dashboard_export_shim.rs",
                include_bytes!("../../../tests/fixtures/dashboard_export_shim.rs"),
                "843e90a21bd0018028b3343c17f08b5ae491fd650c48da37095120f8d5238a47",
            ),
            (
                "rust-native/tests/fixtures/fcntl_abi.rs",
                include_bytes!("../../../tests/fixtures/fcntl_abi.rs"),
                "b2b7194db0bf1adef57aab17e3424a90c67818eadac43ee6b651d3a7b4eb0029",
            ),
            (
                "rust-native/src/test_fixtures.rs",
                include_bytes!("../../test_fixtures.rs"),
                "3ee56f1afc0008bda747798f54aa1d3a039603d16451ed24b67f6ea8fb0b06b5",
            ),
            (
                "Sources/RuycastApp/AnalyticsChecks.swift",
                include_bytes!("../../../../Sources/RuycastApp/AnalyticsChecks.swift"),
                "b7e97f390de005268b1a7aa0ceeda22ca165bb2a84f8fcf1917858c4dcb73876",
            ),
            ("rust-native/tests/analytics_runtime_process.rs", include_bytes!("../../../tests/analytics_runtime_process.rs"), "8ef18a852c782df0a469ea5ca4f22e2e4d2c9f0392f1edad6103389adedd871b"),
            ("rust-native/tests/fixtures/analytics_write_shim.rs", include_bytes!("../../../tests/fixtures/analytics_write_shim.rs"), "9e2add20464e7336d854171083e3861f79a4dbe6d49f67ecf723cf72f98b790c"),
            ("rust-native/tests/analytics_cast_reference.rs", include_bytes!("../../../tests/analytics_cast_reference.rs"), "9af609d17784a42a1847feec74d5e70ceccf84efc83be03c5a9578e3aac482c3"),
            ("rust-native/tests/fixtures/analytics_cast_reference.json", include_bytes!("../../../tests/fixtures/analytics_cast_reference.json"), "6bd89546d290d6e3f634c185af4cf322b4d141a0426f77eb034ff115bc8df9cb"),
            ("rust-native/tests/fixtures/analytics_lifecycle_reference.json", include_bytes!("../../../tests/fixtures/analytics_lifecycle_reference.json"), "cd0b799d618f950147b3f318c698bd29aaf1e7790d785c597b3643639eeeaac9"),
            ("rust-native/tests/fixtures/analytics_retention_reference.json", include_bytes!("../../../tests/fixtures/analytics_retention_reference.json"), "ddaea60e8771a91a4a51332b14e2e1a0d58f3b361155759eaa6919da1397f7e6"),
            ("rust-native/tests/fixtures/analytics_decoder_reference.json", include_bytes!("../../../tests/fixtures/analytics_decoder_reference.json"), "22a3ec58d6f51df2173d686b2f98770a3b0d39ce015af85eaa674327a15b32cc"),
            ("rust-native/tests/fixtures/analytics_integer_reference.json", include_bytes!("../../../tests/fixtures/analytics_integer_reference.json"), "3c0fed3cabad017dc90b7d2396b27b455c57489eba80d58ef19682d524befc4c"),
            ("rust-native/tests/fixtures/analytics_limits_reference.json", include_bytes!("../../../tests/fixtures/analytics_limits_reference.json"), "e58f134fddec3a7ecb65a6c9fa341dca1feee40dde9abd7c3b03e11bc2563bfb"),
            ("rust-native/tests/dashboard_presentation_reference.rs", include_bytes!("../../../tests/dashboard_presentation_reference.rs"), "bf3748beebc236ffecd75289e95011704dbef19dbe8f52083b263e0ff34efc85"),
            ("rust-native/tests/fixtures/dashboard_presentation_reference.json", include_bytes!("../../../tests/fixtures/dashboard_presentation_reference.json"), "3e1bf1619960487cf831ad00001a7b575b1b434c89bb395522734e44a2cba283"),
        ],
        "--check-visual-assets" => &[
            (
                "rust-native/src/presentation.rs",
                include_bytes!("../../presentation.rs"),
                "e178daee544e7ddec8120aeceae5fddf6f7704cff10cce7613d5b43d6361e8cd",
            ),
            (
                "rust-native/src/website_icons.rs",
                include_bytes!("../../website_icons.rs"),
                "7748d58700ea937351bbd2c31d76ef3f8e14a346f3cda873a87a55608efe217e",
            ),
            (
                "rust-native/src/clipboard_image.rs",
                include_bytes!("../../clipboard_image.rs"),
                "37bc43506344b20c0334bc4737e29e5769d10384a908abd891134293ca1625db",
            ),
            (
                "rust-native/src/native_rows.rs",
                include_bytes!("../../native_rows.rs"),
                "456a2d80b8a29c7902b023211b31bd9eddc56311e8468a177f0044060a44b895",
            ),
            (
                "rust-native/src/native.rs",
                include_bytes!("../../native.rs"),
                "fa986dcb9ba2fd5b1eac22a28afc67e6a328d70f184dd1fb0542acfaa8937b10",
            ),
            (
                "Sources/RuycastApp/VisualAssetChecks.swift",
                include_bytes!("../../../../Sources/RuycastApp/VisualAssetChecks.swift"),
                "4bd5994c6cc34a504e3f1576a0a21ca903636ed8293986ccde561a4c3b7304f0",
            ),
        ],
        _ => &[],
    }
}
fn settings_mapping_state(gate: &SettingsGate) -> &'static str {
    if !gate.unproven_invariants.is_empty() || gate.native_cases.is_empty() {
        return "mapped-with-unproven-invariants";
    }
    // These exact inputs have completed original-case crosswalks and scoped
    // independent review. This qualifies mapping, never a validation pass.
    let sources = settings_sources(gate.id);
    let current = !sources.is_empty()
        && sources
            .iter()
            .all(|(_, bytes, expected)| format!("{:x}", Sha256::digest(bytes)) == *expected);
    if current {
        "source-qualified-rust-case-crosswalk"
    } else {
        "mapped-requires-source-requalification"
    }
}
fn root_git_mapping_state(source: &[u8]) -> &'static str {
    if format!("{:x}", Sha256::digest(source)) == "08e8adeaa358ca7d5acedb5a569615b2a558a57cd311c20115384ea9796678a8" {
        "source-qualified-root-bound-whitespace-check"
    } else {
        "requires-source-requalification"
    }
}
pub(super) fn inventory(kind: &str) -> Result<serde_json::Value> {
    let gates: Vec<(&str, &str)> = match kind {
        "check" => vec![
            ("LauncherCoreChecks", "source-qualified-pure-oracle"),
            ("--check-search", "source-qualified-pre-main-oracle"),
            ("--check-concurrency", "source-qualified-pre-main-oracle"),
            (
                "python-unittest-discover-Tests-test_*.py",
                "requires-complete-transport-and-case-mapping",
            ),
            ("git-diff-check", root_git_mapping_state(include_bytes!("compat.rs"))),
        ],
        "settings" => SETTINGS
            .iter()
            .map(|gate| (gate.id, settings_mapping_state(gate)))
            .collect(),
        _ => return Err("Unknown legacy gate inventory.".into()),
    };
    let python_modules = if kind == "check" {
        vec![
            "Tests/test_architecture.py",
            "Tests/test_benchmark_host.py",
            "Tests/test_build_pipeline.py",
            "Tests/test_cli.py",
            "Tests/test_clipboard_import.py",
            "Tests/test_config_activation.py",
            "Tests/test_config_directory.py",
            "Tests/test_install_migration.py",
            "Tests/test_local_signing.py",
            "Tests/test_package_app.py",
            "Tests/test_private_storage.py",
            "Tests/test_product_version.py",
            "Tests/test_public_contract.py",
            "Tests/test_socket_recovery.py",
            "Tests/test_bindings_v2.py",
            "Tests/test_personal_bridge.py",
            "Tests/test_transition_catalog.py",
            "Tests/test_utilities_import.py",
        ]
    } else {
        vec![]
    };
    let unresolved = gates
        .iter()
        .filter(|(_, state)| !state.starts_with("source-qualified"))
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    let mut qualified = BTreeMap::new();
    for gate in SETTINGS {
        if settings_mapping_state(gate) == "source-qualified-rust-case-crosswalk" {
            qualified.insert(
                gate.id,
                settings_sources(gate.id)
                    .iter()
                    .map(|(path, _, sha)| (*path, *sha))
                    .collect::<BTreeMap<_, _>>(),
            );
        }
    }
    Ok(
        serde_json::json!({"schema":"ruycast.original-gate-inventory.v1","pipeline":kind,"gates":gates,"unmappedGateIDs":unresolved,"settingsCaseCrosswalk":if kind=="settings"{serde_json::to_value(SETTINGS).map_err(error)?}else{serde_json::Value::Null},"qualifiedCaseSourceBindings":if kind=="settings"{serde_json::to_value(qualified).map_err(error)?}else{serde_json::Value::Null},"requiresCurrentCompleteNativeValidation":true,"pythonModuleInventory":python_modules,"pythonDiscovery":"Every test discovered by the original Tests/test_*.py selector remains required; the optional integration modules need their own presence/safety qualification, never a blanket skip","unmappedIsFailure":true,"productionAccepted":false}),
    )
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HeadlessReceipt {
    schema: String,
    sources: BTreeMap<String, String>,
    artifacts: BTreeMap<String, String>,
    outcomes: BTreeMap<String, serde_json::Value>,
    commands: Vec<Vec<String>>,
    scope: String,
    production_accepted: bool,
}
fn headless_sources(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut values = core_sources(root)?;
    validation::source_folder(root, &root.join("Sources"), &mut values)?;
    Ok(values)
}
pub(super) fn headless(root: &Path, lease: &BuildLock) -> Result<serde_json::Value> {
    verify_core(root, lease)?;
    let before = headless_sources(root)?;
    tool(
        root,
        lease,
        &[
            "python3".into(),
            "rust-native/scripts/check_rust_only.py".into(),
        ],
    )?;
    let archive = root.join(".build/rust/release/librustcast_core.a");
    let archive_hash = digest(&archive)?;
    let scratch = format!(".build/oracle-swift/{archive_hash}");
    let compile = vec![
        "swift".into(),
        "build".into(),
        "--scratch-path".into(),
        scratch.clone(),
        "--configuration".into(),
        "debug".into(),
        "--product".into(),
        "RuycastApp".into(),
    ];
    tool(root, lease, &compile)?;
    let bin_path = tool(
        root,
        lease,
        &[
            "swift".into(),
            "build".into(),
            "--scratch-path".into(),
            scratch,
            "--configuration".into(),
            "debug".into(),
            "--show-bin-path".into(),
        ],
    )?;
    let bin_path = PathBuf::from(bin_path.trim());
    if !bin_path.starts_with(root.join(".build/oracle-swift"))
        || bin_path.components().any(|part| {
            !matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
    {
        return Err("Unexpected frozen host oracle path.".into());
    }
    let binary = bin_path.join("RuycastApp");
    let binary_hash = digest(&binary)?;
    tool(
        root,
        lease,
        &[
            "python3".into(),
            "rust-native/scripts/check_rust_only.py".into(),
        ],
    )?;
    if headless_sources(root)? != before || digest(&archive)? != archive_hash {
        return Err("Frozen oracle source or static library changed before entry.".into());
    }
    let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".build/oracle-fixtures"));
    if !parent.is_absolute() {
        return Err("Oracle fixture parent must be absolute.".into());
    }
    let fixture = preparation::Stage::in_parent(&parent, "h")?;
    use std::os::unix::ffi::OsStrExt;
    if fixture.path().as_os_str().as_bytes().len() + 32 >= 104 {
        return Err(
            "Use a shorter explicit RUYCAST_NATIVE_TEST_FIXTURE_DIR for the frozen socket oracle."
                .into(),
        );
    }
    let mut outcomes = BTreeMap::new();
    let mut commands = vec![compile];
    // This closed selector list is the exact audited pre-AppDelegate scope.
    // It is deliberately not an arbitrary host/check flag dispatcher.
    for flag in ["--check-search", "--check-concurrency"] {
        let mut command = Command::new(&binary);
        command
            .arg(flag)
            .current_dir(root)
            .env_clear()
            .env("HOME", fixture.path())
            .env("TMPDIR", fixture.path())
            .env("RUYCAST_HOME", fixture.path());
        lease.apply_to_child(&mut command)?;
        let value = process::capture(command, std::time::Duration::from_secs(30), 2 * 1024 * 1024)?;
        let name = flag.trim_start_matches('-');
        ruycast_native::private_storage::write(
            &root
                .join(".build/receipts")
                .join(format!("pure-{name}.stdout")),
            &value.stdout,
            true,
        )?;
        ruycast_native::private_storage::write(
            &root
                .join(".build/receipts")
                .join(format!("pure-{name}.stderr")),
            &value.stderr,
            true,
        )?;
        let outcome: serde_json::Value = serde_json::from_slice(&value.stdout).map_err(error)?;
        if !value.status.success() || outcome["ok"] != true {
            return Err(format!("Frozen {flag} oracle failed."));
        }
        outcomes.insert(flag.into(), outcome);
        commands.push(vec![
            binary.to_str().ok_or("Invalid oracle binary path.")?.into(),
            flag.into(),
        ]);
    }
    if headless_sources(root)? != before
        || digest(&archive)? != archive_hash
        || digest(&binary)? != binary_hash
    {
        return Err("Frozen oracle inputs changed during branch checks.".into());
    }
    verify_core(root, lease)?;
    let mut artifacts = BTreeMap::new();
    for path in [&archive, &binary] {
        artifacts.insert(
            path.strip_prefix(root)
                .map_err(error)?
                .to_str()
                .ok_or("Invalid oracle artifact path.")?
                .into(),
            digest(path)?,
        );
    }
    let receipt=HeadlessReceipt{schema:"ruycast.pre-main-headless-oracles.v1".into(),sources:before,artifacts,outcomes,commands,scope:"Exact frozen --check-search/--check-concurrency branches before AppDelegate; fresh owned HOME/TMPDIR/RUYCAST_HOME; no other flags, normal startup, status, UI, global SDK or account-lookup acceptance".into(),production_accepted:false};
    lease.validate()?;
    ruycast_native::private_storage::write(
        &root.join(".build/receipts/pre-main-headless-oracles.json"),
        &serde_json::to_vec_pretty(&receipt).map_err(error)?,
        true,
    )?;
    verify_headless(root, lease)
}
pub(super) fn verify_headless(root: &Path, lease: &BuildLock) -> Result<serde_json::Value> {
    lease.validate()?;
    let receipt: HeadlessReceipt = serde_json::from_slice(&regular_bytes(
        &root.join(".build/receipts/pre-main-headless-oracles.json"),
    )?)
    .map_err(error)?;
    if receipt.schema != "ruycast.pre-main-headless-oracles.v1"
        || receipt.production_accepted
        || receipt.sources != headless_sources(root)?
        || receipt.artifacts.len() != 2
        || receipt.outcomes.len() != 2
        || ["--check-search", "--check-concurrency"]
            .iter()
            .any(|flag| {
                receipt
                    .outcomes
                    .get(*flag)
                    .is_none_or(|value| value["ok"] != true)
            })
    {
        return Err("Frozen pre-main branch receipt is invalid or stale.".into());
    }
    for (name, expected) in &receipt.artifacts {
        if Path::new(name).is_absolute()
            || Path::new(name)
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || digest(&root.join(name))? != *expected
        {
            return Err("Frozen pre-main branch artifact changed.".into());
        }
    }
    verify_core(root, lease)?;
    serde_json::to_value(receipt).map_err(error)
}
pub(super) fn require_complete(kind: &str) -> Result<()> {
    let inventory = inventory(kind)?;
    if inventory["unmappedGateIDs"]
        .as_array()
        .ok_or("Missing original gate inventory")?
        .is_empty()
    {
        return Ok(());
    }
    Err(format!("Complete {kind} validation is not yet qualified: {inventory}. Native-only checks cannot replace these original gates."))
}
#[cfg(test)]
mod tests {
    #[test]
    fn script_result_basic_binds_current_sources_and_route() {
        use sha2::Digest;
        let inputs = settings_sources("--check-apple-shortcuts");
        for path in ["rust-native/tests/script_result_basic_process.rs", "rust-native/tests/script_result_native_process.rs", "rust-native/src/bin/private_results_fixture.rs"] {
            let (_, bytes, expected) = inputs.iter().find(|entry| entry.0 == path).unwrap();
            assert_eq!(format!("{:x}", sha2::Sha256::digest(bytes)), *expected, "{path}");
        }
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-apple-shortcuts").unwrap();
        assert!(gate.required_native_steps.contains(&"script-result-process"));
        assert!(gate.required_native_steps.contains(&"script-result-native-process"));
    }
    #[test]
    fn library_checkbox_crosswalk_binds_current_sources_and_cases() {
        use sha2::{Digest, Sha256};
        for id in ["--check-feedback", "--check-actions-menu", "--check-interactions", "--check-settings"] {
            let inputs = settings_sources(id);
            let matches: Vec<_> = inputs.iter().filter(|(path, _, _)| path.ends_with("library_checkbox_process.rs") || path.ends_with("library_drafts_process.rs") || path.ends_with("library_navigation_process.rs") || path.ends_with("library_lifecycle_process.rs")).collect();
            assert_eq!(matches.len(), 4, "{id}");
            for (path, bytes, hash) in matches {
                assert_eq!(format!("{:x}", Sha256::digest(bytes)), *hash, "{id}: {path}");
            }
        }
        let gate = SETTINGS.iter().find(|g| g.id == "--check-settings").unwrap();
        assert!(gate.required_native_steps.contains(&"library-checkbox-process"));
        assert!(gate.required_native_steps.contains(&"library-drafts-process"));
        assert!(gate.required_native_steps.contains(&"library-navigation-process"));
        let lifecycle = include_str!("../../../tests/library_lifecycle_process.rs");
        let cases: Vec<_> = gate.native_cases.iter().filter_map(|s| s.strip_prefix("library_lifecycle_process::")).collect();
        assert_eq!(cases.len(), 5);
        for case in cases { assert!(lifecycle.contains(&format!("fn {case}(")), "{case}"); }
        let navigation = include_str!("../../../tests/library_navigation_process.rs");
        let cases: Vec<_> = gate.native_cases.iter().filter_map(|s| s.strip_prefix("library_navigation_process::")).collect();
        assert_eq!(cases.len(), 4);
        for case in cases { assert!(navigation.contains(&format!("fn {case}(")), "{case}"); }
        let drafts = include_str!("../../../tests/library_drafts_process.rs");
        let cases: Vec<_> = gate.native_cases.iter().filter_map(|s| s.strip_prefix("library_drafts_process::")).collect();
        assert_eq!(cases.len(), 5);
        for case in cases { assert!(drafts.contains(&format!("fn {case}(")), "{case}"); }
        let source = include_str!("../../../tests/library_checkbox_process.rs");
        let cases: Vec<_> = gate.native_cases.iter().filter_map(|s| s.strip_prefix("library_checkbox_process::")).collect();
        assert_eq!(cases.len(), 5);
        for case in cases { assert!(source.contains(&format!("fn {case}(")), "{case}"); }
    }
    #[test]
    fn creation_crosswalk_uses_real_rust_cases_and_shared_actions_fixture() {
        use sha2::Digest;
        let source = include_str!("../../../tests/creation_process.rs");
        for (id,case) in [("--check-feedback","empty_results_keep_all_three_creation_actions_available"),("--check-interactions","external_changes_survive_and_identity_collision_never_overwrites")] {
            let gate=super::SETTINGS.iter().find(|gate|gate.id==id).unwrap();
            assert!(gate.native_cases.contains(&format!("creation_process::{case}").as_str()));
            assert!(source.contains(&format!("fn {case}()")));
        }
        for id in ["--check-feedback","--check-actions-menu","--check-interactions"] {
            let entries=super::settings_sources(id);
            for path in ["rust-native/tests/creation_process.rs","rust-native/tests/support/native_actions.rs"] {
                let matches=entries.iter().filter(|entry|entry.0==path).collect::<Vec<_>>();
                assert_eq!(matches.len(),1,"{id}: {path}");
                assert_eq!(format!("{:x}",sha2::Sha256::digest(matches[0].1)),matches[0].2);
            }
            assert!(!entries.iter().any(|entry|entry.0=="rust-native/tests/creation_process.py"));
        }
    }
    #[test]
    fn analytics_dashboard_export_crosswalk_uses_existing_rust_cases() {
        use sha2::Digest;
        let gate = super::SETTINGS.iter().find(|gate|gate.id=="--check-analytics").unwrap();
        assert!(gate.required_native_steps.contains(&"dashboard-export-process"));
        assert!(gate.required_native_steps.contains(&"native-dashboard-dialog-process"));
        let entries=super::settings_sources(gate.id);
        let dialog = entries.iter().find(|entry| entry.0=="rust-native/tests/native_dashboard_dialog_process.rs").unwrap();
        assert_eq!(format!("{:x}",sha2::Sha256::digest(dialog.1)),dialog.2);
        let dialog_source = std::str::from_utf8(dialog.1).unwrap();
        for name in ["owned_save_dialog_takes_current_data_and_cancel_writes_nothing", "retired_controls_and_dialog_cannot_mutate_a_reopened_dashboard", "management_routing_controls_custom_retention_and_reset", "repeated_confirmation_retires_old_queued_reset_callback", "standard_builtin_cli_native_sources_usage_logs_and_override", "builtin_input_validation_and_native_prompt_do_not_log_private_values", "actual_dashboard_renders_themes_widths_and_preserves_scroll", "populated_paused_errors_confirmation_roles_values_and_local_key_loop"] {
            assert!(gate.native_cases.contains(&format!("native_dashboard_dialog_process::{name}").as_str()));
            assert!(dialog_source.contains(&format!("fn {name}()")));
        }
        assert!(!entries.iter().any(|entry|entry.0=="rust-native/tests/dashboard_export_process.py"));
        let sources=entries.iter().filter(|entry|entry.0=="rust-native/tests/dashboard_export_process.rs").collect::<Vec<_>>();
        assert_eq!(sources.len(),1);
        assert_eq!(format!("{:x}",sha2::Sha256::digest(sources[0].1)),sources[0].2);
        let source=std::str::from_utf8(sources[0].1).unwrap();
        let cases=gate.native_cases.iter().filter_map(|case|case.strip_prefix("dashboard_export_process::")).collect::<Vec<_>>();
        assert_eq!(cases.len(),5);
        for case in cases {assert!(source.contains(&format!("fn {case}()")));}
    }
    #[test]
    fn settings_preferences_crosswalk_uses_four_current_rust_cases() {
        use sha2::Digest;
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        let names = gate.native_cases.iter().filter_map(|case|case.strip_prefix("settings_preferences_process::")).collect::<Vec<_>>();
        assert_eq!(names.len(),4);
        for id in ["--check-feedback","--check-actions-menu","--check-interactions","--check-settings"] {
            let entries=super::settings_sources(id);
            let source=entries.iter().filter(|entry|entry.0=="rust-native/tests/settings_preferences_process.rs").collect::<Vec<_>>();
            assert_eq!(source.len(),1,"{id}");
            assert_eq!(format!("{:x}",sha2::Sha256::digest(source[0].1)),source[0].2);
            let text=std::str::from_utf8(source[0].1).unwrap();
            for name in &names {assert!(text.contains(&format!("fn {name}()")));}
            assert!(!entries.iter().any(|entry|entry.0=="rust-native/tests/settings_preferences_process.py"));
        }
    }
    #[test]
    fn general_keyboard_crosswalk_names_both_rust_tests() {
        use sha2::Digest;
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        let cases = gate.native_cases.iter().filter_map(|name|name.strip_prefix("settings_keyboard_process::")).collect::<Vec<_>>();
        assert_eq!(cases.len(), 2);
        let entries = super::settings_sources("--check-settings");
        let source = entries.iter().find(|entry|entry.0 == "rust-native/tests/settings_keyboard_process.rs").unwrap();
        assert_eq!(format!("{:x}",sha2::Sha256::digest(source.1)),source.2);
        let text = std::str::from_utf8(source.1).unwrap();
        for case in cases { assert!(text.contains(&format!("fn {case}()"))); }
        assert!(!gate.native_cases.iter().any(|name|name.starts_with("settings_keyboard_process.")));
    }
    #[test]
    fn settings_keyboard_crosswalk_uses_current_rust_suite_and_owned_sdk() {
        use sha2::Digest;
        let source = include_str!("../../../tests/native_keyboard_process.rs");
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        let names = gate.native_cases.iter().filter_map(|name| name.strip_prefix("native_keyboard_process::")).collect::<Vec<_>>();
        assert_eq!(names.len(), 15);
        for name in names { assert!(source.contains(&format!("fn {name}()")), "{name}"); }
        assert!(!gate.native_cases.iter().any(|name| name.starts_with("native_keyboard_process.")));
        for id in ["--check-actions-menu", "--check-settings"] {
            let entries = super::settings_sources(id);
            for path in ["rust-native/tests/native_keyboard_process.rs", "rust-native/tests/support/native_keyboard.rs", "rust-native/src/bin/private_results_fixture.rs"] {
                let matches = entries.iter().filter(|entry| entry.0 == path).collect::<Vec<_>>();
                assert_eq!(matches.len(), 1, "{id}: {path}");
                assert_eq!(format!("{:x}", sha2::Sha256::digest(matches[0].1)), matches[0].2);
            }
        }
        let matrix = include_str!("validation.rs");
        assert!(matrix.contains("native-keyboard-sdk-build|c|build --manifest-path rust-native/Cargo.toml --features test-tools --example keyboard-host --locked"));
        assert!(matrix.contains("native-keyboard-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test native_keyboard_process --locked"));
    }
    #[test]
    fn settings_actions_crosswalk_uses_current_rust_suite_and_helpers() {
        use sha2::Digest;
        let source = include_str!("../../../tests/actions_process.rs");
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        let names = gate.native_cases.iter().filter_map(|name| name.strip_prefix("actions_process::")).collect::<Vec<_>>();
        assert_eq!(names.len(), 16);
        for name in names { assert!(source.contains(&format!("fn {name}()")), "{name}"); }
        assert!(!gate.native_cases.iter().any(|name| name.starts_with("actions_process.")));
        for id in ["--check-feedback", "--check-actions-menu", "--check-settings"] {
            let entries = super::settings_sources(id);
            for path in ["rust-native/tests/actions_process.rs", "rust-native/tests/support/native_actions.rs", "rust-native/tests/support/native_bridge.rs", "rust-native/tests/support/runtime_host.rs", "rust-native/tests/support/status_rpc.rs", "rust-native/tests/support/child_output.rs"] {
                let matches = entries.iter().filter(|entry| entry.0 == path).collect::<Vec<_>>();
                assert_eq!(matches.len(), 1, "{id}: {path}");
                assert_eq!(format!("{:x}", sha2::Sha256::digest(matches[0].1)), matches[0].2);
            }
        }
        assert!(include_str!("validation.rs").contains("actions-process|c|test --manifest-path rust-native/Cargo.toml --features test-tools --test actions_process --locked"));
    }
    #[test]
    fn settings_snippet_crosswalk_names_the_eleven_compiled_rust_tests() {
        let gate = super::SETTINGS.iter().find(|gate| gate.id == "--check-settings").unwrap();
        assert!(gate.required_native_steps.contains(&"snippet-process"));
        assert!(!gate.native_cases.iter().any(|case| case.starts_with("snippet_process.")));
        let source = include_str!("../../../tests/snippet_process.rs");
        let names = gate.native_cases.iter().filter_map(|case| case.strip_prefix("snippet_process::"))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), 11);
        let generated = source.lines().filter_map(|line| line.trim().strip_prefix("method!(")
            .and_then(|line| line.strip_suffix(");"))).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(generated.len(), 10);
        assert!(generated.is_subset(&names));
        // These ten names expand to actual #[test] functions, not just labels.
        let compact = source.split_whitespace().collect::<String>();
        assert!(compact.contains("macro_rules!method{($name:ident)=>{#[test]fn$name(){compare(stringify!($name));}};"));
        let direct = names.difference(&generated).copied().collect::<Vec<_>>();
        assert_eq!(direct, ["invalid_long_url_does_not_repeat_legacy_exponential_match"]);
        assert!(compact.contains(&format!("#[test]fn{}()", direct[0])));
    }
    use super::*;
    #[test]
    fn bridge_rust_crosswalk_resolves_cases_and_current_shared_inputs() {
        let gate = SETTINGS
            .iter()
            .find(|gate| gate.id == "--check-interactions")
            .unwrap();
        let inputs = settings_sources(gate.id);
        assert_eq!(inputs.len(), 49);
        assert_eq!(
            inputs
                .iter()
                .map(|entry| entry.0)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            49
        );
        let source = std::str::from_utf8(
            inputs
                .iter()
                .find(|entry| entry.0 == "rust-native/tests/bridge_process.rs")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(
            gate.native_cases
                .iter()
                .filter(|case| case.starts_with("bridge_process::"))
                .count(),
            2
        );
        assert!(!gate
            .native_cases
            .iter()
            .any(|case| case.starts_with("bridge_process.")));
        for name in [
            "show_search_and_local_native_query_share_the_owner",
            "rapid_arrows_and_click_on_other_row_keep_intended_identity",
        ] {
            let case = format!("bridge_process::{name}");
            assert!(gate.native_cases.contains(&case.as_str()));
            assert!(source.contains(&format!("fn {name}(")), "{case}");
        }
        for path in [
            "rust-native/tests/bridge_process.rs",
            "rust-native/tests/support/native_bridge.rs",
            "rust-native/tests/support/runtime_host.rs",
            "rust-native/tests/support/status_rpc.rs",
            "rust-native/tests/support/child_output.rs",
        ] {
            let (_, bytes, expected) = inputs.iter().find(|entry| entry.0 == path).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), *expected, "{path}");
        }
        for id in [
            "--check-feedback",
            "--check-actions-menu",
            "--check-interactions",
        ] {
            let (_, bytes, expected) = settings_sources(id)
                .iter()
                .find(|entry| entry.0 == "rust-native/tests/bridge_process.py")
                .unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes)),
                *expected,
                "{id}: shared Python fixture remains visible"
            );
        }
    }
    #[test]
    fn hid_goldens_bind_frozen_corpus_current_sources_and_actual_probe_copy() {
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        for name in hid_state_goldens::SOURCES {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                if *name == hid_state_goldens::PATH {
                    include_bytes!("../../../tests/hid_state_reference.json").as_slice()
                } else {
                    b"owned fixture source"
                },
            )
            .unwrap();
        }
        let alias = "rust-native/target/debug/hid-state-probe";
        let probe = "rust-native/target/owned-hid-probe";
        for name in [alias, probe] {
            fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
            fs::write(root.join(name), b"owned producer and identical probe copy").unwrap();
        }
        let binaries = serde_json::json!({alias:digest(&root.join(alias)).unwrap()});
        let sources = hid_state_goldens::SOURCES
            .iter()
            .map(|name| (*name, digest(&root.join(name)).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let original = serde_json::json!({"schema":"ruycast.hid-state-golden-binding.v1","sources":sources,"artifacts":{probe:digest(&root.join(probe)).unwrap()},"probe":probe,"goldenSHA256":hid_state_goldens::SHA256,"positiveCases":4623,"observedSnapshots":10940,"firstOperationRefusals":4});
        let raw = serde_json::to_vec(&original).unwrap();
        let qualified = qualify_hid_reference(root, &binaries, &raw).unwrap();
        assert_eq!(
            qualified["hidGoldenQualification"]["rawBindingSHA256"],
            format!("{:x}", Sha256::digest(&raw))
        );
        assert!(
            qualify_hid_reference(root, &binaries, &serde_json::to_vec(&qualified).unwrap()).is_err()
        );
        for field in [
            "goldenSHA256",
            "positiveCases",
            "observedSnapshots",
            "firstOperationRefusals",
            "probe",
        ] {
            let mut changed = original.clone();
            changed[field] = serde_json::json!("changed");
            assert!(
                qualify_hid_reference(root, &binaries, &serde_json::to_vec(&changed).unwrap()).is_err()
            );
        }
        let mut omitted = original.clone();
        omitted["sources"]
            .as_object_mut()
            .unwrap()
            .remove("rust-native/tests/support/hid_state_goldens.rs");
        assert!(
            qualify_hid_reference(root, &binaries, &serde_json::to_vec(&omitted).unwrap()).is_err()
        );
        fs::write(root.join(probe), b"different probe").unwrap();
        let mut rebound = original.clone();
        rebound["artifacts"][probe] = serde_json::json!(digest(&root.join(probe)).unwrap());
        assert!(
            qualify_hid_reference(root, &binaries, &serde_json::to_vec(&rebound).unwrap()).is_err()
        );
        fs::write(root.join(probe), b"owned producer and identical probe copy").unwrap();
        fs::write(root.join(hid_state_goldens::PATH), b"changed golden corpus").unwrap();
        rebound = original;
        rebound["sources"][hid_state_goldens::PATH] =
            serde_json::json!(digest(&root.join(hid_state_goldens::PATH)).unwrap());
        assert!(
            qualify_hid_reference(root, &binaries, &serde_json::to_vec(&rebound).unwrap()).is_err()
        );
    }
    #[test]
    fn swift_compatibility_fresh_and_plain_debug_directories_support_legacy_import() {
        use std::os::unix::fs::symlink;
        for existing_debug in [false, true] {
            let fixture = super::super::tests::Fixture::new();
            let root = &fixture.0;
            let scratch = root.join(".build/oracle-swift/owned");
            let bin = scratch.join("arm64-apple-macosx/debug");
            let source = bin.join("CRuycastCore.build/module.modulemap");
            fs::create_dir_all(source.parent().unwrap()).unwrap();
            fs::write(&source, b"fresh legacy modulemap").unwrap();
            let debug = root.join(".build/swift/debug");
            if existing_debug { fs::create_dir_all(&debug).unwrap(); }
            let binding = swift_compatibility(root, &scratch, &bin, "aarch64-apple-darwin").unwrap();
            assert!(binding.alias_target.is_none());
            assert_eq!(binding.modulemap, ".build/swift/debug/CRuycastCore.build/module.modulemap");
            data_copy(root, &source, &root.join(&binding.modulemap)).unwrap();
            // Legacy import compatibility remains for the other Swift/Core oracles.
            let legacy = debug.join("CRuycastCore.build/module.modulemap");
            assert_eq!(regular_bytes(&legacy).unwrap(), b"fresh legacy modulemap");
            let artifacts = BTreeMap::from([(binding.modulemap.clone(), digest(&legacy).unwrap())]);
            verify_swift_compatibility(root, &binding, &artifacts).unwrap();
            fs::rename(&debug, root.join(".build/swift/retained-debug")).unwrap();
            symlink("arm64-apple-macosx/debug", &debug).unwrap();
            assert!(verify_swift_compatibility(root, &binding, &artifacts).is_err());
        }
    }
    #[test]
    fn swift_compatibility_uses_physical_path_and_receipt_rejects_repointed_alias() {
        use std::os::unix::fs::symlink;
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        let scratch = root.join(".build/oracle-swift/owned");
        let bin = scratch.join("arm64-apple-macosx/debug");
        let source = bin.join("CRuycastCore.build/module.modulemap");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"owned modulemap").unwrap();
        fs::create_dir_all(root.join(".build/swift")).unwrap();
        let alias = root.join(".build/swift/debug");
        symlink("arm64-apple-macosx/debug", &alias).unwrap();
        let binding = swift_compatibility(root, &scratch, &bin, "aarch64-apple-darwin").unwrap();
        let physical = root.join(&binding.modulemap);
        data_copy(root, &source, &physical).unwrap();
        assert_eq!(fs::read_link(&alias).unwrap(), Path::new("arm64-apple-macosx/debug"));
        let artifacts = BTreeMap::from([(binding.modulemap.clone(), digest(&physical).unwrap())]);
        // Read the exact serialized field used in CoreReceipt verification.
        let captured: SwiftCompatibility = serde_json::from_slice(&serde_json::to_vec(&binding).unwrap()).unwrap();
        verify_swift_compatibility(root, &captured, &artifacts).unwrap();
        fs::remove_file(&alias).unwrap();
        symlink("x86_64-apple-macosx/debug", &alias).unwrap();
        assert!(verify_swift_compatibility(root, &captured, &artifacts).is_err());
        assert_eq!(fs::read(&physical).unwrap(), b"owned modulemap");
    }
    #[test]
    fn swift_compatibility_refuses_hostile_aliases_and_copy_escapes_before_write() {
        use std::os::unix::fs::symlink;
        let fixture = super::super::tests::Fixture::new();
        let outside = super::super::tests::Fixture::new();
        let root = &fixture.0;
        let scratch = root.join(".build/oracle-swift/owned");
        let bin = scratch.join("arm64-apple-macosx/debug");
        let source = bin.join("CRuycastCore.build/module.modulemap");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"must stay inside").unwrap();
        fs::create_dir_all(root.join(".build/swift")).unwrap();
        let alias = root.join(".build/swift/debug");
        let sentinel = outside.0.join("sentinel"); fs::write(&sentinel, b"unchanged").unwrap();
        for target in [outside.0.clone(), PathBuf::from("../escape"), PathBuf::from("arm64-apple-macosx/../debug"), PathBuf::from("arm64-apple-macosx/./debug"), PathBuf::from("arm64-apple-macosx//debug"), PathBuf::from("x86_64-apple-macosx/debug")] {
            symlink(target, &alias).unwrap();
            assert!(swift_compatibility(root, &scratch, &bin, "aarch64-apple-darwin").is_err());
            assert!(!root.join(".build/swift/arm64-apple-macosx/debug/CRuycastCore.build/module.modulemap").exists());
            assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
            fs::remove_file(&alias).unwrap();
        }
        assert!(data_copy(root, &source, &sentinel).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
        assert!(data_copy(root, &source, &root.join("../escaped-modulemap")).is_err());
        // An otherwise correct alias cannot permit a linked physical parent.
        symlink("arm64-apple-macosx/debug", &alias).unwrap();
        symlink(&outside.0, root.join(".build/swift/arm64-apple-macosx")).unwrap();
        let binding = swift_compatibility(root, &scratch, &bin, "aarch64-apple-darwin").unwrap();
        assert!(data_copy(root, &source, &root.join(binding.modulemap)).is_err());
        assert!(!outside.0.join("debug/CRuycastCore.build/module.modulemap").exists());
    }
    #[test]
    fn settings_crosswalk_preserves_every_original_selector_in_order() {
        let original = include_str!("../../../../scripts/check-settings-locked.sh");
        let ids = original
            .lines()
            .filter_map(|line| line.strip_prefix(".build/swift/debug/RuycastApp "))
            .filter_map(|line| line.split_whitespace().next())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 16);
        assert_eq!(SETTINGS.iter().map(|gate| gate.id).collect::<Vec<_>>(), ids);
        assert_eq!(
            ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
            16
        );
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        for gate in SETTINGS {
            regular_file(&root.join(gate.original_source)).unwrap();
        }
    }
    #[test]
    fn settings_mapping_only_names_existing_matrix_steps_and_real_log_cases() {
        let plan = validation::plan("aarch64-apple-darwin").unwrap();
        let mut names = plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["name"].as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        for render in plan["renders"].as_array().unwrap() {
            names.insert(format!("native-{}-{}", render[0].as_str().unwrap(), render[1].as_str().unwrap()));
        }
        for gate in SETTINGS {
            assert!(!gate.required_native_steps.is_empty(), "{}", gate.id);
            for step in gate.required_native_steps {
                assert!(names.contains(*step), "{}: {step}", gate.id);
            }
        }
        let log = SETTINGS
            .iter()
            .find(|gate| gate.id == "--check-command-log")
            .unwrap();
        assert_eq!(log.native_cases.len(), 13);
        assert_eq!(log.required_native_steps, &["unit", "private-results-process"]);
        assert_eq!(settings_sources(log.id).len(), 8);
        let sources = [
            ("activity::tests::", include_str!("../../activity.rs")),
            ("private_results::", include_str!("../../../tests/private_results.rs")),
            ("runtime::system::shortcut_budget_tests::", include_str!("../../runtime_system.rs")),
        ];
        for case in log.native_cases {
            let (_,source)=sources.iter().find(|(prefix,_)|case.starts_with(*prefix)).expect("mapped command log source");
            assert!(
                source.contains(&format!("fn {}(", case.rsplit("::").next().unwrap())),
                "{case}"
            );
        }
    }
    #[test]
    fn browser_and_performance_maps_name_existing_assertions_and_complete_routes() {
        let sources = [
            (
                "runtime::system::shortcut_budget_tests::",
                include_str!("../../runtime_system.rs"),
            ),
            (
                "system_search::providers::gate_tests::",
                include_str!("../../system_search/providers.rs"),
            ),
            (
                "system_search::tests::",
                include_str!("../../system_search/tests.rs"),
            ),
            (
                "system_search_sdk::",
                include_str!("../../../tests/system_search_sdk.rs"),
            ),
            ("system_search_cli_process::", include_str!("../../../tests/system_search_cli_process.rs")),
            (
                "runtime::inline_tests::",
                include_str!("../../runtime_private_tests.rs"),
            ),
            (
                "configuration_jobs::tests::",
                include_str!("../../configuration_jobs.rs"),
            ),
            (
                "clipboard_history::tests::",
                include_str!("../../clipboard_history.rs"),
            ),
            (
                "native_services::",
                include_str!("../../native_services.rs"),
            ),
            (
                "clipboard_rich_paste::",
                include_str!("../../../tests/clipboard_rich_paste.rs"),
            ),
            (
                "clipboard_history_process.",
                include_str!("../../../tests/clipboard_history_process.py"),
            ),
        ];
        for (id, expected_steps, expected_sources) in [
            (
                "--check-browser-tabs",
                &["unit",  "system-search-cli-process", "system-search-sdk"][..],
                31,
            ),
            (
                "--check-performance-pipeline",
                &["unit", "clipboard-history-process", "clipboard-rich-paste"][..],
                29,
            ),
        ] {
            let gate = SETTINGS.iter().find(|gate| gate.id == id).unwrap();
            assert_eq!(gate.required_native_steps, expected_steps);
            assert_eq!(settings_sources(id).len(), expected_sources);
            assert!(gate.unproven_invariants.is_empty());
            assert!(!gate.native_cases.is_empty());
            for case in gate.native_cases {
                let (prefix, source) = sources
                    .iter()
                    .find(|(prefix, _)| case.starts_with(*prefix))
                    .unwrap();
                let name = case
                    .strip_prefix(*prefix)
                    .unwrap()
                    .rsplit('.')
                    .next()
                    .unwrap();
                assert!(
                    source.contains(&format!("fn {name}("))
                        || source.contains(&format!("def {name}(")),
                    "{id}: {case}"
                );
            }
        }
    }
    #[test]
    fn window_shortcuts_and_render_maps_preserve_named_cases_routes_and_source_inputs() {
        let case_sources = [
            ("system_search::tests::", "rust-native/src/system_search/tests.rs"),
            ("system_search::providers::gate_tests::", "rust-native/src/system_search/providers.rs"),
            ("system_search::apple_shortcuts::tests::", "rust-native/src/system_search/apple_shortcuts.rs"),
            ("runtime::system::shortcut_budget_tests::", "rust-native/src/runtime_system.rs"),
            ("system_search_sdk::", "rust-native/tests/system_search_sdk.rs"),
            ("system_search_cli_process::", "rust-native/tests/system_search_cli_process.rs"),
            ("script_result_basic_process::", "rust-native/tests/script_result_basic_process.rs"),
            ("ui_completion::", "rust-native/examples/ui_completion.rs"),
        ];
        for (id, steps, case_count, source_count) in [
            ("--check-window-search", &["unit",  "system-search-cli-process", "system-search-sdk"][..], 18, 34),
            ("--check-apple-shortcuts", &["unit",  "system-search-cli-process", "system-search-sdk", "private-results-process", "script-process", "script-result-process", "script-result-native-process"][..], 10, 46),
            ("--check-render-updates", &["render-updates-process"][..], 1, 14),
        ] {
            let gate = SETTINGS.iter().find(|gate| gate.id == id).unwrap();
            assert_eq!(gate.required_native_steps, steps);
            assert!(gate.unproven_invariants.is_empty());
            assert_eq!(gate.native_cases.len(), case_count);
            let inputs = settings_sources(id);
            assert_eq!(inputs.len(), source_count);
            assert!(inputs.iter().any(|(path, _, _)| *path == gate.original_source));
            assert_eq!(inputs.iter().map(|(path, _, _)| *path).collect::<std::collections::BTreeSet<_>>().len(), source_count);
            for case in gate.native_cases {
                let (prefix, path) = case_sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
                let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
                let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
                assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{id}: {case}");
                if prefix.ends_with('.') {
                    let class = case.strip_prefix(*prefix).unwrap().split('.').next().unwrap();
                    assert!(source.contains(&format!("class {class}(")), "{id}: {case}");
                }
            }
        }
        assert!(require_complete("settings").is_ok());
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }
    #[test]
    fn menu_map_preserves_original_cases_required_routes_and_source_inputs() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-menu-actions").unwrap();
        assert_eq!(gate.required_native_steps, &["unit",  "system-search-cli-process", "system-search-sdk"]);
        assert_eq!(gate.native_cases.len(), 22);
        assert!(gate.unproven_invariants.is_empty());
        let inputs = settings_sources(gate.id);
        assert_eq!(inputs.len(), 38);
        assert!(inputs.iter().any(|(path, _, _)| *path == gate.original_source));
        assert_eq!(inputs.iter().map(|(path, _, _)| *path).collect::<std::collections::BTreeSet<_>>().len(), 38);
        let sources = [
            ("system_search::tests::", "rust-native/src/system_search/tests.rs"),
            ("system_search::native::tests::", "rust-native/src/system_search/native.rs"),
            ("runtime::system::shortcut_budget_tests::", "rust-native/src/runtime_system.rs"),
            ("system_search_sdk::", "rust-native/tests/system_search_sdk.rs"),
            ("system_search_cli_process::", "rust-native/tests/system_search_cli_process.rs"),
        ];
        for case in gate.native_cases {
            let (prefix, path) = sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
            let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
            let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
            assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{}: {case}", gate.id);
        }
        assert!(require_complete("settings").is_ok());
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }
    #[test]
    fn rich_resources_map_carries_original_groups_and_existing_preview_route() {
        let gate = SETTINGS.iter().find(|gate| gate.id == "--check-rich-resources").unwrap();
        assert_eq!(gate.required_native_steps, &["unit", "clipboard-preview-original", "remaining-macos-process", "clipboard-history-process", "clipboard-rich-paste"]);
        assert_eq!(gate.native_cases.len(), 16);
        assert!(gate.unproven_invariants.is_empty());
        let inputs = settings_sources(gate.id);
        assert_eq!(inputs.len(), 40);
        assert!(inputs.iter().any(|(path, _, _)| *path == gate.original_source));
        assert_eq!(inputs.iter().map(|(path, _, _)| *path).collect::<std::collections::BTreeSet<_>>().len(), 40);
        let sources = [
            ("remaining_macos_process::", "rust-native/tests/remaining_macos_process.rs"),
            ("clipboard_preview::tests::", "rust-native/src/clipboard_preview.rs"),
            ("system_search::providers::gate_tests::", "rust-native/src/system_search/providers.rs"),
            ("processes::tests::", "rust-native/src/processes.rs"),
            ("monitor::lifecycle_tests::", "rust-native/src/monitor.rs"),
            ("clipboard_history_process.", "rust-native/tests/clipboard_history_process.py"),
            ("clipboard_rich_paste::", "rust-native/tests/clipboard_rich_paste.rs"),
        ];
        for case in gate.native_cases {
            let (prefix, path) = sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
            let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
            let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
            assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{}: {case}", gate.id);
        }
        assert!(require_complete("settings").is_ok());
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }
    #[test]
    fn polish_composition_and_partial_original_gates_keep_precise_cases_and_refusals() {
        let sources = [
            ("ui_completion::", "rust-native/examples/ui_completion.rs"),
            ("native_menu_bar::tests::", "rust-native/src/native_menu_bar.rs"),
            ("runtime_health::tests::", "rust-native/src/runtime_health.rs"),
            ("management_commands::catalog_tests::", "rust-native/src/management_commands.rs"),
            ("clipboard_preview::tests::", "rust-native/src/clipboard_preview.rs"),
            ("task_date::tests::", "rust-native/src/task_date.rs"),
            ("remaining_macos_process::", "rust-native/tests/remaining_macos_process.rs"),
            ("utility_process::", "rust-native/tests/utility_process.rs"),
            ("actions::tests::", "rust-native/src/actions.rs"),
            ("processes::tests::", "rust-native/src/processes.rs"),
            ("system_search::providers::gate_tests::", "rust-native/src/system_search/providers.rs"),
            ("actions_process::", "rust-native/tests/actions_process.rs"),
            ("system_search::tests::", "rust-native/src/system_search/tests.rs"),
            ("system_actions::tests::", "rust-native/src/system_actions.rs"),
            ("file_search::tests::", "rust-native/src/file_search.rs"),
            ("system_search_cli_process::", "rust-native/tests/system_search_cli_process.rs"),
            ("native_edit::", "rust-native/src/native_edit.rs"),
            ("native_snippet_choice_process.", "rust-native/tests/native_snippet_choice_process.py"),
            ("native_keyboard_process::", "rust-native/tests/native_keyboard_process.rs"),
            ("bridge_process::", "rust-native/tests/bridge_process.rs"),
            ("input_process.", "rust-native/tests/input_process.py"),
            ("input_process::", "rust-native/tests/input_process.rs"),
            ("native_open_basic_process::", "rust-native/tests/native_open_basic_process.rs"),
            ("creation_process::", "rust-native/tests/creation_process.rs"),
            ("library_lifecycle_process::", "rust-native/tests/library_lifecycle_process.rs"),
            ("library_checkbox_process::", "rust-native/tests/library_checkbox_process.rs"),
            ("library_drafts_process::", "rust-native/tests/library_drafts_process.rs"),
            ("library_navigation_process::", "rust-native/tests/library_navigation_process.rs"),
            ("settings_preferences_process::", "rust-native/tests/settings_preferences_process.rs"),
            ("native_choice_host_process::", "rust-native/tests/native_choice_host_process.rs"),
        ];
        for (id, count, input_count, complete) in [
            ("--check-polish", 10, 33, true),
            ("--check-feedback", 21, 40, true),
            ("--check-actions-menu", 11, 29, true),
            ("--check-interactions", 16, 48, true),
        ] {
            let gate = SETTINGS.iter().find(|gate| gate.id == id).unwrap();
            assert_eq!(gate.native_cases.len(), count);
            assert_eq!(gate.unproven_invariants.is_empty(), complete);
            let inputs = settings_sources(id);
            assert_eq!(inputs.len(), input_count);
            assert_eq!(inputs.iter().map(|(path, _, _)| *path).collect::<std::collections::BTreeSet<_>>().len(), input_count);
            assert!(inputs.iter().any(|(path, _, _)| *path == gate.original_source));
            for case in gate.native_cases {
                let (prefix, path) = sources.iter().find(|(prefix, _)| case.starts_with(*prefix)).unwrap();
                let source = std::str::from_utf8(inputs.iter().find(|(input, _, _)| input == path).unwrap().1).unwrap();
                let name = case.strip_prefix(*prefix).unwrap().rsplit('.').next().unwrap();
                assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{id}: {case}");
                if prefix.ends_with('.') {
                    let class = case.strip_prefix(*prefix).unwrap().split('.').next().unwrap();
                    assert!(source.contains(&format!("class {class}(")), "{id}: {case}");
                }
            }
            if !complete {
                assert_eq!(settings_mapping_state(gate), "mapped-with-unproven-invariants");
            }
        }
        let polish = SETTINGS.iter().find(|gate| gate.id == "--check-polish").unwrap();
        assert_eq!(polish.required_native_steps, &["unit", "polish-original-process", "script-advanced-process", "task-date-process", "polish-bindings-process", "remaining-macos-process", "clipboard-preview-cancel-original"]);
        let matrix = include_str!("validation.rs");
        assert!(matrix.contains("polish-original-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --polish-original-only"));
        assert!(matrix.contains("clipboard-preview-cancel-original|c|test --manifest-path rust-native/Cargo.toml --features test-tools --lib clipboard_preview::tests::cancelled_same_id_reopen_cannot_publish_old_error_or_asset --locked -- --exact"));
        assert!(matrix.contains("actions-menu-process|c|run --manifest-path rust-native/Cargo.toml --features test-tools --example ui_completion --locked -- --actions-menu-only"));
        assert!(require_complete("settings").is_ok());
        assert_eq!(inventory("settings").unwrap()["productionAccepted"], false);
    }
    #[test]
    fn complete_settings_mapping_does_not_accept_original_check_or_unknown_inventory() {
        let value = inventory("settings").unwrap();
        let unresolved = value["unmappedGateIDs"].as_array().unwrap();
        assert!(unresolved.is_empty());
        assert!(!unresolved.contains(&serde_json::json!("--check-interactions")));
        assert!(!unresolved.contains(&serde_json::json!("--check-feedback")));
        assert!(!unresolved.contains(&serde_json::json!("--check-actions-menu")));
        assert!(!unresolved.contains(&serde_json::json!("--check-command-log")));
        assert!(!unresolved.contains(&serde_json::json!("--check-edit-commands")));
        assert!(!unresolved.contains(&serde_json::json!("--check-analytics")));
        assert!(!unresolved.contains(&serde_json::json!("--check-performance-pipeline")));
        assert!(!unresolved.contains(&serde_json::json!("--check-browser-tabs")));
        assert!(!unresolved.contains(&serde_json::json!("--check-menu-actions")));
        assert!(!unresolved.contains(&serde_json::json!("--check-rich-resources")));
        assert!(!unresolved.contains(&serde_json::json!("--check-polish")));
        assert!(!unresolved.contains(&serde_json::json!("--check-window-search")));
        assert!(!unresolved.contains(&serde_json::json!("--check-apple-shortcuts")));
        assert!(!unresolved.contains(&serde_json::json!("--check-render-updates")));
        assert!(!unresolved.contains(&serde_json::json!("--check-settings")));
        assert!(!unresolved.contains(&serde_json::json!("--check-visual-assets")));
        assert!(value["settingsCaseCrosswalk"]
            .as_array()
            .unwrap()
            .iter()
            .all(|gate| [
                "--check-command-log",
                "--check-edit-commands",
                "--check-analytics",
                "--check-visual-assets",
                "--check-settings",
                "--check-browser-tabs",
                "--check-performance-pipeline",
                "--check-window-search",
                "--check-apple-shortcuts",
                "--check-render-updates",
                "--check-menu-actions",
                "--check-rich-resources",
                "--check-polish",
                "--check-feedback",
                "--check-actions-menu",
                "--check-interactions",
            ]
            .iter()
            .any(|id| gate["id"] == *id)
                || !gate["unprovenInvariants"].as_array().unwrap().is_empty()));
        assert!(require_complete("settings").is_ok());
        assert_eq!(inventory("check").unwrap()["unmappedGateIDs"], serde_json::json!(["python-unittest-discover-Tests-test_*.py"]));
        assert_eq!(root_git_mapping_state(b"unreviewed compatibility source"), "requires-source-requalification");
        assert!(require_complete("check").is_err());
        assert!(require_complete("unknown").is_err());
        assert_eq!(value["productionAccepted"], false);
    }
    #[test]
    fn absent_sources_and_explicit_unproven_invariants_still_refuse_qualification() {
        let unknown = SettingsGate {
            id: "--not-reviewed", original_source: "Sources/RuycastApp/PolishChecks.swift",
            required_native_steps: &[], native_cases: &["owned-case"], unproven_invariants: &[],
        };
        assert_eq!(settings_mapping_state(&unknown), "mapped-requires-source-requalification");
        let unproven = SettingsGate {
            id: "--check-polish", unproven_invariants: &["not yet proved"], ..unknown
        };
        assert_eq!(settings_mapping_state(&unproven), "mapped-with-unproven-invariants");
    }
}
