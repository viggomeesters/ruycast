//! Export normalization with file metadata only: never read referenced contents,
//! create image pixels, access the pasteboard or write clipboard history.
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, SecondsFormat, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

pub struct Normalized {
    pub items: Vec<Value>,
    pub counts: BTreeMap<String, usize>,
}

pub struct ParsedExport {
    data: Value,
    numeric_marker: Option<String>,
}

const CLIPBOARD_BATCH_ITEMS: usize = 100;
const CLIPBOARD_BATCH_MAX_WIRE: usize = 1_000_000;

/// Build deterministic, bounded clipboard import requests without contacting the host.
pub fn batches(items: &[Value], source_hash: &str) -> Result<Vec<Value>, String> {
    let mut requests = Vec::new();
    let mut batch = Vec::new();
    for item in items {
        let mut proposed = batch.clone();
        proposed.push(item.clone());
        let request = clipboard_request(&proposed, source_hash)?;
        if !batch.is_empty()
            && (proposed.len() > CLIPBOARD_BATCH_ITEMS
                || python_json(&request, false)?.len() >= CLIPBOARD_BATCH_MAX_WIRE)
        {
            requests.push(clipboard_request(&batch, source_hash)?);
            batch.clear();
        }
        batch.push(item.clone());
        let request = clipboard_request(&batch, source_hash)?;
        if batch.len() > CLIPBOARD_BATCH_ITEMS
            || python_json(&request, false)?.len() >= CLIPBOARD_BATCH_MAX_WIRE
        {
            return Err("A clipboard item exceeds the import transport limit.".into());
        }
    }
    if !batch.is_empty() {
        requests.push(clipboard_request(&batch, source_hash)?);
    }
    Ok(requests)
}

fn clipboard_request(items: &[Value], source_hash: &str) -> Result<Value, String> {
    let item_json = python_json(&Value::Array(items.to_vec()), true)?;
    let batch_id = format!("{:x}", Sha256::digest(item_json.as_bytes()));
    Ok(json!({
        "op": "clipboard.import",
        "sourceSHA256": source_hash,
        "batchID": batch_id,
        "items": items,
    }))
}

// Python's json.dumps defaults to ensure_ascii=True and, unless compact is set,
// inserts one space after separators. Keys are explicitly sorted for the
// stable retry fingerprint; this does not affect the wire byte count.
fn python_json(value: &Value, compact: bool) -> Result<String, String> {
    let mut output = String::new();
    append_python_json(value, &mut output, compact)?;
    Ok(output)
}

fn append_python_json(value: &Value, output: &mut String, compact: bool) -> Result<(), String> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) if value.is_f64() => {
            let value = value
                .as_f64()
                .ok_or_else(|| "Invalid clipboard float value.".to_owned())?;
            output.push_str(&python_float_literal(value)?);
        }
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => append_python_string(value, output),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                    if !compact {
                        output.push(' ');
                    }
                }
                append_python_json(value, output, compact)?;
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(',');
                    if !compact {
                        output.push(' ');
                    }
                }
                append_python_string(key, output);
                output.push(':');
                if !compact {
                    output.push(' ');
                }
                append_python_json(value, output, compact)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

fn append_python_string(value: &str, output: &mut String) {
    use std::fmt::Write;
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{0000}'..='\u{001f}' => write!(output, "\\u{:04x}", character as u32).unwrap(),
            '\u{0020}'..='\u{007e}' => output.push(character),
            '\u{007f}'..='\u{ffff}' => write!(output, "\\u{:04x}", character as u32).unwrap(),
            _ => {
                let code = character as u32 - 0x1_0000;
                write!(
                    output,
                    "\\u{:04x}\\u{:04x}",
                    0xd800 + (code >> 10),
                    0xdc00 + (code & 0x3ff)
                )
                .unwrap();
            }
        }
    }
    output.push('"');
}

struct NumericToken {
    range: Range<usize>,
    raw: String,
    kind: char,
}

