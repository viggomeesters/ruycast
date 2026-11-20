//! Actual compiler-to-bound-parser inventory. No source-language program is synthesized.
use super::*;
use serde_json::Value;
#[cfg(test)]
use std::collections::BTreeSet;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Parser {
    artifact: String,
    sha256: String,
    sources: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg(test)]
struct Input {
    module: String,
    paths: BTreeSet<String>,
    sdk: String,
    plugins: Vec<String>,
}
fn relative(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && path
            .components()
            .all(|v| matches!(v, std::path::Component::Normal(_)))
}
const REQUIRED_SOURCES: &[&str] = &[
    "rust-native/src/bin/parity_ast.rs",
    "rust-native/src/bin/ruycast-build/parity_compile.rs",
    "rust-native/src/bin/ruycast-build/process.rs",
    "rust-native/src/bin/ruycast-build/preparation.rs",
    "rust-native/src/bin/ruycast-build/parity_inventory.rs",
];
pub(super) fn validate_parser(root: &Path, binding: &Value) -> Result<()> {
    let parser: Parser = serde_json::from_value(binding.clone()).map_err(error)?;
    parser_binding(root, &parser)
}
fn parser_binding(root: &Path, parser: &Parser) -> Result<()> {
    let sha = &parser.sha256;
    if sha.len() != 64
        || !sha
            .bytes()
            .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
        || parser.artifact != format!(".build/delivery-tools/parity-ast-artifacts/{sha}")
    {
        return Err("Invalid canonical AST parser binding.".into());
    }
    for required in REQUIRED_SOURCES {
        if !parser.sources.contains_key(*required) {
            return Err("Incomplete AST parser source binding.".into());
        }
    }
    for (source, hash) in &parser.sources {
        if !relative(source) || digest(&root.join(source))? != *hash {
            return Err("AST parser source binding changed.".into());
        }
    }
    if digest(&root.join(&parser.artifact))? != *sha {
        return Err("AST parser snapshot changed.".into());
    }
    Ok(())
}
#[cfg(test)]
fn sources(root: &Path, module: &str) -> Result<BTreeMap<String, String>> {
    let directory = root.join("Sources").join(module);
    let metadata = fs::symlink_metadata(&directory).map_err(error)?;
    if metadata.is_symlink() || !metadata.is_dir() {
        return Err("Linked/invalid compiler module directory.".into());
    }
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(directory).map_err(error)? {
        let path = entry.map_err(error)?.path();
        if path.extension().is_some_and(|v| v == "swift") {
            let name = path
                .strip_prefix(root)
                .map_err(error)?
                .to_str()
                .ok_or("Non-UTF8 compiler source.")?
                .to_owned();
            files.insert(name, digest(&path)?);
        }
    }
    Ok(files)
}
#[cfg(test)]
fn arguments(root: &Path, input: &Input) -> Result<(Vec<String>, BTreeMap<String, String>)> {
    if input.module.is_empty()
        || !input
            .module
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
        || input.paths.is_empty()
        || !Path::new(&input.sdk).is_absolute()
        || !input.plugins.len().is_multiple_of(2)
    {
        return Err("Invalid compiler module/SDK/plugin request.".into());
    }
    for pair in input.plugins.chunks_exact(2) {
        if ![
            "-plugin-path",
            "-external-plugin-path",
            "-load-plugin-library",
            "-load-plugin-executable",
        ]
        .contains(&pair[0].as_str())
            || pair[1].is_empty()
        {
            return Err("Invalid compiler plugin arguments.".into());
        }
    }
    let before = sources(root, &input.module)?;
    if input
        .paths
        .iter()
        .any(|path| !relative(path) || !before.contains_key(path))
    {
        return Err("Requested compiler primary source missing.".into());
    }
    let mut args = vec![
        "swiftc".into(),
        "-frontend".into(),
        "-dump-ast".into(),
        "-whole-module-optimization".into(),
        "-sdk".into(),
        input.sdk.clone(),
        "-swift-version".into(),
        "6".into(),
        "-module-name".into(),
        input.module.clone(),
        "-I".into(),
        ".build/swift/debug/Modules".into(),
        "-I".into(),
        ".build/swift/debug/CRuycastCore.build".into(),
    ];
    args.extend(input.plugins.clone());
    if input.module != "RuycastApp" {
        args.push("-parse-as-library".into());
    }
    for path in before.keys() {
        if input.paths.contains(path) {
            args.push("-primary-file".into());
        }
        args.push(path.clone());
    }
    Ok((args, before))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    #[test]
    fn parser_snapshot_drift_and_links_are_refused() {
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        let bytes = b"owned parser hash data";
        let sha = format!("{:x}", Sha256::digest(bytes));
        let artifact = format!(".build/delivery-tools/parity-ast-artifacts/{sha}");
        let mut sources = BTreeMap::new();
        for source in REQUIRED_SOURCES {
            let path = root.join(source);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"owned source hash data").unwrap();
            sources.insert((*source).into(), digest(&path).unwrap());
        }
        let path = root.join(&artifact);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        let parser = Parser {
            artifact,
            sha256: sha,
            sources,
        };
        assert!(parser_binding(root, &parser).is_ok());
        fs::write(&path, b"drift").unwrap();
        assert!(parser_binding(root, &parser).is_err());
        fs::remove_file(&path).unwrap();
        let target = root.join("target.data");
        fs::write(&target, bytes).unwrap();
        std::os::unix::fs::symlink(target, &path).unwrap();
        assert!(parser_binding(root, &parser).is_err());
    }
    #[test]
    fn rejects_noncanonical_and_unbound_parser_requests() {
        let root = Path::new("/");
        let mut parser = Parser {
            artifact: "mutable-parser".into(),
            sha256: "a".repeat(64),
            sources: BTreeMap::new(),
        };
        assert!(parser_binding(root, &parser).is_err());
        parser.artifact = format!(
            ".build/delivery-tools/parity-ast-artifacts/{}",
            parser.sha256
        );
        assert!(parser_binding(root, &parser)
            .unwrap_err()
            .contains("Incomplete"));
    }
    #[test]
    fn exact_primary_arguments_and_invalid_requests() {
        let fixture = super::super::tests::Fixture::new();
        let directory = fixture.0.join("Sources/Models");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("A.swift"), b"compiler source hash data").unwrap();
        fs::write(directory.join("B.swift"), b"compiler context hash data").unwrap();
        let mut input = Input {
            module: "Models".into(),
            paths: BTreeSet::from(["Sources/Models/A.swift".into()]),
            sdk: "/sdk".into(),
            plugins: vec![],
        };
        let (args, hashes) = arguments(&fixture.0, &input).unwrap();
        assert_eq!(
            &args[args.len() - 4..],
            [
                "-parse-as-library",
                "-primary-file",
                "Sources/Models/A.swift",
                "Sources/Models/B.swift"
            ]
        );
        assert_eq!(hashes.len(), 2);
        input.paths.insert("Sources/Models/Missing.swift".into());
        assert!(arguments(&fixture.0, &input).is_err());
        input.module = "../escape".into();
        assert!(arguments(&fixture.0, &input).is_err());
    }
}
