//! Pure text/link export normalization. No history, pasteboard, filesystem or
//! transport access. File/image normalization and the importer caller remain
//! Python-owned until their independent migration; reject those rows here.
use chrono::{DateTime, Datelike, SecondsFormat, Timelike, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub struct Normalized {
    pub items: Vec<Value>,
    pub counts: BTreeMap<String, usize>,
}
fn canonical_numbers(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.is_i64() || number.is_u64(),
        Value::Array(values) => values.iter().all(canonical_numbers),
        Value::Object(values) => values.values().all(canonical_numbers),
        _ => true,
    }
}

/// Initial text-only building block, not a replacement for the Python importer.
/// Dates currently use RFC3339; broader Python fromisoformat inputs remain open.
pub fn normalize_text(data: &Value, max_text_bytes: i64) -> Result<Normalized, String> {
    let empty = json!({});
    let package = data
        .get("builtin_package_clipboardHistory")
        .unwrap_or(&empty);
    if !data.is_object() || !package.is_object() {
        return Err("Invalid clipboard history export structure.".into());
    }
    let rows = match package.get("clipboardHistoryRecords") {
        None => &[][..],
        Some(Value::Array(rows)) => rows.as_slice(),
        _ => return Err("Invalid clipboard history export structure.".into()),
    };
    let mut counts: BTreeMap<String, usize> = [
        "records",
        "text",
        "files",
        "missingImageAssets",
        "unsupportedImageAssets",
        "missingFileReferences",
        "unsupported",
        "invalid",
        "oversized",
        "duplicate",
    ]
    .into_iter()
    .map(|key| (key.into(), 0))
    .collect();
    *counts.get_mut("records").unwrap() = rows.len();
    let mut items = Vec::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        if !row.is_object() {
            *counts.get_mut("invalid").unwrap() += 1;
            continue;
        }
        match row.get("category").and_then(Value::as_str) {
            Some("file" | "image") => {
                return Err("File/image normalization is not migrated.".into())
            }
            Some("text" | "link") => {}
            _ => {
                *counts.get_mut("unsupported").unwrap() += 1;
                continue;
            }
        }
        let text = row.get("text").and_then(Value::as_str);
        let source = match row.get("applicationPath") {
            None => Some(""),
            Some(value) => value.as_str(),
        };
        let date = row
            .get("createdAt")
            .and_then(Value::as_str)
            .filter(|raw| !raw.ends_with('z'))
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .filter(|date| {
                date.nanosecond() < 1_000_000_000
                    && (1..=9999).contains(&date.year())
                    && (1..=9999).contains(&date.with_timezone(&Utc).year())
            });
        let (Some(text), Some(source), Some(date)) = (text, source, date) else {
            *counts.get_mut("invalid").unwrap() += 1;
            continue;
        };
        // Python and serde_json spell floating metadata differently in hashes.
        // Refuse the incomplete canonicalization rather than change identity.
        if !canonical_numbers(row) {
            return Err("Floating numeric metadata canonicalization is not migrated.".into());
        }
        let canonical = serde_json::to_vec(row).map_err(|error| error.to_string())?;
        let id = format!("raycast-{:x}", Sha256::digest(canonical));
        if text.is_empty()
            || max_text_bytes < 0
            || text.len() as u64 > max_text_bytes as u64
            || source.len() > 16384
        {
            *counts.get_mut("oversized").unwrap() += 1;
            continue;
        }
        if !seen.insert(id.clone()) {
            *counts.get_mut("duplicate").unwrap() += 1;
            continue;
        }
        items.push(json!({"id":id,"text":text,"source":source,"date":date.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Secs,true)}));
        *counts.get_mut("text").unwrap() += 1;
    }
    items.sort_by(|left, right| {
        (right["date"].as_str(), right["id"].as_str())
            .cmp(&(left["date"].as_str(), left["id"].as_str()))
    });
    Ok(Normalized { items, counts })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> Value {
        json!({"category":"text","text":"Synthetic content","applicationPath":"/Applications/Fixture.app","createdAt":"2026-10-25T00:00:00Z"})
    }
    fn data(rows: Vec<Value>) -> Value {
        json!({"builtin_package_clipboardHistory":{"clipboardHistoryRecords":rows}})
    }
    #[test]
    fn text_stable_identity_dedup_and_date() {
        let input = data(vec![row(), row()]);
        let unchanged = input.clone();
        let result = normalize_text(&input, 100_000).unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(
            result.items,
            normalize_text(&data(vec![row()]), 100_000).unwrap().items
        );
        assert_eq!(result.counts["duplicate"], 1);
        assert_eq!(result.items[0]["date"], "2026-10-25T00:00:00Z");
        assert_eq!(
            result.items[0]["id"],
            "raycast-766de9e0f6245551fc57a7ec9baf901831757676ae9ed34d5db98bbbb142fb10"
        );
        assert_eq!(input, unchanged);
    }
    #[test]
    fn malformed_oversize_and_unsupported() {
        let mut invalid = row();
        invalid["createdAt"] = json!("bad");
        let mut unsupported = row();
        unsupported["category"] = json!("encrypted");
        let result = normalize_text(&data(vec![invalid, unsupported, row()]), 1).unwrap();
        assert!(result.items.is_empty());
        assert_eq!(
            (
                result.counts["invalid"],
                result.counts["unsupported"],
                result.counts["oversized"]
            ),
            (1, 1, 1)
        );
    }
    #[test]
    fn floating_metadata_never_changes_python_identity_silently() {
        for literal in ["1e20", "1e-7", "1.0", "-0.0"] {
            let mut value = row();
            value["extra"] = serde_json::from_str(literal).unwrap();
            assert!(normalize_text(&data(vec![value]), 100_000)
                .err()
                .unwrap()
                .contains("numeric metadata"));
        }
    }
    #[test]
    fn python_datetime_boundaries_are_invalid_without_losing_other_rows() {
        let mut rows = vec![row()];
        for date in [
            "2026-10-25T00:00:00z",
            "2016-12-31T23:59:60Z",
            "0000-01-01T00:00:00Z",
            "9999-12-31T23:59:59-01:00",
            "0001-01-01T00:00:00+01:00",
        ] {
            let mut value = row();
            value["createdAt"] = json!(date);
            rows.push(value);
        }
        let result = normalize_text(&data(rows), 100_000).unwrap();
        assert_eq!(result.counts["invalid"], 5);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0]["date"], "2026-10-25T00:00:00Z");
    }
}
