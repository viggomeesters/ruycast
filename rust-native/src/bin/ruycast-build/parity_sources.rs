//! Bind the actual production source roster to an explicit reference checkout.
use super::*;
use serde_json::{json, Value};
fn walk(root: &Path, directory: &Path, sources: &mut BTreeMap<String, String>) -> Result<()> {
    let metadata = fs::symlink_metadata(directory).map_err(error)?;
    if metadata.is_symlink() || !metadata.is_dir() {
        return Err("Expected an unlinked production source directory.".into());
    }
    for entry in fs::read_dir(directory).map_err(error)? {
        let entry = entry.map_err(error)?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or("Production source path is not UTF-8.")?;
        // Original inventory excludes every path component containing Checks.
        if name.contains("Checks") {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(error)?;
        if metadata.is_symlink() {
            return Err("Linked production source input.".into());
        }
        if metadata.is_dir() {
            if name.ends_with(".swift") {
                return Err("Production Swift input is a directory.".into());
            }
            walk(root, &path, sources)?;
        } else if name.ends_with(".swift") {
            let relative = path
                .strip_prefix(root)
                .map_err(error)?
                .to_str()
                .ok_or("Production source path is not UTF-8.")?
                .to_owned();
            sources.insert(relative, digest(&path)?);
        }
    }
    Ok(())
}
fn snapshot(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut sources = BTreeMap::new();
    walk(root, &root.join("Sources"), &mut sources)?;
    if sources.is_empty() {
        return Err("Empty production source inventory.".into());
    }
    Ok(sources)
}
fn compare(
    root: &Path,
    reference: &Path,
    owners: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Value>> {
    let current = snapshot(root)?;
    let accepted = snapshot(reference)?;
    if current.keys().ne(accepted.keys()) {
        return Err("Production source inventories differ from accepted release checkout".into());
    }
    let mut modules = BTreeMap::new();
    for (path, sha) in &current {
        let name = Path::new(path)
            .file_stem()
            .and_then(|v| v.to_str())
            .ok_or("Invalid production module name.")?;
        let owner = owners
            .get(name)
            .ok_or_else(|| format!("Unclassified production module: {path}"))?;
        if accepted.get(path) != Some(sha) {
            return Err(format!("Reference source differs: {path}"));
        }
        modules.insert(path.clone(), json!({"sha256":sha,"contractFamily":owner}));
    }
    if snapshot(root)? != current || snapshot(reference)? != accepted {
        return Err("Production/reference source inputs changed during inventory.".into());
    }
    Ok(modules)
}
pub(super) fn run(root: &Path, reference: &Path) -> Result<Value> {
    let (owners, definitions) = parity_families::owners(root)?;
    let sources = compare(root, reference, &owners)?;
    if parity_families::owners(root)?.1 != definitions {
        return Err("Contract family definitions changed during source inventory.".into());
    }
    Ok(json!({"sourceFiles":sources,"definitionsSHA256":definitions}))
}
#[cfg(test)]
mod tests {
    use super::super::tests::Fixture;
    use super::*;
    fn sources(root: &Path) {
        fs::create_dir_all(root.join("Sources/Models")).unwrap();
        fs::write(root.join("Sources/Models/Files.swift"), b"reference bytes").unwrap();
    }
    fn owners() -> BTreeMap<String, String> {
        BTreeMap::from([("Files".into(), "profile-configuration".into())])
    }
    #[test]
    fn exact_reference_hashes_and_checks_exclusion() {
        let current = Fixture::new();
        let reference = Fixture::new();
        sources(&current.0);
        sources(&reference.0);
        fs::create_dir_all(current.0.join("Sources/ExtraChecks")).unwrap();
        fs::write(
            current.0.join("Sources/ExtraChecks/Ignored.swift"),
            b"excluded data",
        )
        .unwrap();
        let value = compare(&current.0, &reference.0, &owners()).unwrap();
        assert_eq!(value.len(), 1);
        assert_eq!(
            value["Sources/Models/Files.swift"]["sha256"],
            digest(&current.0.join("Sources/Models/Files.swift")).unwrap()
        );
        assert_eq!(
            value["Sources/Models/Files.swift"]["contractFamily"],
            "profile-configuration"
        );
        fs::write(
            reference.0.join("Sources/Models/Files.swift"),
            b"different data",
        )
        .unwrap();
        assert!(compare(&current.0, &reference.0, &owners())
            .unwrap_err()
            .contains("Reference source differs"));
    }
    #[test]
    fn refuses_roster_mismatch_unclassified_and_links() {
        let current = Fixture::new();
        let reference = Fixture::new();
        sources(&current.0);
        sources(&reference.0);
        assert!(compare(&current.0, &reference.0, &BTreeMap::new())
            .unwrap_err()
            .contains("Unclassified"));
        fs::write(current.0.join("Sources/Added.swift"), b"extra data").unwrap();
        assert!(compare(&current.0, &reference.0, &owners())
            .unwrap_err()
            .contains("inventories differ"));
        fs::remove_file(current.0.join("Sources/Added.swift")).unwrap();
        let file = current.0.join("Sources/Models/Files.swift");
        fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(reference.0.join("Sources/Models/Files.swift"), &file).unwrap();
        assert!(compare(&current.0, &reference.0, &owners()).is_err());
        fs::remove_file(file).unwrap();
        fs::remove_dir(current.0.join("Sources/Models")).unwrap();
        std::os::unix::fs::symlink(
            reference.0.join("Sources/Models"),
            current.0.join("Sources/Models"),
        )
        .unwrap();
        assert!(snapshot(&current.0).is_err());
    }
}
