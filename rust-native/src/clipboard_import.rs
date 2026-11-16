//! Export normalization with file metadata only: never read referenced contents,
//! create image pixels, access the pasteboard or write clipboard history.
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, SecondsFormat, Utc};
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

fn parse_python_iso_date(raw: &str) -> Option<NaiveDate> {
    let bytes = raw.as_bytes();
    match bytes.len() {
        10 if bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes[..4].iter().all(u8::is_ascii_digit)
            && bytes[5..7].iter().all(u8::is_ascii_digit)
            && bytes[8..].iter().all(u8::is_ascii_digit) =>
        {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()
        }
        10 if bytes[4] == b'-'
            && bytes[5] == b'W'
            && bytes[8] == b'-'
            && bytes[..4].iter().all(u8::is_ascii_digit)
            && bytes[6..8].iter().all(u8::is_ascii_digit)
            && bytes[9].is_ascii_digit() =>
        {
            NaiveDate::parse_from_str(raw, "%G-W%V-%u").ok()
        }
        8 if bytes[4] == b'-'
            && bytes[5] == b'W'
            && bytes[..4].iter().all(u8::is_ascii_digit)
            && bytes[6..].iter().all(u8::is_ascii_digit) =>
        {
            NaiveDate::parse_from_str(&format!("{raw}-1"), "%G-W%V-%u").ok()
        }
        8 if bytes.iter().all(u8::is_ascii_digit) => {
            NaiveDate::parse_from_str(raw, "%Y%m%d").ok()
        }
        8 if bytes[4] == b'W'
            && bytes[..4].iter().all(u8::is_ascii_digit)
            && bytes[5..].iter().all(u8::is_ascii_digit) =>
        {
            NaiveDate::parse_from_str(raw, "%GW%V%u").ok()
        }
        7 if bytes[4] == b'W'
            && bytes[..4].iter().all(u8::is_ascii_digit)
            && bytes[5..].iter().all(u8::is_ascii_digit) =>
        {
            NaiveDate::parse_from_str(&format!("{raw}1"), "%GW%V%u").ok()
        }
        _ => None,
    }
}

fn parse_python_iso_time(raw: &str) -> Option<(NaiveTime, bool)> {
    let (clock, fraction) = raw
        .split_once(['.', ','])
        .map_or((raw, None), |(clock, fraction)| (clock, Some(fraction)));
    if !clock.is_ascii() {
        return None;
    }
    let components: Vec<&str> = if clock.contains(':') {
        clock.split(':').collect()
    } else {
        match clock.len() {
            2 => vec![clock],
            4 => vec![&clock[..2], &clock[2..]],
            6 => vec![&clock[..2], &clock[2..4], &clock[4..]],
            _ => return None,
        }
    };
    if !(1..=3).contains(&components.len())
        || components.iter().any(|part| {
            part.len() != 2 || !part.bytes().all(|byte| byte.is_ascii_digit())
        })
        || (fraction.is_some() && components.len() != 3)
    {
        return None;
    }
    let number = |index: usize| -> Option<u32> {
        components.get(index).map_or(Some(0), |part| part.parse().ok())
    };
    let (hour, minute, second) = (number(0)?, number(1)?, number(2)?);
    let microsecond = fraction.map_or(Some(0), |digits| {
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let first_six = &digits[..digits.len().min(6)];
        Some(first_six.parse::<u32>().ok()? * 10_u32.pow((6 - first_six.len()) as u32))
    })?;
    if hour == 24 {
        return (minute == 0 && second == 0 && microsecond == 0)
            .then_some((NaiveTime::MIN, true));
    }
    NaiveTime::from_hms_micro_opt(hour, minute, second, microsecond).map(|time| (time, false))
}

fn parse_offset_component(raw: &str) -> Option<i64> {
    let sign = match raw.as_bytes().first()? {
        b'+' => 1_i64,
        b'-' => -1_i64,
        _ => return None,
    };
    let value = &raw[1..];
    let (clock, fraction) = value.split_once(['.', ',']).map_or((value, None), |(a, b)| (a, Some(b)));
    if !clock.is_ascii() {
        return None;
    }
    let parts: Vec<&str> = if clock.contains(':') {
        clock.split(':').collect()
    } else {
        match clock.len() {
            2 => vec![&clock[0..2]],
            4 => vec![&clock[0..2], &clock[2..4]],
            6 => vec![&clock[0..2], &clock[2..4], &clock[4..6]],
            _ => return None,
        }
    };
    if !(1..=3).contains(&parts.len()) || parts.iter().any(|part| part.len() != 2) {
        return None;
    }
    if fraction.is_some() && parts.len() != 3 {
        return None;
    }
    let numbers: Vec<i64> = parts
        .iter()
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    let (hour, minute, second) = (
        numbers[0],
        *numbers.get(1).unwrap_or(&0),
        *numbers.get(2).unwrap_or(&0),
    );
    let fractional_micros = fraction.map_or(Some(0), |digits| {
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let first_six = &digits[..digits.len().min(6)];
        let value: i64 = first_six.parse().ok()?;
        Some(value * 10_i64.pow((6 - first_six.len()) as u32))
    })?;
    let whole_seconds = ((hour * 60 + minute) * 60) + second;
    let micros = if whole_seconds == 0 {
        // CPython 3.14.3 discards fractional offset microseconds at zero seconds.
        0
    } else {
        (whole_seconds * 1_000_000) + fractional_micros
    };
    if micros >= 24 * 60 * 60 * 1_000_000 {
        return None;
    }
    Some(sign * micros)
}

