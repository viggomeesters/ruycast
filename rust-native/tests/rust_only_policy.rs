//! Real Rust policy CLI; every mutation is confined to an owned disposable tree.
#[allow(dead_code)]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/rust-only-policy-fixtures");
        fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "rust-policy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let value = Self(root);
        for path in [
            "Sources/Legacy.swift",
            "Package.swift",
            "rust/src/lib.rs",
            "rust-native/src/main.rs",
        ] {
            value.write(path, "// own policy fixture\n");
        }
        let files = ["Sources/Legacy.swift", "Package.swift"]
            .into_iter()
            .map(|path| {
                (
                    path,
                    format!(
                        "{:x}",
                        Sha256::digest(fs::read(value.0.join(path)).unwrap())
                    ),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        value.write("rust-native/swift-reference-lock.json", &json!({"schema":"ruycast.rust-only-reference-lock.v1", "legacy_roots":["Sources"], "runtime_roots":["rust/src","rust-native/src"], "files":files}).to_string());
        value
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn verify(root: &Path) -> Value {
    let output = child_output::wait(
        Command::new(env!("CARGO_BIN_EXE_rust-only-policy"))
            .arg("--root")
            .arg(root)
            .env_clear()
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        Duration::from_secs(6),
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        output.status.code(),
        Some(if result["ok"] == true { 0 } else { 1 }),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(result["productionReady"], false);
    result
}
#[test]
fn test_real_repository_current_freeze_passes() {
    let result = verify(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap());
    assert_eq!(result["ok"], true, "{result}");
    assert!(result["frozenLegacyFiles"].as_u64().unwrap() > 2);
}
#[test]
fn test_rust_changes_and_new_rust_files_remain_allowed() {
    let fixture = Fixture::new();
    fixture.write("rust-native/src/main.rs", "fn main() {}\n");
    fixture.write("rust/src/new.rs", "pub fn execute() {}\n");
    assert_eq!(verify(&fixture.0)["ok"], true);
}
#[test]
fn test_test_oracles_and_build_tools_are_not_product_logic() {
    let fixture = Fixture::new();
    for path in [
        "rust-native/tests/oracle.swift",
        "rust-native/tests/sdk.m",
        "rust-native/scripts/check.py",
    ] {
        fixture.write(path, "// own test fixture\n");
    }
    assert_eq!(verify(&fixture.0)["ok"], true);
}

#[test]
fn frozen_replacement_character_legacy_name_remains_supported() {
    let fixture = Fixture::new();
    let lock = fixture.0.join("rust-native/swift-reference-lock.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&lock).unwrap()).unwrap();
    let sha = value["files"]
        .as_object_mut()
        .unwrap()
        .remove("Sources/Legacy.swift")
        .unwrap();
    value["files"]["Sources/�.swift"] = sha;
    fs::write(lock, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::rename(
        fixture.0.join("Sources/Legacy.swift"),
        fixture.0.join("Sources/�.swift"),
    )
    .unwrap();
    assert_eq!(verify(&fixture.0)["ok"], true);
}

#[test]
fn product_errors_keep_directory_links_then_files_then_descendants() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    fixture.write(
        "rust-native/nested/child.go",
        "// own forbidden source fixture\n",
    );
    fixture.write("rust-native/z.sh", "// own forbidden source fixture\n");
    fixture.write("rust-native/a.rb", "// own forbidden source fixture\n");
    fs::create_dir(fixture.0.join("outside")).unwrap();
    symlink(
        fixture.0.join("outside"),
        fixture.0.join("rust-native/host"),
    )
    .unwrap();
    let mut expected = vec![json!("product directory symlink: rust-native/host")];
    for entry in fs::read_dir(fixture.0.join("rust-native")).unwrap() {
        let name = entry.unwrap().file_name();
        if name == "z.sh" || name == "a.rb" {
            expected.push(json!(format!(
                "non-Rust/unknown product input: rust-native/{}",
                name.to_str().unwrap()
            )));
        }
    }
    expected.push(json!(
        "non-Rust/unknown product input: rust-native/nested/child.go"
    ));
    assert_eq!(verify(&fixture.0)["errors"], json!(expected));
}

#[test]
fn broken_and_file_tooling_links_keep_both_refusals() {
    use std::os::unix::fs::symlink;
    for name in ["tests", "scripts"] {
        for file_target in [false, true] {
            let fixture = Fixture::new();
            let target = fixture.0.join("own-target");
            if file_target {
                fs::write(&target, "// own target fixture").unwrap();
            }
            symlink(&target, fixture.0.join("rust-native").join(name)).unwrap();
            assert_eq!(
                verify(&fixture.0)["errors"],
                json!([
                    format!("tooling source symlink: rust-native/{name}"),
                    format!("non-Rust/unknown product input: rust-native/{name}")
                ])
            );
        }
    }
}

#[test]
fn non_utf8_cli_root_is_a_json_refusal() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let root = fixture
        .0
        .join(std::ffi::OsString::from_vec(b"root-\xff".to_vec()));
    let output = child_output::wait(
        Command::new(env!("CARGO_BIN_EXE_rust-only-policy"))
            .arg("--root")
            .arg(&root)
            .current_dir(&fixture.0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        Duration::from_secs(6),
    );
    assert_eq!(output.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert!(result["errors"][0].as_str().unwrap().contains("not UTF-8"));
    let output = child_output::wait(
        Command::new(env!("CARGO_BIN_EXE_rust-only-policy"))
            .arg(std::ffi::OsString::from_vec(b"--flag-\xff".to_vec()))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        Duration::from_secs(6),
    );
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_legacy_change_addition_removal_and_package_edit_fail() {
    let mut fixtures = Vec::new();
    for mutation in ["change", "add", "remove", "package"] {
        let fixture = Fixture::new();
        match mutation {
            "change" => fixture.write("Sources/Legacy.swift", "// changed\n"),
            "add" => fixture.write("Sources/NewHost.swift", "// new\n"),
            "remove" => fs::remove_file(fixture.0.join("Sources/Legacy.swift")).unwrap(),
            _ => fixture.write("Package.swift", "// new Swift target\n"),
        }
        assert_eq!(verify(&fixture.0)["ok"], false, "{mutation}");
        fixtures.push(fixture);
    }
}

#[test]
fn test_non_rust_runtime_or_sidecar_host_fails() {
    let mut fixtures = Vec::new();
    for path in [
        "rust-native/src/bridge.swift",
        "rust-native/src/bridge.m",
        "rust/src/bridge.py",
        "rust-native/NewHost.swift",
        "rust-native/web/runtime.js",
        "rust/NewHost.swift",
        "rust-native/NewHost.rb",
        "rust/NewHost.go",
        "rust-native/NewHost.sh",
    ] {
        let fixture = Fixture::new();
        fixture.write(path, "// forbidden product code\n");
        assert_eq!(verify(&fixture.0)["ok"], false, "{path}");
        fixtures.push(fixture);
    }
}

#[test]
fn test_symlink_and_missing_or_weakened_lock_fail_closed() {
    use std::os::unix::fs::symlink;
    // Match unittest cleanup: all five independent owned trees survive until completion.
    let mut fixtures = Vec::new();
    let fixture = Fixture::new();
    let file = fixture.0.join("Sources/Legacy.swift");
    let content = fs::read(&file).unwrap();
    fs::remove_file(&file).unwrap();
    let other = fixture.0.join("old.swift");
    fs::write(&other, content).unwrap();
    symlink(&other, &file).unwrap();
    assert_eq!(verify(&fixture.0)["ok"], false);
    fixtures.push(fixture);
    let fixture = Fixture::new();
    symlink(
        fixture.0.join("rust/src/lib.rs"),
        fixture.0.join("rust-native/src/alias.rs"),
    )
    .unwrap();
    assert_eq!(verify(&fixture.0)["ok"], false);
    fixtures.push(fixture);
    let fixture = Fixture::new();
    let external = fixture.0.join("foreign");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("Host.swift"), "// own symlink fixture").unwrap();
    symlink(&external, fixture.0.join("rust-native/host")).unwrap();
    assert_eq!(verify(&fixture.0)["ok"], false);
    fixtures.push(fixture);
    let fixture = Fixture::new();
    fs::remove_file(fixture.0.join("rust-native/swift-reference-lock.json")).unwrap();
    assert_eq!(verify(&fixture.0)["ok"], false);
    fixtures.push(fixture);
    let fixture = Fixture::new();
    let lock = fixture.0.join("rust-native/swift-reference-lock.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&lock).unwrap()).unwrap();
    value["files"]
        .as_object_mut()
        .unwrap()
        .remove("Package.swift");
    fs::write(lock, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(verify(&fixture.0)["ok"], false);
    fixtures.push(fixture);
}
