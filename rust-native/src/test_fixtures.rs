//! Bounded builder for owned Rust syscall fixtures.
use std::{
    fs,
    io::Read,
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn fcntl(log: &Path) -> PathBuf {
    build(
        log,
        "RUYCAST_TEST_FCNTL_SHIM",
        "fcntl-sync-shim",
        "libfcntl_sync_shim.dylib",
    )
}

pub fn dashboard(log: &Path) -> PathBuf {
    build(
        log,
        "RUYCAST_TEST_DASHBOARD_SHIM",
        "dashboard-export-shim",
        "libdashboard_export_shim.dylib",
    )
}

fn build(log: &Path, variable: &str, example: &str, library: &str) -> PathBuf {
    if let Some(path) = std::env::var_os(variable) {
        let path = PathBuf::from(path);
        assert!(
            path.is_absolute() && path.is_file(),
            "explicit Rust {example} fixture required"
        );
        return path;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.parent()
                .unwrap()
                .join(".build/full-rust-release/native")
        });
    let target = if target.is_absolute() {
        target
    } else {
        root.join(target)
    };
    let output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(log)
        .unwrap();
    let mut child = Command::new("cargo")
        .args([
            "build",
            "--locked",
            "--offline",
            "--release",
            "--features",
            "test-tools",
            "--example",
            example,
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .env("CARGO_INCREMENTAL", "0")
        .process_group(0)
        .stdout(Stdio::from(output.try_clone().unwrap()))
        .stderr(Stdio::from(output))
        .spawn()
        .expect("start owned Rust fixture build");
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            // The process group was created for this exact child and its compiler descendants.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            panic!(
                "Rust {example} fixture build exceeded 60 seconds; see {}",
                log.display()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut diagnostics = String::new();
    fs::File::open(log)
        .unwrap()
        .take(32768)
        .read_to_string(&mut diagnostics)
        .unwrap();
    assert!(
        status.success(),
        "Rust {example} fixture build failed: {diagnostics}"
    );
    let artifact = target.join("release/examples").join(library);
    assert!(
        artifact.is_file(),
        "Rust {example} fixture artifact missing"
    );
    artifact
}
