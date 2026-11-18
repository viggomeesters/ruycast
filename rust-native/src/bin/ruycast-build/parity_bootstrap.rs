//! Build and bind the real parser without borrowing Cargo's bootstrap target lock.
use super::*;
use serde_json::{json, Value};
use std::time::Duration;
const SOURCES: &[&str] = &[
    "rust-native/src/bin/parity_ast.rs",
    "rust-native/src/bin/ruycast-build.rs",
    "rust-native/src/bin/ruycast-build/validation.rs",
    "rust-native/src/bin/ruycast-build/test_catalog.rs",
    "rust-native/src/bin/ruycast-build/parity_wire.rs",
    "rust-native/src/bin/ruycast-build/parity_families.rs",
    "rust-native/src/bin/ruycast-build/parity_sources.rs",
    "rust-native/src/bin/ruycast-build/parity_compile.rs",
    "rust-native/src/bin/ruycast-build/process.rs",
    "rust-native/src/bin/ruycast-build/preparation.rs",
    "rust-native/resources/parity-families.json",
    "rust-native/src/artifact_lease.rs",
    "rust/src/private_storage.rs",
    "rust-native/Cargo.toml",
    "rust-native/Cargo.lock",
    "rust/Cargo.toml",
    "rust/Cargo.lock",
    "rust-toolchain.toml",
    "rust-native/src/bin/ruycast-build/parity_inventory.rs",
    "rust-native/src/bin/ruycast-build/parity_bootstrap.rs",
    "rust-native/src/bin/ruycast-build/parity_toolchain.rs",
    "rust-native/src/bin/ruycast-build/parity_report.rs",
    "rust-native/src/bin/ruycast-build/oracles.rs",
];
fn sources(root: &Path) -> Result<BTreeMap<String, String>> {
    SOURCES
        .iter()
        .map(|source| Ok(((*source).to_owned(), digest(&root.join(source))?)))
        .collect()
}
fn validate(root: &Path, binding: &Value) -> Result<()> {
    if binding.get("sources") != Some(&json!(sources(root)?)) {
        return Err("AST parser source binding changed or incomplete.".into());
    }
    parity_compile::validate_parser(root, binding)
}
fn publish(root: &Path, source: &Path, before: &BTreeMap<String, String>) -> Result<Value> {
    if &sources(root)? != before {
        return Err("AST parser inputs changed during build.".into());
    }
    let bytes = regular_bytes(source)?;
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let artifact = format!(".build/delivery-tools/parity-ast-artifacts/{sha}");
    validation::publish_rust_policy_snapshot(&root.join(&artifact), &bytes, &sha, || Ok(()))?;
    let binding = json!({"artifact":artifact,"sha256":sha,"sources":before});
    validate(root, &binding)?;
    Ok(binding)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    binding: Option<Value>,
    deadline_nanos: u64,
}
pub(super) fn monotonic_nanos() -> Result<u64> {
    let mut time: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return Err(error(std::io::Error::last_os_error()));
    }
    let seconds = u64::try_from(time.tv_sec).map_err(error)?;
    let nanos = u64::try_from(time.tv_nsec).map_err(error)?;
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|v| v.checked_add(nanos))
        .ok_or("Monotonic parser deadline overflow.".into())
}
pub(super) fn inner_budget(deadline: u64) -> Result<Duration> {
    let remaining = deadline
        .checked_sub(monotonic_nanos()?)
        .and_then(|v| v.checked_sub(5_000_000_000))
        .filter(|v| *v > 0)
        .ok_or("Parser bootstrap outer deadline leaves no cleanup margin.")?;
    Ok(Duration::from_nanos(remaining).min(Duration::from_secs(180)))
}
fn build(lease: &BuildLock, mut command: Command, timeout: Duration) -> Result<()> {
    lease.apply_to_child(&mut command)?;
    lease.validate()?;
    let built = process::capture(command, timeout, 64 * 1024 * 1024)?;
    lease.validate()?;
    if !built.status.success() {
        return Err(String::from_utf8_lossy(&built.stderr).into_owned());
    }
    Ok(())
}
pub(super) fn run(root: &Path, lease: &BuildLock, payload: &str) -> Result<Value> {
    let request: Request = serde_json::from_str(payload).map_err(error)?;
    inner_budget(request.deadline_nanos)?;
    if let Some(binding) = request.binding {
        validate(root, &binding)?;
        lease.validate()?;
        return Ok(binding);
    }
    let before = sources(root)?;
    let target = root.join(".build/parity-parser-build");
    for directory in [root.join(".build"), target.clone()] {
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(error(e)),
        }
        let metadata = fs::symlink_metadata(&directory).map_err(error)?;
        if metadata.is_symlink() || !metadata.is_dir() {
            return Err("Linked/invalid parser build directory.".into());
        }
    }
    ruycast_native::private_storage::secure_directory(&target)?;
    let mut command = Command::new("cargo");
    command
        .args([
            "build",
            "--quiet",
            "--locked",
            "--offline",
            "--manifest-path",
        ])
        .arg(root.join("rust-native/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .args(["--bin", "parity-ast"])
        .current_dir(root)
        .env("CARGO_INCREMENTAL", "0");
    build(lease, command, inner_budget(request.deadline_nanos)?)?;
    publish(root, &target.join("debug/parity-ast"), &before)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> super::super::tests::Fixture {
        let fixture = super::super::tests::Fixture::new();
        for source in SOURCES {
            let path = fixture.0.join(source);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"owned source hash fixture").unwrap();
        }
        fixture
    }
    #[test]
    fn outer_deadline_reserves_cleanup_even_after_late_helper_start() {
        assert!(inner_budget(monotonic_nanos().unwrap()).is_err());
        assert!(inner_budget(monotonic_nanos().unwrap() + 4_000_000_000).is_err());
        let limited = inner_budget(monotonic_nanos().unwrap() + 6_000_000_000).unwrap();
        assert!(limited > Duration::ZERO && limited <= Duration::from_secs(1));
        assert_eq!(
            inner_budget(monotonic_nanos().unwrap() + 400_000_000_000).unwrap(),
            Duration::from_secs(180)
        );
    }
    #[test]
    #[ignore = "Owned Rust child for the bootstrap cleanup deadline test"]
    fn owned_timeout_child() {
        let path = std::env::var_os("RUYCAST_BOOTSTRAP_PID").unwrap();
        fs::write(path, std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(5));
    }
    #[test]
    fn inner_timeout_reaps_before_outer_cleanup_margin() {
        let fixture = fixture();
        let root = &fixture.0;
        let lease = BuildLock::acquire_from_environment(root, &BTreeMap::new()).unwrap();
        let pidfile = root.join("child.pid");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "parity_bootstrap::tests::owned_timeout_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("RUYCAST_BOOTSTRAP_PID", &pidfile);
        let start = std::time::Instant::now();
        assert!(build(&lease, command, Duration::from_millis(150))
            .unwrap_err()
            .contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
        let pid: libc::pid_t = fs::read_to_string(pidfile).unwrap().parse().unwrap();
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
        lease.validate().unwrap();
    }
    #[test]
    fn bound_snapshot_survives_build_replacement_and_refuses_source_drift() {
        let fixture = fixture();
        let root = &fixture.0;
        let source = root.join("owned-parser.data");
        fs::write(&source, b"owned parser artifact fixture").unwrap();
        let before = sources(root).unwrap();
        let binding = publish(root, &source, &before).unwrap();
        fs::write(&source, b"replacement build fixture").unwrap();
        assert!(validate(root, &binding).is_ok());
        assert_eq!(
            digest(&root.join(binding["artifact"].as_str().unwrap())).unwrap(),
            binding["sha256"]
        );
        fs::write(root.join(SOURCES[0]), b"source drift").unwrap();
        assert!(validate(root, &binding).is_err());
        assert!(publish(root, &source, &before).is_err());
    }
    #[test]
    fn incomplete_and_linked_bindings_are_refused() {
        let fixture = fixture();
        let root = &fixture.0;
        let source = root.join("owned-parser.data");
        fs::write(&source, b"owned parser artifact fixture").unwrap();
        let binding = publish(root, &source, &sources(root).unwrap()).unwrap();
        let mut incomplete = binding.clone();
        incomplete["sources"]
            .as_object_mut()
            .unwrap()
            .remove(SOURCES[0]);
        assert!(validate(root, &incomplete).is_err());
        let artifact = root.join(binding["artifact"].as_str().unwrap());
        fs::remove_file(&artifact).unwrap();
        std::os::unix::fs::symlink(&source, artifact).unwrap();
        assert!(validate(root, &binding).is_err());
    }
}
