//! Offline owned configuration contract; never starts AppKit or global services.
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
struct Fixture {
    base: PathBuf,
    root: PathBuf,
    host: PathBuf,
    cli: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("owned short fixture parent required"),
        );
        assert!(parent.is_absolute());
        let base = parent.join(format!(
            "c-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&base).unwrap();
        let root = base.join("p");
        fs::create_dir(&root).unwrap();
        let cli = PathBuf::from(
            std::env::var_os("RUYCAST_REPAIR_CLI").expect("explicit repaired CLI required"),
        );
        assert!(cli.is_absolute());
        Self {
            base,
            root,
            host: env!("CARGO_BIN_EXE_ruycast-native").into(),
            cli,
        }
    }
    fn command(&self, native: bool) -> Command {
        let mut c = Command::new(if native { &self.host } else { &self.cli });
        c.env_clear()
            .current_dir(&self.base)
            .env("HOME", &self.base)
            .env("RUYCAST_HOME", &self.root)
            .env("RUYCAST_APP_BINARY", &self.host);
        c
    }
    fn source(&self) {
        fs::write(
            self.root.join("config.json"),
            include_bytes!("../../config/default.json"),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}
fn value(output: Output, ok: bool) -> Value {
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        v["ok"],
        ok,
        "{v}; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.success(), ok);
    v
}
fn bounded_value(command: &mut Command, ok: bool) -> Value {
    let output = child_output::wait(
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        Duration::from_secs(10),
    );
    assert_eq!(output.status.code(), Some(if ok { 0 } else { 1 }));
    value(output, ok)
}
fn rejected_activation(external: bool) {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::time::Instant;
    struct OwnedCli(Option<std::process::Child>);
    impl Drop for OwnedCli {
        fn drop(&mut self) {
            if let Some(child) = self.0.as_mut() {
                if matches!(child.try_wait(), Ok(None)) {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
        }
    }
    let f = Fixture::new();
    f.source();
    let config = f.root.join("config.json");
    let original = fs::read(&config).unwrap();
    let mut newer = original.clone();
    newer.extend_from_slice(b"\n\n");
    let listener = UnixListener::bind(f.root.join("control.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    let until = Instant::now() + Duration::from_secs(8);
    let mut child = OwnedCli(Some(
        f.command(false)
            .args(["config", "set", "clipboard.limit", "51"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let accept = || -> UnixStream {
        let deadline = until.min(Instant::now() + Duration::from_secs(5));
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    let left = until.saturating_duration_since(Instant::now());
                    assert!(!left.is_zero(), "activation request deadline");
                    stream.set_read_timeout(Some(left)).unwrap();
                    stream.set_write_timeout(Some(left)).unwrap();
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "activation accept deadline");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("activation accept: {error}"),
            }
        }
    };
    let request = |mut stream: &UnixStream| -> Value {
        let mut line = Vec::new();
        for _ in 0..1024 * 1024 {
            let left = until.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "activation request deadline");
            stream.set_read_timeout(Some(left)).unwrap();
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            if byte[0] == b'\n' {
                return serde_json::from_slice(&line).unwrap();
            }
            line.push(byte[0]);
        }
        panic!("activation request exceeds one MiB");
    };
    {
        let mut stream = accept();
        assert_eq!(request(&stream)["op"], "reload");
        if external {
            fs::write(&config, &newer).unwrap();
        }
        stream
            .write_all(b"{\"ok\":false,\"error\":\"Hotkey unavailable\"}\n")
            .unwrap();
    }
    if !external {
        let mut stream = accept();
        assert_eq!(request(&stream)["op"], "reload");
        assert_eq!(fs::read(&config).unwrap(), original);
        stream.write_all(b"{\"ok\":true}\n").unwrap();
    }
    let remaining = until.saturating_duration_since(Instant::now());
    assert!(!remaining.is_zero(), "activation CLI deadline");
    let output = child_output::wait(child.0.take().unwrap(), remaining);
    assert_eq!(output.status.code(), Some(1));
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], false);
    assert_eq!(response["rolledBack"], !external);
    assert_eq!(response["runtimeRecovered"], !external);
    assert_eq!(
        fs::read(config).unwrap(),
        if external { newer } else { original }
    );
}
#[test]
fn rejected_candidate_restores_previous_bytes_and_runtime() {
    rejected_activation(false);
}
#[test]
fn newer_external_edit_is_never_rolled_back() {
    rejected_activation(true);
}
#[test]
fn raycast_import_defaults_to_active_cli_config() {
    let f = Fixture::new();
    f.source();
    let original = fs::read(f.root.join("config.json")).unwrap();
    let custom = f.base.join("Synced vault with spaces/.appdata/ruycast");
    bounded_value(
        f.command(false).args(["config", "directory"]).arg(&custom),
        true,
    );
    bounded_value(
        f.command(false).args(["config", "add", &json!({"id":"fixture.portable","title":"Portable","kind":"url","target":"https://example.com"}).to_string()]),
        true,
    );
    let export = f.base.join("raycast-export.json");
    let data = serde_json::to_vec(&json!({"builtin_package_rootSearch":{"rootSearch":[]},"raycast_user_activity":{"activityInfo":[]},"builtin_package_quicklinks":{"quicklinks":[]}})).unwrap();
    fs::write(&export, &data).unwrap();
    let selected = fs::read(custom.join("config.json")).unwrap();
    let output = child_output::wait(
        Command::new(env!("CARGO_BIN_EXE_ruycast-build"))
            .env_clear()
            .current_dir(&f.base)
            .env("HOME", &f.base)
            .env("RUYCAST_HOME", &f.root)
            .env("RUYCAST_APP_BINARY", &f.host)
            .arg("import-raycast")
            .arg(&export)
            .arg("--cli")
            .arg(&f.cli)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        Duration::from_secs(10),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    let config: Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(
        summary["totalCommands"],
        config["commands"].as_array().unwrap().len() + 1
    );
    assert_eq!(fs::read(f.root.join("config.json")).unwrap(), original);
    assert_eq!(fs::read(custom.join("config.json")).unwrap(), selected);
    assert_eq!(fs::read(export).unwrap(), data);
}
#[test]
fn both_profile_and_config_aliases_select_the_same_owned_write_target() {
    for legacy_profile in [false, true] {
        for key in ["RUYCAST_CONFIG_DIR", "RUSTCAST_CONFIG_DIR"] {
            let f = Fixture::new();
            f.source();
            let original = fs::read(f.root.join("config.json")).unwrap();
            let selected = f.base.join("s");
            fs::create_dir(&selected).unwrap();
            // A stale selector must not defeat either explicit override, including init.
            fs::write(
                f.root.join("config-location.json"),
                serde_json::to_vec(&json!({"directory":f.base.join("missing")})).unwrap(),
            )
            .unwrap();
            let command = || {
                let mut c = f.command(false);
                if legacy_profile {
                    c.env_remove("RUYCAST_HOME").env("RUSTCAST_HOME", &f.root);
                }
                c.env(key, &selected);
                c
            };
            assert_eq!(
                value(command().args(["config", "path"]).output().unwrap(), true)["result"],
                json!(selected.join("config.json"))
            );
            value(command().args(["config", "init"]).output().unwrap(), true);
            value(
                command()
                    .args(["config", "set", "theme", "\"light\""])
                    .output()
                    .unwrap(),
                true,
            );
            let modified: Value =
                serde_json::from_slice(&fs::read(selected.join("config.json")).unwrap()).unwrap();
            assert_eq!(modified["theme"], "light");
            assert_eq!(fs::read(f.root.join("config.json")).unwrap(), original);
            let destination = f.base.join("deny");
            value(
                command()
                    .args(["config", "directory"])
                    .arg(&destination)
                    .output()
                    .unwrap(),
                false,
            );
            assert!(!destination.exists());
            value(
                f.command(true)
                    .env(key, &selected)
                    .arg("--set-config-directory")
                    .arg(&destination)
                    .output()
                    .unwrap(),
                false,
            );
            assert!(!destination.exists());
        }
    }
}
#[test]
fn override_precedence_empty_shadow_and_invalid_utf8_are_authoritative() {
    let f = Fixture::new();
    let modern = f.base.join("modern");
    let legacy = f.base.join("legacy");
    assert_eq!(
        value(
            f.command(false)
                .args(["config", "path"])
                .env("RUYCAST_CONFIG_DIR", &modern)
                .env("RUSTCAST_CONFIG_DIR", &legacy)
                .output()
                .unwrap(),
            true
        )["result"],
        json!(modern.join("config.json"))
    );
    assert_eq!(
        value(
            f.command(false)
                .args(["config", "path"])
                .env("RUYCAST_CONFIG_DIR", "")
                .env("RUSTCAST_CONFIG_DIR", &legacy)
                .output()
                .unwrap(),
            true
        )["result"],
        json!(f.root.join("config.json"))
    );
    use std::os::unix::ffi::OsStringExt;
    for native in [false, true] {
        let mut c = f.command(native);
        c.env(
            "RUYCAST_CONFIG_DIR",
            std::ffi::OsString::from_vec(vec![0xff]),
        )
        .env("RUSTCAST_CONFIG_DIR", &legacy);
        if native {
            c.arg("--set-config-directory").arg(&modern);
        } else {
            c.args(["config", "path"]);
        }
        value(c.output().unwrap(), false);
    }
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 0);
}
#[test]
fn invalid_explicit_profile_never_uses_cwd_or_legacy_or_override() {
    let f = Fixture::new();
    for root in ["", "../escape", "bad/../escape"] {
        for native in [false, true] {
            let mut c = f.command(native);
            c.env("RUYCAST_HOME", root)
                .env("RUSTCAST_HOME", &f.root)
                .env("RUYCAST_CONFIG_DIR", f.base.join("override"));
            if native {
                c.arg("--set-config-directory").arg(f.base.join("dest"));
            } else {
                c.args(["config", "init"]);
            }
            value(c.output().unwrap(), false);
        }
    }
    assert_eq!(fs::read_dir(&f.base).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 0);
    assert_eq!(
        value(
            f.command(false)
                .args(["config", "path"])
                .env("RUYCAST_HOME", "p")
                .output()
                .unwrap(),
            true
        )["result"],
        json!(f.root.join("config.json"))
    );
}
#[test]
fn offline_cli_relocation_retains_source_and_refuses_existing_target() {
    let f = Fixture::new();
    f.source();
    let original = fs::read(f.root.join("config.json")).unwrap();
    fs::write(f.root.join("history.keep"), b"local").unwrap();
    fs::write(f.root.join("clipboard.json"), b"[]").unwrap();
    let destination = f.base.join("Synced vault with spaces/.appdata/ruycast");
    let output = bounded_value(
        f.command(false)
            .args(["config", "directory"])
            .arg(&destination),
        true,
    );
    assert_eq!(
        output["result"]["path"],
        json!(destination.join("config.json"))
    );
    assert_eq!(output["result"]["originalRetained"], true);
    assert_eq!(output["result"]["durable"], true);
    assert_eq!(fs::read(f.root.join("config.json")).unwrap(), original);
    assert_eq!(fs::read(destination.join("config.json")).unwrap(), original);
    assert!(!destination.join("history.keep").exists());
    assert_eq!(fs::read(f.root.join("clipboard.json")).unwrap(), b"[]");
    let names = fs::read_dir(&destination)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, [std::ffi::OsString::from("config.json")]);
    bounded_value(
        f.command(false)
            .args(["config", "set", "clipboard.limit", "51"]),
        true,
    );
    let edited: Value =
        serde_json::from_slice(&fs::read(destination.join("config.json")).unwrap()).unwrap();
    assert_eq!(edited["clipboard"]["limit"], 51);
    assert_eq!(fs::read(f.root.join("config.json")).unwrap(), original);
    assert_eq!(
        bounded_value(f.command(false).args(["config", "path"]), true)["result"],
        json!(destination.join("config.json"))
    );
    bounded_value(
        f.command(false)
            .args(["config", "directory"])
            .arg(&destination),
        true,
    );
    let selector = fs::read(f.root.join("config-location.json")).unwrap();
    let blocked = f.base.join("blocked");
    fs::create_dir(&blocked).unwrap();
    fs::write(blocked.join("config.json"), b"preserve").unwrap();
    bounded_value(
        f.command(false).args(["config", "directory"]).arg(&blocked),
        false,
    );
    assert_eq!(fs::read(blocked.join("config.json")).unwrap(), b"preserve");
    assert_eq!(
        fs::read(f.root.join("config-location.json")).unwrap(),
        selector
    );
    let fresh = Fixture::new();
    fresh.source();
    let existing = fresh.base.join("Synced vault with spaces/.appdata/ruycast");
    fs::create_dir_all(&existing).unwrap();
    let mut authored = include_bytes!("../../config/default.json").to_vec();
    authored.push(b'\n');
    fs::write(existing.join("config.json"), &authored).unwrap();
    bounded_value(
        fresh
            .command(false)
            .args(["config", "directory"])
            .arg(&existing),
        false,
    );
    assert_eq!(fs::read(existing.join("config.json")).unwrap(), authored);
    assert!(!fresh.root.join("config-location.json").exists());
}

#[test]
fn offline_cli_busy_writer_and_environment_pin_refuse_relocation() {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    for pinned in [false, true] {
        let f = Fixture::new();
        f.source();
        let original = fs::read(f.root.join("config.json")).unwrap();
        let destination = f.base.join("Synced vault with spaces/.appdata/ruycast");
        let lock = if pinned {
            None
        } else {
            let file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(f.root.join("config.lock"))
                .unwrap();
            assert_eq!(
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
                0
            );
            Some(file)
        };
        let mut command = f.command(false);
        if pinned {
            command.env("RUYCAST_CONFIG_DIR", &f.root);
        }
        let refusal = bounded_value(
            command.args(["config", "directory"]).arg(&destination),
            false,
        );
        assert!(refusal["error"].as_str().unwrap().contains(if pinned {
            "RUYCAST_CONFIG_DIR controls"
        } else {
            "Configuration is being saved"
        }));
        drop(lock);
        assert!(!destination.exists());
        assert!(!f.root.join("config-location.json").exists());
        assert_eq!(fs::read(f.root.join("config.json")).unwrap(), original);
        assert_eq!(
            bounded_value(f.command(false).args(["config", "path"]), true)["result"],
            json!(f.root.join("config.json"))
        );
    }
}

#[test]
fn offline_cli_default_path_and_invalid_selectors_never_use_stale_config() {
    let f = Fixture::new();
    f.source();
    let original = fs::read(f.root.join("config.json")).unwrap();
    assert_eq!(
        bounded_value(f.command(false).args(["config", "path"]), true)["result"],
        json!(f.root.join("config.json"))
    );
    let missing = f.base.join("missing/ruycast-config");
    for content in [
        "{".to_owned(),
        serde_json::to_string(&json!({"directory":"relative"})).unwrap(),
        serde_json::to_string(&json!({"directory":missing})).unwrap(),
    ] {
        let selector = f.root.join("config-location.json");
        fs::write(&selector, &content).unwrap();
        bounded_value(f.command(false).args(["config", "get"]), false);
        assert_eq!(fs::read(f.root.join("config.json")).unwrap(), original);
        assert_eq!(fs::read_to_string(selector).unwrap(), content);
        assert!(!missing.exists());
    }
}

#[test]
fn offline_cli_and_host_initializer_refuse_unavailable_persisted_config() {
    let f = Fixture::new();
    f.source();
    let original = fs::read(f.root.join("config.json")).unwrap();
    let missing = f.base.join("Synced vault with spaces/.appdata/ruycast");
    let selector = f.root.join("config-location.json");
    let content = serde_json::to_vec(&json!({"directory":missing})).unwrap();
    fs::write(&selector, &content).unwrap();
    let refusal = bounded_value(f.command(false).args(["config", "init"]), false);
    assert!(refusal["error"]
        .as_str()
        .unwrap()
        .contains("Selected configuration is unavailable"));
    assert!(!missing.exists());
    let native_refusal = bounded_value(f.command(true).arg("--init-config"), false);
    assert!(native_refusal["error"]
        .as_str()
        .unwrap()
        .contains("Selected configuration is unavailable"));
    let paths = ruycast_native::profile::ProfilePaths::new(f.root.clone(), f.base.clone(), None);
    assert!(paths
        .initialize()
        .unwrap_err()
        .contains("Selected configuration is unavailable"));
    assert!(!missing.exists());
    assert_eq!(fs::read(f.root.join("config.json")).unwrap(), original);
    assert_eq!(fs::read(selector).unwrap(), content);
}

#[test]
fn offline_cli_and_host_initializer_refuse_parent_component_after_symlink() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.source();
    let original = fs::read(f.root.join("config.json")).unwrap();
    let elsewhere = f.base.join("elsewhere/child");
    fs::create_dir_all(&elsewhere).unwrap();
    let link = f.base.join("link");
    symlink(&elsewhere, &link).unwrap();
    let path = format!("{}/../ruycast", link.display());
    let refusal = bounded_value(
        f.command(false)
            .env("RUYCAST_CONFIG_DIR", &path)
            .args(["config", "path"]),
        false,
    );
    assert!(refusal["error"]
        .as_str()
        .unwrap()
        .contains("without '..' components"));
    let native_refusal = bounded_value(
        f.command(true)
            .env("RUYCAST_CONFIG_DIR", &path)
            .arg("--init-config"),
        false,
    );
    assert!(native_refusal["error"]
        .as_str()
        .unwrap()
        .contains("without '..' components"));
    let paths = ruycast_native::profile::ProfilePaths::new(
        f.root.clone(),
        f.base.clone(),
        Some(path.clone()),
    );
    assert!(paths
        .initialize()
        .unwrap_err()
        .contains("without '..' components"));
    let refusal = bounded_value(f.command(false).args(["config", "directory", &path]), false);
    assert!(refusal["error"]
        .as_str()
        .unwrap()
        .contains("without '..' components"));
    assert!(!elsewhere.parent().unwrap().join("ruycast").exists());
    assert!(!f.base.join("ruycast").exists());
    assert_eq!(fs::read_link(link).unwrap(), elsewhere);
    assert_eq!(fs::read(f.root.join("config.json")).unwrap(), original);
    assert!(!f.root.join("config-location.json").exists());
}

#[test]
fn offline_cli_and_host_initializer_override_persisted_location_without_overwrite() {
    for existing in [false, true] {
        let f = Fixture::new();
        f.source();
        let original = fs::read(f.root.join("config.json")).unwrap();
        let missing = f.base.join("missing/persisted");
        let selector = f.root.join("config-location.json");
        let selector_bytes = serde_json::to_vec(&json!({"directory":missing})).unwrap();
        fs::write(&selector, &selector_bytes).unwrap();
        let selected = f.base.join("environment");
        fs::create_dir(&selected).unwrap();
        let file = selected.join("config.json");
        let authored = existing.then(|| {
            let mut bytes = original.clone();
            bytes.extend_from_slice(b" \n");
            bytes
        });
        if let Some(bytes) = &authored {
            fs::write(&file, bytes).unwrap();
        }
        assert_eq!(
            bounded_value(
                f.command(false)
                    .env("RUYCAST_CONFIG_DIR", &selected)
                    .args(["config", "path"]),
                true
            )["result"],
            json!(file)
        );
        let initialized = bounded_value(
            f.command(true)
                .env("RUYCAST_CONFIG_DIR", &selected)
                .arg("--init-config"),
            true,
        );
        assert_eq!(initialized["path"], json!(file));
        let after = fs::read(&file).unwrap();
        if let Some(bytes) = authored {
            assert_eq!(after, bytes);
        } else {
            let value: Value = serde_json::from_slice(&after).unwrap();
            rustcast_core::validate(&value).unwrap();
        }
        assert_eq!(
            bounded_value(
                f.command(false)
                    .env("RUYCAST_CONFIG_DIR", &selected)
                    .args(["config", "init"]),
                true
            )["path"],
            json!(file)
        );
        assert_eq!(fs::read(&file).unwrap(), after);
        assert_eq!(fs::read(f.root.join("config.json")).unwrap(), original);
        assert_eq!(fs::read(&selector).unwrap(), selector_bytes);
        assert!(!missing.exists());
    }
    let f = Fixture::new();
    let uncreated = f.base.join("uncreated-profile");
    let refusal = bounded_value(
        f.command(true)
            .env("RUYCAST_HOME", &uncreated)
            .env("RUYCAST_CONFIG_DIR", "relative")
            .args(["--init-config", "extra"]),
        false,
    );
    assert!(refusal["error"]
        .as_str()
        .unwrap()
        .starts_with("Unknown arguments"));
    assert!(!uncreated.exists());
}

#[test]
fn missing_invalid_and_overridden_sources_fail_before_creation() {
    for invalid in [false, true] {
        let f = Fixture::new();
        if invalid {
            fs::write(f.root.join("config.json"), b"invalid").unwrap();
        }
        let target = f.base.join("d");
        value(
            f.command(true)
                .arg("--set-config-directory")
                .arg(&target)
                .output()
                .unwrap(),
            false,
        );
        assert!(!target.exists());
        assert!(!f.root.join("config.lock").exists());
        assert!(!f.root.join("config-location.json").exists());
        assert_eq!(f.root.join("config.json").exists(), invalid);
    }
    let f = Fixture::new();
    value(
        f.command(true)
            .env("RUSTCAST_CONFIG_DIR", f.base.join("override"))
            .arg("--set-config-directory")
            .arg(f.base.join("d"))
            .output()
            .unwrap(),
        false,
    );
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 0);
}
