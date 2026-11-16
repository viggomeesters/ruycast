//! Strict reader for the one Rust product-version literal, never Swift or a
//! runtime profile. Comments and whitespace do not introduce extra authority.
use super::*;
const SOURCE: &str = "rust/src/product_version.rs";
#[derive(Debug, PartialEq)]
enum Token {
    Word(String),
    Literal(String),
    Punctuation(u8),
}
fn invalid() -> String {
    "Expected one canonical Rust CURRENT semantic-version literal.".into()
}
fn semantic(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
}
fn parse(bytes: &[u8]) -> Result<String> {
    let source = std::str::from_utf8(bytes)
        .map_err(|_| invalid())?
        .as_bytes();
    let mut tokens = vec![];
    let mut at = 0;
    while at < source.len() {
        let byte = source[at];
        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if source[at..].starts_with(b"//") {
            at += 2;
            while at < source.len() && source[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = at;
            at += 1;
            while at < source.len() && (source[at].is_ascii_alphanumeric() || source[at] == b'_') {
                at += 1;
            }
            tokens.push(Token::Word(
                std::str::from_utf8(&source[start..at])
                    .map_err(|_| invalid())?
                    .into(),
            ));
        } else if byte == b'"' {
            at += 1;
            let start = at;
            while at < source.len() && source[at] != b'"' {
                if source[at] == b'\\' || source[at] < b' ' || !source[at].is_ascii() {
                    return Err(invalid());
                }
                at += 1;
            }
            if at == source.len() {
                return Err(invalid());
            }
            tokens.push(Token::Literal(
                std::str::from_utf8(&source[start..at])
                    .map_err(|_| invalid())?
                    .into(),
            ));
            at += 1;
        } else if b":&=;".contains(&byte) {
            tokens.push(Token::Punctuation(byte));
            at += 1;
        } else {
            return Err(invalid());
        }
    }
    match tokens.as_slice() {
        [Token::Word(public), Token::Word(constant), Token::Word(name), Token::Punctuation(b':'), Token::Punctuation(b'&'), Token::Word(kind), Token::Punctuation(b'='), Token::Literal(value), Token::Punctuation(b';')]
            if public == "pub"
                && constant == "const"
                && name == "CURRENT"
                && kind == "str"
                && semantic(value) =>
        {
            Ok(value.clone())
        }
        _ => Err(invalid()),
    }
}
pub(super) fn read(root: &Path) -> Result<String> {
    let path = root.join(SOURCE);
    let mut file = regular_file(&path).map_err(|_| {
        "Cannot read canonical product version as a regular source file.".to_string()
    })?;
    let before = file.metadata().map_err(error)?;
    if before.len() > 65536 || before.nlink() != 1 {
        return Err("Canonical product version must be a bounded single-link source file.".into());
    }
    let mut bytes = vec![];
    (&mut file)
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    let after = file.metadata().map_err(error)?;
    if bytes.len() > 65536
        || (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
        )
    {
        return Err("Canonical product version changed while reading.".into());
    }
    parse(&bytes)
}
#[cfg(test)]
#[path = "../ruycast-source/policy.rs"]
mod export_policy;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Owned Rust child for bounded nonregular-source reader regression"]
    fn nonregular_reader_child() {
        let root = PathBuf::from(std::env::var_os("RUYCAST_VERSION_FIFO_ROOT").unwrap());
        let error = read(&root).unwrap_err();
        eprintln!("{error}");
        std::process::exit(1);
    }
    #[test]
    fn changed_compiled_version_exports_and_invalid_source_leaves_no_outputs() {
        use std::time::Duration;
        let fixture = super::super::tests::Fixture::new();
        let source = fixture.0.join("public-source");
        fs::create_dir(&source).unwrap();
        let actual = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        for name in export_policy::PATHS.lines().filter(|name| !name.is_empty()) {
            let to = source.join(name);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::write(&to, regular_bytes(&actual.join(name)).unwrap()).unwrap();
            fs::set_permissions(&to, fs::metadata(actual.join(name)).unwrap().permissions())
                .unwrap();
        }
        let path = source.join(SOURCE);
        let original = fs::read_to_string(&path).unwrap();
        let changed = original.replace(
            &format!("\"{}\"", rustcast_core::product_version::CURRENT),
            "\"17.23.409\"",
        );
        assert_ne!(original, changed);
        fs::write(&path, &changed).unwrap();
        assert_eq!(read(&source).unwrap(), "17.23.409");
        let target = fixture.0.join("variant-target");
        let mut compiler = Command::new("cargo");
        compiler
            .args([
                "build",
                "--quiet",
                "--locked",
                "--offline",
                "--manifest-path",
            ])
            .arg(source.join("rust-native/Cargo.toml"))
            .arg("--target-dir")
            .arg(&target)
            .args(["--bin", "ruycast-source"])
            .current_dir(&source)
            .env("CARGO_INCREMENTAL", "0");
        let compiled =
            process::capture(compiler, Duration::from_secs(180), 16 * 1024 * 1024).unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let tool = target.join("debug/ruycast-source");
        let output = fixture.0.join("export");
        let mut command = Command::new(&tool);
        command
            .arg("export")
            .arg("--repo")
            .arg(&source)
            .arg("--out")
            .arg(&output);
        let result = process::capture(command, Duration::from_secs(30), 1024 * 1024).unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let response: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join("PUBLIC-SOURCE.json")).unwrap()).unwrap();
        assert_eq!(manifest["version"], "17.23.409");
        assert_eq!(manifest["versionAuthority"], SOURCE);
        assert_eq!(read(&output).unwrap(), "17.23.409");
        assert!(manifest["files"]
            .get("scripts/product_version.py")
            .is_some());
        assert!(manifest["files"]
            .get("rust-native/src/bin/ruycast-build/version.rs")
            .is_some());
        assert!(manifest["files"]
            .get("Tests/test_product_version.py")
            .is_none());
        assert!(Path::new(response["archive"].as_str().unwrap()).is_file());
        let invalid = b"pub const CURRENT: &str = \"not-a-version\";\n";
        fs::write(&path, invalid).unwrap();
        assert!(read(&source).unwrap_err().contains("one canonical"));
        let refused = fixture.0.join("invalid-export");
        let mut command = Command::new(&tool);
        command
            .arg("export")
            .arg("--repo")
            .arg(&source)
            .arg("--out")
            .arg(&refused);
        let result = process::capture(command, Duration::from_secs(30), 1024 * 1024).unwrap();
        assert!(!result.status.success());
        assert!(!refused.exists());
        assert!(!fixture.0.join("invalid-export.tar.gz").exists());
        assert!(!fixture.0.join("invalid-export.tar.gz.sha256").exists());
        assert_eq!(fs::read(path).unwrap(), invalid);
    }
    #[test]
    fn owned_literal_matches_the_rust_compiler_and_rejects_link_fifo_directory_or_missing() {
        let fixture = super::super::tests::Fixture::new();
        let path = fixture.0.join(SOURCE);
        for value in ["0.0.1", "1.2.3", "17.23.409"] {
            fs::write(&path, format!("pub const CURRENT: &str = \"{value}\";\n")).unwrap();
            let source = fixture.0.join("version_probe.rs");
            fs::write(
                &source,
                "include!(\"rust/src/product_version.rs\"); fn main(){println!(\"{}\",CURRENT);}\n",
            )
            .unwrap();
            let binary = fixture.0.join("version_probe");
            let mut compiler = Command::new("rustc");
            compiler
                .arg("--edition=2021")
                .arg(&source)
                .arg("-o")
                .arg(&binary)
                .current_dir(&fixture.0);
            let compiled =
                process::capture(compiler, std::time::Duration::from_secs(60), 1024 * 1024)
                    .unwrap();
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            assert_eq!(read(&fixture.0).unwrap(), value);
            let ran = process::capture(
                Command::new(&binary),
                std::time::Duration::from_secs(2),
                65536,
            )
            .unwrap();
            assert!(ran.status.success());
            assert_eq!(String::from_utf8(ran.stdout).unwrap().trim(), value);
        }
        let sentinel = fixture.0.join("owned_version_sentinel");
        fs::write(&sentinel, b"owned sentinel").unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&sentinel, &path).unwrap();
        assert!(read(&fixture.0).unwrap_err().contains("regular source"));
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(read(&fixture.0).unwrap_err().contains("regular source"));
        fs::remove_dir(&path).unwrap();
        let name = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let start = std::time::Instant::now();
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--ignored",
                "--exact",
                "version::tests::nonregular_reader_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("RUYCAST_VERSION_FIFO_ROOT", &fixture.0);
        let rejected = process::capture(child, std::time::Duration::from_secs(2), 65536).unwrap();
        assert!(!rejected.status.success());
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("regular source"));
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        fs::remove_file(&path).unwrap();
        assert!(read(&fixture.0).unwrap_err().contains("Cannot read"));
        assert_eq!(fs::read(sentinel).unwrap(), b"owned sentinel");
    }
    #[test]
    fn owned_rust_literal_tokens_preserve_whitespace_and_reject_ambiguity() {
        for value in ["0.0.1", "17.23.409", "0.9.0"] {
            let source = format!("//! Owned version\npub\nconst CURRENT : & str = \"{value}\" ;\n");
            assert_eq!(parse(source.as_bytes()).unwrap(), value);
        }
        for source in [
            "pubconst CURRENT: &str = \"1.2.3\";",
            "pub const CURRENT: &str = \"01.2.3\";",
            "pub const CURRENT: &str = \"1.2\";",
            "pub const CURRENT: &str = \"1.2.3-beta\";",
            "pub const CURRENT: &str = \"invalid\";",
            "pub const CURRENT: &str = String(\"1.2.3\");",
            "pub const CURRENT: &str = \"1.2.3\";pub const CURRENT: &str = \"4.5.6\";",
        ] {
            assert!(
                parse(source.as_bytes())
                    .unwrap_err()
                    .contains("one canonical"),
                "{source}"
            );
        }
    }
}
