//! The current Rust build tool owns source/target/preparation verification.
//! This consumer binds its exact release receipt and copies only its two outputs.
use super::*;
use serde::Deserialize;
pub const SCHEMA: &str = "ruycast.rust-development-build.v4";
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    schema: String,
    pub product_version: String,
    pub target: String,
    pub sources: BTreeMap<String, String>,
    pub artifacts: BTreeMap<String, String>,
    linked_libraries: BTreeMap<String, Vec<String>>,
    toolchains: BTreeMap<String, String>,
    commands: Vec<Vec<String>>,
    environment_overrides: BTreeMap<String, String>,
    production_accepted: bool,
    installed_or_published: bool,
    configuration: String,
    release_preparation: Value,
}
impl Receipt {
    pub fn paths(&self) -> [(String, &'static str); 2] {
        [
            (
                format!(
                    ".build/full-rust-release/native/{}/release/ruycast-native",
                    self.target
                ),
                "host",
            ),
            (
                format!(
                    ".build/full-rust-release/core/{}/release/ruycast",
                    self.target
                ),
                "cli",
            ),
        ]
    }
}
pub fn verify(
    root: &Path,
    home: &Path,
    lease: &Lease,
    r: &mut dyn tools::Runner,
) -> Result<(Receipt, Vec<u8>)> {
    lease.validate()?;
    let authority=tools::checked(r,compiler_call(root,home,lease,"verify")?).map_err(|_|"Rust release verification failed; run scripts/install.sh to create a fresh prepared build.".to_string())?.json()?;
    lease.validate()?;
    let path = root.join(".build/full-rust-release/development-build.json");
    if authority["ok"] != true
        || authority["schema"] != SCHEMA
        || authority["configuration"] != "release"
        || authority["receipt"] != json!(path)
    {
        return Err(
            "Current Rust build authority did not verify the exact release receipt.".into(),
        );
    }
    let raw = fs::bytes(&path)?;
    let value: Receipt = serde_json::from_slice(&raw).map_err(err)?;
    if value.schema != SCHEMA
        || value.configuration != "release"
        || value.product_version != VERSION
        || value.production_accepted
        || value.installed_or_published
        || !value.release_preparation.is_object()
        || value.sources.is_empty()
        || value.artifacts.len() != 2
        || value.linked_libraries.len() != 2
        || !value.toolchains.contains_key("cargo")
        || !value.toolchains.contains_key("rustc")
        || value.commands.is_empty()
        || !matches!(
            value.target.as_str(),
            "aarch64-apple-darwin" | "x86_64-apple-darwin"
        )
        || authority["target"] != value.target
        || authority["artifacts"] != json!(value.artifacts)
        || authority["releasePreparation"] != value.release_preparation
    {
        return Err("Invalid, unprepared or mismatching current Rust release receipt.".into());
    }
    let prep_path = ".build/full-rust-release/release-preparation.json";
    if value
        .release_preparation
        .as_object()
        .is_none_or(|m| m.len() != 2)
        || value.release_preparation["path"] != prep_path
        || value.release_preparation["sha256"] != fs::hash(&root.join(prep_path))?
    {
        return Err("Verified release preparation changed before consumption.".into());
    }
    for (source, _) in value.paths() {
        if value.artifacts.get(&source) != Some(&fs::hash(&root.join(&source))?) {
            return Err("Verified Rust artifact changed before consumption.".into());
        }
    }
    let _ = &value.environment_overrides;
    if fs::bytes(&path)? != raw {
        return Err("Release receipt changed during verification.".into());
    }
    lease.validate()?;
    Ok((value, raw))
}
#[cfg(test)]
pub fn inventory(root: &Path) -> Result<BTreeMap<String, String>> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<String, String>) -> Result<()> {
        fs::no_links(at)?;
        let m = std::fs::symlink_metadata(at).map_err(err)?;
        if m.is_dir() {
            for item in std::fs::read_dir(at).map_err(err)? {
                walk(root, &item.map_err(err)?.path(), out)?;
            }
        } else {
            out.insert(
                at.strip_prefix(root)
                    .map_err(err)?
                    .to_str()
                    .ok_or("Non-UTF8 fixture input.")?
                    .into(),
                fs::hash(at)?,
            );
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    for name in [
        "rust/Cargo.toml",
        "rust/Cargo.lock",
        "rust/src",
        "rust-native/Cargo.toml",
        "rust-native/Cargo.lock",
        "rust-native/src",
        "config/default.json",
    ] {
        walk(root, &root.join(name), &mut out)?;
    }
    Ok(out)
}
