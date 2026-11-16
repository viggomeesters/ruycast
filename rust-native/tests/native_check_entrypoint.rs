//! Direct native-check CLI contracts; absent prerequisites never run the matrix.
#[allow(dead_code)]
#[path = "support/child_output.rs"]
mod child_output;
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR")
                .expect("Explicit owned fixture parent"),
        );
        assert!(parent.is_absolute() && parent.is_dir());
        let root = parent.join(format!(
            "native-entry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(root: &Path, tail: &[&str]) -> Output {
    child_output::wait(
        Command::new(env!("CARGO_BIN_EXE_ruycast-build"))
            .args(["compat", "native-check", "--repo"])
            .arg(root)
            .arg("--")
            .args(tail)
            .env_clear()
            .env("HOME", root)
            .env("TMPDIR", root)
            .env("PATH", root.join("no-tools"))
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        Duration::from_secs(6),
    )
}
#[test]
fn direct_native_check_help_preserves_scope_without_starting_validation() {
    let fixture = Fixture::new();
    let output = run(&fixture.0, &["--help"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "native-check (complete native matrix; prepared v4 release required)\n"
    );
    assert!(output.stderr.is_empty());
    assert!(!fixture.0.join(".build").exists());
}
#[test]
fn direct_native_check_refuses_unknown_arguments_before_mutation() {
    let fixture = Fixture::new();
    let output = run(&fixture.0, &["--owned-unknown"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("Unexpected compatibility arguments."));
    assert!(!fixture.0.join(".build").exists());
}
#[test]
fn direct_native_check_refuses_missing_sources_without_running_external_tools() {
    let fixture = Fixture::new();
    let output = run(&fixture.0, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("No such file or directory"), "{error}");
    assert!(!fixture.0.join("rust-native/target/verification").exists());
}
