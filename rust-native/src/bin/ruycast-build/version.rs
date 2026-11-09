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
mod tests {
    use super::*;
    #[test]
    fn owned_literal_matches_the_rust_compiler_and_rejects_link_fifo_directory_or_missing() {
        let fixture = super::super::tests::Fixture::new();
        let path = fixture.0.join(SOURCE);
        for value in ["1.2.3", "17.23.409"] {
            fs::write(&path, format!("pub const CURRENT: &str = \"{value}\";\n")).unwrap();
            let source = fixture.0.join("version_probe.rs");
            fs::write(
                &source,
                "include!(\"rust/src/product_version.rs\"); fn main(){println!(\"{}\",CURRENT);}\n",
            )
            .unwrap();
            let binary = fixture.0.join("version_probe");
            output(
                &fixture.0,
                "rustc",
                &[
                    "--edition=2021",
                    source.to_str().unwrap(),
                    "-o",
                    binary.to_str().unwrap(),
                ],
            )
            .unwrap();
            assert_eq!(read(&fixture.0).unwrap(), value);
            assert_eq!(
                output(&fixture.0, binary.to_str().unwrap(), &[])
                    .unwrap()
                    .trim(),
                value
            );
        }
        let sentinel = fixture.0.join("owned_version_sentinel");
        fs::write(&sentinel, b"owned sentinel").unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&sentinel, &path).unwrap();
        assert!(read(&fixture.0).is_err());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(read(&fixture.0).is_err());
        fs::remove_dir(&path).unwrap();
        let name = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let start = std::time::Instant::now();
        assert!(read(&fixture.0).is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        fs::remove_file(&path).unwrap();
        assert!(read(&fixture.0).is_err());
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
            "pub const CURRENT: &str = String(\"1.2.3\");",
            "pub const CURRENT: &str = \"1.2.3\";pub const CURRENT: &str = \"4.5.6\";",
        ] {
            assert!(parse(source.as_bytes()).is_err(), "{source}");
        }
    }
}
