//! Actual tool CLI metadata/checksum/private-state reads on explicit own files.
//! Never invoke install, prepare, sign, native host or any system signing tool.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let parent = PathBuf::from(
            std::env::var_os("RUYCAST_INSTALL_TEST_ROOT")
                .expect("Owned CLI fixtureparent required"),
        );
        fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!("cli-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(root: &Path, args: &[&str]) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_ruycast-install"))
        .args(args)
        .env_clear()
        .env("HOME", root)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    (
        output.status.success(),
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&output.stdout)})),
    )
}
#[test]
fn actual_installer_cli_help_checksum_and_signing_status_use_only_owned_inputs() {
    let f = Fixture::new();
    let (ok, help) = run(&f.0, &["--help"]);
    assert!(ok);
    assert!(help["text"].as_str().unwrap().contains("verify-download"));
    assert!(!f.0.join(".config").exists());
    let archive = f.0.join("source.tar.gz");
    fs::write(&archive, b"owned CLI source archive").unwrap();
    let digest = format!("{:x}", Sha256::digest(fs::read(&archive).unwrap()));
    let archive_string = archive.display().to_string();
    let (ok, value) = run(
        &f.0,
        &[
            "verify-download",
            "--archive",
            &archive_string,
            "--sha256",
            &digest,
        ],
    );
    assert!(ok);
    assert_eq!(value["sha256"], digest);
    assert!(
        !run(
            &f.0,
            &[
                "verify-download",
                "--archive",
                &archive_string,
                "--sha256",
                &"0".repeat(64)
            ]
        )
        .0
    );
    let signer = f.0.join("signer");
    fs::create_dir(&signer).unwrap();
    fs::set_permissions(&signer, fs::Permissions::from_mode(0o700)).unwrap();
    for name in [
        "identity.keychain-db",
        "keychain-password",
        "certificate.pem",
    ] {
        let path = signer.join(name);
        fs::write(&path, b"owned fake private state").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let fingerprint = "A".repeat(40);
    let identity = json!({"schema":"ruycast.local-signing.v1","name":"Ruycast Local Signing","certificate_sha1":fingerprint,"certificate_sha256":format!("{:x}",Sha256::digest(b"owned fake private state")),"requirement":format!("identifier \"nl.viggomeesters.ruycast\" and certificate leaf = H\"{fingerprint}\"")});
    let path = signer.join("identity.json");
    fs::write(&path, serde_json::to_vec(&identity).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    let repo = f.0.join("repo");
    fs::create_dir(&repo).unwrap();
    let signer_string = signer.display().to_string();
    let repo_string = repo.display().to_string();
    let (ok, value) = run(
        &f.0,
        &[
            "signing",
            "--root",
            &signer_string,
            "status",
            "--repo",
            &repo_string,
        ],
    );
    assert!(ok, "{value}");
    assert_eq!(value, identity);
    let linked = f.0.join("linked-signing");
    symlink(&signer, &linked).unwrap();
    assert!(
        !run(
            &f.0,
            &[
                "signing",
                "status",
                "--root",
                &linked.display().to_string(),
                "--repo",
                &repo_string
            ]
        )
        .0
    );
    assert!(!f.0.join(".config").exists());
    assert!(!f.0.join("Applications").exists());
}
