//! Ordered migration-only inventory. A source/test crosswalk never accepts parity.
use super::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
enum Step {
    Sources,
    Parser(Option<Value>),
    Discover,
    Compile(Value),
    Tests(BTreeSet<String>),
    Families(Value),
    Wire(Value),
    Report(Value),
}
fn flow(
    selectors: BTreeSet<String>,
    definitions: &str,
    mut call: impl FnMut(Step) -> Result<Value>,
) -> Result<Value> {
    let original = call(Step::Sources)?;
    if original["definitionsSHA256"].as_str() != Some(definitions) {
        return Err("Contract family definitions changed since load".into());
    }
    let mut parser = call(Step::Parser(None))?;
    if call(Step::Sources)? != original {
        return Err("Production/reference sources changed during parser bootstrap".into());
    }
    let toolchain = call(Step::Discover)?;
    let files = original["sourceFiles"]
        .as_object()
        .ok_or("Missing production sources.")?;
    let mut modules = BTreeMap::<String, BTreeSet<String>>::new();
    for path in files.keys() {
        let module = Path::new(path)
            .parent()
            .and_then(Path::file_name)
            .and_then(|v| v.to_str())
            .ok_or("Invalid compiler module path.")?;
        modules
            .entry(module.into())
            .or_default()
            .insert(path.clone());
    }
    let mut parsed = serde_json::Map::new();
    let mut contexts = Vec::new();
    for (module, paths) in modules {
        parser = call(Step::Parser(Some(parser)))?;
        let compiled = call(Step::Compile(
            json!({"module":module,"paths":paths,"sdk":toolchain["sdk"],"plugins":toolchain["plugins"],"parser":parser}),
        ))?;
        for (path, rows) in compiled["declarations"]
            .as_object()
            .ok_or("Missing compiler declarations.")?
        {
            if parsed.insert(path.clone(), rows.clone()).is_some() {
                return Err("Duplicate compiler declaration source.".into());
            }
        }
        contexts.push(compiled["context"].clone());
    }
    let current = call(Step::Sources)?;
    if current != original {
        return Err("Production/reference sources changed during compiler inventory".into());
    }
    let mut sources = current["sourceFiles"]
        .as_object()
        .ok_or("Missing production sources.")?
        .clone();
    for (path, value) in &mut sources {
        value.as_object_mut().ok_or("Invalid source row.")?.insert(
            "declarations".into(),
            parsed
                .remove(path)
                .ok_or("Compiler source missing declarations.")?,
        );
    }
    if !parsed.is_empty() {
        return Err("Unexpected compiler declaration sources.".into());
    }
    let tests = call(Step::Tests(selectors))?;
    let source_families = sources
        .iter()
        .map(|(path, value)| (path, &value["contractFamily"]))
        .collect::<BTreeMap<_, _>>();
    let families = call(Step::Families(
        json!({"sourceFamilies":source_families,"testInventory":tests}),
    ))?;
    if families["definitionsSHA256"].as_str() != Some(definitions) {
        return Err("Contract family definitions changed since load".into());
    }
    let app = sources
        .get("Sources/RuycastApp/AppState.swift")
        .ok_or("Missing production wire source.")?;
    let wire = call(Step::Wire(app.clone()))?;
    if wire["sources"]["Sources/RuycastApp/AppState.swift"] != app["sha256"] {
        return Err("Wire source differs from compiler/reference source".into());
    }
    call(Step::Report(
        json!({"sourceFiles":sources,"compilerContext":contexts,"testInventory":tests,"contractFamilies":families["contractFamilies"],"wireInventory":wire}),
    ))
}
fn wire(root: &Path, app: &Value) -> Result<Value> {
    let path = "Sources/RuycastApp/AppState.swift";
    let bytes = regular_bytes(&root.join(path))?;
    if app["sha256"].as_str() != Some(format!("{:x}", Sha256::digest(&bytes)).as_str()) {
        return Err("Wire declaration source changed.".into());
    }
    let rows = app["declarations"]
        .as_array()
        .ok_or("Missing wire compiler declarations.")?;
    let matches = rows
        .iter()
        .filter(|row| row["kind"] == "func_decl" && row["name"] == "handle(_:sink:)")
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err("Wire compiler handler must be unique.".into());
    }
    let range = matches[0]["range"]
        .as_str()
        .ok_or("Missing wire source range.")?;
    let position = range
        .strip_prefix(&format!("{path}:"))
        .and_then(|value| value.split(':').next())
        .ok_or("Wire declaration range is not source-bound.")?
        .parse::<usize>()
        .map_err(error)?;
    let text = std::str::from_utf8(&bytes).map_err(error)?;
    let marker = text
        .lines()
        .nth(
            position
                .checked_sub(1)
                .ok_or("Invalid wire declaration line.")?,
        )
        .filter(|line| !line.is_empty())
        .ok_or("Wire declaration line missing.")?;
    parity_wire::run(root, marker)
}
pub(super) fn run(
    root: &Path,
    lease: &BuildLock,
    reference: &Path,
    out: &Path,
    deadline: u64,
) -> Result<Value> {
    process::with_deadline(deadline, || {
        let (selectors, definitions) = parity_families::metadata(root)?;
        flow(selectors, &definitions, |step| {
            lease.validate()?;
            let value = match step {
                Step::Sources => parity_sources::run(root, reference)?,
                Step::Parser(binding) => parity_bootstrap::run(
                    root,
                    lease,
                    &json!({"binding":binding,"deadlineNanos":deadline}).to_string(),
                )?,
                Step::Discover => parity_toolchain::run(
                    root,
                    lease,
                    "discover",
                    &json!({"deadlineNanos":deadline}).to_string(),
                )?,
                Step::Compile(payload) => parity_compile::run(root, lease, &payload.to_string())?,
                Step::Tests(selectors) => test_catalog::run(root, lease, selectors)?,
                Step::Families(payload) => parity_families::run(root, &payload.to_string())?,
                Step::Wire(app) => wire(root, &app)?,
                Step::Report(mut payload) => {
                    payload["reference"] = json!(reference);
                    payload["out"] = json!(out);
                    payload["deadlineNanos"] = json!(deadline);
                    parity_report::run(root, lease, &payload.to_string())?
                }
            };
            lease.validate()?;
            Ok(value)
        })
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Value {
        json!({"definitionsSHA256":"definitions","sourceFiles":{"Sources/RuycastApp/AppState.swift":{"sha256":"hash","contractFamily":"family"}}})
    }
    #[test]
    fn order_and_payload_preserve_compiler_candidates_and_gaps() {
        let mut order = Vec::new();
        let result=flow(BTreeSet::from(["candidate".into()]),"definitions",|step|Ok(match step {
            Step::Sources=>{order.push("sources");source()},
            Step::Parser(binding)=>{order.push(if binding.is_some(){"parser-check"}else{"parser-build"});json!({"snapshot":"bound"})},
            Step::Discover=>{order.push("discover");json!({"sdk":"/sdk","plugins":[]})},
            Step::Compile(payload)=>{order.push("compile");assert_eq!(payload["module"],"RuycastApp");json!({"declarations":{"Sources/RuycastApp/AppState.swift":[]},"context":{"exitCode":0}})},
            Step::Tests(selectors)=>{order.push("tests");assert!(selectors.contains("candidate"));json!({"unresolvedSelectors":["gap"]})},
            Step::Families(payload)=>{order.push("families");assert_eq!(payload["testInventory"]["unresolvedSelectors"],json!(["gap"]));json!({"definitionsSHA256":"definitions","contractFamilies":[{"remaining":"gap"}]})},
            Step::Wire(_)=>{order.push("wire");json!({"sources":{"Sources/RuycastApp/AppState.swift":"hash"},"wireOperations":[]})},
            Step::Report(payload)=>{order.push("report");assert_eq!(payload["contractFamilies"][0]["remaining"],"gap");json!({"overallComplete":false})}
        })).unwrap();
        assert_eq!(result["overallComplete"], false);
        assert_eq!(
            order,
            vec![
                "sources",
                "parser-build",
                "sources",
                "discover",
                "parser-check",
                "compile",
                "sources",
                "tests",
                "families",
                "wire",
                "report"
            ]
        );
    }
    #[test]
    fn drift_stops_before_discovery_and_missing_handler_refuses_fallback() {
        let mut count = 0;
        assert!(flow(BTreeSet::new(), "definitions", |step| match step {
            Step::Sources => {
                count += 1;
                Ok(if count == 1 { source() } else { json!({}) })
            }
            Step::Parser(_) => Ok(json!({})),
            _ => panic!("drift must stop before next operation"),
        })
        .is_err());
        let fixture = super::super::tests::Fixture::new();
        fs::create_dir_all(fixture.0.join("Sources/RuycastApp")).unwrap();
        fs::write(
            fixture.0.join("Sources/RuycastApp/AppState.swift"),
            b"owned source range data",
        )
        .unwrap();
        let hash = digest(&fixture.0.join("Sources/RuycastApp/AppState.swift")).unwrap();
        assert!(wire(&fixture.0, &json!({"sha256":hash,"declarations":[]})).is_err());
    }
}
