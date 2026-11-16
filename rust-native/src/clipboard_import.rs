//! Export normalization with file metadata only: never read referenced contents,
//! create image pixels, access the pasteboard or write clipboard history.
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
#[derive(Clone, Copy)]
enum Asset {
    Missing,
    File,
    Other,
}
fn asset(path: &str) -> Asset {
    match std::fs::metadata(path) {
        Ok(info) => {
            if info.is_file() {
                Asset::File
            } else {
                Asset::Other
            }
        }
        // pathlib's exists/is_file (the active Python 3.14 route) return false
        // for OSError/ValueError, including inaccessible paths and NUL images.
        Err(_) => Asset::Missing,
    }
}
// pathlib's lexical POSIX form removes empty/dot segments, preserves '..' and
// exactly two leading slashes. It never resolves a symlink or opens a target.
fn posix_path(raw: &str) -> String {
    let prefix = if raw.starts_with("//") && !raw.starts_with("///") {
        "//"
    } else if raw.starts_with('/') {
        "/"
    } else {
        ""
    };
    let parts: Vec<_> = raw
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    let path = format!("{prefix}{}", parts.join("/"));
    if path.is_empty() {
        ".".into()
    } else {
        path
    }
}
fn file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/_.-~".contains(&byte) {
            uri.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(uri, "%{byte:02X}").unwrap();
        }
    }
    uri
}

/// Initial normalization building block, not a replacement for the Python importer.
/// Dates currently use RFC3339; broader Python fromisoformat inputs remain open.
pub fn normalize(data: &Value, max_text_bytes: i64) -> Result<Normalized, String> {
    normalize_with_metadata(data, max_text_bytes, asset)
}
fn normalize_with_metadata(
    data: &Value,
    max_text_bytes: i64,
    mut metadata: impl FnMut(&str) -> Asset,
) -> Result<Normalized, String> {
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
        let category = row.get("category").and_then(Value::as_str);
        match category {
            Some("image") => {
                let path = match row.get("filePath") {
                    None => "",
                    Some(Value::String(path)) => path,
                    _ => {
                        *counts.get_mut("invalid").unwrap() += 1;
                        continue;
                    }
                };
                let state = if path.starts_with('/') {
                    metadata(&posix_path(path))
                } else {
                    Asset::Missing
                };
                let key = match state {
                    Asset::File => "unsupportedImageAssets",
                    _ => "missingImageAssets",
                };
                *counts.get_mut(key).unwrap() += 1;
                continue;
            }
            Some("text" | "link" | "file") => {}
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
        let mut item = json!({"id":id,"text":text,"source":source,"date":date.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Secs,true)});
        if category == Some("file") {
            let Some(path) = row
                .get("filePath")
                .and_then(Value::as_str)
                .filter(|path| path.starts_with('/') && !path.contains(['\0', '\n']))
            else {
                *counts.get_mut("invalid").unwrap() += 1;
                continue;
            };
            let path = posix_path(path);
            if matches!(metadata(&path), Asset::Missing) {
                *counts.get_mut("missingFileReferences").unwrap() += 1;
            }
            let name = path.rsplit('/').next().unwrap();
            item["text"] = json!(format!("Files: {name}"));
            item["contentType"] = json!("files");
            item["fileURLs"] = json!([file_uri(&path)]);
        }
        let text = item["text"].as_str().unwrap();
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
        items.push(item);
        *counts
            .get_mut(if category == Some("file") {
                "files"
            } else {
                "text"
            })
            .unwrap() += 1;
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
        let result = normalize(&input, 100_000).unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(
            result.items,
            normalize(&data(vec![row()]), 100_000).unwrap().items
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
        let result = normalize(&data(vec![invalid, unsupported, row()]), 1).unwrap();
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
    fn files_are_references_and_missing_images_explicit() {
        let mut file = row();
        file["category"] = json!("file");
        file["filePath"] = json!("/nonexistent/synthetic file.txt");
        let mut image = row();
        image["category"] = json!("image");
        image["filePath"] = json!("/nonexistent/synthetic.png");
        let result =
            normalize_with_metadata(&data(vec![file, image]), 100_000, |_| Asset::Missing).unwrap();
        assert_eq!(
            result.items[0]["fileURLs"],
            json!(["file:///nonexistent/synthetic%20file.txt"])
        );
        assert_eq!(result.counts["missingImageAssets"], 1);
        assert_eq!(result.counts["files"], 1);
        assert_eq!(result.counts["missingFileReferences"], 1);
        assert_eq!(result.items[0]["text"], "Files: synthetic file.txt");
        assert_eq!(result.items[0]["contentType"], "files");
    }
    #[test]
    fn file_and_image_metadata_never_substitutes_contents_or_unsafe_paths() {
        let file = |path: Value| {
            let mut value = row();
            value["category"] = json!("file");
            value["filePath"] = path;
            value
        };
        let image = |path: Value| json!({"category":"image","filePath":path});
        let rows = vec![
            file(json!("//owned//./Å %#?.txt/")),
            file(json!("relative")),
            file(json!("/bad\nname")),
            file(json!("/bad\0name")),
            file(Value::Null),
            image(json!("relative.png")),
            image(json!("/image")),
            image(json!("/directory")),
            image(Value::Null),
        ];
        let result = normalize_with_metadata(&data(rows), 100_000, |path| match path {
            "//owned/Å %#?.txt" => Asset::Other,
            "/image" => Asset::File,
            "/directory" => Asset::Other,
            _ => panic!("unexpected metadata probe: {path}"),
        })
        .unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(
            result.items[0]["fileURLs"],
            json!(["file:////owned/%C3%85%20%25%23%3F.txt"])
        );
        assert_eq!(result.counts["invalid"], 5);
        assert_eq!(result.counts["unsupportedImageAssets"], 1);
        assert_eq!(result.counts["missingImageAssets"], 2);
        assert_eq!(result.counts["missingFileReferences"], 0);
        let nul = normalize(
            &data(vec![file(json!("/bad\0name")), image(json!("/bad\0name"))]),
            100_000,
        )
        .unwrap();
        assert_eq!(nul.counts["invalid"], 1);
        assert_eq!(nul.counts["missingImageAssets"], 1);
        assert!(nul.items.is_empty());
        let small =
            normalize_with_metadata(&data(vec![file(json!("/missing"))]), 1, |_| Asset::Missing)
                .unwrap();
        assert_eq!(small.counts["oversized"], 1);
        assert_eq!(small.counts["missingFileReferences"], 1);
    }
    #[test]
    fn floating_metadata_never_changes_python_identity_silently() {
        for literal in ["1e20", "1e-7", "1.0", "-0.0"] {
            let mut value = row();
            value["extra"] = serde_json::from_str(literal).unwrap();
            assert!(normalize(&data(vec![value]), 100_000)
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
        let result = normalize(&data(rows), 100_000).unwrap();
        assert_eq!(result.counts["invalid"], 5);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0]["date"], "2026-10-25T00:00:00Z");
    }
}
