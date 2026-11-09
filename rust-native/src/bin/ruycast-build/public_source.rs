//! Post-build public-source binding. The clean archive verifier runs before
//! compilation; this guard permits build outputs while binding every source.
use super::*;
#[path = "../ruycast-source/policy.rs"]
mod policy;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: String,
    version: String,
    version_authority: String,
    files: BTreeMap<String, String>,
    modes: BTreeMap<String, u32>,
    roles: BTreeMap<String, String>,
    historical_references: Vec<String>,
    publication: String,
}
fn safe(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\0', '\n', '\r', '\\'])
        && !Path::new(name).is_absolute()
        && name.split('/').all(|part| {
            !part.is_empty()
                && ![
                    ".",
                    "..",
                    ".go",
                    ".git",
                    ".go-workflow",
                    "target",
                    ".build",
                    ".dist",
                    "__pycache__",
                    "profiles",
                    "private",
                    "adapters",
                    "workflows",
                ]
                .contains(&part)
        })
}
pub(super) fn bind(root: &Path) -> Result<String> {
    bind_policy(root, policy::PATHS.lines().map(str::to_owned).collect())
}
fn bind_policy(root: &Path, allowlist: std::collections::BTreeSet<String>) -> Result<String> {
    if fs::symlink_metadata(root.join(".go")).is_ok() {
        return Err("Package from a reviewed public source export, not a private checkout.".into());
    }
    let path = root.join("PUBLIC-SOURCE.json");
    let file = regular_file(&path)?;
    let metadata = file.metadata().map_err(error)?;
    if metadata.nlink() != 1 || metadata.mode() & 0o7777 != 0o644 {
        return Err("Public source manifest mode or link count changed.".into());
    }
    if file.metadata().map_err(error)?.len() > 16 * 1024 * 1024 {
        return Err("Public source manifest exceeds its bound.".into());
    }
    let bytes = regular_bytes(&path)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(error)?;
    let mut canonical = serde_json::to_vec_pretty(&manifest).map_err(error)?;
    canonical.push(b'\n');
    let keys = manifest
        .files
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if allowlist.is_empty()
        || allowlist.iter().any(|name| !safe(name))
        || keys != allowlist
        || manifest
            .modes
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            != keys
        || manifest
            .roles
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            != keys
        || manifest.schema != "ruycast.public-source.v2"
        || manifest.version != rustcast_core::product_version::CURRENT
        || manifest.version != version::read(root)?
        || manifest.version_authority != "rust/src/product_version.rs"
        || manifest.historical_references
            != ["Package.swift", "Sources/", "Tests/LauncherCoreTests/", "bench/reference/0.8.26/"]
        || manifest.publication != "export-only; not published, installed or production-accepted"
        || bytes != canonical
    {
        return Err(
            "Public source manifest is invalid, noncanonical or not the explicit export policy."
                .into(),
        );
    }
    for (name, expected) in &manifest.files {
        if !safe(name) {
            return Err("Private or unsafe public-source path.".into());
        }
        let source = root.join(name);
        let meta = regular_file(&source)?.metadata().map_err(error)?;
        if meta.nlink() != 1
            || meta.len() > 32 * 1024 * 1024
            || !matches!(manifest.modes[name], 0o644 | 0o755)
            || meta.mode() & 0o7777 != manifest.modes[name]
            || digest(&source)? != *expected
        {
            return Err(format!("Public source content or mode changed: {name}"));
        }
    }
    for name in inventory(root)?.keys() {
        if !keys.contains(name) {
            return Err(format!("Unreviewed new production source: {name}"));
        }
    }
    if digest(&path)? != format!("{:x}", Sha256::digest(&bytes)) {
        return Err("Public source manifest changed while binding.".into());
    }
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_postbuild_binding_allows_outputs_and_rejects_tamper_new_sources_and_private_authority()
    {
        use std::os::unix::fs::PermissionsExt;
        let f = super::super::tests::Fixture::new();
        fs::write(
            f.0.join("rust/src/product_version.rs"),
            format!(
                "pub const CURRENT: &str = \"{}\";\n",
                rustcast_core::product_version::CURRENT
            ),
        )
        .unwrap();
        let files = inventory(&f.0).unwrap();
        let paths = files
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        for name in &paths {
            fs::set_permissions(f.0.join(name), fs::Permissions::from_mode(0o644)).unwrap();
        }
        let manifest = Manifest {
            schema: "ruycast.public-source.v2".into(),
            version: rustcast_core::product_version::CURRENT.into(),
            version_authority: "rust/src/product_version.rs".into(),
            modes: files.keys().map(|p| (p.clone(), 0o644)).collect(),
            roles: files
                .keys()
                .map(|p| (p.clone(), "owned synthetic policy fixture".into()))
                .collect(),
            files,
            historical_references: vec![
                "Package.swift".into(),
                "Sources/".into(),
                "Tests/LauncherCoreTests/".into(),
                "bench/reference/0.8.26/".into(),
            ],
            publication: "export-only; not published, installed or production-accepted".into(),
        };
        let mut bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        bytes.push(b'\n');
        fs::write(f.0.join("PUBLIC-SOURCE.json"), bytes).unwrap();
        fs::set_permissions(
            f.0.join("PUBLIC-SOURCE.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let original = bind_policy(&f.0, paths.clone()).unwrap();
        let correct = regular_bytes(&f.0.join("PUBLIC-SOURCE.json")).unwrap();
        for lookalike in [None, Some("bench/reference/0.8.260/")] {
            let mut wrong: Manifest = serde_json::from_slice(&correct).unwrap();
            let references = &mut wrong.historical_references;
            if let Some(prefix) = lookalike {
                references[3] = prefix.into();
            } else {
                references.pop();
            }
            let mut bytes = serde_json::to_vec_pretty(&wrong).unwrap();
            bytes.push(b'\n');
            fs::write(f.0.join("PUBLIC-SOURCE.json"), bytes).unwrap();
            assert!(bind_policy(&f.0, paths.clone()).is_err());
        }
        fs::write(f.0.join("PUBLIC-SOURCE.json"), correct).unwrap();
        assert_eq!(bind_policy(&f.0, paths.clone()).unwrap(), original);
        fs::create_dir_all(f.0.join(".build/owned-output")).unwrap();
        fs::write(
            f.0.join(".build/owned-output/binary"),
            b"owned build output",
        )
        .unwrap();
        assert_eq!(bind_policy(&f.0, paths.clone()).unwrap(), original);
        fs::write(
            f.0.join("rust/src/new_unreviewed.rs"),
            b"pub fn owned() {}\n",
        )
        .unwrap();
        assert!(bind_policy(&f.0, paths.clone()).is_err());
        fs::remove_file(f.0.join("rust/src/new_unreviewed.rs")).unwrap();
        let source = f.0.join("rust/src/lib.rs");
        let before = fs::read(&source).unwrap();
        fs::write(&source, b"owned tamper").unwrap();
        assert!(bind_policy(&f.0, paths.clone()).is_err());
        fs::write(&source, before).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(bind_policy(&f.0, paths.clone()).is_err());
        fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();
        fs::create_dir(f.0.join(".go")).unwrap();
        assert!(bind_policy(&f.0, paths.clone()).is_err());
        fs::remove_dir(f.0.join(".go")).unwrap();
        assert_eq!(bind_policy(&f.0, paths).unwrap(), original);
        // The production guard keeps the real, compiled explicit policy;
        // the deliberately tiny owned policy is not a production bypass.
        assert!(bind(&f.0).is_err());
    }
}
