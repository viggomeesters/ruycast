//! Real Rust policy CLI; every mutation is confined to an owned disposable tree.
#[allow(dead_code)]
#[path = "support/child_output.rs"]
mod child_output;
use serde_json::{Value, json};
use std::{
    ffi::{CString, OsString},
    fs,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{DirBuilderExt, PermissionsExt, symlink},
    },
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
        let fixture = Self(root);
        for path in ["rust/src/lib.rs", "rust-native/src/main.rs"] {
            fixture.write(path, "// own policy fixture\n");
        }
        fixture
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

fn run(command: &mut Command) -> std::process::Output {
    child_output::wait(
        command
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        Duration::from_secs(6),
    )
}

fn verify(root: &Path) -> Value {
    let output = run(Command::new(env!("CARGO_BIN_EXE_rust-only-policy"))
        .arg("--root")
        .arg(root));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        output.status.code(),
        Some(if result["ok"] == true { 0 } else { 1 }),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(result["schema"], "ruycast.rust-only-source-policy.v1");
    assert_eq!(result["policy"], "Rust-only executable source");
    assert_eq!(result["productionReady"], false);
    result
}

fn assert_refusal(root: &Path, expected: &str) {
    let result = verify(root);
    assert_eq!(result["ok"], false, "{result}");
    assert!(
        result["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error == expected),
        "{expected}: {result}"
    );
}

#[test]
fn real_repository_is_rust_only() {
    let result = verify(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap());
    assert_eq!(result["ok"], true, "{result}");
    assert!(result["rustRuntimeFiles"].as_u64().unwrap() > 2);
    assert!(
        !result
            .as_object()
            .unwrap()
            .contains_key("frozenLegacyFiles")
    );
    assert!(!result.as_object().unwrap().contains_key("lockSHA256"));
}

#[test]
fn rust_changes_and_data_remain_allowed() {
    let fixture = Fixture::new();
    fixture.write("rust-native/src/main.rs", "fn main() {}\n");
    fixture.write("rust/src/�.rs", "pub fn execute() {}\n");
    fixture.write("scripts/check.rs", "fn main() {}\n");
    fixture.write("rust-native/tests/example.json", "{}");
    fixture.write("tests.md", "# Testing\n");
    fixture.write("assets/icon.svg", "<svg/>");
    fixture.write("docs/old-host.md", "# Historical Swift input\n");
    let result = verify(&fixture.0);
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["rustRuntimeFiles"], 3);
    assert_eq!(result["errors"], json!([]));
}

#[test]
fn repository_owned_inline_interpreter_programs_are_refused() {
    for (path, program) in [
        (
            "rust/src/helper.rs",
            r#"fn helper() { std::process::Command::new("python3").args(["-c", "print(42)"]); }"#,
        ),
        (
            "rust-native/tests/helper.rs",
            r#"fn helper() { std::process::Command::new("/usr/bin/osascript").arg("-e").arg("return 42"); }"#,
        ),
        (
            "bench/f1-runner/src/helper.rs",
            r#"fn helper() { std::process::Command::new(concat!("/bin/", "sh")).arg("-c").arg(format!("printf {}", 42)); }"#,
        ),
    ] {
        let fixture = Fixture::new();
        fixture.write(path, program);
        assert_refusal(
            &fixture.0,
            &format!("repository-owned inline interpreter program: {path}"),
        );
    }
}

#[test]
fn native_tools_user_scripts_and_literal_refusal_fixtures_remain_allowed() {
    let fixture = Fixture::new();
    fixture.write(
        "rust/src/helper.rs",
        r##"
        use std::process::Command;
        fn helper(interpreter: &str, user_script: &str) {
            Command::new("/usr/bin/codesign").args(["-s", "-"]);
            Command::new("/usr/bin/osascript").arg("-e").arg(user_script);
            Command::new(interpreter).arg("-c").arg(user_script);
            Command::new("/usr/bin/osascript").arg("-l").arg("JavaScript").arg(user_script);
            // Command::new("python3").args(["-c", "print(42)"]);
            let _negative_fixture = r#"Command::new("python3").args(["-c", "print(42)"])"#;
        }
    "##,
    );
    assert_eq!(verify(&fixture.0)["ok"], true);
}

#[test]
fn retired_paths_fail_even_with_no_swift_bytes() {
    for path in [
        "Sources",
        "Package.swift",
        "rust-native/swift-reference-lock.json",
        "Tests/LauncherCoreTests",
    ] {
        let fixture = Fixture::new();
        if path == "Sources" || path == "Tests/LauncherCoreTests" {
            fs::create_dir_all(fixture.0.join(path)).unwrap();
        } else {
            fixture.write(path, "");
        }
        assert_refusal(&fixture.0, &format!("retired Swift input: {path}"));
    }
    let fixture = Fixture::new();
    symlink("missing", fixture.0.join("Sources")).unwrap();
    assert_refusal(&fixture.0, "retired Swift input: Sources");
}

