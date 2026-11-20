//! Assemble candidate evidence without upgrading mentions or names to acceptance.
use super::*;
use serde_json::{json, Value};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    source_files: BTreeMap<String, Value>,
    compiler_context: Vec<Value>,
    test_inventory: Value,
    contract_families: Vec<Value>,
    wire_inventory: Value,
}
fn assemble(input: &Input, head: &str, version: &str, imports: Value) -> Value {
    json!({"schema":"ruycast.full-rust.behavior-matrix.candidate.v1","scope":"Current accepted0.8.26 source crosswalk; production modules, compiler-parsed declarations, explicit semantic contract families, wire operations and candidate tests. Not final full behavior acceptance.","baseline":{"version":"0.8.26","build":"66","referenceCommit":head,"sourceOnlyComparison":true,"releaseDeliveryEvidence":".go/evidence/native-edit-shortcuts/delivery-review.json"},"compilerVersion":version,"compilerContext":input.compiler_context,"testInventory":input.test_inventory,"importedModuleHashes":imports,"sourceFiles":input.source_files,"contractFamilies":input.contract_families,"wireOperations":input.wire_inventory["wireOperations"],"wireInventory":input.wire_inventory,"unclassifiedModules":[],"allProductionSourcesMatchReference":true,"acceptance":"Pending independent semantic completeness review and criterion-level proof. No whole original checklist block is checked.","original12Retained":true,"overallComplete":false})
}
fn summary(matrix: &Value) -> Result<Value> {
    let sources = matrix["sourceFiles"]
        .as_object()
        .ok_or("Missing source files.")?;
    let count = sources.values().try_fold(0usize, |sum, value| {
        value["declarations"]
            .as_array()
            .and_then(|rows| sum.checked_add(rows.len()))
            .ok_or("Invalid compiler declarations.")
    })?;
    Ok(
        json!({"modules":sources.len(),"declarations":count,"families":matrix["contractFamilies"].as_array().ok_or("Missing contract families.")?.len(),"wireOperations":matrix["wireOperations"].as_array().ok_or("Missing wire operations.")?.len(),"unclassified":0,"overallComplete":false}),
    )
}
const IMPORTS: &[&str] = &[
    ".build/swift/debug/Modules/LauncherCore.swiftmodule",
    ".build/swift/debug/Modules/LauncherModels.swiftmodule",
    ".build/swift/debug/CRuycastCore.build/module.modulemap",
];
fn imports(root: &Path, architecture: &str) -> Result<Value> {
    let alias = oracles::swift_alias(root, architecture)?;
    let physical = root
        .join(".build/swift")
        .join(alias.as_deref().unwrap_or("debug"));
    let mut hashes = BTreeMap::new();
    for path in IMPORTS {
        let suffix = Path::new(path)
            .strip_prefix(".build/swift/debug")
            .map_err(error)?;
        hashes.insert(*path, digest(&physical.join(suffix))?);
    }
    if oracles::swift_alias(root, architecture)? != alias {
        return Err("SwiftPM debug alias changed during report.".into());
    }
    for (path, hash) in &hashes {
        if digest(
            &physical.join(
                Path::new(path)
                    .strip_prefix(".build/swift/debug")
                    .map_err(error)?,
            ),
        )? != *hash
        {
            return Err("Imported Swift module changed during report.".into());
        }
    }
    Ok(json!(hashes))
}
fn write_output(path: &Path, matrix: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(matrix).map_err(error)?;
    bytes.push(b'\n');
    fs::create_dir_all(path.parent().ok_or("Missing report output parent.")?).map_err(error)?;
    fs::write(path, bytes).map_err(error)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swift_import_directory_valid_alias_and_hostile_alias() {
        use std::os::unix::fs::symlink;
        let fixture = super::super::tests::Fixture::new();
        let root = &fixture.0;
        for path in IMPORTS {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"owned module fixture").unwrap();
        }
        let directory = imports(root, "arm64-apple-macosx").unwrap();
        assert_eq!(directory.as_object().unwrap().len(), 3);
        let debug = root.join(".build/swift/debug");
        let physical = root.join(".build/swift/arm64-apple-macosx/debug");
        fs::create_dir_all(physical.parent().unwrap()).unwrap();
        fs::rename(&debug, &physical).unwrap();
        symlink("arm64-apple-macosx/debug", &debug).unwrap();
        assert_eq!(imports(root, "arm64-apple-macosx").unwrap(), directory);
        fs::remove_file(&debug).unwrap();
        symlink("../escape", &debug).unwrap();
        assert!(imports(root, "arm64-apple-macosx").is_err());
        fs::remove_file(&debug).unwrap();
        symlink("x86_64-apple-macosx/debug", &debug).unwrap();
        assert!(imports(root, "arm64-apple-macosx").is_err());
    }
    #[test]
    fn output_preserves_overwrite_alias_newline_and_failure_semantics() {
        let fixture = super::super::tests::Fixture::new();
        let target = fixture.0.join("owned.data");
        let alias = fixture.0.join("matrix.json");
        fs::write(&target, b"old output").unwrap();
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        write_output(&alias, &json!({"value":1})).unwrap();
        write_output(&alias, &json!({"value":2})).unwrap();
        assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
        let bytes = fs::read(&target).unwrap();
        assert!(bytes.ends_with(b"\n"));
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            json!({"value":2})
        );
        assert!(write_output(&target.join("child"), &json!({})).is_err());
        assert_eq!(fs::read(&target).unwrap(), bytes);
    }
    #[test]
    fn candidate_schema_and_summary_never_claim_acceptance() {
        let input = Input {
            source_files: BTreeMap::from([(
                "Sources/Module.swift".into(),
                json!({"declarations":[{"name":"entry"}]}),
            )]),
            compiler_context: vec![json!({"exitCode":0})],
            test_inventory: json!({"unresolvedSelectors":["gap"]}),
            contract_families: vec![json!({"remaining":"gap"})],
            wire_inventory: json!({"wireOperations":[{"operation":"candidate"}]}),
        };
        let matrix = assemble(&input, "commit", "version", json!({"module":"hash"}));
        assert_eq!(
            matrix["schema"],
            "ruycast.full-rust.behavior-matrix.candidate.v1"
        );
        assert_eq!(matrix["baseline"]["referenceCommit"], "commit");
        assert_eq!(
            matrix["testInventory"]["unresolvedSelectors"],
            json!(["gap"])
        );
        assert_eq!(matrix["overallComplete"], false);
        assert_eq!(
            summary(&matrix).unwrap(),
            json!({"modules":1,"declarations":1,"families":1,"wireOperations":1,"unclassified":0,"overallComplete":false})
        );
        let mut malformed = matrix;
        malformed["sourceFiles"]["Sources/Module.swift"]
            .as_object_mut()
            .unwrap()
            .remove("declarations");
        assert!(summary(&malformed).is_err());
    }
}
