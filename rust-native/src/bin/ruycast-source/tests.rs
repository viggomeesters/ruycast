use super::*;
use std::{
    os::unix::fs::{symlink, DirBuilderExt},
    process::Command,
};
struct Fixture {
    path: PathBuf,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::var_os("RUYCAST_SOURCE_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("CARGO_TARGET_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"))
                    .join("source-fixtures")
            });
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join(format!(
            "fs-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        let root = path.join("source");
        fs::create_dir(&root).unwrap();
        let actual = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        for p in policy().unwrap() {
            let to = root.join(&p);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            let from = actual.join(&p);
            // Fixtures need source bytes and executable modes, without inherited
            // macOS provenance metadata from the developer checkout.
            fs::write(&to, fs::read(&from).unwrap()).unwrap();
            fs::set_permissions(&to, fs::metadata(&from).unwrap().permissions()).unwrap();
        }
        Self { path, root }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
#[test]
fn real_owned_export_is_deterministic_private_free_and_bsdtar_extracts() {
    let f = Fixture::new();
    let secret = format!("owned-sensitive-canary-{}", std::process::id()).into_bytes();
    for p in [
        ".go/evidence/private.txt",
        "profiles/profile.json",
        "scripts/personal_bridge.py",
        "rust-native/examples/stage_private.rs",
        ".github/workflows/never.yml",
    ] {
        let p = f.root.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, &secret).unwrap();
    }
    let a = f.path.join("first");
    let b = f.path.join("second");
    let one = prepare(&f.root, &a, || {}).unwrap();
    let two = prepare(&f.root, &b, || {}).unwrap();
    let published: Value = serde_json::from_slice(&fs::read(a.join("PUBLIC-SOURCE.json")).unwrap()).unwrap();
    assert_eq!(published["version"], rustcast_core::product_version::CURRENT);
    assert_eq!(published["versionAuthority"], VERSION_SOURCE);
    assert_eq!(one["version"], published["version"]);
    assert_eq!(fs::read(a.join(VERSION_SOURCE)).unwrap(), VERSION_BYTES);
    assert_eq!(one["archiveSHA256"], two["archiveSHA256"]);
    assert!(verify(&a, Some(&f.root)).is_ok());
    let ab = fs::read(f.path.join("first.tar.gz")).unwrap();
    assert!(
        ab.len()
            < snapshot(&Dir::open(&f.root).unwrap())
                .unwrap()
                .values()
                .map(|i| i.bytes.len())
                .sum::<usize>()
    );
    let tar = archive::gzip(&f.path.join("first.tar.gz"), true, MAX_TOTAL * 2).unwrap();
    assert!(!tar.windows(secret.len()).any(|b| b == secret.as_slice()));
    let extracted = f.path.join("extracted");
    fs::create_dir(&extracted).unwrap();
    assert!(Command::new("/usr/bin/tar")
        .args(["-xpzf"])
        .arg(f.path.join("first.tar.gz"))
        .arg("-C")
        .arg(&extracted)
        .status()
        .unwrap()
        .success());
    let export = snapshot(&Dir::open(&a).unwrap()).unwrap();
    verify_tree(&extracted.join("ruycast"), &export).unwrap();
    let extracted_manifest: Value = serde_json::from_slice(&fs::read(extracted.join("ruycast/PUBLIC-SOURCE.json")).unwrap()).unwrap();
    assert_eq!(extracted_manifest["version"], rustcast_core::product_version::CURRENT);
    assert_eq!(fs::read(extracted.join("ruycast").join(VERSION_SOURCE)).unwrap(), VERSION_BYTES);
    assert!(!a.join(".go").exists());
    let mut examples = fs::read_dir(a.join("rust-native/examples"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    examples.sort();
    assert_eq!(examples, ["clipboard_history_sdk_probe.rs", "snippet_effects_probe.rs", "ui_completion.rs"].map(std::ffi::OsString::from));
    assert!(!a.join("rust-native/examples/stage_private.rs").exists());
}
#[test]
fn no_overwrite_tamper_and_source_binding() {
    let f = Fixture::new();
    let out = f.path.join("public");
    fs::write(f.path.join("public.tar.gz"), b"user artifact").unwrap();
    assert!(prepare(&f.root, &out, || {}).is_err());
    assert!(!out.exists());
    assert_eq!(
        fs::read(f.path.join("public.tar.gz")).unwrap(),
        b"user artifact"
    );
    fs::remove_file(f.path.join("public.tar.gz")).unwrap();
    prepare(&f.root, &out, || {}).unwrap();
    assert!(prepare(&f.root, &out, || {}).is_err());
    fs::write(out.join("README.md"), b"tampered").unwrap();
    assert!(verify(&out, None).is_err());
    fs::copy(f.root.join("README.md"), out.join("README.md")).unwrap();
    assert!(verify(&out, None).is_ok());
    fs::write(f.root.join("README.md"), b"source changed").unwrap();
    assert!(verify(&out, Some(&f.root)).is_err());
    assert!(verify(&out, None).is_ok());
    fs::write(out.join(".env"), b"secret").unwrap();
    assert!(verify(&out, None).is_err());
    fs::remove_file(out.join(".env")).unwrap();
    fs::create_dir(out.join(".git")).unwrap();
    assert!(verify(&out, None).is_err());
    fs::remove_dir(out.join(".git")).unwrap();
    fs::set_permissions(out.join("README.md"), fs::Permissions::from_mode(0o666)).unwrap();
    assert!(verify(&out, None).is_err());
    fs::set_permissions(out.join("README.md"), fs::Permissions::from_mode(0o644)).unwrap();
    let checksum = f.path.join("public.tar.gz.sha256");
    let saved = fs::read(&checksum).unwrap();
    fs::write(&checksum, b"tampered checksum").unwrap();
    assert!(verify(&out, None).is_err());
    fs::write(&checksum, saved).unwrap();
    let archive = f.path.join("public.tar.gz");
    let original = fs::read(&archive).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    fs::write(&archive, changed).unwrap();
    assert!(verify(&out, None).is_err());
    fs::write(&archive, original).unwrap();
}
#[test]
fn linked_special_hardlinked_and_private_inputs_fail_without_outputs() {
    let f = Fixture::new();
    let file = f.root.join("README.md");
    let original = fs::read(&file).unwrap();
    fs::remove_file(&file).unwrap();
    symlink(f.path.join("missing"), &file).unwrap();
    assert!(prepare(&f.root, &f.path.join("symlink"), || {}).is_err());
    fs::remove_file(&file).unwrap();
    let c = CString::new(file.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(prepare(&f.root, &f.path.join("fifo"), || {}).is_err());
    fs::remove_file(&file).unwrap();
    fs::write(&file, &original).unwrap();
    fs::hard_link(&file, f.path.join("other")).unwrap();
    assert!(prepare(&f.root, &f.path.join("hardlink"), || {}).is_err());
    fs::remove_file(f.path.join("other")).unwrap();
    fs::write(
        &file,
        format!("-----BEGIN {}-----\nowned\n", ["PRIVATE", "KEY"].join(" ")),
    )
    .unwrap();
    assert!(prepare(&f.root, &f.path.join("key"), || {}).is_err());
    fs::write(&file, &original).unwrap();
    let actual = f.root.join("rust-native/src");
    fs::rename(&actual, f.path.join("hidden-src")).unwrap();
    symlink(f.path.join("hidden-src"), &actual).unwrap();
    assert!(prepare(&f.root, &f.path.join("ancestor"), || {}).is_err());
}
#[test]
fn private_key_pem_labels_are_generic_and_fail_before_publication() {
    let f = Fixture::new();
    let file = f.root.join("README.md");
    let original = fs::read(&file).unwrap();
    let preserved = f.path.join("preserved.tar.gz");
    fs::write(&preserved, b"existing output must survive").unwrap();
    for (index, label) in [
        "PRIVATE KEY",
        "ENCRYPTED PRIVATE KEY",
        "OPENSSH PRIVATE KEY",
        "DSA PRIVATE KEY",
        "RSA PRIVATE KEY",
        "EC PRIVATE KEY",
        "ML-DSA-65 PRIVATE KEY",
        "UNREVIEWED-FUTURE-ALGORITHM PRIVATE KEY",
        "PGP PRIVATE KEY BLOCK",
        "custom private key",
        "CUSTOM PRIVATE\t  KEY",
    ]
    .into_iter()
    .enumerate()
    {
        // Only constructed bytes form a full private marker; the exported test
        // source and scanner must not contain such a literal marker themselves.
        let bytes = format!("-----BEGIN {label}-----\r\nOWNED_SYNTHETIC_NOT_A_REAL_KEY\r\n-----END {label}-----\r\n");
        assert!(private_bytes(bytes.as_bytes()).is_err(), "{label}");
        fs::write(&file, bytes).unwrap();
        let out = f.path.join(format!("denied-{index}"));
        assert!(prepare(&f.root, &out, || {}).is_err(), "{label}");
        assert!(!out.exists());
        assert!(!out.with_extension("tar.gz").exists());
        assert!(!out.with_extension("tar.gz.sha256").exists());
        assert_eq!(
            fs::read(&preserved).unwrap(),
            b"existing output must survive"
        );
    }
    for label in [
        "PUBLIC KEY",
        "CERTIFICATE",
        "EC PARAMETERS",
        "UNREVIEWED PUBLIC DATA",
    ] {
        let bytes =
            format!("-----BEGIN {label}-----\nOWNED_PUBLIC_TEST_DATA\n-----END {label}-----\n");
        assert!(private_bytes(bytes.as_bytes()).is_ok(), "{label}");
    }
    assert!(private_bytes(b"PRIVATE KEY is ordinary prose without a PEM header").is_ok());
    fs::write(file, original).unwrap();
    let out = f.path.join("positive");
    prepare(&f.root, &out, || {}).unwrap();
    verify(&out, Some(&f.root)).unwrap();
}
#[test]
fn actual_concurrent_mutation_between_copy_and_recheck_is_rejected() {
    let f = Fixture::new();
    let target = f.root.join("README.md");
    let out = f.path.join("concurrent");
    let (start, wait) = std::sync::mpsc::sync_channel(1);
    let (done, ready) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        wait.recv().unwrap();
        fs::write(target, b"mutated during copy").unwrap();
        done.send(()).unwrap();
    });
    assert!(prepare(&f.root, &out, || {
        start.send(()).unwrap();
        ready.recv().unwrap();
    })
    .is_err());
    worker.join().unwrap();
    assert!(!out.exists());
    assert!(!f.path.join("concurrent.tar.gz").exists());
    assert!(!f.path.join("concurrent.tar.gz.sha256").exists());
}
#[test]
fn source_root_replacement_cannot_receive_old_descriptor_bindings() {
    let f = Fixture::new();
    let out = f.path.join("replaced");
    assert!(prepare(&f.root, &out, || {
        fs::rename(&f.root, f.path.join("moved")).unwrap();
        fs::create_dir(&f.root).unwrap();
    })
    .is_err());
    assert!(!out.exists());
    assert!(!f.path.join("replaced.tar.gz").exists());
}
#[test]
fn unreviewed_product_sources_and_changed_public_defaults_are_rejected() {
    let f = Fixture::new();
    let new = f.root.join("rust-native/src/unknown_product.rs");
    fs::write(&new, b"not reviewed").unwrap();
    assert!(prepare(&f.root, &f.path.join("unknown"), || {}).is_err());
    fs::remove_file(new).unwrap();
    let version = f.root.join(VERSION_SOURCE);
    let original_version = fs::read(&version).unwrap();
    let invalid_version = b"pub const CURRENT: &str = \"not-a-version\";\n";
    fs::write(&version, invalid_version).unwrap();
    let invalid_output = f.path.join("invalid-version");
    let error = prepare(&f.root, &invalid_output, || {}).unwrap_err();
    assert!(error.contains("Rust product version source differs from the compiled source tool"));
    assert!(!invalid_output.exists());
    assert!(!f.path.join("invalid-version.tar.gz").exists());
    assert!(!f.path.join("invalid-version.tar.gz.sha256").exists());
    assert_eq!(fs::read(&version).unwrap(), invalid_version);
    fs::write(version, original_version).unwrap();
    fs::write(f.root.join("config/default.json"), b"{\"personal\":true}").unwrap();
    assert!(prepare(&f.root, &f.path.join("default"), || {}).is_err());
}
#[test]
fn ustar_prefix_utf8_zero_padding_modes_and_real_gzip_roundtrip() {
    let f = Fixture::new();
    let path = format!("{}/{}.txt", "prefix".repeat(20), "é".repeat(40));
    let files = BTreeMap::from([
        (path.clone(), b"unicode".to_vec()),
        ("empty".into(), vec![]),
        ("tool.sh".into(), b"#!/bin/sh\nexit 0\n".to_vec()),
    ]);
    let modes = BTreeMap::from([
        (path.clone(), 0o644),
        ("empty".into(), 0o644),
        ("tool.sh".into(), 0o755),
    ]);
    let tar = archive::tar(&files, &modes).unwrap();
    assert_eq!(tar.len() % 512, 0);
    let input = f.path.join("input.tar");
    write_new(&input, &tar, 0o600).unwrap();
    let compressed = archive::gzip(&input, false, tar.len() * 2).unwrap();
    let gzip = f.path.join("fixture.tar.gz");
    write_new(&gzip, &compressed, 0o600).unwrap();
    assert_eq!(archive::gzip(&gzip, true, tar.len()).unwrap(), tar);
    assert!(archive::gzip(&gzip, true, tar.len() - 1).is_err());
    assert!(Command::new("/usr/bin/tar")
        .args(["-xpzf"])
        .arg(&gzip)
        .arg("-C")
        .arg(&f.path)
        .status()
        .unwrap()
        .success());
    assert_eq!(
        fs::read(f.path.join("ruycast").join(path)).unwrap(),
        b"unicode"
    );
    assert_eq!(
        fs::metadata(f.path.join("ruycast/tool.sh")).unwrap().mode() & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(f.path.join("ruycast")).unwrap().mode() & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(f.path.join("ruycast").join("prefix".repeat(20)))
            .unwrap()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(fs::metadata(f.path.join("ruycast/empty")).unwrap().len(), 0);
}