#[test]
fn non_rust_executable_source_fails_in_every_scope() {
    for (path, scope) in [
        ("rust-native/src/bridge.swift", "runtime"),
        ("rust/src/bridge.m", "runtime"),
        ("rust-native/tests/oracle.swift", "tooling"),
        ("rust-native/scripts/check.py", "tooling"),
        ("rust/tests/oracle.swift", "tooling"),
        ("rust/scripts/check.py", "tooling"),
        ("Tests/fixtures/oracle.swift", "tooling"),
        ("scripts/check.py", "tooling"),
        ("bench/f1-runner/examples/helper.sh", "tooling"),
        ("bench/unowned/target/oracle.swift", "tooling"),
        ("rust-native/NewHost.swift", "product"),
        ("rust-native/web/runtime.js", "product"),
        ("rust/NewHost.go", "product"),
        ("helper.sh", "repository"),
        ("helpers/run", "repository"),
        ("assets/Host.swift", "repository"),
        ("docs/runtime.py", "repository"),
    ] {
        let fixture = Fixture::new();
        fixture.write(path, "// forbidden executable source fixture\n");
        assert_refusal(
            &fixture.0,
            &format!("non-Rust/unknown {scope} input: {path}"),
        );
    }
}

#[test]
fn executable_data_and_disguised_scripts_fail() {
    let fixture = Fixture::new();
    fixture.write("config/default.json", "{}");
    fs::set_permissions(
        fixture.0.join("config/default.json"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fixture.write("rust-native/tests/wrapped.md", "#!/bin/sh\nexit 0\n");
    assert_refusal(
        &fixture.0,
        "non-Rust/unknown repository input: config/default.json",
    );
    assert_refusal(
        &fixture.0,
        "non-Rust/unknown tooling input: rust-native/tests/wrapped.md",
    );
}

#[test]
fn only_real_known_build_outputs_are_skipped() {
    let fixture = Fixture::new();
    for output in [
        ".build",
        ".dist",
        "rust/target",
        "rust-native/target",
        "bench/f1-runner/target",
    ] {
        fixture.write(&format!("{output}/generated.py"), "generated fixture\n");
    }
    fixture.write(".git", "gitdir: /worktree/.git/worktrees/fixture\n");
    assert_eq!(verify(&fixture.0)["ok"], true);
    fs::remove_dir_all(fixture.0.join(".dist")).unwrap();
    symlink(".build", fixture.0.join(".dist")).unwrap();
    assert_refusal(&fixture.0, "repository source symlink: .dist");
}

#[test]
fn missing_runtime_and_source_links_fail_closed() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.0.join("rust/src/lib.rs")).unwrap();
    symlink("missing", fixture.0.join("rust/src/alias.rs")).unwrap();
    symlink("../rust/src", fixture.0.join("rust-native/tests")).unwrap();
    assert_refusal(
        &fixture.0,
        "runtime source root must contain Rust source: rust/src",
    );
    assert_refusal(&fixture.0, "runtime source symlink: rust/src/alias.rs");
    assert_refusal(&fixture.0, "tooling source symlink: rust-native/tests");
}

#[test]
fn symlinked_repository_root_fails_closed() {
    let fixture = Fixture::new();
    let link = fixture.0.join("alias");
    symlink(&fixture.0, &link).unwrap();
    assert_refusal(
        &link,
        "Rust-only policy: repository root must be a real directory",
    );
}

#[test]
fn nonregular_resource_fails_closed() {
    let fixture = Fixture::new();
    let path = fixture.0.join("config");
    fs::create_dir(&path).unwrap();
    let fifo = CString::new(path.join("default.json").as_os_str().as_bytes()).unwrap();
    // The fixture owns the parent directory and the C string lives through the call.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert_refusal(
        &fixture.0,
        "nonregular repository input: config/default.json",
    );
}

#[test]
fn relative_root_argument_scans_the_current_directory() {
    let fixture = Fixture::new();
    let output = run(Command::new(env!("CARGO_BIN_EXE_rust-only-policy"))
        .arg("--root")
        .arg(".")
        .current_dir(&fixture.0));
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["rustRuntimeFiles"], 2);
}

#[test]
fn non_utf8_root_and_entry_are_json_refusals() {
    let fixture = Fixture::new();
    let root = fixture.0.join(OsString::from_vec(b"root-\xff".to_vec()));
    assert_refusal(&root, "Rust-only policy: filesystem path is not UTF-8");
    let path = fixture
        .0
        .join("rust/src")
        .join(OsString::from_vec(b"\xff.rs".to_vec()));
    if fs::write(&path, "// own fixture\n").is_ok() {
        assert_refusal(&fixture.0, "Rust-only policy: filesystem path is not UTF-8");
    }
    let output = run(Command::new(env!("CARGO_BIN_EXE_rust-only-policy"))
        .arg(OsString::from_vec(b"--flag-\xff".to_vec())));
    assert_eq!(output.status.code(), Some(2));
}
