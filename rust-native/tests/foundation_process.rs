//! Private filesystem publication and configuration behavior with owned faults.
#![cfg(feature = "test-tools")]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

struct Fixture {
    base: PathBuf,
    result: Value,
    parent_acl: Vec<String>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
fn command_output(command: &mut Command) -> std::process::Output {
    child_output::wait(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        Duration::from_secs(4),
    )
}
fn acl(path: &Path, full: bool) -> Vec<String> {
    let output = command_output(
        Command::new("/bin/ls")
            .arg(if full { "-lde" } else { "-le" })
            .arg(path),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}
fn fixture(
    mode: &str,
    inject: Option<&str>,
    mutation: Option<&str>,
    portable_acl: bool,
) -> Fixture {
    let parent = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    assert!(parent.is_absolute());
    fs::create_dir_all(&parent).unwrap();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = parent.join(format!(
        "foundation-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
    let mut owned = Fixture {
        base,
        result: Value::Null,
        parent_acl: Vec::new(),
    };
    let profile = owned.base.join("profile");
    fs::DirBuilder::new().mode(0o700).create(&profile).unwrap();
    let config = include_bytes!("../../config/default.json");
    fs::write(profile.join("config.json"), config).unwrap();
    fs::set_permissions(
        profile.join("config.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    if portable_acl {
        let portable = owned.base.join("portable");
        fs::DirBuilder::new().mode(0o755).create(&portable).unwrap();
        fs::set_permissions(&portable, fs::Permissions::from_mode(0o755)).unwrap();
        let output=command_output(Command::new("/bin/chmod").args(["+a","everyone allow read,execute,readattr,readextattr,readsecurity,file_inherit,directory_inherit"]).arg(&portable));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        owned.parent_acl = acl(&portable, true).into_iter().skip(1).collect();
        fs::write(portable.join("config.json"), config).unwrap();
        fs::write(
            profile.join("config-location.json"),
            serde_json::to_vec(&json!({"directory":portable})).unwrap(),
        )
        .unwrap();
    }
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_foundation-probe"));
    assert!(executable.is_absolute() && executable.is_file());
    let mut command = Command::new(&executable);
    command
        .env_clear()
        .arg(mode)
        .arg(&owned.base)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Only the child changes its umask; no parent or parallel test is affected.
    unsafe {
        command.pre_exec(|| {
            libc::umask(0o022);
            Ok(())
        });
    }
    if let Some(directory) = inject {
        let sdk = std::env::var_os("RUYCAST_TEST_FCNTL_SHIM")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                executable
                    .parent()
                    .unwrap()
                    .join("examples/libfcntl_sync_shim.dylib")
            });
        assert!(
            sdk.is_absolute() && sdk.is_file(),
            "Build fcntl-sync-shim first"
        );
        let copy = owned.base.join("fcntl-fixture.dylib");
        fs::copy(&sdk, &copy).unwrap();
        assert_eq!(fs::read(&sdk).unwrap(), fs::read(&copy).unwrap());
        command
            .env("DYLD_INSERT_LIBRARIES", copy)
            .env("RUYCAST_TEST_FSYNC_DIRECTORY", owned.base.join(directory))
            .env("RUYCAST_TEST_FSYNC_MARKER", owned.base.join("injected"));
        if let Some(mutation) = mutation {
            command.env("RUYCAST_TEST_FSYNC_MUTATE", mutation);
            if mutation == "selector" {
                let external = owned.base.join("external");
                fs::DirBuilder::new().mode(0o700).create(&external).unwrap();
                fs::write(external.join("config.json"), config).unwrap();
                command.env(
                    "RUYCAST_TEST_SELECTOR_JSON",
                    json!({"directory":external}).to_string(),
                );
            }
            if mutation == "target" {
                command.env(
                    "RUYCAST_TEST_MUTATION_TARGET",
                    owned.base.join("portable/config.json"),
                );
            }
        }
    }
    let output = child_output::wait(command.spawn().unwrap(), Duration::from_secs(10));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if inject.is_some() {
        assert_eq!(fs::read(owned.base.join("injected")).unwrap(), b"injected");
    }
    owned.result = serde_json::from_slice(&output.stdout).unwrap();
    owned
}
fn injected(mode: &str) -> Fixture {
    fixture(mode, Some("profile"), None, false)
}
fn json_file(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn inline_followup_error_preserves_the_real_postpublication_warning() {
    let f = injected("inline-followup-warning");
    let r = &f.result;
    assert_eq!(r["warning"], true);
    assert!(r["error"].as_str().unwrap().contains("removed"));
    assert_eq!(r["opened"], false);
    assert_eq!(r["draftRetained"], false);
    assert_eq!(r["aliases"], json!(["published-inline"]));
    assert_eq!(r["fileEqualsSnapshot"], true);
}
#[test]
fn inline_followup_cancel_preserves_the_real_postpublication_warning() {
    let f = injected("inline-cancel-warning");
    let r = &f.result;
    assert_eq!(r["warning"], true);
    assert!(r["error"].is_null());
    assert_eq!(r["opened"], false);
    assert_eq!(r["draftRetained"], false);
    assert_eq!(r["aliases"], json!(["published-inline"]));
    assert_eq!(r["fileEqualsSnapshot"], true);
}
#[test]
fn new_command_fsync_warning_preserves_published_state() {
    let f = injected("new-command");
    let r = &f.result;
    assert_eq!(r["ok"], true);
    assert_eq!(r["durable"], false);
    assert_eq!(r["warning"], true);
    assert_eq!(r["countAdded"], 1);
    assert_eq!(r["draftRetained"], false);
    assert_eq!(r["fileEqualsSnapshot"], true);
}
#[test]
fn new_command_external_postpublish_edit_is_not_overwritten() {
    let f = fixture("new-command", Some("profile"), Some("replace"), false);
    let r = &f.result;
    assert_eq!(r["ok"], false);
    assert_eq!(r["draftRetained"], true);
    assert_eq!(r["fileEqualsSnapshot"], false);
    assert_eq!(
        fs::read_to_string(f.base.join("profile/config.json")).unwrap(),
        "external fixture edit"
    );
    assert!(f.base.join("profile/published.copy").exists());
}
#[test]
fn prefill_uses_only_a_native_named_disposable_pasteboard() {
    let f = fixture("clipboard-prefill", None, None, false);
    let r = &f.result;
    assert_eq!(r["prefill"], "https://example.com/path");
    assert_eq!(r["emptyIgnored"], true);
    assert_eq!(r["largeTextRetained"], true);
    assert_eq!(r["generalPasteboardRead"], false);
}
#[test]
fn actual_configuration_reload_updates_only_valid_catalogs() {
    let f = fixture("state", None, None, false);
    assert_eq!(f.result["ok"], true);
    assert_eq!(f.result["lastGoodCatalogRetained"], true);
    assert_eq!(f.result["selectedRemovedWithoutReplacement"], true);
}
#[test]
fn successful_choice_refreshes_ranking_without_stealing_selection() {
    let f = fixture("ranking", None, None, false);
    assert_eq!(f.result["immediateRankRefresh"], true);
    assert_eq!(f.result["selectedIdentityPreserved"], true);
    assert_eq!(f.result["actionsPreserved"], true);
}
#[test]
fn binding_save_after_directory_sync_failure_keeps_published_state_and_warning() {
    let f = injected("binding");
    let r = &f.result;
    assert_eq!(r["ok"], true);
    assert_eq!(r["durable"], false);
    assert_eq!(r["warning"], true);
    assert_eq!(r["aliases"], json!(["changed-alias"]));
    assert_eq!(r["fileEqualsSnapshot"], true);
    assert_eq!(r["draftRetained"], false);
}
#[test]
fn disable_after_directory_sync_failure_keeps_explicit_warning() {
    let f = injected("disable");
    let r = &f.result;
    assert_eq!(r["ok"], true);
    assert_eq!(r["durable"], false);
    assert_eq!(r["warning"], true);
    assert_eq!(r["enabled"], false);
    assert_eq!(r["fileEqualsSnapshot"], true);
}
#[test]
fn binding_postpublication_external_edit_keeps_external_data_and_draft() {
    let f = fixture("binding", Some("profile"), Some("replace"), false);
    let r = &f.result;
    assert_eq!(r["ok"], false);
    assert_eq!(r["draftRetained"], true);
    assert_eq!(r["fileEqualsSnapshot"], false);
    assert_eq!(
        fs::read_to_string(f.base.join("profile/config.json")).unwrap(),
        "external fixture edit"
    );
    assert_eq!(
        json_file(&f.base.join("profile/published.copy"))["commands"][0]["aliases"],
        json!(["changed-alias"])
    );
}
#[test]
fn directory_fsync_failure_adopts_published_save_with_warning() {
    let f = injected("save");
    let r = &f.result;
    assert_eq!(r["ok"], true);
    assert_eq!(r["durable"], false);
    assert_eq!(r["warning"], true);
    assert_eq!(r["activeLimit"], 51);
    assert_eq!(r["revision"], 1);
    assert_eq!(r["fileEqualsSnapshot"], true);
    assert_eq!(
        json_file(&f.base.join("profile/config.json"))["clipboard"]["limit"],
        51
    );
}
#[test]
fn external_replace_after_publish_is_not_adopted_or_overwritten() {
    let f = fixture("save", Some("profile"), Some("replace"), false);
    let r = &f.result;
    assert_eq!(r["ok"], false);
    assert_eq!(r["revision"], 0);
    assert_eq!(r["activeLimit"], 200);
    assert_eq!(
        fs::read(f.base.join("profile/config.json")).unwrap(),
        b"external fixture edit"
    );
    assert_eq!(
        json_file(&f.base.join("profile/published.copy"))["clipboard"]["limit"],
        51
    );
}
#[test]
fn destination_fsync_failure_cleans_only_owned_copy() {
    let f = fixture("relocate", Some("portable"), None, false);
    let r = &f.result;
    assert_eq!(r["ok"], false);
    assert_eq!(r["sourcePreserved"], true);
    assert_eq!(r["targetExists"], false);
    assert_eq!(r["selectorExists"], false);
}
#[test]
fn destination_replacement_is_preserved_on_failed_move() {
    let f = fixture("relocate", Some("portable"), Some("replace"), false);
    let r = &f.result;
    assert_eq!(r["ok"], false);
    assert_eq!(r["sourcePreserved"], true);
    assert_eq!(r["targetExists"], true);
    assert_eq!(r["selectorExists"], false);
    assert_eq!(
        fs::read(f.base.join("portable/config.json")).unwrap(),
        b"external fixture edit"
    );
}
#[test]
fn published_selector_fsync_failure_switches_to_actual_target() {
    let f = injected("relocate");
    let r = &f.result;
    assert_eq!(r["ok"], true);
    assert_eq!(r["durable"], false);
    assert_eq!(r["warning"], true);
    assert_eq!(r["activeTarget"], true);
    assert_eq!(r["sourcePreserved"], true);
    assert_eq!(
        json_file(&f.base.join("profile/config-location.json"))["directory"],
        f.base.join("portable").to_str().unwrap()
    );
}
#[test]
fn portable_parent_acl_and_mode_remain_while_new_payload_is_private() {
    let f = fixture("save", None, None, true);
    assert_eq!(f.result["ok"], true);
    let portable = f.base.join("portable");
    assert_eq!(
        fs::metadata(&portable).unwrap().permissions().mode() & 0o7777,
        0o755
    );
    assert_eq!(
        acl(&portable, true).into_iter().skip(1).collect::<Vec<_>>(),
        f.parent_acl
    );
    let target = portable.join("config.json");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
        0o600
    );
    assert_eq!(acl(&target, false).len(), 1);
}
#[test]
fn selector_changed_during_save_is_not_falsely_adopted() {
    let f = fixture("save", Some("profile"), Some("selector"), false);
    let r = &f.result;
    assert_eq!(r["ok"], false);
    assert_eq!(r["revision"], 0);
    assert_eq!(r["activeLimit"], 200);
    assert_eq!(
        json_file(&f.base.join("profile/config-location.json"))["directory"],
        f.base.join("external").to_str().unwrap()
    );
}
#[test]
fn selector_changed_after_move_publication_keeps_recovery_copy() {
    let f = fixture("relocate", Some("profile"), Some("selector"), false);
    let r = &f.result;
    assert_eq!(r["ok"], false);
    assert_eq!(r["activeTarget"], false);
    assert_eq!(r["sourcePreserved"], true);
    assert_eq!(r["targetExists"], true);
    assert_eq!(
        json_file(&f.base.join("profile/config-location.json"))["directory"],
        f.base.join("external").to_str().unwrap()
    );
}
#[test]
fn target_changed_after_selector_publication_is_preserved() {
    let f = fixture("relocate", Some("profile"), Some("target"), false);
    let r = &f.result;
    assert_eq!(r["ok"], false);
    assert_eq!(r["sourcePreserved"], true);
    assert_eq!(r["activeTarget"], false);
    assert_eq!(
        fs::read(f.base.join("portable/config.json")).unwrap(),
        b"external fixture edit"
    );
}