fn valid_json_number(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let mut index = usize::from(bytes.first() == Some(&b'-'));
    if index == bytes.len() {
        return false;
    }
    if bytes[index] == b'0' {
        index += 1;
        if bytes.get(index).is_some_and(u8::is_ascii_digit) {
            return false;
        }
    } else if (b'1'..=b'9').contains(&bytes[index]) {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
    } else {
        return false;
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == fraction_start {
            return false;
        }
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

/// Preserve exact JSON numbers without enabling serde_json's global arbitrary-
/// precision Value encoding, which reserves a normal object key.
pub fn parse_export(raw: &[u8]) -> Result<ParsedExport, String> {
    let mut preserved_numbers = Vec::new();
    let mut strings = BTreeSet::new();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'"' {
            let start = index;
            index += 1;
            while index < raw.len() {
                match raw[index] {
                    b'\\' => index = (index + 2).min(raw.len()),
                    b'"' => {
                        index += 1;
                        break;
                    }
                    _ => index += 1,
                }
            }
            let decoded: String = serde_json::from_slice(&raw[start..index])
                .map_err(|_| "Invalid clipboard history export JSON.".to_owned())?;
            strings.insert(decoded);
            continue;
        }
        if raw[index] == b'-' || raw[index].is_ascii_digit() {
            let start = index;
            while index < raw.len()
                && !matches!(
                    raw[index],
                    b',' | b']' | b'}' | b':' | b' ' | b'\n' | b'\r' | b'\t'
                )
            {
                index += 1;
            }
            let token = std::str::from_utf8(&raw[start..index])
                .map_err(|_| "Invalid clipboard history export JSON.".to_owned())?;
            if valid_json_number(token) {
                let is_float = token.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E'));
                if is_float {
                    preserved_numbers.push(NumericToken {
                        range: start..index,
                        raw: token.to_owned(),
                        kind: 'F',
                    });
                } else {
                    let digits = token.strip_prefix('-').unwrap_or(token);
                    if digits.len() > 4300 {
                        return Err("Clipboard integer exceeds Python's JSON digit limit.".into());
                    }
                    let exact = if token.starts_with('-') {
                        token.parse::<i64>().is_ok()
                    } else {
                        token.parse::<u64>().is_ok()
                    };
                    if token == "-0" {
                        preserved_numbers.push(NumericToken {
                            range: start..index,
                            raw: "0".into(),
                            kind: 'I',
                        });
                    } else if !exact {
                        preserved_numbers.push(NumericToken {
                            range: start..index,
                            raw: token.to_owned(),
                            kind: 'I',
                        });
                    }
                }
            }
            continue;
        }
        index += 1;
    }

    let numeric_marker = if preserved_numbers.is_empty() {
        None
    } else {
        let mut suffix = 0_u64;
        loop {
            let candidate = format!("$ruycast-private-integer-{suffix}");
            if !strings.contains(&candidate) {
                break Some(candidate);
            }
            suffix = suffix
                .checked_add(1)
                .ok_or_else(|| "Unable to select private integer marker.".to_owned())?;
        }
    };
    let transformed = if let Some(marker) = &numeric_marker {
        let marker_json = serde_json::to_vec(marker).map_err(|error| error.to_string())?;
        let mut output = Vec::with_capacity(raw.len() + preserved_numbers.len() * 64);
        let mut copied = 0;
        for token in &preserved_numbers {
            output.extend_from_slice(&raw[copied..token.range.start]);
            output.extend_from_slice(b"{");
            output.extend_from_slice(&marker_json);
            output.extend_from_slice(b":");
            output.extend_from_slice(
                &serde_json::to_vec(&format!("{}{value}", token.kind, value = token.raw))
                    .map_err(|error| error.to_string())?,
            );
            output.extend_from_slice(b"}");
            copied = token.range.end;
        }
        output.extend_from_slice(&raw[copied..]);
        output
    } else {
        raw.to_vec()
    };
    let data = serde_json::from_slice(&transformed)
        .map_err(|_| "Invalid clipboard history export JSON.".to_owned())?;
    Ok(ParsedExport {
        data,
        numeric_marker,
    })
}

fn marker_number<'a>(value: &'a Value, marker: Option<&str>) -> Option<(char, &'a str)> {
    let marker = marker?;
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let raw = object.get(marker)?.as_str()?;
    let (kind, value) = raw.split_at(1);
    Some((kind.chars().next()?, value))
}
fn python_float_literal(value: f64) -> Result<String, String> {
    if !value.is_finite() {
        return Err("Non-finite clipboard metadata is not supported.".into());
    }
    let negative = value.is_sign_negative();
    if value == 0.0 {
        return Ok(if negative { "-0.0" } else { "0.0" }.into());
    }
    let raw = serde_json::Number::from_f64(value)
        .ok_or_else(|| "Non-finite clipboard metadata is not supported.".to_owned())?
        .to_string();
    let unsigned = raw.strip_prefix('-').unwrap_or(&raw);
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (
            mantissa,
            exponent
                .parse::<i32>()
                .map_err(|_| "Float exponent is out of range.".to_owned())?,
        ),
        None => (unsigned, 0_i32),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut digits = format!("{whole}{fraction}");
    let leading_zeros = digits.bytes().take_while(|byte| *byte == b'0').count();
    digits.drain(..leading_zeros);
    if digits.is_empty() {
        return Ok(if negative { "-0.0" } else { "0.0" }.into());
    }
    let decimal_position = i32::try_from(whole.len())
        .ok()
        .and_then(|position| position.checked_add(exponent))
        .and_then(|position| {
            i32::try_from(leading_zeros)
                .ok()
                .and_then(|leading| position.checked_sub(leading))
        })
        .ok_or_else(|| "Float exponent is out of range.".to_owned())?;
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let scientific_exponent = decimal_position - 1;
    let formatted = if !(-4..16).contains(&scientific_exponent) {
        let significand = if digits.len() == 1 {
            digits.clone()
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        format!("{significand}e{scientific_exponent:+03}")
    } else if decimal_position <= 0 {
        format!(
            "0.{}{}",
            "0".repeat(decimal_position.unsigned_abs() as usize),
            digits
        )
    } else if decimal_position as usize >= digits.len() {
        format!(
            "{}{}.0",
            digits,
            "0".repeat(decimal_position as usize - digits.len())
        )
    } else {
        let split = decimal_position as usize;
        format!("{}.{}", &digits[..split], &digits[split..])
    };
    Ok(if negative {
        format!("-{formatted}")
    } else {
        formatted
    })
}

fn append_python_canonical_json(
    value: &Value,
    output: &mut String,
    numeric_marker: Option<&str>,
) -> Result<(), String> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(number) => {
            let raw = number.to_string();
            if raw.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
                let value = number
                    .as_f64()
                    .ok_or_else(|| "Invalid floating clipboard metadata.".to_owned())?;
                output.push_str(&python_float_literal(value)?);
            } else if raw == "-0" {
                // Python's JSON decoder treats the integer token -0 as integer zero.
                output.push('0');
            } else {
                output.push_str(&raw);
            }
        }
        Value::String(value) => {
            output.push_str(&serde_json::to_string(value).map_err(|error| error.to_string())?);
        }
        Value::Array(values) => {
            output.push('[');
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                append_python_canonical_json(item, output, numeric_marker)?;
            }
            output.push(']');
        }
        Value::Object(values) => {
            if let Some((kind, raw)) = marker_number(value, numeric_marker) {
                match kind {
                    'I' => output.push_str(raw),
                    'F' => {
                        let value = raw
                            .parse::<f64>()
                            .map_err(|_| "Invalid floating clipboard metadata.".to_owned())?;
                        output.push_str(&python_float_literal(value)?);
                    }
                    _ => return Err("Invalid private clipboard number marker.".into()),
                }
                return Ok(());
            }
            output.push('{');
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).map_err(|error| error.to_string())?);
                output.push(':');
                append_python_canonical_json(item, output, numeric_marker)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

