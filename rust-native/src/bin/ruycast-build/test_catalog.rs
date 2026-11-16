//! Compiler-derived test candidates. Listing never runs test bodies or accepts parity.
use super::*;
use std::{collections::BTreeSet, time::Duration};
#[derive(Clone, Serialize)]
struct Target {
    name: String,
    source: String,
    selectors: Vec<String>,
}
fn resolve(
    root: &Path,
    metadata: &serde_json::Value,
    selectors: &BTreeSet<String>,
) -> Result<(Vec<Target>, Vec<String>)> {
    if selectors.is_empty()
        || selectors.iter().any(|name| {
            name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
    {
        return Err("Expected nonempty candidate selectors.".into());
    }
    let manifest = root.join("rust-native/Cargo.toml");
    let package = metadata["packages"]
        .as_array()
        .ok_or("Missing Cargo package metadata.")?
        .iter()
        .find(|package| package["manifest_path"].as_str() == manifest.to_str())
        .ok_or("Actual native package missing from Cargo metadata.")?;
    let mut targets = Vec::new();
    let mut unresolved = Vec::new();
    for selector in selectors {
        let source = format!("rust-native/tests/{selector}_process.rs");
        let matches = package["targets"]
            .as_array()
            .ok_or("Missing Cargo targets.")?
            .iter()
            .filter(|target| {
                target["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|v| v == "test"))
                    && target["src_path"].as_str() == root.join(&source).to_str()
            })
            .collect::<Vec<_>>();
        if matches.is_empty() {
            unresolved.push(selector.clone());
            continue;
        }
        if matches.len() != 1 || matches[0]["test"] != true {
            return Err("Ambiguous or non-test compiler target.".into());
        }
        regular_file(&root.join(&source))?;
        targets.push(Target {
            name: matches[0]["name"]
                .as_str()
                .ok_or("Invalid Cargo target name.")?
                .into(),
            source,
            selectors: vec![selector.clone()],
        });
    }
    Ok((targets, unresolved))
}
fn capture(
    root: &Path,
    lease: &BuildLock,
    mut command: Command,
    timeout: Duration,
) -> Result<process::Captured> {
    command.current_dir(root);
    for (name, _) in std::env::vars_os() {
        let name_text = name.to_string_lossy();
        if name_text.starts_with("DYLD_")
            || name_text.starts_with("RUYCAST_")
            || name_text.starts_with("RUSTCAST_")
        {
            command.env_remove(name);
        }
    }
    command.env("CARGO_INCREMENTAL", "0");
    lease.apply_to_child(&mut command)?;
    lease.validate()?;
    let output = process::capture(command, timeout, 64 * 1024 * 1024)?;
    lease.validate()?;
    if !output.status.success() {
        return Err(format!(
            "Test candidate compiler/list command failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output)
}
fn source_tree(root: &Path, path: &Path, values: &mut BTreeMap<String, String>) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if metadata.is_symlink() {
        return Err("Linked test candidate source.".into());
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(error)? {
            source_tree(root, &entry.map_err(error)?.path(), values)?;
        }
    } else if metadata.is_file() {
        values.insert(
            path.strip_prefix(root)
                .map_err(error)?
                .to_str()
                .ok_or("Non-UTF8 candidate source.")?
                .into(),
            digest(path)?,
        );
    } else {
        return Err("Nonregular candidate source.".into());
    }
    Ok(())
}
fn sources(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for name in [
        "rust-native/src",
        "rust-native/tests",
        "rust/src",
        "rust-native/Cargo.toml",
        "rust-native/Cargo.lock",
        "rust/Cargo.toml",
        "rust/Cargo.lock",
        "rust-toolchain.toml",
        "config/default.json",
        "rust-native/resources/parity-families.json",
    ] {
        source_tree(root, &root.join(name), &mut values)?;
    }
    for name in ["rust-native/build.rs", "rust/build.rs", ".cargo"] {
        if root.join(name).exists() {
            source_tree(root, &root.join(name), &mut values)?;
        }
    }
    Ok(values)
}
fn parse_list(text: &str) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let name = line
            .strip_suffix(": test")
            .ok_or("Non-libtest output in candidate listing.")?;
        if name.is_empty() || names.iter().any(|prior| prior == name) {
            return Err("Empty or duplicate libtest candidate name.".into());
        }
        names.push(name.to_owned());
    }
    names.sort();
    Ok(names)
}
fn executable_rows(text: &str, targets: &[Target]) -> Result<BTreeMap<String, PathBuf>> {
    let mut rows = BTreeMap::new();
    let mut complete = false;
    for line in text.lines() {
        let row: serde_json::Value = serde_json::from_str(line).map_err(error)?;
        if row["reason"] == "build-finished" {
            complete = row["success"] == true;
        }
        if row["reason"] == "compiler-artifact"
            && row["profile"]["test"] == true
            && row["target"]["kind"]
                .as_array()
                .is_some_and(|k| k.iter().any(|v| v == "test"))
        {
            let name = row["target"]["name"]
                .as_str()
                .ok_or("Missing test artifact name.")?;
            if targets.iter().any(|target| target.name == name) {
                let path = PathBuf::from(
                    row["executable"]
                        .as_str()
                        .ok_or("Missing compiler test executable.")?,
                );
                if !path.is_absolute() || rows.insert(name.into(), path).is_some() {
                    return Err("Invalid or duplicate compiler executable binding.".into());
                }
            }
        }
    }
    if !complete || rows.len() != targets.len() {
        return Err("Compiler did not emit every selected test executable.".into());
    }
    Ok(rows)
}
pub(super) fn run(
    root: &Path,
    lease: &BuildLock,
    selectors: BTreeSet<String>,
) -> Result<serde_json::Value> {
    let before = sources(root)?;
    let manifest =
        String::from_utf8(regular_bytes(&root.join("rust-native/Cargo.toml"))?).map_err(error)?;
    // Metadata does not expose harness=false. This read-only route supports the
    // current ordinary libtest manifest; custom/escaped harness syntax refuses.
    if manifest.contains("harness") || manifest.contains("\\u") || manifest.contains("\\U") {
        return Err(
            "Custom/escaped test harness manifests are unsupported for safe listing.".into(),
        );
    }
    let mut metadata_command = Command::new("cargo");
    metadata_command.args([
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
        "--locked",
        "--offline",
        "--manifest-path",
        "rust-native/Cargo.toml",
    ]);
    let metadata = capture(root, lease, metadata_command, Duration::from_secs(30))?;
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout).map_err(error)?;
    let (targets, unresolved) = resolve(root, &metadata, &selectors)?;
    if targets.is_empty() {
        return Err("No actual Rust test targets resolved from these selectors.".into());
    }
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".build/full-rust-release/native"));
    if !target_dir.is_absolute() || target_dir == root.join(".build/delivery-tools") {
        return Err(
            "Choose an absolute test build directory separate from the bootstrap tool.".into(),
        );
    }
    let mut arguments = vec![
        "test".into(),
        "--no-run".into(),
        "--release".into(),
        "--features".into(),
        "test-tools".into(),
        "--locked".into(),
        "--offline".into(),
        "--manifest-path".into(),
        "rust-native/Cargo.toml".into(),
        "--target-dir".into(),
        target_dir
            .to_str()
            .ok_or("Non-UTF8 build target path.")?
            .into(),
        "--message-format=json".into(),
    ];
    for target in &targets {
        arguments.extend(["--test".into(), target.name.clone()]);
    }
    let mut compiler = Command::new("cargo");
    compiler.args(&arguments);
    let built = capture(root, lease, compiler, Duration::from_secs(180))?;
    let executables =
        executable_rows(std::str::from_utf8(&built.stdout).map_err(error)?, &targets)?;
    let home = preparation::Stage::new(root, "parity-test-list")?;
    let mut candidates = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    for target in &targets {
        let bytes = regular_bytes(&executables[&target.name])?;
        let sha = format!("{:x}", Sha256::digest(&bytes));
        let artifact = format!(".build/parity-test-artifacts/{sha}");
        validation::publish_rust_policy_snapshot(&root.join(&artifact), &bytes, &sha, || Ok(()))?;
        if digest(&root.join(&artifact))? != sha {
            return Err("Candidate executable changed before listing.".into());
        }
        let mut list = Command::new(root.join(&artifact));
        list.args(["--list", "--format", "terse"])
            .env_clear()
            .env("HOME", home.path())
            .env("TMPDIR", home.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let listed = process::capture(list, Duration::from_secs(6), 8 * 1024 * 1024)?;
        if !listed.status.success() {
            return Err(format!(
                "Candidate test listing refused: {}",
                String::from_utf8_lossy(&listed.stderr)
            ));
        }
        if digest(&root.join(&artifact))? != sha {
            return Err("Candidate executable changed during listing.".into());
        }
        let names = parse_list(std::str::from_utf8(&listed.stdout).map_err(error)?)?;
        bindings.insert(target.source.clone(),serde_json::json!({"target":target.name,"artifact":artifact,"sha256":sha,"arguments":["--list","--format","terse"],"tests":names.len()}));
        candidates.insert(target.source.clone(), names);
    }
    if before != sources(root)? {
        return Err("Candidate source inputs changed during compiler inventory.".into());
    }
    Ok(
        serde_json::json!({"schema":"ruycast.rust-test-candidates.v1","testCandidates":candidates,"targets":targets,"unresolvedSelectors":unresolved,"sources":before,"artifacts":bindings,"compilerArguments":arguments,"testBodiesExecuted":false,"acceptance":"Unaccepted candidate inventory only; compiler names do not prove behavioral parity."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn libtest_names_require_exact_listing_lines() {
        assert_eq!(
            parse_list("a::one: test\nb::two: test\n").unwrap(),
            vec!["a::one", "b::two"]
        );
        for text in [
            "running tests",
            "x: benchmark",
            "x: test\nx: test\n",
            ": test",
        ] {
            assert!(parse_list(text).is_err());
        }
    }
    #[test]
    fn target_resolution_uses_actual_cargo_test_sources_and_exposes_gaps() {
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        fs::create_dir_all(root.join("rust-native/tests")).unwrap();
        fs::write(
            root.join("rust-native/tests/runtime_process.rs"),
            b"// owned source fixture",
        )
        .unwrap();
        let metadata = serde_json::json!({"packages":[{"manifest_path":root.join("rust-native/Cargo.toml"),"targets":[{"name":"runtime_process","kind":["test"],"test":true,"src_path":root.join("rust-native/tests/runtime_process.rs")}]}]});
        let selectors = ["runtime".into(), "missing".into()].into_iter().collect();
        let (targets, gaps) = resolve(root, &metadata, &selectors).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "runtime_process");
        assert_eq!(gaps, vec!["missing"]);
        assert!(resolve(root, &metadata, &["../escape".into()].into_iter().collect()).is_err());
    }
}
