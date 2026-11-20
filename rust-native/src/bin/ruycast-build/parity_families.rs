//! Contract-family metadata and source/test candidates remain unaccepted.
use super::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
const DEFINITIONS: &str = "rust-native/resources/parity-families.json";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    id: String,
    files: Vec<String>,
    rust: Vec<String>,
    tests: Vec<String>,
    contract: String,
    remaining: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    source_families: BTreeMap<String, String>,
    test_inventory: TestInventory,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestInventory {
    targets: Vec<Target>,
    test_candidates: BTreeMap<String, Value>,
    unresolved_selectors: Vec<String>,
}
#[derive(Deserialize)]
struct Target {
    source: String,
    selectors: Vec<String>,
}
fn definitions(bytes: &[u8]) -> Result<Vec<Family>> {
    let groups: Vec<Family> = serde_json::from_slice(bytes).map_err(error)?;
    let mut ids = BTreeSet::new();
    let mut owners = BTreeSet::new();
    for group in &groups {
        if group.id.is_empty() || !ids.insert(&group.id) {
            return Err("Ambiguous contract family identity.".into());
        }
        for name in group.files.iter().chain(&group.rust).chain(&group.tests) {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
            {
                return Err("Invalid contract family source/test selector.".into());
            }
        }
        for name in &group.files {
            if !owners.insert(name) {
                return Err(format!("Ambiguous module ownership: {name}"));
            }
        }
    }
    Ok(groups)
}
fn assemble(root: &Path, groups: Vec<Family>, input: Input) -> Result<Vec<Value>> {
    let owners = groups
        .iter()
        .flat_map(|g| g.files.iter().map(|name| (name.as_str(), g.id.as_str())))
        .collect::<BTreeMap<_, _>>();
    for (path, family) in &input.source_families {
        let path = Path::new(path);
        let name = path
            .file_stem()
            .and_then(|v| v.to_str())
            .ok_or("Invalid production module path.")?;
        if path.is_absolute()
            || !path.starts_with("Sources")
            || path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || owners.get(name) != Some(&family.as_str())
        {
            return Err(format!(
                "Unclassified production module or family mismatch: {}",
                path.display()
            ));
        }
    }
    groups.into_iter().map(|group| {
        let mut row = serde_json::to_value(&group).map_err(error)?;
        row["files"] = json!(input.source_families.iter().filter_map(|(path,id)| (id==&group.id).then_some(path)).collect::<Vec<_>>());
        let mut sources = Vec::new();
        for name in &group.rust {
            let path = format!("rust-native/src/{name}.rs");
            if root.join(&path).exists() { regular_file(&root.join(&path))?;sources.push(path); }
        }
        let mut candidates = BTreeMap::new();
        for target in &input.test_inventory.targets {
            if target.selectors.iter().any(|name| group.tests.contains(name)) {
                candidates.insert(&target.source,input.test_inventory.test_candidates.get(&target.source).ok_or("Missing test target candidates.")?);
            }
        }
        row["rustSources"] = json!(sources);
        row["testCandidates"] = json!(candidates);
        row["testCandidateGaps"] = json!(group.tests.iter().filter(|name| input.test_inventory.unresolved_selectors.contains(name)).collect::<Vec<_>>());
        row["acceptance"] = json!("Not accepted as a complete family. Candidate source/test crosswalk; compiler inventory and test names do not prove behavioral parity.");
        Ok(row)
    }).collect()
}
pub(super) fn owners(root: &Path) -> Result<(BTreeMap<String, String>, String)> {
    let bytes = regular_bytes(&root.join(DEFINITIONS))?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let groups = definitions(&bytes)?;
    let owners = groups
        .into_iter()
        .flat_map(|group| {
            group
                .files
                .into_iter()
                .map(move |name| (name, group.id.clone()))
        })
        .collect();
    if digest(&root.join(DEFINITIONS))? != hash {
        return Err("Contract family definitions changed.".into());
    }
    Ok((owners, hash))
}
pub(super) fn run(root: &Path, input: &str) -> Result<Value> {
    let bytes = regular_bytes(&root.join(DEFINITIONS))?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let groups = definitions(&bytes)?;
    let input = serde_json::from_str(input).map_err(error)?;
    let rows = assemble(root, groups, input)?;
    if digest(&root.join(DEFINITIONS))? != hash {
        return Err("Contract family definitions changed.".into());
    }
    Ok(json!({"contractFamilies":rows,"definitionsSHA256":hash}))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn group() -> Family {
        Family {
            id: "family".into(),
            files: vec!["Module".into()],
            rust: vec![],
            tests: vec!["present".into(), "gap".into()],
            contract: "contract".into(),
            remaining: "remaining".into(),
        }
    }
    fn input() -> Input {
        Input {
            source_families: BTreeMap::from([(
                "Sources/Models/Module.swift".into(),
                "family".into(),
            )]),
            test_inventory: TestInventory {
                targets: vec![Target {
                    source: "target.rs".into(),
                    selectors: vec!["present".into()],
                }],
                test_candidates: BTreeMap::from([("target.rs".into(), json!(["test_name"]))]),
                unresolved_selectors: vec!["gap".into()],
            },
        }
    }
    #[test]
    fn preserves_candidates_and_gaps_without_acceptance() {
        let rows = assemble(Path::new("/"), vec![group()], input()).unwrap();
        assert_eq!(rows[0]["testCandidateGaps"], json!(["gap"]));
        assert_eq!(rows[0]["testCandidates"]["target.rs"], json!(["test_name"]));
        assert_eq!(rows[0]["files"], json!(["Sources/Models/Module.swift"]));
        assert!(rows[0]["acceptance"]
            .as_str()
            .unwrap()
            .starts_with("Not accepted"));
    }
    #[test]
    fn rejects_ambiguous_unclassified_and_missing_candidates() {
        assert!(definitions(&serde_json::to_vec(&vec![group(), group()]).unwrap()).is_err());
        let mut bad = input();
        bad.source_families
            .insert("Sources/Unknown.swift".into(), "family".into());
        assert!(assemble(Path::new("/"), vec![group()], bad).is_err());
        let mut bad = input();
        bad.test_inventory.test_candidates.clear();
        assert!(assemble(Path::new("/"), vec![group()], bad).is_err());
        let mut bad = group();
        bad.rust.push("../escape".into());
        assert!(definitions(&serde_json::to_vec(&vec![bad]).unwrap()).is_err());
    }
}
