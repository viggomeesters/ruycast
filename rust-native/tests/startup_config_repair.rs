//! Offline owned configuration contract; never starts AppKit or global services.
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
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
    let destination = f.base.join("dest");
    let output = value(
        f.command(false)
            .args(["config", "directory"])
            .arg(&destination)
            .output()
            .unwrap(),
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
    assert_eq!(
        value(
            f.command(false).args(["config", "path"]).output().unwrap(),
            true
        )["result"],
        json!(destination.join("config.json"))
    );
    value(
        f.command(false)
            .args(["config", "directory"])
            .arg(&destination)
            .output()
            .unwrap(),
        true,
    );
    let selector = fs::read(f.root.join("config-location.json")).unwrap();
    let blocked = f.base.join("blocked");
    fs::create_dir(&blocked).unwrap();
    fs::write(blocked.join("config.json"), b"preserve").unwrap();
    value(
        f.command(false)
            .args(["config", "directory"])
            .arg(&blocked)
            .output()
            .unwrap(),
        false,
    );
    assert_eq!(fs::read(blocked.join("config.json")).unwrap(), b"preserve");
    assert_eq!(
        fs::read(f.root.join("config-location.json")).unwrap(),
        selector
    );
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