fn python_canonical_json(value: &Value, numeric_marker: Option<&str>) -> Result<String, String> {
    let mut output = String::new();
    append_python_canonical_json(value, &mut output, numeric_marker)?;
    Ok(output)
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
        8 if bytes.iter().all(u8::is_ascii_digit) => NaiveDate::parse_from_str(raw, "%Y%m%d").ok(),
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
        || components
            .iter()
            .any(|part| part.len() != 2 || !part.bytes().all(|byte| byte.is_ascii_digit()))
        || (fraction.is_some() && components.len() != 3)
    {
        return None;
    }
    let number = |index: usize| -> Option<u32> {
        components
            .get(index)
            .map_or(Some(0), |part| part.parse().ok())
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
        return (minute == 0 && second == 0 && microsecond == 0).then_some((NaiveTime::MIN, true));
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
    let (clock, fraction) = value
        .split_once(['.', ','])
        .map_or((value, None), |(a, b)| (a, Some(b)));
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
    normalize_with_metadata_and_marker(data, max_text_bytes, asset, None)
}
#[cfg(test)]
fn normalize_with_metadata(
    data: &Value,
    max_text_bytes: i64,
    metadata: impl FnMut(&str) -> Asset,
) -> Result<Normalized, String> {
    normalize_with_metadata_and_marker(data, max_text_bytes, metadata, None)
}
pub fn normalize_parsed(parsed: &ParsedExport, max_text_bytes: i64) -> Result<Normalized, String> {
    normalize_with_metadata_and_marker(
        &parsed.data,
        max_text_bytes,
        asset,
        parsed.numeric_marker.as_deref(),
    )
}
fn normalize_with_metadata_and_marker(
    data: &Value,
    max_text_bytes: i64,
    mut metadata: impl FnMut(&str) -> Asset,
    numeric_marker: Option<&str>,
) -> Result<Normalized, String> {
    let empty = json!({});
    let package = data
        .get("builtin_package_clipboardHistory")
        .unwrap_or(&empty);
    if marker_number(data, numeric_marker).is_some()
        || marker_number(package, numeric_marker).is_some()
        || !data.is_object()
        || !package.is_object()
    {
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
        if marker_number(row, numeric_marker).is_some() || !row.is_object() {
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
        let canonical = python_canonical_json(row, numeric_marker)?.into_bytes();
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
    fn batch_retry_identity_matches_python_unicode_serialization() {
        let items = [json!({"id":"1","text":"Å😀"})];
        let requests = batches(&items, &"0".repeat(64)).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]["batchID"],
            "2d7db1700f5b9c741037d75b5e603dff50f8b2cbe47e7d7160cb62504bacd416"
        );
        assert_eq!(python_json(&requests[0], false).unwrap().len(), 243);
    }
    #[test]
    fn batch_retry_identity_matches_python_float_exponents() {
        let items = [json!({"id":"1","value":1e-7})];
        let requests = batches(&items, &"0".repeat(64)).unwrap();
        assert_eq!(
            requests[0]["batchID"],
            "5b78f01a099213c1b8c6a32f454b044ac1fe0cdd8260373f4d9936f98410244e"
        );
        assert_eq!(python_json(&requests[0], false).unwrap().len(), 229);
    }
    #[test]
    fn batches_are_bounded_ordered_and_retry_stable() {
        let items: Vec<_> = (0..151)
            .map(|index| json!({"id":index.to_string(),"text":"a".repeat(20_000)}))
            .collect();
        let source_hash = "0".repeat(64);
        let requests = batches(&items, &source_hash).unwrap();
        assert!(requests.len() > 2);
        assert_eq!(requests, batches(&items, &source_hash).unwrap());
        assert!(requests.iter().all(|request| {
            request["items"].as_array().unwrap().len() <= CLIPBOARD_BATCH_ITEMS
                && python_json(request, false).unwrap().len() < CLIPBOARD_BATCH_MAX_WIRE
        }));
        let batched: Vec<_> = requests
            .iter()
            .flat_map(|request| request["items"].as_array().unwrap())
            .collect();
        assert_eq!(batched.len(), items.len());
        assert!(
            batched
                .iter()
                .zip(&items)
                .all(|(actual, expected)| *actual == expected)
        );

        let count_limited = vec![json!({"id":"stable","text":"x"}); 101];
        let count_batches = batches(&count_limited, &source_hash).unwrap();
        assert_eq!(
            count_batches
                .iter()
                .map(|request| request["items"].as_array().unwrap().len())
                .collect::<Vec<_>>(),
            [100, 1]
        );
    }
    #[test]
    fn batch_wire_size_boundary_rejects_exact_limit_and_single_oversize() {
        let source_hash = "0".repeat(64);
        let last_accepted = json!({"id":"1","text":"a".repeat(999_774)});
        let requests = batches(&[last_accepted], &source_hash).unwrap();
        assert_eq!(
            python_json(&requests[0], false).unwrap().len(),
            CLIPBOARD_BATCH_MAX_WIRE - 1
        );

        let exact_limit = json!({"id":"1","text":"a".repeat(999_775)});
        assert_eq!(
            batches(&[exact_limit], &source_hash).unwrap_err(),
            "A clipboard item exceeds the import transport limit."
        );
        let split = [
            json!({"id":"1","text":"a".repeat(600_000)}),
            json!({"id":"2","text":"b".repeat(400_000)}),
        ];
        assert_eq!(batches(&split, &source_hash).unwrap().len(), 2);
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
    fn python_float_metadata_has_stable_canonical_json_and_identity() {
        for (literal, expected) in [
            ("1.0", "1.0"),
            ("-0.0", "-0.0"),
            ("1e-7", "1e-07"),
            ("1e-6", "1e-06"),
            ("1e-5", "1e-05"),
            ("1e-4", "0.0001"),
            ("1e15", "1000000000000000.0"),
            ("1e16", "1e+16"),
            ("1e20", "1e+20"),
            ("1.2345678901234567", "1.2345678901234567"),
            ("1.23456789012345678", "1.2345678901234567"),
            ("1000000000000000100.0", "1.0000000000000001e+18"),
            ("-0", "0"),
        ] {
            let parsed = parse_export(literal.as_bytes()).unwrap();
            assert_eq!(
                python_canonical_json(&parsed.data, parsed.numeric_marker.as_deref()).unwrap(),
                expected,
                "{literal}"
            );
        }
        let nested = parse_export(r#"{"z":[-0.0,1e-7],"a":{"Å":"é","b":1.0}}"#.as_bytes()).unwrap();
        assert_eq!(
            python_canonical_json(&nested.data, nested.numeric_marker.as_deref()).unwrap(),
            r#"{"a":{"b":1.0,"Å":"é"},"z":[-0.0,1e-07]}"#
        );
        let mut value = row();
        value["extra"] = serde_json::from_str("1e-7").unwrap();
        let result = normalize(&data(vec![value]), 100_000).unwrap();
        assert_eq!(
            result.items[0]["id"],
            "raycast-828da4495e1c0fd575abc13ba7cceff1bcd4288ad1b1e4c00fbc07475d98c5ef"
        );
        let non_finite = parse_export(b"1e400").unwrap();
        assert!(
            python_canonical_json(&non_finite.data, non_finite.numeric_marker.as_deref())
                .unwrap_err()
                .contains("Non-finite")
        );
        assert!(serde_json::Number::from_f64(f64::INFINITY).is_none());
    }
    #[test]
    fn python_integer_tokens_above_u64_keep_identity_with_the_python_digit_limit() {
        let large_integer = "1844674407370955161600000000000000000";
        let input = format!(
            "{{\"builtin_package_clipboardHistory\":{{\"clipboardHistoryRecords\":[{{\"category\":\"text\",\"text\":\"Synthetic content\",\"applicationPath\":\"/Applications/Fixture.app\",\"createdAt\":\"2026-10-25T00:00:00Z\",\"extra\":{large_integer}}}]}}}}"
        );
        let parsed = parse_export(input.as_bytes()).unwrap();
        let result = normalize_parsed(&parsed, 100_000).unwrap();
        assert_eq!(
            result.items[0]["id"],
            "raycast-9e01c5f5b815fec301a7c0d38d1245736a021ef0ec7ec820d7d1468d77f242bf"
        );

        let digit_boundary = "9".repeat(4300);
        let accepted = parse_export(format!("{{\"n\":{digit_boundary}}}").as_bytes()).unwrap();
        let canonical =
            python_canonical_json(&accepted.data, accepted.numeric_marker.as_deref()).unwrap();
        assert_eq!(canonical, format!("{{\"n\":{digit_boundary}}}"));
        let too_many = "9".repeat(4301);
        assert!(parse_export(format!("{{\"n\":{too_many}}}").as_bytes()).is_err());

        let private_key: ParsedExport =
            parse_export(br#"{"$serde_json::private::Number":"1","kept":true}"#).unwrap();
        assert_eq!(private_key.data["$serde_json::private::Number"], "1");
        assert_eq!(private_key.data["kept"], true);
    }
    #[test]
    fn preserved_number_tokens_keep_the_original_container_types() {
        let root = parse_export(b"1.0").unwrap();
        assert!(normalize_parsed(&root, 100_000)
            .err()
            .unwrap()
            .contains("Invalid clipboard history export structure"));

        let package = parse_export(br#"{"builtin_package_clipboardHistory":1.0}"#).unwrap();
        assert!(normalize_parsed(&package, 100_000)
            .err()
            .unwrap()
            .contains("Invalid clipboard history export structure"));

        let row = parse_export(
            br#"{"builtin_package_clipboardHistory":{"clipboardHistoryRecords":[1.0]}}"#,
        )
        .unwrap();
        let result = normalize_parsed(&row, 100_000).unwrap();
        assert_eq!(result.counts["invalid"], 1);
        assert_eq!(result.counts["unsupported"], 0);
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
            ("2026-10-25T00:00:00.283+00:00:00.5", "2026-10-25T00:00:00Z"),
            ("2026-10-25T00:00:00.283+00:00:01.5", "2026-10-24T23:59:58Z"),
        ];
        for (input, expected) in accepted {
            let mut item = row();
            item["createdAt"] = json!(input);
            let result = normalize(&data(vec![item]), 100_000).unwrap();
            assert_eq!(result.counts["invalid"], 0, "{input}");
            assert_eq!(result.items[0]["date"], expected, "{input}");
        }
        for input in [
            "2011-11-04T00:05:23",       // Python accepts it but it has no zone.
            "2011-11-04T00:05:23z",      // The source only substitutes uppercase Z.
            "2026-10-25T00.5:00+00:00",  // Fractional hours are rejected.
            "2026-10-25T00:00:00+24:00", // datetime.timezone rejects 24 hours.
            "2026-1-25T00:00:00+00:00",  // Calendar components have fixed widths.
            "2026-10-25T0:00:00+00:00",  // Time components have fixed widths.
            "2026-10-25T24:00:01+00:00", // Hour 24 only allows an exact midnight.
            "9999-12-31T24:00:00+00:00", // The local year cannot roll past 9999.
            "0000-12-31T23:00:00-02:00", // UTC cannot rescue an invalid local year.
            "2026-10-25Z00:00:00+00:00", // The Python caller replaces every uppercase Z.
            " 026-W43T00:00:00+00:00",   // Chrono must not accept padded ISO years.
            "2026-10-25T🕒+00:00",       // Malformed Unicode must count invalid, never panic.
            "2026-10-25T00:00:00+🕒",    // Malformed Unicode offset must count invalid.
        ] {
            let mut item = row();
            item["createdAt"] = json!(input);
            let result = normalize(&data(vec![item]), 100_000).unwrap();
            assert_eq!(result.counts["invalid"], 1, "{input}");
            assert!(result.items.is_empty(), "{input}");
        }
    }
}
