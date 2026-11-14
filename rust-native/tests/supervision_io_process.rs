//! Real supervisor/legacy CLI I/O through a deterministic Rust script fixture.
#![cfg(feature = "test-tools")]
#![allow(
    clippy::duplicate_mod,
    reason = "Owned fixtures share bounded child output support"
)]
#[path = "support/child_output.rs"]
mod child_output;
#[allow(dead_code)]
#[path = "support/runtime_host.rs"]
mod runtime_host;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
fn cli() -> PathBuf {
    let path = std::env::var_os("RUYCAST_TEST_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_host::root().join(".build/rust/release/ruycast"));
    assert!(
        path.is_absolute() && path.is_file(),
        "Build the real Rust CLI first"
    );
    path
}
fn command(target: &Path, args: Value) -> Value {
    json!({"id":"script.fixture","title":"Fixture","kind":"script","target":target,"arguments":args,"timeout":2})
}
fn run(
    name: &str,
    program: &Path,
    args: &[&str],
    input: &Value,
    timeout: Duration,
    require_success: bool,
) -> Value {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "DYLD_INSERT_LIBRARIES",
        "RUYCAST_HOME",
        "RUSTCAST_HOME",
        "RUYCAST_CONFIG_DIR",
        "RUSTCAST_CONFIG_DIR",
    ] {
        cmd.env_remove(key);
    }
    let mut child = cmd.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let bytes = serde_json::to_vec(input).unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let out = child_output::wait(child, timeout);
    writer.join().unwrap().unwrap();
    assert!(
        !require_success || out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    if let Some(folder) = std::env::var_os("RUYCAST_NATIVE_EVIDENCE_DIR") {
        let folder = PathBuf::from(folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(format!("supervision-io-{name}.json")),serde_json::to_vec_pretty(&json!({"input":input,"result":result,"fixtureSha256":format!("{:x}",Sha256::digest(fs::read(env!("CARGO_BIN_EXE_supervision-io-fixture")).unwrap()))})).unwrap()).unwrap();
    }
    result
}
fn probe(name: &str, home: &Path, c: Value) -> Value {
    let result = run(
        name,
        Path::new(env!("CARGO_BIN_EXE_supervision-probe")),
        &[],
        &json!({"helper":cli(),"command":c,"home":home}),
        Duration::from_secs(7),
        true,
    );
    assert_eq!(result["noChildren"], true);
    result.get("result").unwrap().clone()
}
#[test]
fn success_failure_and_signal_preserve_legacy_response() {
    let home = runtime_host::OwnedDir::new();
    for mode in ["success", "exit7", "signal"] {
        let c = command(
            Path::new(env!("CARGO_BIN_EXE_supervision-io-fixture")),
            json!([mode]),
        );
        let expected = run(
            &format!("legacy-{mode}"),
            &cli(),
            &["__run-script"],
            &c,
            Duration::from_secs(5),
            false,
        );
        assert_eq!(probe(mode, &home.0, c).get("response").unwrap(), &expected);
    }
}
#[test]
fn literal_null_stdin_and_explicit_home() {
    let home = runtime_host::OwnedDir::new();
    let copy = home.0.join("fixture");
    let original = Path::new(env!("CARGO_BIN_EXE_supervision-io-fixture"));
    fs::copy(original, &copy).unwrap();
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        Sha256::digest(fs::read(original).unwrap()),
        Sha256::digest(fs::read(&copy).unwrap())
    );
    let c = command(
        Path::new("~/fixture"),
        json!(["stdin-literal", "$(literal) Ω\nline"]),
    );
    assert_eq!(
        probe("literal-stdin-home", &home.0, c)["response"]["result"]["stdout"],
        "1|$(literal) Ω\nline"
    );
}
#[test]
fn output_bounds_survive_json_replacement_and_escaping() {
    let home = runtime_host::OwnedDir::new();
    let c = command(
        Path::new(env!("CARGO_BIN_EXE_supervision-io-fixture")),
        json!(["invalid-bytes"]),
    );
    let result = probe("output-bounds", &home.0, c);
    let stdout = result["response"]["result"]["stdout"].as_str().unwrap();
    let stderr = result["response"]["result"]["stderr"].as_str().unwrap();
    assert_eq!(stdout.chars().count(), 65536);
    assert_eq!(stderr.chars().count(), 65536);
    assert!(stdout.chars().all(|c| c == '�'));
    assert!(stderr.chars().all(|c| c == '\0'));
}
