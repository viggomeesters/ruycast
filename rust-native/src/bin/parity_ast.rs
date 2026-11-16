//! Parse compiler-emitted AST data; declarations never imply behavioral parity.
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};
#[derive(Clone, Serialize)]
struct Declaration {
    kind: String,
    range: String,
    name: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Inventory {
    declarations: BTreeMap<String, Vec<Declaration>>,
    #[serde(rename = "sourceASTs")]
    source_asts: usize,
}
fn quoted(text: &str, start: usize) -> Option<(String, usize)> {
    let mut position = start;
    while let Some(offset) = text[position..].find('"') {
        let open = position + offset;
        if text[position..open].contains('\n') {
            return None;
        }
        let close = open + 1 + text[open + 1..].find('"')?;
        let name = &text[open + 1..close];
        if name.contains('\n') {
            return None;
        }
        if !name.is_empty() {
            return Some((name.into(), close + 1));
        }
        position = open + 1;
    }
    None
}
fn parse(text: &str, expected: BTreeSet<String>) -> Result<Inventory, String> {
    let mut seen = BTreeSet::new();
    let mut position = 0;
    while let Some(offset) = text[position..].find("(source_file \"") {
        let start = position + offset + 14;
        if let Some(end) = text[start..].find('"') {
            let name = &text[start..start + end];
            if !name.is_empty() && !name.contains('\n') {
                seen.insert(name.to_owned());
            }
        }
        position = start;
    }
    if seen != expected {
        return Err(format!(
            "Compiler did not produce exactly the requested source ASTs: missing={:?}, extra={:?}",
            expected.difference(&seen).collect::<Vec<_>>(),
            seen.difference(&expected).collect::<Vec<_>>()
        ));
    }
    let mut declarations = expected
        .into_iter()
        .map(|name| (name, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let mut unique = BTreeSet::new();
    let mut position = 0;
    while let Some(offset) = text[position..].find('(') {
        let open = position + offset;
        position = open + 1;
        let tail = &text[position..];
        let Some(kind) = [
            "func_decl",
            "constructor_decl",
            "destructor_decl",
            "var_decl",
            "class_decl",
            "struct_decl",
            "enum_decl",
            "protocol",
        ]
        .into_iter()
        .find(|kind| tail.starts_with(&format!("{kind} "))) else {
            continue;
        };
        let Some(range_offset) = tail.find("range=[") else {
            continue;
        };
        if tail[..range_offset].contains('\n') {
            continue;
        }
        let start = position + range_offset + 7;
        let Some(length) = text[start..].find(']') else {
            continue;
        };
        let range = &text[start..start + length];
        if range.is_empty() {
            continue;
        }
        let Some((name, end)) = quoted(text, start + length + 1) else {
            continue;
        };
        position = end;
        let source = range.split(':').next().unwrap();
        if let Some(items) = declarations.get_mut(source) {
            if unique.insert((kind.to_owned(), range.to_owned(), name.clone())) {
                items.push(Declaration {
                    kind: kind.into(),
                    range: range.into(),
                    name,
                });
            }
        }
    }
    Ok(Inventory {
        declarations,
        source_asts: seen.len(),
    })
}
fn main() {
    let mut expected = BTreeSet::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("parity-ast --source SOURCE [--source SOURCE...] < compiler.ast");
            return;
        }
        if arg != "--source" {
            eprintln!("Unknown parser argument");
            std::process::exit(2);
        }
        let Some(source) = args.next().and_then(|value| value.into_string().ok()) else {
            eprintln!("Expected a UTF-8 source name");
            std::process::exit(2)
        };
        expected.insert(source);
    }
    if expected.is_empty() {
        eprintln!("At least one --source is required");
        std::process::exit(2);
    }
    let mut text = String::new();
    let result = std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())
        .and_then(|_| parse(&text, expected));
    match result {
        Ok(value) => println!("{}", serde_json::to_string(&value).unwrap()),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn expected() -> BTreeSet<String> {
        ["Sources/Model.swift".into()].into_iter().collect()
    }
    #[test]
    fn compiler_ast_fields_and_first_occurrence_order_are_preserved() {
        // Compiler AST records are data, not executable source programs.
        let text="(source_file \"Sources/Model.swift\"\n (struct_decl range=[Sources/Model.swift:1:1 - line:4:1] \"Model\")\n (func_decl range=[Sources/Model.swift:2:2 - line:2:8] \"gå()\")\n (func_decl range=[Sources/Model.swift:2:2 - line:2:8] \"gå()\")\n (var_decl range=[Other.swift:1:1 - line:1:3] \"foreign\"))";
        let result = parse(text, expected()).unwrap();
        let rows = &result.declarations["Sources/Model.swift"];
        assert_eq!(result.source_asts, 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "struct_decl");
        assert_eq!(rows[0].name, "Model");
        assert_eq!(rows[1].name, "gå()");
        assert_eq!(rows[1].range, "Sources/Model.swift:2:2 - line:2:8");
    }
    #[test]
    fn missing_extra_and_duplicate_source_records_keep_set_contract() {
        assert!(parse("", expected()).is_err());
        assert!(parse(
            "(source_file \"Sources/Model.swift\")(source_file \"Extra.swift\")",
            expected()
        )
        .is_err());
        let value = parse(
            "(source_file \"Sources/Model.swift\")(source_file \"Sources/Model.swift\")",
            expected(),
        )
        .unwrap();
        assert_eq!(value.source_asts, 1);
        assert!(value.declarations["Sources/Model.swift"].is_empty());
    }
    #[test]
    fn declarations_without_requested_ranges_or_complete_names_are_ignored() {
        let text="(source_file \"Sources/Model.swift\"\n(func_decl implicit \"noRange\")\n(func_decl range=[Sources/Model.swift:1:1]\n \"newLine\")\n(protocol_decl range=[Sources/Model.swift:2:1] \"notOriginalKind\"))";
        assert!(parse(text, expected()).unwrap().declarations["Sources/Model.swift"].is_empty());
    }
}