fn parse_python_iso_datetime(raw: &str) -> Option<DateTime<Utc>> {
    let normalized = raw.replace('Z', "+00:00");
    let value = normalized.as_str();
    for (separator_index, separator) in value.char_indices() {
        if separator_index == 0 || value[separator_index + separator.len_utf8()..].is_empty() {
            continue;
        }
        let Some(date) = parse_python_iso_date(&value[..separator_index]) else {
            continue;
        };
        if !(1..=9999).contains(&date.year()) {
            continue;
        }
        let time_and_offset = &value[separator_index + separator.len_utf8()..];
        let offset_index = time_and_offset
            .char_indices()
            .rev()
            .find_map(|(index, character)| matches!(character, '+' | '-').then_some(index));
        let Some(offset_index) = offset_index else {
            continue;
        };
        let Some(offset_micros) = parse_offset_component(&time_and_offset[offset_index..]) else {
            continue;
        };
        let Some((time, next_day)) = parse_python_iso_time(&time_and_offset[..offset_index]) else {
            continue;
        };
        let local_date = if next_day { date.succ_opt()? } else { date };
        if !(1..=9999).contains(&local_date.year()) {
            continue;
        }
        let utc = local_date
            .and_time(time)
            .checked_sub_signed(Duration::microseconds(offset_micros))?;
        return Some(DateTime::from_naive_utc_and_offset(utc, Utc));
    }
    None
}

/// Initial normalization building block, not a replacement for the Python importer.
/// Timestamps follow its timezone-aware `datetime.fromisoformat` path.
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
            .and_then(parse_python_iso_datetime)
            .filter(|date| (1..=9999).contains(&date.year()));
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
    #[test]
    fn python_iso_datetime_forms_require_timezone_and_normalize_to_utc() {
        let accepted = [
            ("2011-11-04 00:05:23.283+00:00", "2011-11-04T00:05:23Z"),
            ("20111104T000523+0000", "2011-11-04T00:05:23Z"),
            ("2011-W01-2T00:05:23.283+00:00", "2011-01-04T00:05:23Z"),
            ("2011W012T000523+0000", "2011-01-04T00:05:23Z"),
            ("2026-W43T00:00:00+00:00", "2026-10-19T00:00:00Z"),
            ("2026W43T000000+0000", "2026-10-19T00:00:00Z"),
            ("2026-10-25🕒00:00:00+01:00", "2026-10-24T23:00:00Z"),
            ("2026-10-25T24:00:00+00:00", "2026-10-26T00:00:00Z"),
            ("2026-10-25T24:00+00:00", "2026-10-26T00:00:00Z"),
            ("2026-10-25T24+00:00", "2026-10-26T00:00:00Z"),
            ("2026-10-25T00:00:00+00:60", "2026-10-24T23:00:00Z"),
            ("2026-10-25T00:00:00+00:00:60", "2026-10-24T23:59:00Z"),
            (
                "2026-10-25T00:00:00.283+00:00:00.5",
                "2026-10-25T00:00:00Z",
            ),
            (
                "2026-10-25T00:00:00.283+00:00:01.5",
                "2026-10-24T23:59:58Z",
            ),
        ];
        for (input, expected) in accepted {
            let mut item = row();
            item["createdAt"] = json!(input);
            let result = normalize(&data(vec![item]), 100_000).unwrap();
            assert_eq!(result.counts["invalid"], 0, "{input}");
            assert_eq!(result.items[0]["date"], expected, "{input}");
        }
        for input in [
            "2011-11-04T00:05:23", // Python accepts it but it has no zone.
            "2011-11-04T00:05:23z", // The source only substitutes uppercase Z.
            "2026-10-25T00.5:00+00:00", // Fractional hours are rejected.
            "2026-10-25T00:00:00+24:00", // datetime.timezone rejects 24 hours.
            "2026-1-25T00:00:00+00:00", // Calendar components have fixed widths.
            "2026-10-25T0:00:00+00:00", // Time components have fixed widths.
            "2026-10-25T24:00:01+00:00", // Hour 24 only allows an exact midnight.
            "9999-12-31T24:00:00+00:00", // The local year cannot roll past 9999.
            "0000-12-31T23:00:00-02:00", // UTC cannot rescue an invalid local year.
            "2026-10-25Z00:00:00+00:00", // The Python caller replaces every uppercase Z.
            " 026-W43T00:00:00+00:00", // Chrono must not accept padded ISO years.
            "2026-10-25T🕒+00:00", // Malformed Unicode must count invalid, never panic.
            "2026-10-25T00:00:00+🕒", // Malformed Unicode offset must count invalid.
        ] {
            let mut item = row();
            item["createdAt"] = json!(input);
            let result = normalize(&data(vec![item]), 100_000).unwrap();
            assert_eq!(result.counts["invalid"], 1, "{input}");
            assert!(result.items.is_empty(), "{input}");
        }
    }
}
